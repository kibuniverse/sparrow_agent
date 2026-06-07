use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::tool_result_processor::ToolResultMetadata;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextArtifactRef {
    pub kind: String,
    pub path: PathBuf,
    pub description: String,
    pub original_chars: usize,
    pub truncated: bool,
}

impl ContextArtifactRef {
    pub fn from_tool_result(
        path: PathBuf,
        tool_name: impl Into<String>,
        metadata: &ToolResultMetadata,
    ) -> Self {
        Self {
            kind: "tool_output".into(),
            path,
            description: format!("complete output for tool `{}`", tool_name.into()),
            original_chars: metadata.original_chars,
            truncated: metadata.truncated,
        }
    }

    pub fn render(&self) -> String {
        format!(
            "- {}: {} (original_chars={}, truncated={})",
            self.kind,
            self.path.display(),
            self.original_chars,
            self.truncated
        )
    }
}
