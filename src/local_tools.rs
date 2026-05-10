use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::{
    api::{ToolCall, ToolDef},
    tool_provider::ToolProvider,
    tools,
};

const WEB_SEARCH_TOOL: &str = "webSearch";
const RUN_RUST_WASM_TOOL: &str = "runRustWasm";

pub struct LocalToolProvider {
    tavily_api_key: String,
    definitions: Vec<ToolDef>,
}

impl LocalToolProvider {
    pub fn new(tavily_api_key: impl Into<String>) -> Self {
        Self {
            tavily_api_key: tavily_api_key.into(),
            definitions: vec![web_search_tool(), run_rust_wasm_tool()],
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

async fn call_web_search_tool(arguments: &str, tavily_api_key: &str) -> Result<String> {
    let args: WebSearchArgs = parse_arguments(WEB_SEARCH_TOOL, arguments)?;
    tools::web_search(tavily_api_key, &args.query).await
}

async fn call_run_rust_wasm_tool(arguments: &str) -> Result<String> {
    let args: RunRustWasmArgs = parse_arguments(RUN_RUST_WASM_TOOL, arguments)?;
    tools::run_rust_wasm(&args.code).await
}

fn parse_arguments<T>(tool_name: &str, arguments: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(arguments).with_context(|| format!("invalid arguments for {tool_name}"))
}
