use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

// ── MCP Initialize ─────────────────────────────────────────────────────

pub fn initialize_params(client_name: &str, client_version: &str) -> Value {
    serde_json::json!({
        "capabilities": {
            "roots": {
                "listChanged": true
            }
        },
        "clientInfo": {
            "name": client_name,
            "version": client_version
        },
        "protocolVersion": "2025-06-18"
    })
}

#[derive(Debug, Clone, Deserialize)]
#[allow(non_snake_case)]
pub struct InitializeResult {
    #[allow(dead_code)]
    pub capabilities: Value,
    #[allow(dead_code)]
    pub serverInfo: ServerInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub version: String,
}

// ── MCP Roots ──────────────────────────────────────────────────────────

pub fn roots_list_params(roots: &[RootEntry]) -> Value {
    serde_json::json!({
        "roots": roots.iter().map(|r| serde_json::json!({
            "uri": r.uri,
            "name": r.name,
        })).collect::<Vec<_>>()
    })
}

#[derive(Debug, Clone)]
pub struct RootEntry {
    pub uri: String,
    pub name: String,
}

impl RootEntry {
    pub fn from_path(path: &std::path::Path, name: &str) -> Self {
        let uri = format!("file://{}", path.display());
        Self {
            uri,
            name: name.into(),
        }
    }
}

// ── MCP Tools ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ToolsListResult {
    #[serde(default)]
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(non_snake_case)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub inputSchema: Option<Value>,
}

pub fn tools_call_params(tool_name: &str, arguments: Value) -> Value {
    serde_json::json!({
        "name": tool_name,
        "arguments": arguments,
    })
}

// ── MCP Tool Call Result ───────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[allow(non_snake_case)]
pub struct ToolCallResult {
    #[serde(default)]
    pub content: Vec<ToolContent>,
    #[serde(default)]
    pub isError: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(non_snake_case)]
pub struct ToolContent {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub mimeType: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl ToolCallResult {
    pub fn to_text(&self) -> String {
        let mut parts = Vec::new();
        for content in &self.content {
            match content.kind.as_str() {
                "text" => {
                    if let Some(text) = &content.text {
                        parts.push(text.clone());
                    }
                }
                "image" | "audio" => {
                    let kind = &content.kind;
                    let mime = content.mimeType.as_deref().unwrap_or("unknown");
                    parts.push(format!("[{kind} content: {mime}]"));
                }
                "resource" => {
                    let uri = content.uri.as_deref().unwrap_or("unknown");
                    let name = content.name.as_deref().unwrap_or("unknown");
                    let mime = content.mimeType.as_deref().unwrap_or("unknown");
                    parts.push(format!("[resource: {name} ({mime}) {uri}]"));
                }
                _ => {
                    if let Some(text) = &content.text {
                        parts.push(text.clone());
                    }
                }
            }
        }

        if self.isError.unwrap_or(false) {
            format!("MCP tool error: {}", parts.join("\n"))
        } else {
            parts.join("\n")
        }
    }
}

// ── Helper ─────────────────────────────────────────────────────────────

pub fn build_request(id: u64, method: &str, params: Option<Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id,
        method: method.into(),
        params,
    }
}

pub fn build_notification(method: &str, params: Option<Value>) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("jsonrpc".into(), Value::String("2.0".into()));
    map.insert("method".into(), Value::String(method.into()));
    if let Some(params) = params {
        map.insert("params".into(), params);
    }
    Value::Object(map)
}
