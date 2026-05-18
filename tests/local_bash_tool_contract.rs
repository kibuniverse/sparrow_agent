use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use sparrow_agent::{
    api::{FunctionCall, ToolCall},
    config::{
        AppConfig, BashApprovalMode, BashConfig, ConfirmationPolicy, FilesystemConfig,
        FilesystemMode, StreamingConfig, ToolResultConfig,
    },
    local_tools::LocalToolProvider,
    server::ServerState,
    tool_provider::ToolProvider,
    trace_store::TraceStore,
};

fn bash_config(enabled: bool, root: PathBuf) -> BashConfig {
    BashConfig {
        enabled,
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

fn test_app_config(bash: BashConfig) -> AppConfig {
    AppConfig {
        api_key: "test".into(),
        tavily_api_key: "test".into(),
        model: "deepseek-chat".into(),
        system_prompt: "You are a test agent.".into(),
        reasoning_effort: "high".into(),
        max_tool_rounds: 1,
        filesystem: FilesystemConfig {
            enabled: false,
            roots: Vec::new(),
            mode: FilesystemMode::ReadOnly,
            confirm: ConfirmationPolicy::Never,
            deny_patterns: Vec::new(),
            max_read_bytes: 1,
            max_write_bytes: 1,
        },
        mcp_servers: Vec::new(),
        tool_results: ToolResultConfig {
            max_injected_chars: 20_000,
            output_dir: ".sparrow_agent/tool_outputs".into(),
        },
        streaming: StreamingConfig {
            enabled: true,
            show_reasoning: true,
            show_tool_call_deltas: false,
        },
        bash,
    }
}

#[test]
fn local_provider_excludes_bash_tool_when_disabled() {
    let root = tempfile::tempdir().unwrap();
    let provider =
        LocalToolProvider::new("test", bash_config(false, root.path().to_path_buf()), None);

    assert!(
        !provider
            .definitions()
            .iter()
            .any(|tool| tool.function.name == "runBashCommand")
    );
}

#[tokio::test]
async fn local_provider_dispatches_bash_tool_when_enabled() {
    let root = tempfile::tempdir().unwrap();
    let provider =
        LocalToolProvider::new("test", bash_config(true, root.path().to_path_buf()), None);

    assert!(
        provider
            .definitions()
            .iter()
            .any(|tool| tool.function.name == "runBashCommand")
    );

    let result = provider
        .execute(&ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "runBashCommand".into(),
                arguments: json!({
                    "command": "printf 'ok'",
                    "cwd": root.path(),
                    "timeout_ms": 5_000
                })
                .to_string(),
            },
        })
        .await
        .unwrap()
        .unwrap();

    let body: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(body["status"], "exited");
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["stdout"], "ok");
}

#[test]
fn server_state_disables_interactive_bash_tools() {
    let root = tempfile::tempdir().unwrap();
    let state = ServerState::new(
        test_app_config(bash_config(true, root.path().to_path_buf())),
        Arc::new(TraceStore::new()),
    );

    assert!(!state.config.bash.enabled);
}
