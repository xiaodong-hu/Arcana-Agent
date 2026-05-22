use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// === IPC Protocol ===

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Request {
    #[serde(rename = "read")]
    Read { path: String },
    #[serde(rename = "write")]
    Write { path: String, content: String },
    #[serde(rename = "delete")]
    Delete { path: String },
    #[serde(rename = "rename")]
    Rename { src: String, dst: String },
    #[serde(rename = "query")]
    Query { path: String },
    #[serde(rename = "fetch")]
    Fetch { url: String, tag: Option<String> },
    #[serde(rename = "exec")]
    Exec { cmd: String, args: Vec<String> },
    #[serde(rename = "register_tool")]
    RegisterTool { name: String, path: String, args: Vec<String>, description: String },
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
    #[serde(rename = "fetched")]
    Fetched { path: String, bytes: u64 },
    #[serde(rename = "exec_result")]
    ExecResult { stdout: String, stderr: String, code: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    None,
    Read,
    Write,
}

// === Access Rules ===

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleVerdict {
    Allow,
    Deny,
    Prompt,
}

#[derive(Debug, Deserialize)]
pub struct AccessConfig {
    pub rules: AccessRules,
    #[serde(default)]
    pub web: WebConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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
        Self { default: "prompt".into(), allow_domains: vec![], deny_domains: vec![] }
    }
}

#[derive(Debug, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_runtime_registration: bool,
    #[serde(default = "default_prompt")]
    pub default: String,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self { allow: vec![], deny: vec![], allow_runtime_registration: true, default: "prompt".into() }
    }
}

fn default_prompt() -> String { "prompt".into() }
fn default_true() -> bool { true }

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
