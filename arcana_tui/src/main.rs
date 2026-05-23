mod app;
mod approval;
mod banner;
mod cli;
mod composer;
mod config;
mod diff_panel;
mod event;
mod highlight;
mod llm;
mod onboard;
mod overlay;
mod panels;
mod render_md;
mod status_bar;
mod theme;
mod tui;
mod types;
mod viewport;

use clap::Parser;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use cli::{Cli, Command};
use config::Config;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Handle --reset: remove project workspace or factory config with confirmation.
    if cli.reset {
        if cli.factory {
            factory_reset_with_confirmation();
        } else {
            let project = cli
                .project
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            project_reset_with_confirmation(&project);
        }
        // Reset is a one-shot operation — exit after completion.
        process::exit(0);
    }

    // --factory without --reset is an error.
    if cli.factory {
        eprintln!("[Arcana] Error: `--factory` requires `--reset`.");
        eprintln!("Usage: arcana --reset           → reset project workspace ./.arcana/");
        eprintln!("       arcana --reset --factory → reset global config ~/.arcana/");
        process::exit(1);
    }

    // Ensure ~/.arcana exists on every launch
    if let Err(e) = Config::ensure_home() {
        eprintln!("[Arcana] Failed to create ~/.arcana: {}", e);
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
            // Resolve project path — may prompt user interactively.
            let project = match resolve_project_path(cli.project) {
                Some(p) => p,
                None => process::exit(0),
            };
            // All relative paths (`.arcana/...`) resolve against the project root.
            if let Err(e) = std::env::set_current_dir(&project) {
                eprintln!("[Arcana] Cannot enter {:?}: {}", project, e);
                process::exit(1);
            }
            if let Some(query) = cli.query {
                app::single_shot(&query, &cli.model, &cli.provider).await
            } else {
                app::interactive(cli.model, cli.provider).await
            }
        }
    };

    if let Err(e) = result {
        eprintln!("[Arcana] Error: {}", e);
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Project path resolution with interactive confirmation
// ---------------------------------------------------------------------------

/// Resolve the project root directory, prompting the user when necessary.
///
/// - If `project_arg` is `Some(path)`: use that path directly (canonicalised).
/// - If `project_arg` is `None`: ask the user whether to use the current
///   working directory (`y`/`c` = yes, `n`/`q` = quit).
///
/// Once a path is settled, if it lacks a `.arcana/` workspace the user is
/// asked whether to create one.  Returns `None` when the user declines.
fn resolve_project_path(project_arg: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(ref path) = project_arg {
        let canonical = canonicalize(path);
        ensure_workspace(&canonical)
    } else {
        let cwd = std::env::current_dir().ok()?;
        eprintln!();
        eprintln!(
            "Project path for `Arcana-Agent` NOT specified. \
             Set current path `{}` to launch?",
            cwd.display()
        );
        eprint!("    - Yes and Continue  [y/c]\n    - No and Quit       [n/q]\n> ");
        io::stderr().flush().ok();

        let mut input = String::new();
        io::stdin().read_line(&mut input).ok()?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" | "c" | "continue" => ensure_workspace(&cwd),
            _ => {
                eprintln!("Aborted.");
                None
            }
        }
    }
}

/// Check whether `path/.arcana` exists; if not, ask the user to create it.
fn ensure_workspace(path: &Path) -> Option<PathBuf> {
    let arcana_dir = path.join(".arcana");
    if arcana_dir.exists() {
        return Some(path.to_path_buf());
    }

    eprintln!();
    eprintln!(
        "`Arcana-Agent` launch for the first time in this project. \
         Create a project-level workspace `{}/.arcana`?",
        path.display()
    );
    eprint!("    - Yes and Continue  [y/c]\n    - No and Quit       [n/q]\n> ");
    io::stderr().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok()?;
    match input.trim().to_lowercase().as_str() {
        "y" | "yes" | "c" | "continue" => {
            if let Err(e) = std::fs::create_dir_all(&arcana_dir) {
                eprintln!("[Arcana] Failed to create workspace: {}", e);
                return None;
            }
            eprintln!("[Arcana] Created workspace at {}", arcana_dir.display());
            Some(path.to_path_buf())
        }
        _ => {
            eprintln!("Aborted.");
            None
        }
    }
}

/// Best-effort canonicalisation; falls back to the original path.
fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Reset operations with interactive confirmation
// ---------------------------------------------------------------------------

/// Reset the project-level workspace `./.arcana/` after user confirmation.
fn project_reset_with_confirmation(project: &Path) {
    let arcana_dir = canonicalize(project).join(".arcana");

    if !arcana_dir.exists() {
        eprintln!(
            "[Arcana] No workspace found at {}. Nothing to reset.",
            arcana_dir.display()
        );
        return;
    }

    eprintln!();
    eprintln!(
        "You are about to DELETE the project workspace at\n  {}/\n\
         This includes ALL session history, access rules, and cached data for this project.\n[WARNING] This action CANNOT be undone!\n",
        arcana_dir.display()
    );
    eprint!("Type 'yes' to confirm, anything else to abort: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        eprintln!("\n[Arcana] Aborted.");
        return;
    }
    if input.trim() != "yes" {
        eprintln!("[Arcana] Aborted.");
        return;
    }

    match std::fs::remove_dir_all(&arcana_dir) {
        Ok(()) => eprintln!("[Arcana] Removed {}", arcana_dir.display()),
        Err(e) => eprintln!("[Arcana] Failed to remove workspace: {}", e),
    }
}

/// Reset the global `~/.arcana/` directory after extra warning + confirmation.
fn factory_reset_with_confirmation() {
    let home = match dirs::home_dir() {
        Some(d) => d,
        None => {
            eprintln!("[Arcana] Cannot determine home directory.");
            return;
        }
    };
    let global_dir = home.join(".arcana");

    if !global_dir.exists() {
        eprintln!(
            "[Arcana] No global config found at {}. Nothing to reset.",
            global_dir.display()
        );
        return;
    }

    eprintln!();
    eprintln!("╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║                      ⚠  FACTORY  RESET  ⚠                    ║");
    eprintln!("╠══════════════════════════════════════════════════════════════╣");
    eprintln!("║  You are about to DELETE the ENTIRE Arcana configuration:    ║");
    eprintln!("║                                                              ║");
    eprintln!("║    {:<54}    ║", global_dir.display());
    eprintln!("║                                                              ║");
    eprintln!("║  This includes:                                              ║");
    eprintln!("║    • Global config (providers, models, API keys)             ║");
    eprintln!("║    • Agent personality (SOUL.md)                             ║");
    eprintln!("║    • User portrait (USER.md)                                 ║");
    eprintln!("║    • Authority rules (~/.arcana/authority.toml)              ║");
    eprintln!("║    • Knowledge & error memory databases                      ║");
    eprintln!("║    • All installed skills                                    ║");
    eprintln!("║    • Embedding model                                         ║");
    eprintln!("║                                                              ║");
    eprintln!("║  [WARNING] This action CANNOT be undone!                     ║");
    eprintln!("╚══════════════════════════════════════════════════════════════╝");
    eprintln!();
    eprint!("Type 'YES DELETE' (exactly) to confirm, anything else to abort: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        eprintln!("\n[Arcana] Aborted.");
        return;
    }
    if input.trim() != "YES DELETE" {
        eprintln!("[Arcana] Aborted.");
        return;
    }

    match std::fs::remove_dir_all(&global_dir) {
        Ok(()) => {
            eprintln!(
                "[Arcana] Removed {}. Run `arcana onboard` to set up again.",
                global_dir.display()
            );
        }
        Err(e) => eprintln!("[Arcana] Failed to remove global config: {}", e),
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
    use crate::cli::{AuthAction, AuthArgs};
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
            "cargo build",
            "cargo test",
            "cargo clippy",
            "cargo fmt",
            "git status",
            "git diff",
            "git log",
            "ls",
            "cat",
            "find",
            "grep",
            "rg",
            "curl",
            "wget",
            "w3m",
            "python3",
            "node",
            "make",
            "head",
            "tail",
            "wc",
            "sort",
            "uniq",
            "sed",
            "awk",
            "jq",
            "tree",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    fn default_confirm() -> Vec<String> {
        vec!["git push", "git commit", "rm -rf", "sudo *"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn default_network_allow() -> Vec<String> {
        vec![
            "api.deepseek.com",
            "api.openai.com",
            "api.anthropic.com",
            "scholar.google.com",
            "arxiv.org",
            "*.arxiv.org",
            "en.wikipedia.org",
            "*.wikipedia.org",
            "wiki.archlinux.org",
            "stackoverflow.com",
            "*.stackoverflow.com",
            "*.stackexchange.com",
            "superuser.com",
            "docs.rs",
            "crates.io",
            "github.com",
            "raw.githubusercontent.com",
            "gitlab.com",
            "pkg.go.dev",
            "pypi.org",
            "npmjs.com",
            "zhihu.com",
            "*.zhihu.com",
            "juejin.cn",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    fn default_network_deny() -> Vec<String> {
        vec!["*".into()]
    }

    fn default_writable() -> Vec<String> {
        vec![".".into()]
    }

    fn default_fs_deny() -> Vec<String> {
        vec!["~/.ssh", "~/.gnupg", "~/.arcana/authority.toml"]
            .into_iter()
            .map(String::from)
            .collect()
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
    use crate::cli::{ConfigAction, ConfigArgs};
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
                let status = std::process::Command::new(&editor).arg(&path).status()?;
                if !status.success() {
                    eprintln!("[Arcana] Editor exited with non-zero status");
                }
            }
        }
        Ok(())
    }
}
