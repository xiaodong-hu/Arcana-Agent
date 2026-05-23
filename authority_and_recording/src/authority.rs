use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use glob_match::glob_match;
use serde::Deserialize;

use crate::types::{AccessConfig, AccessRules, AuthoritySnapshot, RuleVerdict, ToolsConfig, WebConfig};

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
        let global_config_path = global_authority_path();
        let project_config_path = project_root.join(".arcana/access.toml");
        let (rules, web, tools) = if global_config_path.exists() {
            let text = fs::read_to_string(&global_config_path)?;
            let config: GlobalAuthorityConfig =
                toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            config.into_access_config()
        } else if project_config_path.exists() {
            let text = fs::read_to_string(&project_config_path)?;
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
            if domain_rule_matches(d, domain) { return RuleVerdict::Deny; }
        }
        for d in &self.web.allow_domains {
            if domain_rule_matches(d, domain) { return RuleVerdict::Allow; }
        }
        match self.web.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }

    /// Check if a command is allowed to execute.
    pub fn check_tool(&self, cmd: &str, args: &[String]) -> RuleVerdict {
        let full = if args.is_empty() { cmd.to_string() } else { format!("{} {}", cmd, args.join(" ")) };
        for d in &self.tools.deny {
            if command_rule_matches(d, cmd, &full) { return RuleVerdict::Deny; }
        }
        for a in &self.tools.allow {
            if command_rule_matches(a, cmd, &full) { return RuleVerdict::Allow; }
        }
        for p in &self.tools.prompt {
            if command_rule_matches(p, cmd, &full) { return RuleVerdict::Prompt; }
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

    pub fn snapshot(&self) -> AuthoritySnapshot {
        let mut runtime_tools: Vec<String> = self.runtime_tools.iter().cloned().collect();
        runtime_tools.sort();
        let mut filesystem = self.rules.clone();
        filesystem.deny_write = sanitize_paths(filesystem.deny_write);
        filesystem.deny_read = sanitize_paths(filesystem.deny_read);
        AuthoritySnapshot {
            filesystem,
            network: self.web.clone(),
            commands: self.tools.clone(),
            runtime_tools,
        }
    }

    fn default_verdict(&self) -> RuleVerdict {
        match self.rules.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GlobalAuthorityConfig {
    #[serde(default)]
    commands: GlobalCommandsConfig,
    #[serde(default)]
    network: GlobalNetworkConfig,
    #[serde(default)]
    filesystem: GlobalFilesystemConfig,
}

impl GlobalAuthorityConfig {
    fn into_access_config(self) -> (AccessRules, WebConfig, ToolsConfig) {
        let deny_read = self.filesystem.deny.clone();
        let mut deny_write = self.filesystem.deny;
        deny_write.extend(self.filesystem.readonly);

        let allow_write = self.filesystem.writable.into_iter()
            .map(|path| if path == "." { "**".to_string() } else { path })
            .collect();

        let network_default = if self.network.deny.iter().any(|domain| domain == "*") {
            "deny"
        } else {
            "prompt"
        };

        let web = WebConfig {
            default: network_default.into(),
            allow_domains: self.network.allow,
            deny_domains: self.network.deny.into_iter()
                .filter(|domain| domain != "*")
                .collect(),
        };

        let tools = ToolsConfig {
            allow: self.commands.allow,
            prompt: self.commands.confirm,
            deny: vec![],
            allow_runtime_registration: true,
            default: "prompt".into(),
        };

        let rules = AccessRules {
            allow_write,
            deny_write,
            deny_read,
            default: "prompt".into(),
        };

        (rules, web, tools)
    }
}

#[derive(Debug, Default, Deserialize)]
struct GlobalCommandsConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    confirm: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GlobalNetworkConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GlobalFilesystemConfig {
    #[serde(default)]
    writable: Vec<String>,
    #[serde(default)]
    readonly: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

fn global_authority_path() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arcana")
        .join("authority.toml")
}

fn command_rule_matches(rule: &str, cmd: &str, full: &str) -> bool {
    rule == cmd || rule == full || glob_match(rule, full)
}

fn domain_rule_matches(rule: &str, domain: &str) -> bool {
    if let Some(suffix) = rule.strip_prefix("*.") {
        return domain.ends_with(&format!(".{}", suffix));
    }
    domain == rule || domain.ends_with(&format!(".{}", rule))
}

fn sanitize_paths(paths: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let sanitized = if path.contains(".arcana/authority.toml") {
            "<authority-config>".to_string()
        } else {
            path
        };
        if !out.contains(&sanitized) {
            out.push(sanitized);
        }
    }
    out
}
