use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::debug_log;

pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl StdioTransport {
    pub async fn spawn(command: &str, args: &[String]) -> Result<Self> {
        debug_log!("Spawning MCP server: {command} {}", args.join(" "));

        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn MCP server: {command}"))?;

        let stdin = child.stdin.take().context("no stdin for MCP server")?;
        let stdout = child.stdout.take().context("no stdout for MCP server")?;
        let stderr = child.stderr.take().context("no stderr for MCP server")?;

        // Spawn a task to log stderr
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug_log!("[MCP stderr] {line}");
            }
        });

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub async fn send(&mut self, message: &str) -> Result<()> {
        debug_log!("[MCP ->] {message}");
        self.stdin
            .write_all(message.as_bytes())
            .await
            .context("failed to write to MCP server stdin")?;
        self.stdin
            .write_all(b"\n")
            .await
            .context("failed to write newline to MCP server stdin")?;
        self.stdin
            .flush()
            .await
            .context("failed to flush MCP stdin")?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<String> {
        let mut line = String::new();
        let n = self
            .stdout
            .read_line(&mut line)
            .await
            .context("failed to read from MCP server stdout")?;

        if n == 0 {
            bail!("MCP server closed stdout (EOF)");
        }

        let trimmed = line.trim_end().to_string();
        debug_log!("[MCP <-] {trimmed}");
        Ok(trimmed)
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.kill().await;
        Ok(())
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Best-effort kill on drop
        let _ = self.child.start_kill();
    }
}
