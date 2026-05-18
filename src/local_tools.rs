use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::{
    api::{ToolCall, ToolDef},
    bash_runner::{BashCommandRequest, BashRunner},
    config::BashConfig,
    tool_provider::ToolProvider,
    tools,
};

const WEB_SEARCH_TOOL: &str = "webSearch";
const RUN_RUST_WASM_TOOL: &str = "runRustWasm";
const RUN_BASH_COMMAND_TOOL: &str = "runBashCommand";

pub struct LocalToolProvider {
    tavily_api_key: String,
    bash_runner: Option<BashRunner>,
    definitions: Vec<ToolDef>,
}

impl LocalToolProvider {
    pub fn new(
        tavily_api_key: impl Into<String>,
        bash_config: BashConfig,
        deepseek_api_key: Option<String>,
    ) -> Self {
        let mut definitions = vec![web_search_tool(), run_rust_wasm_tool()];
        let bash_runner = if bash_config.enabled {
            definitions.push(run_bash_command_tool(&bash_config));
            Some(BashRunner::new(bash_config, deepseek_api_key))
        } else {
            None
        };

        Self {
            tavily_api_key: tavily_api_key.into(),
            bash_runner,
            definitions,
        }
    }
}

#[async_trait::async_trait]
impl ToolProvider for LocalToolProvider {
    fn id(&self) -> &str {
        "local"
    }

    fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
        let name = &tool_call.function.name;
        match name.as_str() {
            WEB_SEARCH_TOOL => {
                let result =
                    call_web_search_tool(&tool_call.function.arguments, &self.tavily_api_key)
                        .await?;
                Ok(Some(result))
            }
            RUN_RUST_WASM_TOOL => {
                let result = call_run_rust_wasm_tool(&tool_call.function.arguments).await?;
                Ok(Some(result))
            }
            RUN_BASH_COMMAND_TOOL => {
                let Some(runner) = &self.bash_runner else {
                    return Ok(None);
                };
                let result =
                    call_run_bash_command_tool(&tool_call.function.arguments, runner).await?;
                Ok(Some(result))
            }
            _ => Ok(None),
        }
    }
}

#[derive(serde::Deserialize)]
struct WebSearchArgs {
    query: String,
}

#[derive(serde::Deserialize)]
struct RunRustWasmArgs {
    code: String,
}

#[derive(serde::Deserialize)]
struct RunBashCommandArgs {
    command: String,
    #[serde(default)]
    cwd: Option<std::path::PathBuf>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

fn web_search_tool() -> ToolDef {
    let mut tool = ToolDef::function(
        WEB_SEARCH_TOOL,
        "Search the web for information using Tavily.",
    );
    tool.function.parameters = Some(json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "The search query."
            }
        },
        "required": ["query"]
    }));
    tool
}

fn run_rust_wasm_tool() -> ToolDef {
    let mut tool = ToolDef::function(
        RUN_RUST_WASM_TOOL,
        "Compile and execute Rust code as WebAssembly. The code must define `pub fn run() -> String`.",
    );
    tool.function.parameters = Some(json!({
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "description": "Rust code that defines `pub fn run() -> String`."
            }
        },
        "required": ["code"]
    }));
    tool
}

fn run_bash_command_tool(config: &BashConfig) -> ToolDef {
    let mut tool = ToolDef::function(
        RUN_BASH_COMMAND_TOOL,
        "Run one non-interactive bash command in an approved local CLI session. The command runs with a cleaned environment, bounded output, a timeout, and cwd validation against allowed roots.",
    );
    tool.function.parameters = Some(json!({
        "type": "object",
        "properties": {
            "command": {
                "type": "string",
                "description": "The bash command to execute via a non-interactive bash shell."
            },
            "cwd": {
                "type": ["string", "null"],
                "description": "Working directory for the command. It must be inside one of the configured allowed roots."
            },
            "timeout_ms": {
                "type": ["integer", "null"],
                "minimum": 1,
                "maximum": config.max_timeout_ms,
                "description": "Optional command timeout in milliseconds. Values above the configured cap are clamped."
            }
        },
        "required": ["command"],
        "additionalProperties": false
    }));
    tool
}

async fn call_web_search_tool(arguments: &str, tavily_api_key: &str) -> Result<String> {
    let args: WebSearchArgs = parse_arguments(WEB_SEARCH_TOOL, arguments)?;
    tools::web_search(tavily_api_key, &args.query).await
}

async fn call_run_rust_wasm_tool(arguments: &str) -> Result<String> {
    let args: RunRustWasmArgs = parse_arguments(RUN_RUST_WASM_TOOL, arguments)?;
    tools::run_rust_wasm(&args.code).await
}

async fn call_run_bash_command_tool(arguments: &str, runner: &BashRunner) -> Result<String> {
    let args: RunBashCommandArgs = parse_arguments(RUN_BASH_COMMAND_TOOL, arguments)?;
    let output = runner
        .run(BashCommandRequest {
            command: args.command,
            cwd: args.cwd,
            timeout_ms: args.timeout_ms,
        })
        .await?;
    serde_json::to_string(&output).context("failed to serialize bash command output")
}

fn parse_arguments<T>(tool_name: &str, arguments: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(arguments).with_context(|| format!("invalid arguments for {tool_name}"))
}
