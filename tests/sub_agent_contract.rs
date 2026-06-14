use serde_json::{Value, json};
use sparrow_agent::{
    api::{FunctionCall, ToolCall},
    config::{
        AppConfig, BashApprovalMode, BashConfig, ConfirmationPolicy, ContextConfig,
        FilesystemConfig, FilesystemMode, MemoryConfig, StreamingConfig, SubAgentConfig,
        ToolResultConfig,
    },
    sub_agent::{SubAgentCoordinator, SubAgentRequest, SubAgentToolProvider},
    tool_provider::ToolProvider,
};

fn sub_agent_config() -> SubAgentConfig {
    SubAgentConfig {
        enabled: true,
        max_depth: 1,
        max_concurrent: 3,
        max_tool_rounds: 8,
        timeout_ms: 120_000,
        allowed_tools: vec![
            "webSearch".into(),
            "mcp__filesystem__read_file".into(),
            "mcp__filesystem__search_files".into(),
        ],
        inherit_filesystem: true,
        inherit_bash: false,
    }
}

fn app_config() -> AppConfig {
    AppConfig {
        api_key: "test".into(),
        tavily_api_key: "test".into(),
        model: "deepseek-chat".into(),
        system_prompt: "You are a parent test agent.".into(),
        reasoning_effort: "high".into(),
        max_tool_rounds: 100,
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
        bash: BashConfig {
            enabled: true,
            roots: vec![".".into()],
            approval_mode: BashApprovalMode::NeverPrompt,
            approval_policy_path: tempfile::tempdir().unwrap().path().join("policies.json"),
            approval_policy_ttl_days: 90,
            model_low_risk_threshold: 0.85,
            timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            max_command_chars: 8_192,
            stream_max_bytes: 8 * 1024,
            env_allowlist: vec!["PATH".into()],
        },
        sub_agent: sub_agent_config(),
        context: ContextConfig::default(),
        memory: MemoryConfig::default(),
    }
}

#[test]
fn sub_agent_tool_definition_exposes_self_contained_task_schema() {
    let provider = SubAgentToolProvider::new(app_config());
    let definitions = provider.definitions();

    assert_eq!(definitions.len(), 1);
    let tool = &definitions[0];
    assert_eq!(tool.function.name, "runSubAgentTask");

    let parameters = tool.function.parameters.as_ref().unwrap();
    assert_eq!(
        parameters["required"],
        json!(["task", "context_pack", "expected_output"])
    );
    assert_eq!(parameters["additionalProperties"], false);
    assert_eq!(parameters["properties"]["max_tool_rounds"]["maximum"], 20);
}

#[test]
fn coordinator_rejects_context_pack_that_depends_on_parent_conversation() {
    let coordinator = SubAgentCoordinator::new(sub_agent_config());
    let request = SubAgentRequest {
        task: "Summarize the local files listed in the context pack.".into(),
        context_pack: "Use the paths we discussed above.".into(),
        expected_output: "Return a concise findings list.".into(),
        allowed_tools: Vec::new(),
        max_tool_rounds: None,
    };

    let result = coordinator.check_eligibility(&request, 100).unwrap_err();

    assert_eq!(result.status, "rejected");
    assert!(result.reason.unwrap().contains("parent conversation"));
    assert_eq!(result.final_answer, "");
    assert!(result.child_task_id.is_none());
}

#[test]
fn coordinator_applies_tool_and_round_limits() {
    let coordinator = SubAgentCoordinator::new(sub_agent_config());
    let request = SubAgentRequest {
        task: "Search for the current release notes for the named library.".into(),
        context_pack: "Library: tokio. Only public release notes are needed.".into(),
        expected_output: "Return JSON with version and notable changes.".into(),
        allowed_tools: vec!["webSearch".into()],
        max_tool_rounds: Some(12),
    };

    let plan = coordinator.check_eligibility(&request, 10).unwrap();

    assert_eq!(plan.allowed_tools, vec!["webSearch"]);
    assert_eq!(plan.max_tool_rounds, 8);
    assert!(plan.child_task_id.starts_with("task_sub_"));
    assert!(plan.child_conversation_id.starts_with("conv_sub_"));
}

#[test]
fn coordinator_rejects_tools_outside_configured_allowlist() {
    let coordinator = SubAgentCoordinator::new(sub_agent_config());
    let request = SubAgentRequest {
        task: "Inspect the repository.".into(),
        context_pack: "Root: /tmp/project. Use only listed files.".into(),
        expected_output: "Return implementation notes.".into(),
        allowed_tools: vec!["runBashCommand".into()],
        max_tool_rounds: None,
    };

    let result = coordinator.check_eligibility(&request, 100).unwrap_err();

    assert_eq!(result.status, "rejected");
    assert!(result.reason.unwrap().contains("not allowed"));
}

#[tokio::test]
async fn provider_returns_structured_rejection_without_running_child_agent() {
    let provider = SubAgentToolProvider::new(app_config());

    let output = provider
        .execute(&ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "runSubAgentTask".into(),
                arguments: json!({
                    "task": "Read the files.",
                    "context_pack": "See above for the file list.",
                    "expected_output": "Return findings."
                })
                .to_string(),
            },
        })
        .await
        .unwrap()
        .unwrap();

    let body: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(body["status"], "rejected");
    assert_eq!(body["final_answer"], "");
    assert_eq!(body["child_task_id"], Value::Null);
}
