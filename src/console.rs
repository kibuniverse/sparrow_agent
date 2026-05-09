use std::io::{self, IsTerminal, Write};

use crate::config::StreamingConfig;
use crate::streaming::{AgentEventSink, AgentStreamEvent};

#[cfg(unix)]
use std::{process::Command, process::Stdio};

pub fn read_user_input(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    loop {
        let inline_footer = print_input_prompt(prompt, footer)?;

        let mut input = String::new();
        let bytes_read = io::stdin().read_line(&mut input)?;

        if inline_footer {
            println!();
        } else if let Some(footer) = footer.filter(|footer| !footer.is_empty()) {
            println!("{footer}");
        }

        if bytes_read == 0 {
            return Ok(None);
        }

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        return Ok(Some(input));
    }
}

fn print_input_prompt(prompt: &str, footer: Option<&str>) -> io::Result<bool> {
    let footer = footer.filter(|footer| !footer.is_empty());
    let inline_footer = footer.is_some() && supports_inline_footer();

    if inline_footer {
        let footer = footer.expect("footer was checked above");
        print!("{prompt}\n{footer}\x1b[1A\r");
        let prompt_width = prompt.chars().count();
        if prompt_width > 0 {
            print!("\x1b[{prompt_width}C");
        }
    } else {
        print!("{prompt}");
    }

    io::stdout().flush()?;
    Ok(inline_footer)
}

fn supports_inline_footer() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub fn is_exit_command(input: &str) -> bool {
    input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit")
}

pub fn read_secret_input(prompt: &str) -> io::Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush()?;

    let echo_disabled = set_stdin_echo(false).unwrap_or(false);

    let mut input = String::new();
    let read_result = io::stdin().read_line(&mut input);

    if echo_disabled {
        let _ = set_stdin_echo(true);
        println!();
    }

    if read_result? == 0 {
        return Ok(None);
    }

    Ok(Some(input.trim().to_string()))
}

#[cfg(unix)]
fn set_stdin_echo(enabled: bool) -> io::Result<bool> {
    let arg = if enabled { "echo" } else { "-echo" };
    let status = Command::new("stty")
        .arg(arg)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    Ok(status.success())
}

#[cfg(not(unix))]
fn set_stdin_echo(_enabled: bool) -> io::Result<bool> {
    Ok(false)
}

// ── Console trace renderer ────────────────────────────────────────────

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub struct ConsoleTraceRenderer {
    config: StreamingConfig,
    is_tty: bool,
    printed_thinking_header: bool,
    printed_answer_header: bool,
}

impl ConsoleTraceRenderer {
    pub fn new(config: &StreamingConfig) -> Self {
        Self {
            config: config.clone(),
            is_tty: io::stdout().is_terminal(),
            printed_thinking_header: false,
            printed_answer_header: false,
        }
    }
}

impl AgentEventSink for ConsoleTraceRenderer {
    fn on_event(&mut self, event: &AgentStreamEvent) -> anyhow::Result<()> {
        match event {
            AgentStreamEvent::ResponseStarted { round: _ } => {
                self.printed_thinking_header = false;
                self.printed_answer_header = false;
            }

            AgentStreamEvent::ReasoningStarted => {
                if self.config.show_reasoning && !self.printed_thinking_header {
                    if self.is_tty {
                        print!("{DIM}thinking>\n{RESET}");
                    } else {
                        print!("thinking>\n");
                    }
                    io::stdout().flush()?;
                    self.printed_thinking_header = true;
                }
            }

            AgentStreamEvent::ReasoningDelta(text) => {
                if self.config.show_reasoning {
                    if self.is_tty {
                        print!("{DIM}{text}{RESET}");
                    } else {
                        print!("{text}");
                    }
                    io::stdout().flush()?;
                }
            }

            AgentStreamEvent::AnswerStarted => {
                if self.printed_thinking_header {
                    println!();
                }
                if !self.printed_answer_header {
                    print!("agent> ");
                    io::stdout().flush()?;
                    self.printed_answer_header = true;
                }
            }

            AgentStreamEvent::AnswerDelta(text) => {
                print!("{text}");
                io::stdout().flush()?;
            }

            AgentStreamEvent::ToolCallDelta {
                index,
                name,
                arguments_delta,
                ..
            } => {
                if self.config.show_tool_call_deltas {
                    let name = name.as_deref().unwrap_or("unknown");
                    if let Some(args) = arguments_delta {
                        print!("tool[{index}] {name} {args}");
                    } else {
                        print!("tool[{index}] {name}");
                    }
                    io::stdout().flush()?;
                }
            }

            AgentStreamEvent::ResponseFinished { finish_reason: _ } => {
                if self.printed_answer_header {
                    println!();
                }
            }

            AgentStreamEvent::Usage(_) => {}
        }

        Ok(())
    }
}
