use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::bash_risk::{BashRiskLevel, NormalizedCommand, PolicyCandidate};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BashApprovalPolicyMatcher {
    ExactNormalizedCommand { command: String },
    ArgvExact { program: String, args: Vec<String> },
    ArgvPrefix { program: String, args: Vec<String> },
    ToolFamily { program: String },
}

impl BashApprovalPolicyMatcher {
    pub fn from_candidate(candidate: &PolicyCandidate) -> Self {
        match candidate {
            PolicyCandidate::ExactNormalizedCommand { command } => Self::ExactNormalizedCommand {
                command: command.clone(),
            },
            PolicyCandidate::ArgvExact { program, args } => Self::ArgvExact {
                program: program.clone(),
                args: args.clone(),
            },
            PolicyCandidate::ArgvPrefix { program, args } => Self::ArgvPrefix {
                program: program.clone(),
                args: args.clone(),
            },
            PolicyCandidate::ToolFamily { program } => Self::ToolFamily {
                program: program.clone(),
            },
        }
    }

    fn matches(&self, command: &NormalizedCommand) -> bool {
        match self {
            Self::ExactNormalizedCommand { command: expected } => &command.normalized == expected,
            Self::ArgvExact { program, args } => {
                is_simple_argv_shape(command)
                    && command.commands[0].program == *program
                    && command.commands[0].args == *args
            }
            Self::ArgvPrefix { program, args } => {
                is_simple_argv_shape(command)
                    && command.commands[0].program == *program
                    && command.commands[0].args.starts_with(args)
            }
            Self::ToolFamily { program } => {
                is_simple_argv_shape(command) && command.commands[0].program == *program
            }
        }
    }
}

fn is_simple_argv_shape(command: &NormalizedCommand) -> bool {
    command.commands.len() == 1
        && !command.has_pipe
        && !command.has_control_operator
        && !command.has_redirection
        && !command.has_complex_syntax
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashApprovalPolicy {
    pub id: String,
    pub matcher: BashApprovalPolicyMatcher,
    pub cwd_scope: PathBuf,
    pub risk: BashRiskLevel,
    pub source: String,
    pub confidence: f32,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub hit_count: u64,
    pub last_hit_at: Option<DateTime<Utc>>,
}

impl BashApprovalPolicy {
    pub fn new(
        matcher: BashApprovalPolicyMatcher,
        cwd_scope: PathBuf,
        risk: BashRiskLevel,
        source: String,
        confidence: f32,
        reason: String,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: format!("policy_{}", Ulid::new()),
            matcher,
            cwd_scope,
            risk,
            source,
            confidence,
            reason,
            created_at: Utc::now(),
            expires_at,
            hit_count: 0,
            last_hit_at: None,
        }
    }

    fn is_active_for(&self, cwd: &Path) -> bool {
        self.risk == BashRiskLevel::Low
            && self.expires_at > Utc::now()
            && cwd.starts_with(&self.cwd_scope)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PolicyDocument {
    schema_version: u32,
    policies: Vec<BashApprovalPolicy>,
}

impl Default for PolicyDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            policies: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct BashApprovalPolicyStore {
    path: PathBuf,
    ttl_days: u64,
    document: PolicyDocument,
}

impl BashApprovalPolicyStore {
    pub fn load_or_create(path: PathBuf, ttl_days: u64) -> Result<Self> {
        let document = if path.exists() {
            match fs::read_to_string(&path)
                .ok()
                .and_then(|contents| serde_json::from_str(&contents).ok())
            {
                Some(document) => document,
                None => {
                    let corrupt = path.with_extension(format!(
                        "json.corrupt.{}",
                        Utc::now().format("%Y%m%d%H%M%S")
                    ));
                    let _ = fs::rename(&path, corrupt);
                    PolicyDocument::default()
                }
            }
        } else {
            PolicyDocument::default()
        };

        let store = Self {
            path,
            ttl_days,
            document,
        };
        store.save()?;
        Ok(store)
    }

    pub fn policies(&self) -> &[BashApprovalPolicy] {
        &self.document.policies
    }

    pub fn default_expiry(&self) -> DateTime<Utc> {
        Utc::now() + chrono::Duration::days(self.ttl_days as i64)
    }

    pub fn add_policy(&mut self, policy: BashApprovalPolicy) -> Result<String> {
        let id = policy.id.clone();
        self.document.policies.push(policy);
        self.save()?;
        Ok(id)
    }

    pub fn find_matching_low_risk_policy(
        &mut self,
        command: &str,
        cwd: &Path,
    ) -> Result<Option<BashApprovalPolicy>> {
        let normalized = NormalizedCommand::parse(command);
        let now = Utc::now();
        let Some(policy) = self
            .document
            .policies
            .iter_mut()
            .find(|policy| policy.is_active_for(cwd) && policy.matcher.matches(&normalized))
        else {
            return Ok(None);
        };

        policy.hit_count = policy.hit_count.saturating_add(1);
        policy.last_hit_at = Some(now);
        let found = policy.clone();
        self.save()?;
        Ok(Some(found))
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let contents =
            serde_json::to_string_pretty(&self.document).context("failed to serialize policies")?;

        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)
                .with_context(|| format!("failed to open {}", self.path.display()))?;
            file.write_all(contents.as_bytes())
                .with_context(|| format!("failed to write {}", self.path.display()))?;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to chmod {}", self.path.display()))?;
        }

        #[cfg(not(unix))]
        {
            fs::write(&self.path, contents)
                .with_context(|| format!("failed to write {}", self.path.display()))?;
        }

        Ok(())
    }
}
