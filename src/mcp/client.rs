use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::debug_log;

use super::protocol::*;
use super::stdio_transport::StdioTransport;

pub struct McpClient {
    server_id: String,
    transport: StdioTransport,
    next_id: u64,
    roots: Vec<RootEntry>,
}

impl McpClient {
    pub async fn connect(
        server_id: String,
        command: &str,
        args: &[String],
        roots: Vec<PathBuf>,
    ) -> Result<Self> {
        let transport = StdioTransport::spawn(command, args).await?;

        let root_entries: Vec<RootEntry> = roots
            .iter()
            .map(|p| {
                let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
                RootEntry::from_path(&canonical, "sparrow workspace")
            })
            .collect();

        let mut client = Self {
            server_id,
            transport,
            next_id: 1,
            roots: root_entries,
        };

        client.handshake().await?;
        Ok(client)
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    async fn handshake(&mut self) -> Result<()> {
        // Step 1: Send initialize request
        let params = initialize_params("sparrow_agent", "0.1.0");
        let response = self.send_request("initialize", Some(params)).await?;

        let init_result: InitializeResult =
            serde_json::from_value(response.result.context("initialize returned no result")?)
                .context("failed to parse initialize result")?;

        debug_log!(
            "MCP server '{}' initialized: {} v{}",
            self.server_id,
            init_result.serverInfo.name,
            init_result.serverInfo.version,
        );

        // Step 2: Send initialized notification
        let notification = build_notification("notifications/initialized", None);
        self.transport
            .send(&serde_json::to_string(&notification)?)
            .await?;

        Ok(())
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let response = self.send_request("tools/list", None).await?;
        let result: ToolsListResult =
            serde_json::from_value(response.result.context("tools/list returned no result")?)
                .context("failed to parse tools/list result")?;

        debug_log!(
            "MCP server '{}' provides {} tools: {:?}",
            self.server_id,
            result.tools.len(),
            result.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
        );

        Ok(result.tools)
    }

    pub async fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<String> {
        let params = tools_call_params(tool_name, arguments);
        let response = self.send_request("tools/call", Some(params)).await?;

        if let Some(error) = &response.error {
            bail!(
                "MCP tool error: code={}, message={}",
                error.code,
                error.message,
            );
        }

        let result: ToolCallResult =
            serde_json::from_value(response.result.context("tools/call returned no result")?)
                .context("failed to parse tools/call result")?;

        Ok(result.to_text())
    }

    async fn send_request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self.next_id;
        self.next_id += 1;

        let request = build_request(id, method, params);
        let message = serde_json::to_string(&request)?;
        self.transport.send(&message).await?;

        // Read responses until we get one matching our id
        loop {
            let line = self.transport.recv().await?;
            let response: JsonRpcResponse = serde_json::from_str(&line)
                .with_context(|| format!("failed to parse MCP response: {line}"))?;

            // Check if this is a server-initiated request (e.g., roots/list)
            if response.method.as_deref() == Some("roots/list") {
                self.handle_roots_list().await?;
                continue;
            }

            // Check if this is a notification
            if response.id.is_none() && response.method.is_some() {
                debug_log!("MCP notification: {:?}", response.method,);
                continue;
            }

            // This should be our response
            if response.id == Some(id) {
                return Ok(response);
            }

            debug_log!(
                "MCP: skipping response with id={:?}, expected id={id}",
                response.id,
            );
        }
    }

    async fn handle_roots_list(&mut self) -> Result<()> {
        let response_id = self.next_id;
        self.next_id += 1;

        let params = roots_list_params(&self.roots);
        let response = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: Some(response_id),
            result: Some(params),
            error: None,
            method: None,
            params: None,
        };

        let message = serde_json::to_string(&response)?;
        self.transport.send(&message).await?;
        Ok(())
    }

    pub async fn shutdown(mut self) -> Result<()> {
        self.transport.shutdown().await
    }
}
