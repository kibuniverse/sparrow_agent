use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::GlobSet;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    api::{ToolCall, ToolDef},
    config::{FilesystemConfig, FilesystemMode},
    tool_provider::ToolProvider,
};

use super::client::McpClient;
use super::protocol::McpTool;

const WRITE_TOOL_NAMES: &[&str] = &["write_file", "edit_file", "create_directory", "move_file"];

pub struct McpToolProvider {
    server_id: String,
    definitions: Vec<ToolDef>,
    tool_map: HashMap<String, String>, // mcp__{id}__{tool} -> original tool name
    client: Mutex<McpClient>,
    config: FilesystemConfig,
    deny_glob: GlobSet,
}

impl McpToolProvider {
    pub async fn new(config: FilesystemConfig, client: McpClient) -> Result<Self> {
        let server_id = client.server_id().to_string();

        // Build deny globset
        let mut builder = globset::GlobSet::builder();
        for pattern in &config.deny_patterns {
            builder.add(globset::Glob::new(pattern)?);
        }
        let deny_glob = builder.build()?;

        let mut locked_client = client;

        // Discover tools
        let mcp_tools = locked_client.list_tools().await?;

        // Filter tools based on mode
        let filtered_tools: Vec<&McpTool> = mcp_tools
            .iter()
            .filter(|tool| {
                if config.mode == FilesystemMode::ReadOnly {
                    !is_write_tool(&tool.name)
                } else {
                    true
                }
            })
            .collect();

        // Build definitions and mapping
        let mut definitions = Vec::new();
        let mut tool_map = HashMap::new();

        for tool in &filtered_tools {
            let namespaced = format!("mcp__{}__{}", server_id, tool.name);
            let mut def = ToolDef::function(
                &namespaced,
                tool.description.as_deref().unwrap_or(&tool.name),
            );
            def.function.parameters = tool.inputSchema.clone();
            definitions.push(def);
            tool_map.insert(namespaced, tool.name.clone());
        }

        Ok(Self {
            server_id,
            definitions,
            tool_map,
            client: Mutex::new(locked_client),
            config,
            deny_glob,
        })
    }
}

#[async_trait::async_trait]
impl ToolProvider for McpToolProvider {
    fn id(&self) -> &str {
        &self.server_id
    }

    fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
        let namespaced_name = &tool_call.function.name;

        let original_name = match self.tool_map.get(namespaced_name) {
            Some(name) => name.clone(),
            None => return Ok(None),
        };

        // Parse arguments
        let arguments: Value = serde_json::from_str(&tool_call.function.arguments)
            .with_context(|| format!("invalid arguments for {namespaced_name}"))?;

        // Path validation
        self.validate_paths(&original_name, &arguments)?;

        // Write tool checks
        let is_write = is_write_tool(&original_name);

        if is_write && self.config.mode == FilesystemMode::ReadOnly {
            bail!(
                "Tool '{}' is not available in read-only mode",
                original_name,
            );
        }

        // Confirmation for write tools
        if self.config.confirm.should_confirm(is_write) {
            let confirmed = prompt_confirmation(&original_name, &arguments)?;
            if !confirmed {
                return Ok(Some("Tool execution denied by user".into()));
            }
        }

        // edit_file: force dry run first
        if original_name == "edit_file" {
            let result = self.execute_edit_file(&original_name, &arguments).await?;
            return Ok(Some(result));
        }

        // Normal tool call
        let mut client = self.client.lock().await;
        let result = client.call_tool(&original_name, arguments).await?;
        Ok(Some(result))
    }
}

impl McpToolProvider {
    fn validate_paths(&self, tool_name: &str, arguments: &Value) -> Result<()> {
        let path_fields = ["path", "source", "destination"];

        for field in &path_fields {
            if let Some(path_str) = arguments.get(field).and_then(|v| v.as_str()) {
                self.validate_single_path(path_str)?;
            }
        }

        // Handle paths array (e.g., read_multiple_files)
        if let Some(paths) = arguments.get("paths").and_then(|v| v.as_array()) {
            for path_val in paths {
                if let Some(path_str) = path_val.as_str() {
                    self.validate_single_path(path_str)?;
                }
            }
        }

        let _ = tool_name; // used only in future logging
        Ok(())
    }

    fn validate_single_path(&self, path_str: &str) -> Result<()> {
        let path = Path::new(path_str);

        // Resolve relative to CWD
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Canonicalize if exists, otherwise canonicalize parent
        let canonical = if resolved.exists() {
            resolved.canonicalize()?
        } else if let Some(parent) = resolved.parent() {
            if parent.exists() {
                let canonical_parent = parent.canonicalize()?;
                canonical_parent.join(
                    resolved
                        .file_name()
                        .context("path has no filename component")?,
                )
            } else {
                bail!("parent directory does not exist: {}", parent.display());
            }
        } else {
            bail!("path does not exist and has no parent: {}", path_str);
        };

        // Check against roots
        let in_root = self.config.roots.iter().any(|root| {
            let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
            canonical.starts_with(&canonical_root)
        });

        if !in_root {
            bail!("path is outside allowed roots: {}", path_str);
        }

        // Check deny patterns
        // Match against the path relative to current dir or the path itself
        let relative = path_str.to_string();
        if self.deny_glob.is_match(&relative) {
            bail!("path matches deny pattern: {}", path_str);
        }

        Ok(())
    }

    async fn execute_edit_file(&self, tool_name: &str, arguments: &Value) -> Result<String> {
        let mut client = self.client.lock().await;

        // Step 1: Dry run first
        let mut dry_args = arguments.clone();
        if let Some(obj) = dry_args.as_object_mut() {
            obj.insert("dryRun".into(), Value::Bool(true));
        }

        let dry_result = client.call_tool(tool_name, dry_args).await?;

        // Step 2: Show diff to user
        let confirmed = prompt_edit_confirmation(&dry_result)?;
        if !confirmed {
            return Ok("Tool execution denied by user".into());
        }

        // Step 3: Apply for real
        let mut real_args = arguments.clone();
        if let Some(obj) = real_args.as_object_mut() {
            obj.insert("dryRun".into(), Value::Bool(false));
        }

        client.call_tool(tool_name, real_args).await
    }
}

fn is_write_tool(name: &str) -> bool {
    WRITE_TOOL_NAMES.contains(&name)
}

fn prompt_confirmation(tool_name: &str, arguments: &Value) -> Result<bool> {
    let path = arguments
        .get("path")
        .or_else(|| arguments.get("source"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    println!("Sparrow wants to call filesystem tool:");
    println!("  tool: {tool_name}");
    println!("  path: {path}");
    println!("  mode: write");
    println!();
    print!("Approve? [y/N] ");

    use std::io::{self, Write};
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_lowercase();

    Ok(answer == "y" || answer == "yes")
}

fn prompt_edit_confirmation(dry_run_result: &str) -> Result<bool> {
    println!("Preview:");
    println!("{dry_run_result}");
    println!();
    print!("Apply changes? [y/N] ");

    use std::io::{self, Write};
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_lowercase();

    Ok(answer == "y" || answer == "yes")
}
