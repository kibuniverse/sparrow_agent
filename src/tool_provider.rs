use anyhow::Result;

use crate::api::{ToolCall, ToolDef};
use crate::trace::TraceSinkContext;

#[derive(Clone)]
pub struct ToolExecutionTraceContext {
    pub parent_trace: TraceSinkContext,
    pub parent_model_output_id: String,
    pub tool_call_id: String,
}

#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    fn id(&self) -> &str;
    fn definitions(&self) -> &[ToolDef];
    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>>;

    async fn execute_traced(
        &self,
        tool_call: &ToolCall,
        _trace_context: Option<&ToolExecutionTraceContext>,
    ) -> Result<Option<String>> {
        self.execute(tool_call).await
    }
}
