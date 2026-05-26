use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// === IPC Protocol ===

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Request {
    #[serde(rename = "read")]
    Read { path: String },
    #[serde(rename = "read_text")]
    ReadText { path: String },
    #[serde(rename = "write")]
    Write { path: String, content: String },
    #[serde(rename = "write_text")]
    WriteText { path: String, content: String },
    #[serde(rename = "write_confirmed")]
    WriteConfirmed { path: String, content: String },
    #[serde(rename = "write_text_confirmed")]
    WriteTextConfirmed { path: String, content: String },
    #[serde(rename = "delete")]
    Delete { path: String },
    #[serde(rename = "delete_confirmed")]
    DeleteConfirmed { path: String },
    #[serde(rename = "rename")]
    Rename { src: String, dst: String },
    #[serde(rename = "rename_confirmed")]
    RenameConfirmed { src: String, dst: String },
    #[serde(rename = "query")]
    Query { path: String },
    #[serde(rename = "fetch")]
    Fetch { url: String, tag: Option<String> },
    #[serde(rename = "fetch_confirmed")]
    FetchConfirmed { url: String, tag: Option<String> },
    #[serde(rename = "exec")]
    Exec { cmd: String, args: Vec<String> },
    #[serde(rename = "exec_confirmed")]
    ExecConfirmed { cmd: String, args: Vec<String> },
    #[serde(rename = "exec_shell")]
    ExecShell { command: String },
    #[serde(rename = "exec_shell_confirmed")]
    ExecShellConfirmed { command: String },
    #[serde(rename = "register_tool")]
    RegisterTool {
        name: String,
        path: String,
        args: Vec<String>,
        description: String,
    },
    #[serde(rename = "register_tool_confirmed")]
    RegisterToolConfirmed {
        name: String,
        path: String,
        args: Vec<String>,
        description: String,
    },
    #[serde(rename = "register_command")]
    RegisterCommand { pattern: String },
    #[serde(rename = "register_command_confirmed")]
    RegisterCommandConfirmed { pattern: String },
    #[serde(rename = "register_web")]
    RegisterWeb { domain: String },
    #[serde(rename = "register_web_confirmed")]
    RegisterWebConfirmed { domain: String },
    #[serde(rename = "register_filesystem")]
    RegisterFilesystem {
        access: FilesystemAccess,
        path: String,
    },
    #[serde(rename = "register_filesystem_confirmed")]
    RegisterFilesystemConfirmed {
        access: FilesystemAccess,
        path: String,
    },
    #[serde(rename = "instruction")]
    Instruction,
    #[serde(rename = "list_authority")]
    ListAuthority,
    #[serde(rename = "prompt")]
    Prompt,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum Response {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "denied")]
    Denied { reason: String },
    #[serde(rename = "permission")]
    Permission { level: AccessLevel },
    #[serde(rename = "content")]
    Content { data: String },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "fetched")]
    Fetched { path: String, bytes: u64 },
    #[serde(rename = "exec_result")]
    ExecResult {
        stdout: String,
        stderr: String,
        code: i32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        records: Vec<MutationRecord>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        diff: String,
    },
    #[serde(rename = "mutation")]
    Mutation {
        records: Vec<MutationRecord>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        diff: String,
    },
    #[serde(rename = "instruction")]
    Instruction { content: String },
    #[serde(rename = "prompt")]
    Prompt { content: String },
    #[serde(rename = "authority")]
    Authority { snapshot: AuthoritySnapshot },
    #[serde(rename = "aborted")]
    Aborted {
        error_type: AuthorityErrorType,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    None,
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    Writable,
    Readonly,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorityErrorType {
    ToolCallAbortError,
    FileAccessAbortError,
    WebAccessAbortError,
    ToolRegistrationAbortError,
    FileAccessRegistrationAbortError,
    WebAccessRegistrationAbortError,
}

// === Access Rules ===

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleVerdict {
    Allow,
    Deny,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRules {
    #[serde(default)]
    pub allow_write: Vec<String>,
    #[serde(default)]
    pub deny_write: Vec<String>,
    #[serde(default)]
    pub deny_read: Vec<String>,
    #[serde(default = "default_prompt")]
    pub default: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_prompt")]
    pub default: String,
    #[serde(default)]
    pub allow_domains: Vec<String>,
    #[serde(default)]
    pub deny_domains: Vec<String>,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            default: "prompt".into(),
            allow_domains: vec![],
            deny_domains: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub safe: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub prompt: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_runtime_registration: bool,
    #[serde(default = "default_prompt")]
    pub default: String,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            safe: vec![],
            allow: vec![],
            prompt: vec![],
            deny: vec![],
            allow_runtime_registration: true,
            default: "prompt".into(),
        }
    }
}

fn default_prompt() -> String {
    "prompt".into()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthoritySnapshot {
    pub filesystem: AccessRules,
    pub network: WebConfig,
    pub commands: ToolsConfig,
    pub runtime_tools: Vec<String>,
    pub configs: AuthorityConfigSources,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityConfigSources {
    pub system_toml: Option<String>,
    pub project_toml: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationRecord {
    pub seq: u64,
    pub op: String,
    pub path: String,
}

// === Action Record ===

#[derive(Debug, Serialize, Deserialize)]
pub struct ActionRecord {
    pub seq: u64,
    pub ts: String,
    pub op: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_blob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst: Option<String>,
}

// === Snapshot ===

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub ts: String,
    pub tree: HashMap<String, String>,
}
