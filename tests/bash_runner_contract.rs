use std::path::PathBuf;

use sparrow_agent::{
    bash_runner::{BashCommandRequest, BashCommandStatus, BashRunner},
    config::{BashApprovalMode, BashConfig},
};

fn test_config(root: PathBuf) -> BashConfig {
    BashConfig {
        enabled: true,
        roots: vec![root],
        approval_mode: BashApprovalMode::NeverPrompt,
        approval_policy_path: tempfile::tempdir().unwrap().path().join("policies.json"),
        approval_policy_ttl_days: 90,
        model_low_risk_threshold: 0.85,
        timeout_ms: 30_000,
        max_timeout_ms: 120_000,
        max_command_chars: 8_192,
        stream_max_bytes: 8 * 1024,
        env_allowlist: vec!["PATH".into()],
    }
}

#[tokio::test]
async fn bash_runner_executes_command_and_returns_structured_output() {
    let root = tempfile::tempdir().unwrap();
    let runner = BashRunner::new(test_config(root.path().to_path_buf()), None);

    let output = runner
        .run(BashCommandRequest {
            command: "printf 'hello'".into(),
            cwd: Some(root.path().to_path_buf()),
            timeout_ms: Some(5_000),
        })
        .await
        .unwrap();

    assert_eq!(output.status, BashCommandStatus::Exited);
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout, "hello");
    assert_eq!(output.stderr, "");
    assert_eq!(output.cwd, root.path().canonicalize().unwrap());
    assert!(!output.stdout_truncated);
    assert!(!output.stderr_truncated);
}

#[tokio::test]
async fn bash_runner_output_includes_approval_summary() {
    let root = tempfile::tempdir().unwrap();
    let runner = BashRunner::new(test_config(root.path().to_path_buf()), None);

    let output = runner
        .run(BashCommandRequest {
            command: "printf 'hello'".into(),
            cwd: Some(root.path().to_path_buf()),
            timeout_ms: Some(5_000),
        })
        .await
        .unwrap();

    let approval = output.approval.unwrap();
    assert_eq!(approval.mode, "never");
    assert_eq!(approval.approved_by, "never_prompt");
}

#[tokio::test]
async fn bash_runner_reports_nonzero_exit_without_treating_it_as_tool_failure() {
    let root = tempfile::tempdir().unwrap();
    let runner = BashRunner::new(test_config(root.path().to_path_buf()), None);

    let output = runner
        .run(BashCommandRequest {
            command: "printf 'bad' >&2; exit 7".into(),
            cwd: Some(root.path().to_path_buf()),
            timeout_ms: Some(5_000),
        })
        .await
        .unwrap();

    assert_eq!(output.status, BashCommandStatus::Exited);
    assert_eq!(output.exit_code, Some(7));
    assert_eq!(output.stdout, "");
    assert_eq!(output.stderr, "bad");
}

#[tokio::test]
async fn bash_runner_rejects_cwd_outside_allowed_roots() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let runner = BashRunner::new(test_config(root.path().to_path_buf()), None);

    let error = runner
        .run(BashCommandRequest {
            command: "pwd".into(),
            cwd: Some(outside.path().to_path_buf()),
            timeout_ms: Some(5_000),
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("outside allowed roots"));
}

#[tokio::test]
async fn bash_runner_times_out_and_kills_long_running_commands() {
    let root = tempfile::tempdir().unwrap();
    let runner = BashRunner::new(test_config(root.path().to_path_buf()), None);

    let output = runner
        .run(BashCommandRequest {
            command: "sleep 2".into(),
            cwd: Some(root.path().to_path_buf()),
            timeout_ms: Some(100),
        })
        .await
        .unwrap();

    assert_eq!(output.status, BashCommandStatus::TimedOut);
    assert_eq!(output.exit_code, None);
}

#[tokio::test]
async fn bash_runner_truncates_stdout_without_splitting_utf8() {
    let root = tempfile::tempdir().unwrap();
    let mut config = test_config(root.path().to_path_buf());
    config.stream_max_bytes = 3;
    let runner = BashRunner::new(config, None);

    let output = runner
        .run(BashCommandRequest {
            command: "printf '\\303\\251\\303\\251'".into(),
            cwd: Some(root.path().to_path_buf()),
            timeout_ms: Some(5_000),
        })
        .await
        .unwrap();

    assert_eq!(output.stdout, "\u{e9}");
    assert!(output.stdout_truncated);
}
