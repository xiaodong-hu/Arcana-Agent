use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "arcana", about = "Arcana Agent — Memory · Skills · Authority")]
pub struct Cli {
    /// Project path to open
    pub project: Option<PathBuf>,

    /// Single-shot query (non-interactive)
    #[arg(short, long)]
    pub query: Option<String>,

    /// Override model for this session
    #[arg(long)]
    pub model: Option<String>,

    /// Override provider for this session
    #[arg(long)]
    pub provider: Option<String>,

    /// Accessibility mode (no animations, no alternate screen)
    #[arg(long)]
    pub accessible: bool,

    /// Reset project workspace `./.arcana/` (requires confirmation).
    /// Combine with `--factory` to reset the global `~/.arcana/` instead.
    #[arg(long)]
    pub reset: bool,

    /// Target the global `~/.arcana/` directory for `--reset`
    /// (requires extra warning confirmation).
    #[arg(long)]
    pub factory: bool,

    /// Project root directory (defaults to current directory with confirmation)
    pub project: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// First-time setup wizard
    Onboard(OnboardArgs),
    /// Resume a previous session
    Resume(ResumeArgs),
    /// Recover project state from git_record
    Recover(RecoverArgs),
    /// Check system health and connectivity
    Check,
    /// Print version
    Version,
    /// Session management
    Session(SessionArgs),
    /// Command authorization management
    Auth(AuthArgs),
    /// Configuration management
    Config(ConfigArgs),
}

#[derive(Parser)]
pub struct OnboardArgs {
    /// Provider to configure (skip interactive selection)
    #[arg(long)]
    pub provider: Option<String>,
    /// Model to set as default
    #[arg(long)]
    pub model: Option<String>,
    /// Non-interactive mode (use env vars for keys)
    #[arg(long)]
    pub non_interactive: bool,
}

#[derive(Parser)]
pub struct ResumeArgs {
    /// Resume the most recent session
    #[arg(long)]
    pub last: bool,
    /// Session ID or name to resume
    pub session: Option<String>,
}

#[derive(Parser)]
pub struct RecoverArgs {
    /// Project root directory
    pub project: PathBuf,
    /// Recover to specific sequence number
    #[arg(long)]
    pub to_seq: Option<u64>,
}

#[derive(Parser)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub action: Option<SessionAction>,
}

#[derive(Subcommand)]
pub enum SessionAction {
    List,
    Resume { id: String },
    Rename { id: String, name: String },
    Delete { id: String },
    Export { id: String },
    Import { file: PathBuf },
}

#[derive(Parser)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub action: Option<AuthAction>,
}

#[derive(Subcommand)]
pub enum AuthAction {
    /// Show all authorized commands/network/fs rules
    Status,
    /// Show ~/.arcana/INSTRUCTION.md
    Instruction,
    /// Add a command to the allow list
    Allow {
        /// Command pattern to allow
        pattern: String,
    },
    /// Add a command to the deny/confirm list
    Deny {
        /// Command pattern to deny
        pattern: String,
    },
    /// Remove a command from the allow list
    Revoke {
        /// Command pattern to revoke
        pattern: String,
    },
    /// Reset authority config to defaults
    Reset,
}

#[derive(Parser)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show current configuration
    Show,
    /// Open config file in editor
    Edit,
    /// Print config file path
    Path,
}
