use std::{
    collections::HashSet,
    env,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

use crate::config::BashConfig;

const BASH_PROGRAM: &str = "/bin/bash";

#[derive(Debug, Clone)]
pub struct BashCommandRequest {
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BashCommandStatus {
    Exited,
    TimedOut,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashCommandOutput {
    pub status: BashCommandStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub cwd: PathBuf,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone)]
pub struct BashRunner {
    config: BashConfig,
}

impl BashRunner {
    pub fn new(config: BashConfig) -> Self {
        Self { config }
    }

    pub async fn run(&self, request: BashCommandRequest) -> Result<BashCommandOutput> {
        self.validate_command(&request.command)?;

        let cwd = self.resolve_cwd(request.cwd.as_deref())?;
        let timeout_ms = self.resolve_timeout_ms(request.timeout_ms)?;

        if self.config.require_confirmation
            && !prompt_confirmation(&request.command, &cwd, timeout_ms)?
        {
            return Ok(BashCommandOutput {
                status: BashCommandStatus::Denied,
                exit_code: None,
                duration_ms: 0,
                cwd,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            });
        }

        let started = Instant::now();
        let mut command = Command::new(BASH_PROGRAM);
        command
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(&request.command)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();

        for (name, value) in filtered_current_env(&self.config) {
            command.env(name, value);
        }

        let mut child = command.spawn().context("failed to spawn bash")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to capture bash stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("failed to capture bash stderr")?;

        let stdout_limit = self.config.stream_max_bytes;
        let stderr_limit = self.config.stream_max_bytes;
        let stdout_task = tokio::spawn(async move { read_limited(stdout, stdout_limit).await });
        let stderr_task = tokio::spawn(async move { read_limited(stderr, stderr_limit).await });

        let timeout = Duration::from_millis(timeout_ms);
        let (status, exit_code) = tokio::select! {
            wait_result = child.wait() => {
                let status = wait_result.context("failed to wait for bash process")?;
                (BashCommandStatus::Exited, status.code())
            }
            _ = tokio::time::sleep(timeout) => {
                child.kill().await.context("failed to kill timed out bash process")?;
                let _ = child.wait().await;
                (BashCommandStatus::TimedOut, None)
            }
        };

        let stdout = stdout_task
            .await
            .context("stdout reader task failed")?
            .context("failed to read bash stdout")?;
        let stderr = stderr_task
            .await
            .context("stderr reader task failed")?
            .context("failed to read bash stderr")?;

        Ok(BashCommandOutput {
            status,
            exit_code,
            duration_ms: started.elapsed().as_millis() as u64,
            cwd,
            stdout: stdout.text,
            stderr: stderr.text,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        })
    }

    fn validate_command(&self, command: &str) -> Result<()> {
        if command.trim().is_empty() {
            bail!("bash command cannot be empty");
        }

        if command.chars().count() > self.config.max_command_chars {
            bail!(
                "bash command is too long: {} chars (limit: {} chars)",
                command.chars().count(),
                self.config.max_command_chars
            );
        }

        if command.contains('\0') {
            bail!("bash command cannot contain NUL bytes");
        }

        Ok(())
    }

    fn resolve_timeout_ms(&self, timeout_ms: Option<u64>) -> Result<u64> {
        let timeout_ms = timeout_ms.unwrap_or(self.config.timeout_ms);
        if timeout_ms == 0 {
            bail!("timeout_ms must be greater than zero");
        }
        Ok(timeout_ms.min(self.config.max_timeout_ms))
    }

    fn resolve_cwd(&self, cwd: Option<&Path>) -> Result<PathBuf> {
        let path = match cwd {
            Some(path) => path.to_path_buf(),
            None => env::current_dir().context("failed to read current directory")?,
        };
        let absolute = if path.is_absolute() {
            path
        } else {
            env::current_dir()
                .context("failed to read current directory")?
                .join(path)
        };
        let canonical = absolute
            .canonicalize()
            .with_context(|| format!("bash cwd does not exist: {}", absolute.display()))?;

        if !self.cwd_is_inside_allowed_root(&canonical)? {
            bail!("bash cwd is outside allowed roots: {}", canonical.display());
        }

        Ok(canonical)
    }

    fn cwd_is_inside_allowed_root(&self, cwd: &Path) -> Result<bool> {
        for root in &self.config.roots {
            let absolute = if root.is_absolute() {
                root.clone()
            } else {
                env::current_dir()
                    .context("failed to read current directory")?
                    .join(root)
            };
            let canonical_root = absolute
                .canonicalize()
                .with_context(|| format!("bash root does not exist: {}", absolute.display()))?;
            if cwd.starts_with(canonical_root) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[derive(Debug)]
struct LimitedOutput {
    text: String,
    truncated: bool,
}

async fn read_limited<R>(mut reader: R, limit: usize) -> Result<LimitedOutput>
where
    R: AsyncRead + Unpin,
{
    let mut retained = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 4096];

    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }

        total = total.saturating_add(bytes_read);
        if retained.len() < limit {
            let remaining = limit - retained.len();
            let keep = remaining.min(bytes_read);
            retained.extend_from_slice(&buffer[..keep]);
        }
    }

    let truncated = total > retained.len();
    Ok(LimitedOutput {
        text: bytes_to_utf8_text(&retained, truncated),
        truncated,
    })
}

fn bytes_to_utf8_text(bytes: &[u8], truncated: bool) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(error) if truncated && error.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned()
        }
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn filtered_current_env(config: &BashConfig) -> Vec<(String, String)> {
    filter_env_pairs(config, env::vars())
}

fn filter_env_pairs<I, K, V>(config: &BashConfig, pairs: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let allowlist = config
        .env_allowlist
        .iter()
        .map(|name| name.as_str())
        .collect::<HashSet<_>>();

    pairs
        .into_iter()
        .filter_map(|(name, value)| {
            let name = name.as_ref();
            if allowlist.contains(name) && !is_secret_env_name(name) {
                Some((name.to_string(), value.as_ref().to_string()))
            } else {
                None
            }
        })
        .collect()
}

fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "AUTHORIZATION"]
        .iter()
        .any(|needle| upper.contains(needle))
}

fn prompt_confirmation(command: &str, cwd: &Path, timeout_ms: u64) -> Result<bool> {
    println!("Sparrow wants to run bash command:");
    println!("  cwd: {}", cwd.display());
    println!("  timeout: {timeout_ms} ms");
    println!("  command:");
    for line in command.lines() {
        println!("    {line}");
    }
    print!("Approve? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> BashConfig {
        BashConfig {
            enabled: true,
            roots: vec![PathBuf::from(".")],
            require_confirmation: false,
            timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            max_command_chars: 8_192,
            stream_max_bytes: 8 * 1024,
            env_allowlist: vec!["PATH".into(), "API_KEY".into(), "AUTHORIZATION".into()],
        }
    }

    #[test]
    fn filter_env_pairs_omits_secret_names_even_when_allowlisted() {
        let filtered = filter_env_pairs(
            &test_config(),
            [
                ("PATH", "/bin"),
                ("API_KEY", "secret"),
                ("AUTHORIZATION", "Bearer secret"),
                ("UNLISTED", "value"),
            ],
        );

        assert_eq!(filtered, vec![("PATH".to_string(), "/bin".to_string())]);
    }
}
