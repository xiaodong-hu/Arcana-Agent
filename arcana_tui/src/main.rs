mod app;
mod banner;
mod cli;
mod config;
mod event;
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
    use crate::cli::AuthArgs;

    pub async fn run(_args: AuthArgs) -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("Auth management not yet implemented.");
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
