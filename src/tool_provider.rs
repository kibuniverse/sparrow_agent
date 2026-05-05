use anyhow::Result;

use crate::api::{ToolCall, ToolDef};

#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    fn id(&self) -> &str;
    fn definitions(&self) -> &[ToolDef];
    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>>;
}
