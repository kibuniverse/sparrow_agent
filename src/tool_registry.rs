use anyhow::Result;

use crate::{
    api::{ToolCall, ToolDef},
    debug_log,
    tool_provider::ToolProvider,
};

pub struct ToolRegistry {
    providers: Vec<Box<dyn ToolProvider>>,
    definitions: Vec<ToolDef>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            definitions: Vec::new(),
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn ToolProvider>) {
        debug_log!("Adding tool provider: {}", provider.id());
        self.definitions
            .extend(provider.definitions().iter().cloned());
        self.providers.push(provider);
    }

    pub fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    pub async fn execute_all(&self, tool_calls: &[ToolCall]) -> Vec<ToolExecutionResult> {
        let mut results = Vec::with_capacity(tool_calls.len());

        for tool_call in tool_calls {
            debug_log!(
                "Executing tool: name={}, id={}, args={}",
                tool_call.function.name,
                tool_call.id,
                tool_call.function.arguments,
            );
            let content = match self.execute(tool_call).await {
                Ok(content) => {
                    debug_log!(
                        "Tool '{}' succeeded, result length: {}",
                        tool_call.function.name,
                        content.len()
                    );
                    content
                }
                Err(error) => {
                    debug_log!("Tool '{}' failed: {error}", tool_call.function.name);
                    format!("Tool execution failed: {error}")
                }
            };

            results.push(ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                content,
            });
        }

        results
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<String> {
        for provider in &self.providers {
            if let Some(result) = provider.execute(tool_call).await? {
                return Ok(result);
            }
        }
        anyhow::bail!("unknown tool: {}", tool_call.function.name);
    }
}

pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub content: String,
}
