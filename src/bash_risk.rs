use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BashRiskLevel {
    Low,
    Medium,
    High,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashRiskSignal {
    pub kind: String,
    pub value: String,
    pub severity: BashRiskLevel,
}

impl BashRiskSignal {
    fn new(kind: &str, value: impl Into<String>, severity: BashRiskLevel) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
            severity,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BashRiskRequest {
    pub command: String,
    pub cwd: PathBuf,
    pub allowed_roots: Vec<PathBuf>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct BashRiskDecision {
    pub risk: BashRiskLevel,
    pub confidence: f32,
    pub reason: String,
    pub signals: Vec<BashRiskSignal>,
    pub policy_candidate: Option<PolicyCandidate>,
    pub normalized: NormalizedCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyCandidate {
    ExactNormalizedCommand { command: String },
    ArgvExact { program: String, args: Vec<String> },
    ArgvPrefix { program: String, args: Vec<String> },
    ToolFamily { program: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCommand {
    pub original: String,
    pub normalized: String,
    pub commands: Vec<ParsedCommand>,
    pub has_pipe: bool,
    pub has_control_operator: bool,
    pub has_redirection: bool,
    pub has_complex_syntax: bool,
}

impl NormalizedCommand {
    pub fn parse(command: &str) -> Self {
        let normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");
        let has_pipe = command.contains('|');
        let has_control_operator =
            command.contains("&&") || command.contains("||") || command.contains(';');
        let has_redirection = command.contains('>') || command.contains('<');
        let has_complex_syntax = command.contains("$(")
            || command.contains('`')
            || command.contains("<(")
            || command.contains("<<");

        let segments = split_command_segments(command);
        let commands = segments
            .into_iter()
            .filter_map(|segment| {
                let tokens = shell_words(&segment);
                let (program, args) = tokens.split_first()?;
                Some(ParsedCommand {
                    program: program.to_string(),
                    args: args.to_vec(),
                })
            })
            .collect();

        Self {
            original: command.into(),
            normalized,
            commands,
            has_pipe,
            has_control_operator,
            has_redirection,
            has_complex_syntax,
        }
    }
}

#[derive(Debug, Default)]
pub struct BashRiskAssessor;

impl BashRiskAssessor {
    pub fn new() -> Self {
        Self
    }

    pub fn classify(&self, request: BashRiskRequest) -> BashRiskDecision {
        let normalized = NormalizedCommand::parse(&request.command);
        LocalRuleClassifier::classify(normalized)
    }
}

struct LocalRuleClassifier;

impl LocalRuleClassifier {
    fn classify(normalized: NormalizedCommand) -> BashRiskDecision {
        let mut signals = Vec::new();

        if normalized.original.contains('\0') {
            signals.push(BashRiskSignal::new(
                "invalid_input",
                "NUL byte",
                BashRiskLevel::Blocked,
            ));
            return decision(
                BashRiskLevel::Blocked,
                "Command contains a NUL byte.",
                signals,
                None,
                normalized,
            );
        }

        if looks_like_fork_bomb(&normalized.original) {
            signals.push(BashRiskSignal::new(
                "resource_exhaustion",
                "fork bomb",
                BashRiskLevel::Blocked,
            ));
            return decision(
                BashRiskLevel::Blocked,
                "Command resembles a fork bomb.",
                signals,
                None,
                normalized,
            );
        }

        if deletes_protected_path(&normalized) {
            signals.push(BashRiskSignal::new(
                "protected_path_delete",
                "system or home root",
                BashRiskLevel::Blocked,
            ));
            return decision(
                BashRiskLevel::Blocked,
                "Command attempts to delete a protected path.",
                signals,
                None,
                normalized,
            );
        }

        if remote_script_pipe(&normalized) {
            signals.push(BashRiskSignal::new(
                "remote_script_execution",
                "curl/wget piped to shell",
                BashRiskLevel::High,
            ));
        }

        for command in &normalized.commands {
            if is_dangerous_program(&command.program) {
                signals.push(BashRiskSignal::new(
                    "dangerous_program",
                    command.program.clone(),
                    BashRiskLevel::High,
                ));
            }

            if is_high_risk_git(command) {
                signals.push(BashRiskSignal::new(
                    "destructive_git",
                    format!("git {}", command.args.join(" ")),
                    BashRiskLevel::High,
                ));
            }

            if touches_sensitive_path(command) {
                signals.push(BashRiskSignal::new(
                    "sensitive_path",
                    command.args.join(" "),
                    BashRiskLevel::High,
                ));
            }
        }

        if signals
            .iter()
            .any(|signal| signal.severity == BashRiskLevel::High)
        {
            return decision(
                BashRiskLevel::High,
                "Command matches a high-risk local rule.",
                signals,
                None,
                normalized,
            );
        }

        if normalized.has_complex_syntax {
            signals.push(BashRiskSignal::new(
                "complex_shell_syntax",
                "command substitution or heredoc",
                BashRiskLevel::Medium,
            ));
        }

        if normalized.has_control_operator {
            signals.push(BashRiskSignal::new(
                "control_operator",
                "&&, ||, or ;",
                BashRiskLevel::Medium,
            ));
        }

        if normalized.has_redirection {
            signals.push(BashRiskSignal::new(
                "redirection",
                "shell redirection",
                BashRiskLevel::Medium,
            ));
        }

        if signals
            .iter()
            .any(|signal| signal.severity == BashRiskLevel::Medium)
        {
            let policy_candidate = Some(PolicyCandidate::ExactNormalizedCommand {
                command: normalized.normalized.clone(),
            });
            return decision(
                BashRiskLevel::Medium,
                "Command uses shell features that require confirmation.",
                signals,
                policy_candidate,
                normalized,
            );
        }

        if normalized.commands.len() == 1 {
            let command = &normalized.commands[0];
            if let Some(candidate) = low_risk_candidate(command) {
                signals.push(BashRiskSignal::new(
                    "low_risk_program",
                    command.program.clone(),
                    BashRiskLevel::Low,
                ));
                return decision(
                    BashRiskLevel::Low,
                    "Command matches a read-only or check/test local rule.",
                    signals,
                    Some(candidate),
                    normalized,
                );
            }
        }

        signals.push(BashRiskSignal::new(
            "unknown_command",
            normalized.normalized.clone(),
            BashRiskLevel::Medium,
        ));
        decision(
            BashRiskLevel::Medium,
            "Command is not covered by local low-risk rules.",
            signals,
            Some(PolicyCandidate::ExactNormalizedCommand {
                command: normalized.normalized.clone(),
            }),
            normalized,
        )
    }
}

fn decision(
    risk: BashRiskLevel,
    reason: &str,
    signals: Vec<BashRiskSignal>,
    policy_candidate: Option<PolicyCandidate>,
    normalized: NormalizedCommand,
) -> BashRiskDecision {
    BashRiskDecision {
        risk,
        confidence: match risk {
            BashRiskLevel::Low | BashRiskLevel::Blocked => 1.0,
            BashRiskLevel::High => 0.95,
            BashRiskLevel::Medium => 0.7,
        },
        reason: reason.into(),
        signals,
        policy_candidate,
        normalized,
    }
}

fn split_command_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
            current.push(ch);
            continue;
        }

        if quote.is_none() && (ch == '|' || ch == ';') {
            if !current.trim().is_empty() {
                segments.push(current.trim().to_string());
            }
            current.clear();
            if ch == '|' && chars.peek() == Some(&'|') {
                chars.next();
            }
            continue;
        }

        if quote.is_none() && ch == '&' && chars.peek() == Some(&'&') {
            chars.next();
            if !current.trim().is_empty() {
                segments.push(current.trim().to_string());
            }
            current.clear();
            continue;
        }

        current.push(ch);
    }

    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }
    segments
}

fn shell_words(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = segment.chars().peekable();

    while let Some(ch) = chars.next() {
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            } else {
                current.push(ch);
            }
            continue;
        }

        if quote.is_none() && ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }

        if quote.is_none() && matches!(ch, '>' | '<') {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if chars.peek() == Some(&ch) {
                chars.next();
            }
            continue;
        }

        current.push(ch);
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_dangerous_program(program: &str) -> bool {
    matches!(
        basename(program),
        "rm" | "rmdir" | "truncate" | "dd" | "chmod" | "chown" | "chgrp" | "sudo" | "su"
    )
}

fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

fn is_high_risk_git(command: &ParsedCommand) -> bool {
    if basename(&command.program) != "git" {
        return false;
    }
    let args = &command.args;
    matches!(
        args.as_slice(),
        [cmd, flag, ..] if cmd == "reset" && flag == "--hard"
    ) || matches!(args.as_slice(), [cmd, ..] if cmd == "clean")
        || matches!(args.as_slice(), [cmd, flag, ..] if cmd == "checkout" && flag == "--")
        || matches!(args.as_slice(), [cmd, flag, ..] if cmd == "restore" && flag.starts_with("--source"))
}

fn deletes_protected_path(normalized: &NormalizedCommand) -> bool {
    normalized.commands.iter().any(|command| {
        basename(&command.program) == "rm"
            && command
                .args
                .iter()
                .any(|arg| arg.contains('r') || arg.contains('f'))
            && command.args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "/" | "~" | "$HOME" | "/System" | "/bin" | "/usr" | "/etc"
                )
            })
    })
}

fn remote_script_pipe(normalized: &NormalizedCommand) -> bool {
    if !normalized.has_pipe {
        return false;
    }
    let has_fetch = normalized
        .commands
        .iter()
        .any(|command| matches!(basename(&command.program), "curl" | "wget"));
    let has_shell = normalized
        .commands
        .iter()
        .any(|command| matches!(basename(&command.program), "sh" | "bash" | "zsh"));
    has_fetch && has_shell
}

fn looks_like_fork_bomb(command: &str) -> bool {
    let compact = command.replace(char::is_whitespace, "");
    compact.contains(":(){:|:&};:") || compact.contains(":(){:|:&};")
}

fn touches_sensitive_path(command: &ParsedCommand) -> bool {
    command.args.iter().any(|arg| {
        arg.contains(".ssh/config")
            || arg.contains(".gitconfig")
            || arg.contains(".bashrc")
            || arg.contains(".zshrc")
            || arg.starts_with("/etc/")
            || arg.starts_with("/System/")
    })
}

fn low_risk_candidate(command: &ParsedCommand) -> Option<PolicyCandidate> {
    let program = basename(&command.program);
    let args = &command.args;

    if matches!(
        program,
        "pwd" | "ls" | "cat" | "head" | "tail" | "rg" | "grep" | "wc"
    ) {
        return Some(PolicyCandidate::ToolFamily {
            program: program.into(),
        });
    }

    if program == "find"
        && !args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-delete" | "-exec"))
    {
        return Some(PolicyCandidate::ToolFamily {
            program: program.into(),
        });
    }

    if program == "sed" && args.first().is_some_and(|arg| arg == "-n") {
        return Some(PolicyCandidate::ArgvPrefix {
            program: program.into(),
            args: vec!["-n".into()],
        });
    }

    if program == "git" {
        match args.as_slice() {
            [cmd, ..] if matches!(cmd.as_str(), "status" | "diff" | "log" | "show") => {
                return Some(PolicyCandidate::ArgvPrefix {
                    program: program.into(),
                    args: vec![cmd.clone()],
                });
            }
            [cmd, flag] if cmd == "branch" && flag == "--show-current" => {
                return Some(PolicyCandidate::ArgvExact {
                    program: program.into(),
                    args: args.clone(),
                });
            }
            _ => {}
        }
    }

    if program == "cargo"
        && args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "metadata" | "check" | "test"))
    {
        return Some(PolicyCandidate::ArgvPrefix {
            program: program.into(),
            args: vec![args[0].clone()],
        });
    }

    if matches!(program, "npm" | "pnpm")
        && args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "test" | "lint"))
    {
        return Some(PolicyCandidate::ArgvPrefix {
            program: program.into(),
            args: vec![args[0].clone()],
        });
    }

    None
}
