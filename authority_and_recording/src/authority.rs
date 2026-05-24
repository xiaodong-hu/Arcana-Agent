use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use glob_match::glob_match;
use serde::{Deserialize, Serialize};

use crate::types::{
    AccessRules, AuthorityConfigSources, AuthorityErrorType, AuthoritySnapshot, FilesystemAccess,
    RuleVerdict, ToolsConfig, WebConfig,
};

pub enum Approval {
    Approved(String),
    Aborted {
        error_type: AuthorityErrorType,
        message: String,
    },
}

pub struct Authority {
    project_root: PathBuf,
    rules: AccessRules,
    pub web: WebConfig,
    tools: ToolsConfig,
    source_configs: AuthorityConfigSources,
    runtime_tools: HashSet<String>,
}

impl Authority {
    pub fn load(project_root: PathBuf) -> io::Result<Self> {
        let global_config_path = global_authority_path();
        let project_config_path = project_authority_path(&project_root);

        let mut merged = NormalizedAuthorityConfig::default();
        let mut source_configs = AuthorityConfigSources {
            system_toml: None,
            project_toml: None,
        };

        if global_config_path.exists() {
            let text = fs::read_to_string(&global_config_path)?;
            source_configs.system_toml = Some(text.clone());
            let config = GlobalAuthorityConfig::from_toml(&text)?;
            merged.merge(config);
        }

        if project_config_path.exists() {
            let text = fs::read_to_string(&project_config_path)?;
            source_configs.project_toml = Some(text.clone());
            let config = GlobalAuthorityConfig::from_toml(&text)?;
            merged.merge(config);
        }

        if merged.is_empty() {
            merged.merge(GlobalAuthorityConfig::default_project());
        }

        let (rules, web, tools) = merged.into_access_config();
        Ok(Self {
            project_root,
            rules,
            web,
            tools,
            source_configs,
            runtime_tools: HashSet::new(),
        })
    }

    pub fn check_write(&self, path: &str) -> RuleVerdict {
        for pattern in &self.rules.deny_write {
            if glob_match(pattern, path) {
                return RuleVerdict::Deny;
            }
        }
        for pattern in &self.rules.allow_write {
            if glob_match(pattern, path) {
                return RuleVerdict::Allow;
            }
        }
        self.default_verdict()
    }

    pub fn check_read(&self, path: &str) -> RuleVerdict {
        for pattern in &self.rules.deny_read {
            if glob_match(pattern, path) {
                return RuleVerdict::Deny;
            }
        }
        RuleVerdict::Allow
    }

    pub fn check_web(&self, domain: &str) -> RuleVerdict {
        for d in &self.web.deny_domains {
            if domain_rule_matches(d, domain) {
                return RuleVerdict::Deny;
            }
        }
        for d in &self.web.allow_domains {
            if domain_rule_matches(d, domain) {
                return RuleVerdict::Allow;
            }
        }
        match self.web.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }

    pub fn check_tool(&self, cmd: &str, args: &[String]) -> RuleVerdict {
        let full = if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{} {}", cmd, args.join(" "))
        };
        for d in &self.tools.deny {
            if command_rule_matches(d, cmd, &full) {
                return RuleVerdict::Deny;
            }
        }
        for a in &self.tools.allow {
            if command_rule_matches(a, cmd, &full) {
                return RuleVerdict::Allow;
            }
        }
        for p in &self.tools.prompt {
            if command_rule_matches(p, cmd, &full) {
                return RuleVerdict::Prompt;
            }
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

    pub fn register_tool_runtime(&mut self, name: &str) -> bool {
        if !self.tools.allow_runtime_registration {
            return false;
        }
        self.runtime_tools.insert(name.to_string());
        true
    }

    pub fn runtime_registration_allowed(&self) -> bool {
        self.tools.allow_runtime_registration
    }

    pub fn register_command(&mut self, pattern: &str) -> io::Result<()> {
        self.append_project_entry("commands", "allow", pattern)?;
        self.tools.allow.push(pattern.to_string());
        self.refresh_project_source()
    }

    pub fn register_web(&mut self, domain: &str) -> io::Result<()> {
        self.append_project_entry("network", "allow", domain)?;
        self.web.allow_domains.push(domain.to_string());
        self.refresh_project_source()
    }

    pub fn register_filesystem(&mut self, access: FilesystemAccess, path: &str) -> io::Result<()> {
        let key = match access {
            FilesystemAccess::Writable => "writable",
            FilesystemAccess::Readonly => "readonly",
            FilesystemAccess::Deny => "deny",
        };
        self.append_project_entry("filesystem", key, path)?;
        match access {
            FilesystemAccess::Writable => self.rules.allow_write.push(path.to_string()),
            FilesystemAccess::Readonly => self.rules.deny_write.push(path.to_string()),
            FilesystemAccess::Deny => {
                self.rules.deny_write.push(path.to_string());
                self.rules.deny_read.push(path.to_string());
            }
        }
        self.refresh_project_source()
    }

    pub fn approval(&self, kind: &str, target: &str, error_type: AuthorityErrorType) -> Approval {
        self.approval_loop(kind, target, error_type, false)
    }

    pub fn editable_approval(
        &self,
        kind: &str,
        target: &str,
        error_type: AuthorityErrorType,
    ) -> Approval {
        self.approval_loop(kind, target, error_type, true)
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.project_root.join(p)
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn snapshot(&self) -> AuthoritySnapshot {
        let mut runtime_tools: Vec<String> = self.runtime_tools.iter().cloned().collect();
        runtime_tools.sort();
        AuthoritySnapshot {
            filesystem: self.rules.clone(),
            network: self.web.clone(),
            commands: self.tools.clone(),
            runtime_tools,
            configs: self.source_configs.clone(),
        }
    }

    fn default_verdict(&self) -> RuleVerdict {
        match self.rules.default.as_str() {
            "allow" => RuleVerdict::Allow,
            "deny" => RuleVerdict::Deny,
            _ => RuleVerdict::Prompt,
        }
    }

    fn approval_loop(
        &self,
        kind: &str,
        target: &str,
        error_type: AuthorityErrorType,
        editable: bool,
    ) -> Approval {
        let mut current = target.to_string();
        loop {
            eprintln!();
            eprintln!("[{kind}] LLM requires `{current}`. Confirm Allowance?");
            eprintln!("    - Yes and Run [y/Enter]");
            if editable {
                eprintln!("    - No and Edit [e]");
            }
            eprint!("    - No and Abort [n/a]: ");
            io::stderr().flush().ok();

            let mut input = String::new();
            let answer = if io::stdin().read_line(&mut input).is_ok() {
                input.trim().to_ascii_lowercase()
            } else {
                "n".into()
            };

            if answer.is_empty() || answer == "y" || answer == "yes" {
                return Approval::Approved(current);
            }
            if editable && answer == "e" {
                match edit_value(&current) {
                    Ok(edited) if !edited.trim().is_empty() => {
                        current = edited.trim().to_string();
                        continue;
                    }
                    Ok(_) => {
                        return Approval::Aborted {
                            error_type,
                            message: format!("{kind} edit produced an empty request"),
                        };
                    }
                    Err(e) => {
                        return Approval::Aborted {
                            error_type,
                            message: format!("{kind} edit failed: {e}"),
                        };
                    }
                }
            }
            if answer == "n" || answer == "no" || answer == "a" || answer == "abort" {
                return Approval::Aborted {
                    error_type,
                    message: format!("{kind} aborted by user: {current}"),
                };
            }
        }
    }

    fn append_project_entry(&self, section: &str, key: &str, value: &str) -> io::Result<()> {
        let path = project_authority_path(&self.project_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut config = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            default_project_authority_toml()
        };
        let entry = format!("\"{}\"", escape_toml_string(value));
        append_to_array(&mut config, section, key, &entry);
        fs::write(path, config)
    }

    fn refresh_project_source(&mut self) -> io::Result<()> {
        let path = project_authority_path(&self.project_root);
        self.source_configs.project_toml = if path.exists() {
            Some(fs::read_to_string(path)?)
        } else {
            None
        };
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct GlobalAuthorityConfig {
    #[serde(default)]
    commands: GlobalCommandsConfig,
    #[serde(default)]
    network: GlobalNetworkConfig,
    #[serde(default)]
    filesystem: GlobalFilesystemConfig,
}

impl GlobalAuthorityConfig {
    fn from_toml(text: &str) -> io::Result<Self> {
        toml::from_str(text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn default_project() -> Self {
        Self {
            commands: GlobalCommandsConfig::default(),
            network: GlobalNetworkConfig::default(),
            filesystem: GlobalFilesystemConfig {
                writable: vec![],
                readonly: vec![],
                deny: vec![".arcana/git_record/**".into()],
            },
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct GlobalCommandsConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    confirm: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct GlobalNetworkConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct GlobalFilesystemConfig {
    #[serde(default)]
    writable: Vec<String>,
    #[serde(default)]
    readonly: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

#[derive(Default)]
struct NormalizedAuthorityConfig {
    commands_allow: Vec<String>,
    commands_confirm: Vec<String>,
    commands_deny: Vec<String>,
    network_allow: Vec<String>,
    network_deny: Vec<String>,
    filesystem_writable: Vec<String>,
    filesystem_readonly: Vec<String>,
    filesystem_deny: Vec<String>,
}

impl NormalizedAuthorityConfig {
    fn merge(&mut self, config: GlobalAuthorityConfig) {
        extend_unique(&mut self.commands_allow, config.commands.allow);
        extend_unique(&mut self.commands_confirm, config.commands.confirm);
        extend_unique(&mut self.commands_deny, config.commands.deny);
        extend_unique(&mut self.network_allow, config.network.allow);
        extend_unique(&mut self.network_deny, config.network.deny);
        extend_unique(&mut self.filesystem_writable, config.filesystem.writable);
        extend_unique(&mut self.filesystem_readonly, config.filesystem.readonly);
        extend_unique(&mut self.filesystem_deny, config.filesystem.deny);
    }

    fn is_empty(&self) -> bool {
        self.commands_allow.is_empty()
            && self.commands_confirm.is_empty()
            && self.commands_deny.is_empty()
            && self.network_allow.is_empty()
            && self.network_deny.is_empty()
            && self.filesystem_writable.is_empty()
            && self.filesystem_readonly.is_empty()
            && self.filesystem_deny.is_empty()
    }

    fn into_access_config(self) -> (AccessRules, WebConfig, ToolsConfig) {
        let deny_read = self.filesystem_deny.clone();
        let mut deny_write = self.filesystem_deny;
        extend_unique(&mut deny_write, self.filesystem_readonly);

        let allow_write = self
            .filesystem_writable
            .into_iter()
            .map(|path| if path == "." { "**".to_string() } else { path })
            .collect();

        let network_default = if self.network_deny.iter().any(|domain| domain == "*") {
            "deny"
        } else {
            "prompt"
        };

        let web = WebConfig {
            default: network_default.into(),
            allow_domains: self.network_allow,
            deny_domains: self
                .network_deny
                .into_iter()
                .filter(|domain| domain != "*")
                .collect(),
        };

        let tools = ToolsConfig {
            allow: self.commands_allow,
            prompt: self.commands_confirm,
            deny: self.commands_deny,
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

fn edit_value(initial: &str) -> io::Result<String> {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let path = env::temp_dir().join(format!("arcana_authority_edit_{}.txt", std::process::id()));
    fs::write(&path, initial)?;
    let status = Command::new(editor).arg(&path).status()?;
    if !status.success() {
        let _ = fs::remove_file(&path);
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "editor exited unsuccessfully",
        ));
    }
    let content = fs::read_to_string(&path)?;
    let _ = fs::remove_file(&path);
    Ok(content)
}

fn global_authority_path() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arcana")
        .join("authority.toml")
}

fn project_authority_path(project_root: &Path) -> PathBuf {
    project_root.join(".arcana/authority.toml")
}

fn default_project_authority_toml() -> String {
    "[commands]\nallow = []\nconfirm = []\ndeny = []\n\n[network]\nallow = []\ndeny = []\n\n[filesystem]\nwritable = []\nreadonly = []\ndeny = []\n".into()
}

fn append_to_array(config: &mut String, section: &str, key: &str, entry: &str) {
    let header = format!("[{section}]");
    if !config.contains(&header) {
        config.push_str(&format!("\n{header}\n{key} = [{entry}]\n"));
        return;
    }

    let key_prefix = format!("{key} = [");
    let Some(section_start) = config.find(&header) else {
        return;
    };
    let next_section = config[section_start + header.len()..]
        .find("\n[")
        .map(|idx| section_start + header.len() + idx)
        .unwrap_or(config.len());
    let section_text = &config[section_start..next_section];
    let Some(key_rel) = section_text.find(&key_prefix) else {
        config.insert_str(next_section, &format!("{key} = [{entry}]\n"));
        return;
    };
    let key_start = section_start + key_rel;
    let array_start = key_start + key_prefix.len();
    let Some(array_end_rel) = config[array_start..].find(']') else {
        return;
    };
    let array_end = array_start + array_end_rel;
    if config[array_start..array_end].contains(entry) {
        return;
    }
    if config[array_start..array_end].trim().is_empty() {
        config.insert_str(array_end, entry);
    } else {
        config.insert_str(array_end, &format!(", {entry}"));
    }
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn extend_unique(dst: &mut Vec<String>, src: Vec<String>) {
    for value in src {
        if !dst.contains(&value) {
            dst.push(value);
        }
    }
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
