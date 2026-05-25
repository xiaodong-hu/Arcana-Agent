use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Application-wide view state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Main conversation viewport
    Main,
    /// Query sub-agent overlay
    QueryOverlay,
    /// Diff review pending
    DiffReview,
}

/// Separator kind for horizontal delimiter lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorKind {
    /// Full dialogue boundary (white).
    Full,
    /// Within-dialogue break — LLM continues after tool results (dark gray).
    Partial,
}

/// A message in the conversation stream.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub thinking: Option<ThinkingBlock>,
    pub tool_calls: Vec<ToolCall>,
    /// When set, this message is a horizontal separator line.
    pub separator: Option<SeparatorKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
    System,
}

/// A thinking/reasoning block from the model.
#[derive(Debug, Clone)]
pub struct ThinkingBlock {
    pub content: String,
    pub token_count: usize,
    pub duration_ms: u64,
    pub collapsed: bool,
    pub index: usize, // For numbered blocks (Think #1, #2, etc.)
}

/// A tool call made by the agent.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool_type: ToolType,
    pub description: String,
    pub result: Option<String>,
    pub duration_ms: u64,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolType {
    Shell,
    File,
    Search,
    Web,
    Other,
}

impl ToolType {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Shell => "💻",
            Self::File => "📄",
            Self::Search => "🔍",
            Self::Web => "🌐",
            Self::Other => "🔧",
        }
    }
}

/// Status bar data.
#[derive(Debug, Clone)]
pub struct StatusData {
    pub model_name: String,
    pub tokens_used: usize,
    pub tokens_max: usize,
    pub session_input_tokens: usize,
    pub session_output_tokens: usize,
    pub session_cost: f64,
    pub session_requests: usize,
}

impl Default for StatusData {
    fn default() -> Self {
        Self {
            model_name: "deepseek-v4-pro".into(),
            tokens_used: 0,
            tokens_max: 1_000_000,
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cost: 0.0,
            session_requests: 0,
        }
    }
}

/// Per-response statistics appended after every LLM response.
#[derive(Debug, Clone)]
pub struct ResponseStats {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cost: f64,
    pub duration_secs: f64,
}

impl ResponseStats {
    pub fn format_line(&self) -> String {
        let in_str = format_token_count(self.input_tokens);
        let out_str = format_token_count(self.output_tokens);
        format!(
            "Cost: {:.4} ( {} in / {} out )\nTime: {:.1}s",
            self.cost, in_str, out_str, self.duration_secs
        )
    }
}

pub fn format_token_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// LLM error types for user-visible error display.
#[derive(Debug, Clone)]
pub enum LlmError {
    RateLimit {
        retry_after_secs: Option<u64>,
        message: String,
    },
    ApiError {
        code: u16,
        message: String,
    },
    Timeout {
        message: String,
    },
    NetworkError {
        message: String,
    },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimit {
                retry_after_secs,
                message,
            } => {
                write!(f, "Rate limit reached: {}", message)?;
                if let Some(secs) = retry_after_secs {
                    write!(f, " (retry after {}s)", secs)?;
                }
                Ok(())
            }
            Self::ApiError { code, message } => write!(f, "API error {}: {}", code, message),
            Self::Timeout { message } => write!(f, "Timeout: {}", message),
            Self::NetworkError { message } => write!(f, "Network error: {}", message),
        }
    }
}

/// A diff hunk for the review panel.
#[derive(Debug, Clone)]
pub struct DiffReviewData {
    pub file_path: String,
    pub hunks: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    Header,
}

/// User's response to a diff review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffApproval {
    Accept,
    SessionAccept,
    Edit,
    Abort,
}

/// Panel collapse state.
#[derive(Debug, Clone)]
pub struct PanelState {
    pub skills_expanded: bool,
    pub agents_expanded: bool,
    pub tasks_expanded: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            skills_expanded: false,
            agents_expanded: false,
            tasks_expanded: true,
        }
    }
}

/// Skill info for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub mode: String,
    pub trigger_desc: String,
    /// true = system skill (always loaded), false = user-defined
    pub system: bool,
}

/// Sub-agent info for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentInfo {
    pub name: String,
    pub status: String,
    pub turn_count: usize,
    pub max_turns: usize,
    pub scope: String,
}

/// Task info for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub name: String,
    pub status: TaskStatus,
    pub assigned_agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

/// Toast notification.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
}
