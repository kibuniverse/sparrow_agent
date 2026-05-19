use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{console::read_secret_input, tool_result_processor::DEFAULT_TOOL_RESULT_MAX_CHARS};

const DEFAULT_MODEL: &str = "deepseek-v4-pro";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant. when read the file, ignore the build targer files like target dir in rust project, ouptput or dist dir in frontend project.  do not reade the entire project dir tree. read the file in entry file first.";
const DEFAULT_REASONING_EFFORT: &str = "high";
const DEFAULT_MAX_TOOL_ROUNDS: usize = 100;
const CONFIG_DIR_NAME: &str = ".sparrow_agent";
const CONFIG_FILE_NAME: &str = "config.json";

const DEFAULT_MAX_READ_BYTES: u64 = 262_144;
const DEFAULT_MAX_WRITE_BYTES: u64 = 262_144;
const DEFAULT_TOOL_OUTPUT_DIR: &str = ".sparrow_agent/tool_outputs";
const DEFAULT_BASH_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_BASH_MAX_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_BASH_MAX_COMMAND_CHARS: usize = 8_192;
const DEFAULT_BASH_STREAM_MAX_BYTES: usize = 8 * 1024;
const DEFAULT_BASH_APPROVAL_POLICY_TTL_DAYS: u64 = 90;
const DEFAULT_BASH_MODEL_LOW_RISK_THRESHOLD: f32 = 0.85;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub api_key: String,
    pub tavily_api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub reasoning_effort: String,
    pub max_tool_rounds: usize,
    pub filesystem: FilesystemConfig,
    pub mcp_servers: Vec<McpServerConfig>,
    pub tool_results: ToolResultConfig,
    pub streaming: StreamingConfig,
    pub bash: BashConfig,
}

impl AppConfig {
    pub fn load_or_initialize() -> Result<Self> {
        let config_path = config_path()?;
        let mut stored_config = StoredConfig::load(&config_path)?;
        let mut should_save = false;
        let mut setup_header_printed = false;

        let api_key = match read_env_value("DEEPSEEK_API_KEY").or_else(|| {
            stored_config
                .deepseek_api_key
                .as_deref()
                .and_then(clean_value)
        }) {
            Some(value) => value,
            None => {
                print_setup_header_once(&config_path, &mut setup_header_printed);
                let value = prompt_api_key("DEEPSEEK_API_KEY")?;
                stored_config.deepseek_api_key = Some(value.clone());
                should_save = true;
                value
            }
        };

        let tavily_api_key = match read_env_value("TAVILY_API_KEY").or_else(|| {
            stored_config
                .tavily_api_key
                .as_deref()
                .and_then(clean_value)
        }) {
            Some(value) => value,
            None => {
                print_setup_header_once(&config_path, &mut setup_header_printed);
                let value = prompt_api_key("TAVILY_API_KEY")?;
                stored_config.tavily_api_key = Some(value.clone());
                should_save = true;
                value
            }
        };

        if should_save {
            stored_config.save(&config_path)?;
            println!("Configuration saved to {}", config_path.display());
        }

        Ok(Self {
            api_key,
            tavily_api_key,
            model: DEFAULT_MODEL.into(),
            system_prompt: default_system_prompt(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.into(),
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            filesystem: FilesystemConfig::from_env(),
            mcp_servers: vec![McpServerConfig::default_filesystem()],
            tool_results: ToolResultConfig::from_env(),
            streaming: StreamingConfig::from_env(),
            bash: BashConfig::from_env(),
        })
    }

    pub fn from_env() -> Result<Self> {
        let api_key = env::var("DEEPSEEK_API_KEY")
            .context("DEEPSEEK_API_KEY environment variable is not set")?;
        let tavily_api_key =
            env::var("TAVILY_API_KEY").context("TAVILY_API_KEY environment variable is not set")?;

        Ok(Self {
            api_key,
            tavily_api_key,
            model: DEFAULT_MODEL.into(),
            system_prompt: default_system_prompt(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.into(),
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            filesystem: FilesystemConfig::from_env(),
            mcp_servers: vec![McpServerConfig::default_filesystem()],
            tool_results: ToolResultConfig::from_env(),
            streaming: StreamingConfig::from_env(),
            bash: BashConfig::from_env(),
        })
    }

    pub fn without_interactive_tools(mut self) -> Self {
        self.bash.enabled = false;
        self
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deepseek_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tavily_api_key: Option<String>,
}

impl StoredConfig {
    fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }

        let contents = serde_json::to_string_pretty(self).context("failed to serialize config")?;

        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
                .with_context(|| format!("failed to open config file {}", path.display()))?;
            file.write_all(contents.as_bytes())
                .with_context(|| format!("failed to write config file {}", path.display()))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
                format!("failed to set config file permissions {}", path.display())
            })?;
        }

        #[cfg(not(unix))]
        {
            fs::write(path, contents)
                .with_context(|| format!("failed to write config file {}", path.display()))?;
        }

        Ok(())
    }
}

fn read_env_value(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| clean_value(&value))
}

fn clean_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn prompt_api_key(name: &str) -> Result<String> {
    println!("{name} was not found. Enter it to continue.");

    loop {
        let Some(input) = read_secret_input(&format!("{name}: "))
            .with_context(|| format!("failed to read {name}"))?
        else {
            bail!("{name} input was closed");
        };

        match clean_value(&input) {
            Some(value) => return Ok(value),
            None => println!("{name} cannot be empty."),
        }
    }
}

fn print_setup_header_once(config_path: &Path, printed: &mut bool) {
    if *printed {
        return;
    }

    println!("First-time setup: Sparrow Agent needs API keys for DeepSeek and Tavily.");
    println!(
        "Values from environment variables are used first; missing values will be saved to {}.",
        config_path.display()
    );
    *printed = true;
}

fn default_system_prompt() -> String {
    let current_dir = env::current_dir().ok();
    let executable_path = env::current_exe().ok();

    system_prompt_with_runtime_paths(
        DEFAULT_SYSTEM_PROMPT,
        current_dir.as_deref(),
        executable_path.as_deref(),
    )
}

fn system_prompt_with_runtime_paths(
    base_prompt: &str,
    current_dir: Option<&Path>,
    executable_path: Option<&Path>,
) -> String {
    let mut prompt = base_prompt.trim_end().to_string();
    prompt.push_str("\n\nRuntime file system context:");

    if let Some(current_dir) = current_dir {
        prompt.push_str(&format!(
            "\n- Current working directory: {}",
            current_dir.display()
        ));
    }

    if let Some(executable_path) = executable_path {
        prompt.push_str(&format!(
            "\n- Running Sparrow Agent executable path: {}",
            executable_path.display()
        ));
    }

    prompt.push_str(
        "\nUse these exact paths when referring to the agent's local file system. Do not invent or guess absolute paths.",
    );
    prompt
}

fn config_path() -> Result<PathBuf> {
    if let Some(path) = env::var("SPARROW_CONFIG_PATH")
        .ok()
        .and_then(|value| clean_value(&value))
    {
        return Ok(PathBuf::from(path));
    }

    let home = env::var("HOME").context("HOME environment variable is not set")?;
    if home.trim().is_empty() {
        bail!("HOME environment variable is empty");
    }

    Ok(Path::new(&home)
        .join(CONFIG_DIR_NAME)
        .join(CONFIG_FILE_NAME))
}

// ── Filesystem config ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FilesystemConfig {
    pub enabled: bool,
    pub roots: Vec<PathBuf>,
    pub mode: FilesystemMode,
    pub confirm: ConfirmationPolicy,
    pub deny_patterns: Vec<String>,
    pub max_read_bytes: u64,
    pub max_write_bytes: u64,
}

impl FilesystemConfig {
    pub fn from_env() -> Self {
        let enabled = env::var("SPARROW_FILESYSTEM_ENABLED")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(true);

        let roots = env::var("SPARROW_FILESYSTEM_ROOTS")
            .ok()
            .map(|v| {
                v.split(if cfg!(windows) { ';' } else { ':' })
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_else(|| vec![PathBuf::from(".")]);

        let mode = env::var("SPARROW_FILESYSTEM_MODE")
            .ok()
            .and_then(|v| match v.as_str() {
                "read-only" => Some(FilesystemMode::ReadOnly),
                "read-write" => Some(FilesystemMode::ReadWrite),
                _ => None,
            })
            .unwrap_or(FilesystemMode::ReadWrite);

        let confirm = env::var("SPARROW_FILESYSTEM_CONFIRM")
            .ok()
            .and_then(|v| match v.as_str() {
                "never" => Some(ConfirmationPolicy::Never),
                "writes" => Some(ConfirmationPolicy::Writes),
                "always" => Some(ConfirmationPolicy::Always),
                _ => None,
            })
            .unwrap_or(ConfirmationPolicy::Writes);

        Self {
            enabled,
            roots,
            mode,
            confirm,
            deny_patterns: default_deny_patterns(),
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            max_write_bytes: DEFAULT_MAX_WRITE_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationPolicy {
    Never,
    Writes,
    Always,
}

impl ConfirmationPolicy {
    pub fn should_confirm(&self, is_write: bool) -> bool {
        match self {
            Self::Never => false,
            Self::Writes => is_write,
            Self::Always => true,
        }
    }
}

// ── Bash config ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashApprovalMode {
    AlwaysPrompt,
    Smart,
    NeverPrompt,
}

impl BashApprovalMode {
    pub fn from_str(value: &str) -> Self {
        match value {
            "always" => Self::AlwaysPrompt,
            "smart" => Self::Smart,
            "never" => Self::NeverPrompt,
            _ => Self::Smart,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlwaysPrompt => "always",
            Self::Smart => "smart",
            Self::NeverPrompt => "never",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BashConfig {
    pub enabled: bool,
    pub roots: Vec<PathBuf>,
    pub approval_mode: BashApprovalMode,
    pub approval_policy_path: PathBuf,
    pub approval_policy_ttl_days: u64,
    pub model_low_risk_threshold: f32,
    pub timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub max_command_chars: usize,
    pub stream_max_bytes: usize,
    pub env_allowlist: Vec<String>,
}

impl BashConfig {
    pub fn from_env() -> Self {
        let enabled = env::var("SPARROW_BASH_ENABLED")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);

        let roots = read_env_value("SPARROW_BASH_ROOTS")
            .map(|value| split_paths(&value))
            .filter(|roots| !roots.is_empty())
            .unwrap_or_else(|| vec![PathBuf::from(".")]);

        let approval_mode = read_env_value("SPARROW_BASH_APPROVAL_MODE")
            .map(|value| BashApprovalMode::from_str(&value))
            .unwrap_or(BashApprovalMode::Smart);

        let approval_policy_path = read_env_value("SPARROW_BASH_APPROVAL_POLICY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(default_bash_approval_policy_path);

        let approval_policy_ttl_days = read_env_value("SPARROW_BASH_APPROVAL_POLICY_TTL_DAYS")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_BASH_APPROVAL_POLICY_TTL_DAYS);

        let model_low_risk_threshold = read_env_value("SPARROW_BASH_MODEL_LOW_RISK_THRESHOLD")
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| (0.0..=1.0).contains(value))
            .unwrap_or(DEFAULT_BASH_MODEL_LOW_RISK_THRESHOLD);

        let timeout_ms = read_env_value("SPARROW_BASH_TIMEOUT_MS")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(|value| value.min(DEFAULT_BASH_MAX_TIMEOUT_MS))
            .unwrap_or(DEFAULT_BASH_TIMEOUT_MS);

        let max_command_chars = read_env_value("SPARROW_BASH_MAX_COMMAND_CHARS")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_BASH_MAX_COMMAND_CHARS);

        let stream_max_bytes = read_env_value("SPARROW_BASH_STREAM_MAX_BYTES")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_BASH_STREAM_MAX_BYTES);

        let env_allowlist = read_env_value("SPARROW_BASH_ENV_ALLOWLIST")
            .map(|value| split_csv(&value))
            .filter(|values| !values.is_empty())
            .unwrap_or_else(default_bash_env_allowlist);

        Self {
            enabled,
            roots,
            approval_mode,
            approval_policy_path,
            approval_policy_ttl_days,
            model_low_risk_threshold,
            timeout_ms,
            max_timeout_ms: DEFAULT_BASH_MAX_TIMEOUT_MS,
            max_command_chars,
            stream_max_bytes,
            env_allowlist,
        }
    }
}

fn default_bash_approval_policy_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home)
        .join(CONFIG_DIR_NAME)
        .join("bash_approval_policies.json")
}

fn split_paths(value: &str) -> Vec<PathBuf> {
    value
        .split(if cfg!(windows) { ';' } else { ':' })
        .filter_map(clean_value)
        .map(PathBuf::from)
        .collect()
}

fn split_csv(value: &str) -> Vec<String> {
    value.split(',').filter_map(clean_value).collect()
}

fn default_bash_env_allowlist() -> Vec<String> {
    ["PATH", "HOME", "USER", "TERM", "TMPDIR"]
        .into_iter()
        .map(String::from)
        .collect()
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
}

impl McpServerConfig {
    pub fn default_filesystem() -> Self {
        let command = env::var("SPARROW_MCP_FILESYSTEM_COMMAND")
            .ok()
            .unwrap_or_else(|| "npx".into());

        let path = env::current_dir();
        let current_dir = path.unwrap().to_string_lossy().into_owned();
        let args = env::var("SPARROW_MCP_FILESYSTEM_ARGS")
            .ok()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_else(|| {
                vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into(),
                    current_dir,
                ]
            });

        Self {
            id: "filesystem".into(),
            command,
            args,
            enabled: true,
        }
    }
}

// ── Tool result config ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolResultConfig {
    pub max_injected_chars: usize,
    pub output_dir: PathBuf,
}

impl ToolResultConfig {
    pub fn from_env() -> Self {
        let max_injected_chars = env::var("SPARROW_TOOL_RESULT_MAX_CHARS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_TOOL_RESULT_MAX_CHARS);

        let output_dir = env::var("SPARROW_TOOL_OUTPUT_DIR")
            .ok()
            .and_then(|value| clean_value(&value))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOOL_OUTPUT_DIR));

        Self {
            max_injected_chars,
            output_dir,
        }
    }
}

// ── Streaming config ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StreamingConfig {
    pub enabled: bool,
    pub show_reasoning: bool,
    pub show_tool_call_deltas: bool,
}

impl StreamingConfig {
    pub fn from_env() -> Self {
        let enabled = env::var("SPARROW_STREAMING_ENABLED")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(true);

        let show_reasoning = env::var("SPARROW_SHOW_REASONING")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(true);

        let show_tool_call_deltas = env::var("SPARROW_SHOW_TOOL_CALL_DELTAS")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);

        Self {
            enabled,
            show_reasoning,
            show_tool_call_deltas,
        }
    }
}

fn default_deny_patterns() -> Vec<String> {
    vec![
        ".git/**".into(),
        ".env".into(),
        ".env.*".into(),
        "**/id_rsa".into(),
        "**/id_ed25519".into(),
        "**/*.pem".into(),
        "**/*.key".into(),
        ".sparrow_agent/**".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_approval_mode_from_str_parses_known_values() {
        assert_eq!(
            BashApprovalMode::from_str("always"),
            BashApprovalMode::AlwaysPrompt
        );
        assert_eq!(BashApprovalMode::from_str("smart"), BashApprovalMode::Smart);
        assert_eq!(
            BashApprovalMode::from_str("never"),
            BashApprovalMode::NeverPrompt
        );
    }

    #[test]
    fn bash_approval_mode_from_str_defaults_to_smart() {
        assert_eq!(
            BashApprovalMode::from_str("surprise"),
            BashApprovalMode::Smart
        );
    }

    #[test]
    fn bash_config_from_env_uses_smart_approval_by_default() {
        let config = BashConfig::from_env();

        assert_eq!(config.approval_mode, BashApprovalMode::Smart);
        assert_eq!(config.approval_policy_ttl_days, 90);
        assert_eq!(config.model_low_risk_threshold, 0.85);
    }

    #[test]
    fn system_prompt_includes_runtime_file_system_paths() {
        let prompt = system_prompt_with_runtime_paths(
            "base prompt",
            Some(Path::new("/workspace/sparrow_agent")),
            Some(Path::new(
                "/workspace/sparrow_agent/target/debug/sparrow_agent",
            )),
        );

        assert!(prompt.contains("base prompt"));
        assert!(prompt.contains("Current working directory: /workspace/sparrow_agent"));
        assert!(
            prompt.contains(
                "Running Sparrow Agent executable path: /workspace/sparrow_agent/target/debug/sparrow_agent"
            )
        );
        assert!(prompt.contains("Do not invent or guess absolute paths."));
    }
}
