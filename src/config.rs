use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::console::read_secret_input;

const DEFAULT_MODEL: &str = "deepseek-v4-flash";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const DEFAULT_REASONING_EFFORT: &str = "high";
const DEFAULT_MAX_TOOL_ROUNDS: usize = 100;
const CONFIG_DIR_NAME: &str = ".sparrow_agent";
const CONFIG_FILE_NAME: &str = "config.json";

const DEFAULT_MAX_READ_BYTES: u64 = 262_144;
const DEFAULT_MAX_WRITE_BYTES: u64 = 262_144;

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
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.into(),
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            filesystem: FilesystemConfig::from_env(),
            mcp_servers: vec![McpServerConfig::default_filesystem()],
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
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.into(),
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            filesystem: FilesystemConfig::from_env(),
            mcp_servers: vec![McpServerConfig::default_filesystem()],
        })
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
            .unwrap_or(FilesystemMode::ReadOnly);

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

        let args = env::var("SPARROW_MCP_FILESYSTEM_ARGS")
            .ok()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_else(|| {
                vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into(),
                    "/Users/yankaizhi/RustProjects/sparrow_agent".into(),
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
