use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

pub const DEFAULT_TOOL_RESULT_MAX_CHARS: usize = 20_000;

#[derive(Debug, Clone)]
pub struct ToolResultProcessorConfig {
    pub max_injected_chars: usize,
    pub output_dir: PathBuf,
}

impl Default for ToolResultProcessorConfig {
    fn default() -> Self {
        Self {
            max_injected_chars: DEFAULT_TOOL_RESULT_MAX_CHARS,
            output_dir: PathBuf::from(".sparrow_agent/tool_outputs"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolResultProcessor {
    config: ToolResultProcessorConfig,
}

impl ToolResultProcessor {
    pub fn new(config: ToolResultProcessorConfig) -> Self {
        Self { config }
    }

    pub fn process(&self, input: ToolResultInput) -> Result<ProcessedToolResult> {
        let original_chars = input.content.chars().count();

        if original_chars <= self.config.max_injected_chars {
            return Ok(ProcessedToolResult {
                metadata: ToolResultMetadata {
                    original_chars,
                    injected_chars: original_chars,
                    truncated: false,
                    artifact_path: None,
                },
                content: input.content,
            });
        }

        fs::create_dir_all(&self.config.output_dir).with_context(|| {
            format!(
                "failed to create tool output directory {}",
                self.config.output_dir.display()
            )
        })?;

        let artifact_path = self.artifact_path(&input.tool_call_id, &input.tool_name);
        fs::write(&artifact_path, input.content.as_bytes()).with_context(|| {
            format!(
                "failed to save oversized tool output to {}",
                artifact_path.display()
            )
        })?;

        let notice = format!(
            "工具输出过长：本次工具调用 `{}` 的结果总长度为 {} 个字符，已经被截断。完整的输出已经被保存在文件 `{}` 中。请考虑以更精确的参数调用该函数，或者更换更适合检索、分页或摘要的函数。\n\n--- 截断后的输出预览 ---\n",
            input.tool_name,
            original_chars,
            artifact_path.display(),
        );

        let notice_chars = notice.chars().count();
        let preview_budget = self.config.max_injected_chars.saturating_sub(notice_chars);
        let preview = take_chars(&input.content, preview_budget);
        let content = format!("{notice}{preview}");
        let injected_chars = content.chars().count();

        Ok(ProcessedToolResult {
            content,
            metadata: ToolResultMetadata {
                original_chars,
                injected_chars,
                truncated: true,
                artifact_path: Some(artifact_path),
            },
        })
    }

    fn artifact_path(&self, tool_call_id: &str, tool_name: &str) -> PathBuf {
        let id = ulid::Ulid::new();
        let safe_tool_name = sanitize_filename_part(tool_name);
        let safe_tool_call_id = sanitize_filename_part(tool_call_id);
        self.config
            .output_dir
            .join(format!("{id}-{safe_tool_name}-{safe_tool_call_id}.txt"))
    }
}

impl Default for ToolResultProcessor {
    fn default() -> Self {
        Self::new(ToolResultProcessorConfig::default())
    }
}

#[derive(Debug)]
pub struct ToolResultInput {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ProcessedToolResult {
    pub content: String,
    pub metadata: ToolResultMetadata,
}

#[derive(Debug, Clone)]
pub struct ToolResultMetadata {
    pub original_chars: usize,
    pub injected_chars: usize,
    pub truncated: bool,
    pub artifact_path: Option<PathBuf>,
}

impl ToolResultMetadata {
    pub fn artifact_path_display(&self) -> Option<String> {
        self.artifact_path
            .as_ref()
            .map(|path| path.display().to_string())
    }
}

fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn sanitize_filename_part(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        DEFAULT_TOOL_RESULT_MAX_CHARS, ToolResultInput, ToolResultProcessor,
        ToolResultProcessorConfig,
    };

    #[test]
    fn leaves_small_tool_output_unchanged() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 80,
            output_dir: temp.path().join("tool_outputs"),
        });

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_small".to_string(),
                tool_name: "read_file".to_string(),
                content: "short output".to_string(),
            })
            .unwrap();

        assert_eq!(processed.content, "short output");
        assert!(!processed.metadata.truncated);
        assert_eq!(processed.metadata.original_chars, 12);
        assert_eq!(processed.metadata.injected_chars, 12);
        assert!(processed.metadata.artifact_path.is_none());
        assert!(!temp.path().join("tool_outputs").exists());
    }

    #[test]
    fn truncates_large_output_and_saves_complete_original_output() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 700,
            output_dir: temp.path().join("tool_outputs"),
        });
        let original = "abcdef".repeat(160);

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_large".to_string(),
                tool_name: "read_file".to_string(),
                content: original.clone(),
            })
            .unwrap();

        assert!(processed.metadata.truncated);
        assert_eq!(processed.metadata.original_chars, original.chars().count());
        assert!(processed.metadata.injected_chars <= 700);
        assert!(processed.content.contains("工具输出过长"));
        assert!(processed.content.contains("结果总长度为 960 个字符"));
        assert!(processed.content.contains("已经被截断"));
        assert!(processed.content.contains("完整的输出已经被保存在文件"));
        assert!(processed.content.contains("请考虑以更精确的参数调用该函数"));
        assert!(processed.content.contains("--- 截断后的输出预览 ---"));

        let artifact_path = processed.metadata.artifact_path.as_ref().unwrap();
        assert!(artifact_path.starts_with(temp.path().join("tool_outputs")));
        assert_eq!(fs::read_to_string(artifact_path).unwrap(), original);
    }

    #[test]
    fn default_limit_truncates_outputs_above_twenty_thousand_chars() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            output_dir: temp.path().join("tool_outputs"),
            ..ToolResultProcessorConfig::default()
        });
        let original = "x".repeat(DEFAULT_TOOL_RESULT_MAX_CHARS + 1);

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_default_limit".to_string(),
                tool_name: "default_limit_tool".to_string(),
                content: original.clone(),
            })
            .unwrap();

        assert!(processed.metadata.truncated);
        assert_eq!(processed.metadata.original_chars, 20_001);
        assert!(processed.metadata.injected_chars <= DEFAULT_TOOL_RESULT_MAX_CHARS);
        assert!(processed.content.starts_with("工具输出过长"));
        assert_eq!(
            fs::read_to_string(processed.metadata.artifact_path.unwrap()).unwrap(),
            original,
        );
    }

    #[test]
    fn truncates_without_splitting_multibyte_characters() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 700,
            output_dir: temp.path().join("tool_outputs"),
        });
        let original = "你好🙂".repeat(300);

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_unicode".to_string(),
                tool_name: "unicode_tool".to_string(),
                content: original.clone(),
            })
            .unwrap();

        assert!(processed.metadata.truncated);
        assert!(processed.metadata.injected_chars <= 700);
        assert!(processed.content.is_char_boundary(processed.content.len()));
        assert_eq!(
            fs::read_to_string(processed.metadata.artifact_path.unwrap()).unwrap(),
            original,
        );
    }

    #[test]
    fn sanitizes_tool_name_and_tool_call_id_in_artifact_filename() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 160,
            output_dir: temp.path().join("tool_outputs"),
        });

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call/with:unsafe chars".to_string(),
                tool_name: "mcp__filesystem__read/file".to_string(),
                content: "x".repeat(300),
            })
            .unwrap();

        let filename = processed
            .metadata
            .artifact_path
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
        assert!(filename.contains("mcp__filesystem__read_file"));
        assert!(filename.contains("call_with_unsafe_chars"));
    }
}
