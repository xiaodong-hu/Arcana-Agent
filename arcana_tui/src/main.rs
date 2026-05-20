mod app;
mod banner;
mod cli;
mod config;
mod event;
mod llm;
mod onboard;
mod status_bar;
mod theme;
mod tui;
mod types;
mod viewport;
mod composer;
mod overlay;
mod panels;

use clap::Parser;
use std::process;

use cli::{Cli, Command};
use config::Config;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Handle --reset: remove ~/.arcana then recreate
    if cli.reset {
        if let Err(e) = Config::reset() {
            eprintln!("[arcana] Failed to reset config: {}", e);
            process::exit(1);
        }
        println!("[arcana] Configuration reset.");
    }

    // Ensure ~/.arcana exists on every launch
    if let Err(e) = Config::ensure_home() {
        eprintln!("[arcana] Failed to create ~/.arcana: {}", e);
        process::exit(1);
    }

    let result = match cli.command {
        Some(Command::Onboard(args)) => onboard::run(args).await,
        Some(Command::Resume(args)) => app::resume(args).await,
        Some(Command::Recover(args)) => {
            eprintln!("Recovery delegates to authority_and_record binary.");
            eprintln!("Run: authority_and_record recover {:?}", args.project);
            Ok(())
        }
        Some(Command::Check) => check::run().await,
        Some(Command::Version) => {
            println!("arcana {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Command::Session(args)) => session_cmd::run(args).await,
        Some(Command::Auth(args)) => auth_cmd::run(args).await,
        Some(Command::Config(args)) => config_cmd::run(args).await,
        None => {
            if let Some(query) = cli.query {
                app::single_shot(&query, &cli.model, &cli.provider).await
            } else {
                app::interactive(cli.model, cli.provider).await
            }
        }
    };

    if let Err(e) = result {
        eprintln!("[arcana] Error: {}", e);
        process::exit(1);
    }
}

mod check {
    use crate::config::Config;

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let home = dirs::home_dir().ok_or("cannot find home directory")?;
        let arcana_home = home.join(".arcana");

        println!();

        let config_path = arcana_home.join("config.toml");
        if config_path.exists() {
            println!("  ✓ Global config (~/.arcana/config.toml)");
        } else {
            println!("  ✗ Global config (~/.arcana/config.toml not found)");
            println!("    → Run: arcana onboard");
        }

        let model_path = arcana_home.join("models").join("all-MiniLM-L6-v2.onnx");
        if model_path.exists() {
            println!("  ✓ Embedding model (all-MiniLM-L6-v2.onnx)");
        } else {
            println!("  ✗ Embedding model (not found)");
            println!("    → Run: arcana onboard");
        }

        if std::env::var("DEEPSEEK_API_KEY").is_ok() {
            println!("  ✓ DeepSeek API key (from DEEPSEEK_API_KEY env var)");
        } else if config_path.exists() {
            let cfg = Config::load()?;
            if cfg.providers.deepseek.api_key.is_empty() {
                println!("  ✗ DeepSeek API key (not configured)");
                println!("    → Set DEEPSEEK_API_KEY or run: arcana auth set --provider deepseek");
            } else {
                println!("  ✓ DeepSeek API key (from config.toml)");
            }
        } else {
            println!("  ✗ DeepSeek API key (not configured)");
        }

        let cwd = std::env::current_dir()?;
        if cwd.join(".arcana").exists() {
            println!("  ✓ Workspace (.arcana/ exists)");
        } else {
            println!("  ○ Workspace (.arcana/ not found in current directory)");
        }

        println!();
        Ok(())
    }
}

mod session_cmd {
    use crate::cli::SessionArgs;

    pub async fn run(_args: SessionArgs) -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("Session management not yet implemented.");
        Ok(())
    }
}

mod auth_cmd {
    use crate::cli::{AuthArgs, AuthAction};
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AuthorityConfig {
        #[serde(default)]
        pub commands: CommandsConfig,
        #[serde(default)]
        pub network: NetworkConfig,
        #[serde(default)]
        pub filesystem: FilesystemConfig,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct CommandsConfig {
        #[serde(default = "default_allow")]
        pub allow: Vec<String>,
        #[serde(default = "default_confirm")]
        pub confirm: Vec<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct NetworkConfig {
        #[serde(default = "default_network_allow")]
        pub allow: Vec<String>,
        #[serde(default = "default_network_deny")]
        pub deny: Vec<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct FilesystemConfig {
        #[serde(default = "default_writable")]
        pub writable: Vec<String>,
        #[serde(default)]
        pub readonly: Vec<String>,
        #[serde(default = "default_fs_deny")]
        pub deny: Vec<String>,
    }

    fn default_allow() -> Vec<String> {
        vec![
            "cargo build", "cargo test", "cargo clippy", "cargo fmt",
            "git status", "git diff", "git log",
            "ls", "cat", "find", "grep", "rg",
        ].into_iter().map(String::from).collect()
    }

    fn default_confirm() -> Vec<String> {
        vec!["git push", "git commit", "rm -rf", "sudo *"]
            .into_iter().map(String::from).collect()
    }

    fn default_network_allow() -> Vec<String> {
        vec!["api.deepseek.com", "api.openai.com", "api.anthropic.com"]
            .into_iter().map(String::from).collect()
    }

    fn default_network_deny() -> Vec<String> {
        vec!["*".into()]
    }

    fn default_writable() -> Vec<String> {
        vec![".".into()]
    }

    fn default_fs_deny() -> Vec<String> {
        vec!["~/.ssh", "~/.gnupg", "~/.arcana/authority.toml"]
            .into_iter().map(String::from).collect()
    }

    impl Default for AuthorityConfig {
        fn default() -> Self {
            Self {
                commands: CommandsConfig {
                    allow: default_allow(),
                    confirm: default_confirm(),
                },
                network: NetworkConfig {
                    allow: default_network_allow(),
                    deny: default_network_deny(),
                },
                filesystem: FilesystemConfig {
                    writable: default_writable(),
                    readonly: vec!["/etc".into(), "/usr".into()],
                    deny: default_fs_deny(),
                },
            }
        }
    }

    fn authority_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = dirs::home_dir().ok_or("cannot find home directory")?;
        Ok(home.join(".arcana").join("authority.toml"))
    }

    fn load() -> Result<AuthorityConfig, Box<dyn std::error::Error>> {
        let path = authority_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(AuthorityConfig::default())
        }
    }

    fn save(config: &AuthorityConfig) -> Result<(), Box<dyn std::error::Error>> {
        let path = authority_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(config)?)?;
        Ok(())
    }

    pub async fn run(args: AuthArgs) -> Result<(), Box<dyn std::error::Error>> {
        match args.action {
            Some(AuthAction::Status) | None => {
                let config = load()?;
                let path = authority_path()?;
                println!("  Authority config: {}\n", path.display());
                println!("  [commands.allow]");
                for cmd in &config.commands.allow {
                    println!("    ✓ {}", cmd);
                }
                println!("\n  [commands.confirm]");
                for cmd in &config.commands.confirm {
                    println!("    ⚠ {}", cmd);
                }
                println!("\n  [network.allow]");
                for host in &config.network.allow {
                    println!("    ✓ {}", host);
                }
                println!("\n  [network.deny]");
                for host in &config.network.deny {
                    println!("    ✗ {}", host);
                }
                println!("\n  [filesystem.writable]");
                for p in &config.filesystem.writable {
                    println!("    ✓ {}", p);
                }
                println!("\n  [filesystem.deny]");
                for p in &config.filesystem.deny {
                    println!("    ✗ {}", p);
                }
                println!();
            }
            Some(AuthAction::Allow { pattern }) => {
                let mut config = load()?;
                if !config.commands.allow.contains(&pattern) {
                    config.commands.allow.push(pattern.clone());
                    save(&config)?;
                    println!("  ✓ Added to allow list: {}", pattern);
                } else {
                    println!("  Already in allow list: {}", pattern);
                }
            }
            Some(AuthAction::Deny { pattern }) => {
                let mut config = load()?;
                if !config.commands.confirm.contains(&pattern) {
                    config.commands.confirm.push(pattern.clone());
                    save(&config)?;
                    println!("  ✓ Added to confirm list: {}", pattern);
                } else {
                    println!("  Already in confirm list: {}", pattern);
                }
            }
            Some(AuthAction::Revoke { pattern }) => {
                let mut config = load()?;
                let before = config.commands.allow.len();
                config.commands.allow.retain(|c| c != &pattern);
                if config.commands.allow.len() < before {
                    save(&config)?;
                    println!("  ✓ Revoked from allow list: {}", pattern);
                } else {
                    println!("  Not found in allow list: {}", pattern);
                }
            }
            Some(AuthAction::Reset) => {
                let config = AuthorityConfig::default();
                save(&config)?;
                println!("  ✓ Authority config reset to defaults.");
            }
        }
        Ok(())
    }
}

mod config_cmd {
    use crate::cli::{ConfigArgs, ConfigAction};
    use crate::config::Config;

    pub async fn run(args: ConfigArgs) -> Result<(), Box<dyn std::error::Error>> {
        match args.action {
            Some(ConfigAction::Show) | None => {
                let cfg = Config::load()?;
                cfg.show()?;
            }
            Some(ConfigAction::Path) => {
                let path = Config::path()?;
                println!("{}", path.display());
            }
            Some(ConfigAction::Edit) => {
                let path = Config::path()?;
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".into());
                let status = std::process::Command::new(&editor)
                    .arg(&path)
                    .status()?;
                if !status.success() {
                    eprintln!("[arcana] Editor exited with non-zero status");
                }
            }
        }
        Ok(())
    }
}
