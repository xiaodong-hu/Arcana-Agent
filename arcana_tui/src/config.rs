use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global configuration loaded from ~/.arcana/config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub editor: EditorConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub session: SessionConfig,
}

/// Per-agent LLM configuration: main-agent, persistent-query-agent, sub-agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default = "AgentLlmConfig::default_main")]
    pub main: AgentLlmConfig,
    #[serde(default = "AgentLlmConfig::default_query")]
    pub query: AgentLlmConfig,
    #[serde(default = "AgentLlmConfig::default_sub")]
    pub sub: AgentLlmConfig,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            main: AgentLlmConfig::default_main(),
            query: AgentLlmConfig::default_query(),
            sub: AgentLlmConfig::default_sub(),
        }
    }
}

/// LLM configuration for a single agent role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLlmConfig {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub thinking: ThinkingConfig,
    #[serde(default)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub temperature: Option<f64>,
}

impl AgentLlmConfig {
    pub fn default_main() -> Self {
        Self {
            provider: "deepseek".into(),
            model: "deepseek-v4-pro".into(),
            thinking: ThinkingConfig::default(),
            max_tokens: None,
            temperature: None,
        }
    }

    pub fn default_query() -> Self {
        Self {
            provider: "deepseek".into(),
            model: "deepseek-v4-pro".into(),
            thinking: ThinkingConfig {
                enabled: true,
                reasoning_effort: "high".into(),
            },
            max_tokens: None,
            temperature: None,
        }
    }

    pub fn default_sub() -> Self {
        Self {
            provider: "deepseek".into(),
            model: "deepseek-v4-flash".into(),
            thinking: ThinkingConfig {
                enabled: true,
                reasoning_effort: "high".into(),
            },
            max_tokens: None,
            temperature: None,
        }
    }
}

/// DeepSeek thinking mode configuration.
/// Maps to API params: `thinking.type`, `reasoning_effort`, `output_config.effort`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    /// Whether thinking mode is enabled (maps to `{"thinking": {"type": "enabled/disabled"}}`)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Reasoning effort level: "high" or "max"
    /// OpenAI format: `reasoning_effort`
    /// Anthropic format: `output_config.effort`
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reasoning_effort: "high".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub deepseek: ProviderEntry,
    #[serde(default)]
    pub openai: ProviderEntry,
    #[serde(default)]
    pub anthropic: ProviderEntry,
    #[serde(default)]
    pub local: ProviderEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
}

impl Default for ProviderEntry {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            models: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub animations: bool,
    #[serde(default = "default_true")]
    pub bell_on_complete: bool,
    #[serde(default = "default_collapsed")]
    pub thinking_default: String,
    #[serde(default = "default_collapsed")]
    pub tool_detail: String,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            animations: true,
            bell_on_complete: true,
            thinking_default: "collapsed".into(),
            tool_detail: "collapsed".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    #[serde(default = "default_editor")]
    pub command: String,
    #[serde(default)]
    pub diff_command: String,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            command: default_editor(),
            diff_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    #[serde(default = "default_true")]
    pub desktop: bool,
    #[serde(default = "default_true")]
    pub bell: bool,
    #[serde(default = "default_toast_duration")]
    pub toast_duration_secs: u64,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            desktop: true,
            bell: true,
            toast_duration_secs: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_max_sessions")]
    pub max_sessions_kept: usize,
    #[serde(default = "default_true")]
    pub auto_freeze_on_disconnect: bool,
    #[serde(default = "default_true")]
    pub crash_recovery_prompt: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_sessions_kept: 50,
            auto_freeze_on_disconnect: true,
            crash_recovery_prompt: true,
        }
    }
}

// Default value helpers
fn default_theme() -> String {
    "arcane".into()
}
fn default_editor() -> String {
    std::env::var("EDITOR").unwrap_or_else(|_| "vim".into())
}
fn default_collapsed() -> String {
    "collapsed".into()
}
fn default_true() -> bool {
    true
}
fn default_reasoning_effort() -> String {
    "high".into()
}
fn default_toast_duration() -> u64 {
    5
}
fn default_max_sessions() -> usize {
    50
}

impl Config {
    /// Load config from ~/.arcana/config.toml
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Save config to ~/.arcana/config.toml
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Get the config file path
    pub fn path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = dirs::home_dir().ok_or("cannot find home directory")?;
        Ok(home.join(".arcana").join("config.toml"))
    }

    /// Ensure ~/.arcana directory exists
    pub fn ensure_home() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = dirs::home_dir().ok_or("cannot find home directory")?;
        let arcana_home = home.join(".arcana");
        std::fs::create_dir_all(&arcana_home)?;
        Ok(arcana_home)
    }

    /// Remove the project-level `.arcana/` workspace (for `--reset`).
    pub fn reset_project() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = std::env::current_dir()?;
        let arcana_dir = cwd.join(".arcana");
        if arcana_dir.exists() {
            std::fs::remove_dir_all(&arcana_dir)?;
        }
        Ok(())
    }

    /// Remove `~/.arcana/` directory (for `--reset --factory`).
    pub fn reset_factory() -> Result<(), Box<dyn std::error::Error>> {
        let home = dirs::home_dir().ok_or("cannot find home directory")?;
        let arcana_home = home.join(".arcana");
        if arcana_home.exists() {
            std::fs::remove_dir_all(&arcana_home)?;
        }
        Ok(())
    }

    /// Resolve the effective API key for a provider (config → env var)
    pub fn resolve_api_key(&self, provider: &str) -> Option<String> {
        let (config_key, env_var) = match provider {
            "deepseek" => (&self.providers.deepseek.api_key, "DEEPSEEK_API_KEY"),
            "openai" => (&self.providers.openai.api_key, "OPENAI_API_KEY"),
            "anthropic" => (&self.providers.anthropic.api_key, "ANTHROPIC_API_KEY"),
            _ => return None,
        };
        if !config_key.is_empty() {
            // Handle $ENV_VAR references in config
            if config_key.starts_with('$') {
                let var_name = &config_key[1..];
                if let Ok(val) = std::env::var(var_name) {
                    return Some(val);
                }
            } else {
                return Some(config_key.clone());
            }
        }
        std::env::var(env_var).ok()
    }

    /// Print config to stdout (for `arcana config show`)
    pub fn show(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::path()?;
        println!("Config path: {}\n", path.display());
        let content = toml::to_string_pretty(self)?;
        println!("{}", content);
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agents: AgentsConfig::default(),
            providers: ProvidersConfig::default(),
            display: DisplayConfig::default(),
            editor: EditorConfig::default(),
            notifications: NotificationsConfig::default(),
            session: SessionConfig::default(),
        }
    }
}
