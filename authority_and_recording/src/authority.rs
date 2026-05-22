use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use glob_match::glob_match;

use crate::types::{AccessConfig, AccessRules, RuleVerdict, ToolsConfig, WebConfig};

pub struct Authority {
    project_root: PathBuf,
    rules: AccessRules,
    pub web: WebConfig,
    tools: ToolsConfig,
    /// Runtime-registered tools (session-only, not persisted)
    runtime_tools: HashSet<String>,
}

impl Authority {
    pub fn load(project_root: PathBuf) -> io::Result<Self> {
        let config_path = project_root.join(".arcana/access.toml");
        let (rules, web, tools) = if config_path.exists() {
            let text = fs::read_to_string(&config_path)?;
            let config: AccessConfig =
                toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            (config.rules, config.web, config.tools)
        } else {
            (
                AccessRules {
                    allow_write: vec![],
                    deny_write: vec![".arcana/git_record/**".into()],
                    deny_read: vec![],
                    default: "prompt".into(),
                },
                WebConfig::default(),
                ToolsConfig::default(),
            )
        };
        Ok(Self { project_root, rules, web, tools, runtime_tools: HashSet::new() })
    }

    /// Check write access for a path.
    pub fn check_write(&self, path: &str) -> RuleVerdict {
        for pattern in &self.rules.deny_write {
            if glob_match(pattern, path) { return RuleVerdict::Deny; }
        }
        for pattern in &self.rules.allow_write {
            if glob_match(pattern, path) { return RuleVerdict::Allow; }
        }
        self.default_verdict()
    }

    /// Check read access for a path.
    pub fn check_read(&self, path: &str) -> RuleVerdict {
        for pattern in &self.rules.deny_read {
            if glob_match(pattern, path) { return RuleVerdict::Deny; }
        }
        RuleVerdict::Allow // reads are allowed by default unless denied
    }

    /// Check web access for a domain.
    pub fn check_web(&self, domain: &str) -> RuleVerdict {
        for d in &self.web.deny_domains {
            if domain == d || domain.ends_with(&format!(".{}", d)) { return RuleVerdict::Deny; }
        }
        for d in &self.web.allow_domains {
            if domain == d || domain.ends_with(&format!(".{}", d)) { return RuleVerdict::Allow; }
        }
        match self.web.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }

    /// Check if a command is allowed to execute.
    pub fn check_tool(&self, cmd: &str) -> RuleVerdict {
        for d in &self.tools.deny {
            if cmd == d || cmd.starts_with(&format!("{} ", d)) { return RuleVerdict::Deny; }
        }
        for a in &self.tools.allow {
            if cmd == a { return RuleVerdict::Allow; }
        }
        if self.runtime_tools.contains(cmd) {
            return RuleVerdict::Allow;
        }
        match self.tools.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }

    /// Register a runtime tool. Returns false if runtime registration is disabled.
    pub fn register_tool(&mut self, name: &str) -> bool {
        if !self.tools.allow_runtime_registration { return false; }
        self.runtime_tools.insert(name.to_string());
        true
    }

    /// Prompt user on stderr/stdin for approval.
    pub fn prompt_user(&self, op: &str, target: &str) -> bool {
        eprint!("[arcana] Agent requests `{}` on `{}`. Allow? [y/N]: ", op, target);
        io::stderr().flush().ok();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_ok() {
            matches!(input.trim(), "y" | "Y" | "yes")
        } else {
            false
        }
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { self.project_root.join(p) }
    }

    pub fn project_root(&self) -> &Path { &self.project_root }

    pub fn access_rules(&self) -> &AccessRules { &self.rules }
    pub fn web_config(&self) -> &WebConfig { &self.web }
    pub fn tools_config(&self) -> &ToolsConfig { &self.tools }

    fn default_verdict(&self) -> RuleVerdict {
        match self.rules.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }
}
