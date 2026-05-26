mod app;
mod approval;
mod banner;
mod behavioral;
mod cli;
mod composer;
mod config;
mod diff_panel;
mod event;
mod highlight;
mod instruction;
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

    // Enforce global workspace: if ~/.arcana is missing, prompt to create it.
    // This runs for EVERY launch except `onboard` (which creates it itself).
    if !cli
        .command
        .as_ref()
        .is_some_and(|c| matches!(c, Command::Onboard(_)))
    {
        if !ensure_global_workspace() {
            process::exit(0);
        }
    }

    let result = match cli.command {
        Some(Command::Onboard(args)) => onboard::run(args).await,
        Some(Command::Resume(args)) => app::resume(args).await,
        Some(Command::Recover(_)) => Err(
            "`arcana recover` was removed. Use `arcana recovery --list`, then `arcana recovery --to-sequence <N>`.".into()
        ),
        Some(Command::Recovery(args)) => recovery_project(args),
        Some(Command::Completions(args)) => {
            print_completion_script(&args.shell);
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

fn recovery_project(args: cli::RecoveryArgs) -> Result<(), Box<dyn std::error::Error>> {
    let project = args
        .project
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if args.list {
        let binary = app::find_authority_binary()
            .ok_or("cannot find authority_and_recording binary; build it before recovery")?;
        let status = std::process::Command::new(binary)
            .arg("recovery")
            .arg(&project)
            .arg("--list")
            .status()?;
        if !status.success() {
            return Err(format!("authority recovery list exited with status {status}").into());
        }
        return Ok(());
    }
    if args.to_sequence.is_none() {
        return Err("recovery requires `--list` or `--to-sequence <N>`".into());
    }
    if !args.yes && !confirm_recovery_warning(&project, args.to_sequence) {
        return Err("recovery aborted".into());
    }
    let binary = app::find_authority_binary()
        .ok_or("cannot find authority_and_recording binary; build it before recovery")?;
    let mut command = std::process::Command::new(binary);
    command.arg("recovery").arg(&project);
    if let Some(seq) = args.to_sequence {
        command.arg("--to-sequence").arg(seq.to_string());
    }
    command.arg("--yes");
    let status = command.status()?;
    if !status.success() {
        return Err(format!("authority recovery exited with status {status}").into());
    }
    Ok(())
}

fn confirm_recovery_warning(project: &Path, target: Option<u64>) -> bool {
    eprintln!();
    eprintln!("╔════════════════════════════════════════════════════════════════════╗");
    eprintln!("║ WARNING: Arcana recovery will overwrite the working tree.         ║");
    eprintln!("║ Files may be rewritten or removed to match the recorded state.    ║");
    eprintln!("║ The recovery itself is recorded, but unrecorded edits may be lost.║");
    eprintln!("╚════════════════════════════════════════════════════════════════════╝");
    eprintln!("Project: {}", project.display());
    match target {
        Some(seq) => eprintln!("Target record sequence: {seq}"),
        None => eprintln!("Target record sequence: previous sequence"),
    }
    eprint!("Continue recovery? [y/yes to continue]: ");
    io::stderr().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn print_completion_script(shell: &str) {
    match shell {
        "bash" => print!("{}", BASH_COMPLETIONS),
        "zsh" => print!("{}", ZSH_COMPLETIONS),
        "fish" => print!("{}", FISH_COMPLETIONS),
        other => {
            eprintln!("[Arcana] Unsupported shell `{other}`. Use: bash, zsh, fish.");
            process::exit(1);
        }
    }
}

const BASH_COMPLETIONS: &str = r#"_arcana()
{
    local cur prev
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    case "$prev" in
        --model|--provider|--query|-q|recovery|resume|completions)
            return
            ;;
        --to-sequence)
            return
            ;;
    esac
    case "$cur" in
        --*) COMPREPLY=( $(compgen -W "--query --model --provider --accessible --reset --factory --help --version --list --to-sequence --yes" -- "$cur") );;
        *) COMPREPLY=( $(compgen -W "--query -q --model --provider --accessible --reset --factory --help -h --version -V onboard resume recovery completions check version session auth config" -- "$cur") );;
    esac
}
complete -F _arcana arcana
"#;

const ZSH_COMPLETIONS: &str = r#"#compdef arcana
_arcana() {
  local -a commands opts recover_opts shells
  commands=(onboard resume recovery completions check version session auth config)
  opts=(--query -q --model --provider --accessible --reset --factory --help -h --version -V)
  recovery_opts=(--list --to-sequence --yes -y --help -h)
  shells=(bash zsh fish)
  if (( CURRENT > 2 )) && [[ ${words[2]} == recovery ]]; then
    _describe 'recovery options' recovery_opts
  elif (( CURRENT > 2 )) && [[ ${words[2]} == completions ]]; then
    _describe 'shells' shells
  else
    _describe 'commands' commands
    _describe 'options' opts
  fi
}
_arcana "$@"
"#;

const FISH_COMPLETIONS: &str = r#"complete -c arcana -f
complete -c arcana -s q -l query -d 'Single-shot query'
complete -c arcana -l model -d 'Override model'
complete -c arcana -l provider -d 'Override provider'
complete -c arcana -l accessible -d 'Accessibility mode'
complete -c arcana -l reset -d 'Reset project workspace'
complete -c arcana -l factory -d 'Reset global workspace with --reset'
complete -c arcana -l help -s h -d 'Show help'
complete -c arcana -l version -s V -d 'Show version'
complete -c arcana -n '__fish_use_subcommand' -a 'onboard resume recovery completions check version session auth config'
complete -c arcana -n '__fish_seen_subcommand_from recovery' -l list -d 'List recorded mutations'
complete -c arcana -n '__fish_seen_subcommand_from recovery' -l to-sequence -d 'Recover to record sequence'
complete -c arcana -n '__fish_seen_subcommand_from recovery' -l yes -s y -d 'Skip recovery confirmation'
complete -c arcana -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'
"#;

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

/// Check whether `~/.arcana` (the global system workspace) exists.
/// If not, prompt the user to create it.  Returns `true` if it exists
/// or was created, `false` if the user declined.
fn ensure_global_workspace() -> bool {
    let home = match dirs::home_dir() {
        Some(d) => d,
        None => {
            eprintln!("[Arcana] Cannot determine home directory.");
            return false;
        }
    };
    let global_dir = home.join(".arcana");
    if global_dir.exists() {
        return true;
    }

    eprintln!();
    eprintln!(
        "The Arcana system workspace `{}` does not exist.",
        global_dir.display()
    );
    eprintln!("It is required for configuration, memory, skills, and authority.");
    eprint!("Create it now? [y/c = yes, n/q = no]: ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        eprintln!("\n[Arcana] Aborted.");
        return false;
    }
    match input.trim().to_lowercase().as_str() {
        "y" | "yes" | "c" | "continue" => {
            if let Err(e) = std::fs::create_dir_all(&global_dir) {
                eprintln!("[Arcana] Failed to create {}: {}", global_dir.display(), e);
                return false;
            }
            eprintln!(
                "[Arcana] Created system workspace at {}",
                global_dir.display()
            );
            true
        }
        _ => {
            eprintln!("[Arcana] Aborted.  Run `arcana onboard` to set up the workspace.");
            false
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

        let authority_path = arcana_home.join("authority.toml");
        if authority_path.exists() {
            println!("  ✓ Authority policy (~/.arcana/authority.toml)");
        } else {
            println!("  ✗ Authority policy (~/.arcana/authority.toml not found)");
            println!("    → Run: arcana onboard");
        }

        let soul_path = arcana_home.join("SOUL.md");
        if soul_path.exists() {
            println!("  ✓ Agent personality (~/.arcana/SOUL.md)");
        } else {
            println!("  ✗ Agent personality (~/.arcana/SOUL.md not found)");
            println!("    → Run: arcana onboard");
        }

        let user_path = arcana_home.join("USER.md");
        if user_path.exists() {
            println!("  ✓ User portrait (~/.arcana/USER.md)");
        } else {
            println!("  ✗ User portrait (~/.arcana/USER.md not found)");
            println!("    → Run: arcana onboard");
        }

        let instruction_path = arcana_home.join("INSTRUCTION.md");
        if instruction_path.exists() {
            println!("  ✓ Authority instruction (~/.arcana/INSTRUCTION.md)");
        } else {
            println!("  ✗ Authority instruction (~/.arcana/INSTRUCTION.md not found)");
            println!("    → Run: arcana onboard or open `\\instruction show` in the TUI");
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
        #[serde(default = "default_safe")]
        pub safe: Vec<String>,
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

    fn default_safe() -> Vec<String> {
        vec![
            "git status",
            "git diff",
            "git log",
            "git show",
            "pwd",
            "ls",
            "cat",
            "find",
            "grep",
            "rg",
            "head",
            "tail",
            "wc",
            "sort",
            "uniq",
            "cut",
            "tr",
            "file",
            "stat",
            "du",
            "df",
            "which",
            "type",
            "whereis",
            "whoami",
            "hostname",
            "uname",
            "date",
            "git branch",
            "git tag",
            "git remote",
            "tree",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    fn default_allow() -> Vec<String> {
        vec![
            "cargo build",
            "cargo test",
            "cargo clippy",
            "cargo fmt",
            "python3",
            "node",
            "make",
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
                    safe: default_safe(),
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
                println!("  [commands.safe]");
                for cmd in &config.commands.safe {
                    println!("    ✓ {}", cmd);
                }
                println!("\n  [commands.allow]");
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
            Some(AuthAction::Instruction) => {
                let path = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".arcana/INSTRUCTION.md");
                match std::fs::read_to_string(&path) {
                    Ok(content) => println!("{}", content),
                    Err(_) => println!("  No instruction file found at {}", path.display()),
                }
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
