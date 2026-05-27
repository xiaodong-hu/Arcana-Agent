use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

use crate::authority::{Approval, Authority};
use crate::prompt;
use crate::record::{MutationReport, Record};
use crate::types::{AccessLevel, AuthorityErrorType, Request, Response, RuleVerdict};

pub struct Server {
    listener: UnixListener,
    socket_path: PathBuf,
    authority: Authority,
    record: Record,
    web_cache_dir: PathBuf,
    tmp_dir: PathBuf,
    prompt_path: PathBuf,
}

impl Server {
    pub fn new(project_root: PathBuf) -> io::Result<Self> {
        let socket_path = project_root.join(".arcana/authority.sock");
        let web_cache_dir = project_root.join(".arcana/web_cache");
        let tmp_dir = project_root.join(".arcana/tmp");
        let prompt_path = project_root.join(".arcana/authorized_prompt.md");

        // Create directories and bind the socket FIRST — before the potentially
        // slow record scan (scan_project_tree hashes every file in the project).
        // This ensures the TUI can detect the daemon is alive within its 5 s
        // timeout, even for large codebases.
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }
        fs::create_dir_all(socket_path.parent().unwrap())?;
        fs::create_dir_all(web_cache_dir.join("pages"))?;
        fs::create_dir_all(&tmp_dir)?;

        let listener = UnixListener::bind(&socket_path)?;
        eprintln!("[Arcana] Listening on {:?}", socket_path);

        // Now do the heavier initialisation
        let authority = Authority::load(project_root.clone())?;
        let record = Record::open(&project_root)?;

        // Generate authorized_prompt.md on startup
        let prompt_content = prompt::generate_prompt(&authority)?;
        fs::write(&prompt_path, &prompt_content)?;
        eprintln!("[Arcana] Generated {:?}", prompt_path);

        Ok(Self {
            listener,
            socket_path,
            authority,
            record,
            web_cache_dir,
            tmp_dir,
            prompt_path,
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        // Listener is already bound — accept connections in a loop.
        // Use accept() (not incoming()) to avoid a long-lived borrow on
        // self.listener that would conflict with &mut self in handle_connection.
        loop {
            let (stream, _) = self.listener.accept()?;
            if let Err(e) = self.handle_connection(stream) {
                eprintln!("[Arcana] Error: {}", e);
            }
        }
    }

    fn handle_connection(&mut self, stream: UnixStream) -> io::Result<()> {
        let reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            let req: Request = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    send(
                        &mut writer,
                        &Response::Denied {
                            reason: format!("bad request: {}", e),
                        },
                    )?;
                    continue;
                }
            };
            let resp = self.handle_request(req)?;
            send(&mut writer, &resp)?;
        }
        Ok(())
    }

    fn handle_request(&mut self, req: Request) -> io::Result<Response> {
        match req {
            Request::Read { path } => self.handle_read(&path),
            Request::ReadText { path } => self.handle_read_text(&path),
            Request::Write { path, content } => self.handle_write(&path, &content),
            Request::WriteText { path, content } => self.handle_write_text(&path, &content),
            Request::WriteConfirmed { path, content } => {
                self.handle_write_confirmed(&path, &content)
            }
            Request::WriteTextConfirmed { path, content } => {
                self.handle_write_text_confirmed(&path, &content)
            }
            Request::WriteApply { path, content } => self.handle_write_apply(&path, &content),
            Request::WriteAbort { path } => self.handle_write_abort(&path),
            Request::Delete { path } => self.handle_delete(&path),
            Request::DeleteConfirmed { path } => self.handle_delete_confirmed(&path),
            Request::Rename { src, dst } => self.handle_rename(&src, &dst),
            Request::RenameConfirmed { src, dst } => self.handle_rename_confirmed(&src, &dst),
            Request::Query { path } => Ok(self.handle_query(&path)),
            Request::Fetch { url, tag: _ } => self.handle_fetch(&url),
            Request::FetchConfirmed { url, tag: _ } => self.handle_fetch_confirmed(&url),
            Request::Exec { cmd, args } => self.handle_exec(&cmd, &args),
            Request::ExecConfirmed { cmd, args } => self.handle_exec_confirmed(&cmd, &args),
            Request::ExecShell { command, readonly } => self.handle_exec_shell(&command, readonly),
            Request::ExecShellConfirmed { command, readonly } => {
                self.handle_exec_shell_confirmed(&command, readonly)
            }
            Request::RegisterTool {
                name,
                path,
                args,
                description,
            } => self.handle_register_tool(&name, &path, &args, &description),
            Request::RegisterToolConfirmed {
                name,
                path,
                args,
                description,
            } => self.handle_register_tool_confirmed(&name, &path, &args, &description),
            Request::RegisterCommand { pattern } => self.handle_register_command(&pattern),
            Request::RegisterCommandConfirmed { pattern } => {
                self.handle_register_command_confirmed(&pattern)
            }
            Request::RegisterWeb { domain } => self.handle_register_web(&domain),
            Request::RegisterWebConfirmed { domain } => self.handle_register_web_confirmed(&domain),
            Request::RegisterFilesystem { access, path } => {
                self.handle_register_filesystem(access, &path)
            }
            Request::RegisterFilesystemConfirmed { access, path } => {
                self.handle_register_filesystem_confirmed(access, &path)
            }
            Request::Instruction => self.handle_instruction(),
            Request::ListAuthority => Ok(Response::Authority {
                snapshot: self.authority.snapshot(),
            }),
            Request::Prompt => self.handle_prompt(),
        }
    }

    fn handle_read(&self, path: &str) -> io::Result<Response> {
        if let RuleVerdict::Deny = self.authority.check_read(path) {
            return Ok(Response::Denied {
                reason: "read access denied".into(),
            });
        }
        let full_path = self.authority.resolve(path);
        if !full_path.exists() {
            return Ok(Response::Denied {
                reason: "file not found".into(),
            });
        }
        let content = fs::read(&full_path)?;
        Ok(Response::Content {
            data: base64_encode(&content),
        })
    }

    fn handle_read_text(&self, path: &str) -> io::Result<Response> {
        if let RuleVerdict::Deny = self.authority.check_read(path) {
            return Ok(Response::Denied {
                reason: "read access denied".into(),
            });
        }
        let full_path = self.authority.resolve(path);
        if !full_path.exists() {
            return Ok(Response::Denied {
                reason: "file not found".into(),
            });
        }
        let bytes = fs::read(&full_path)?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                return Ok(Response::Denied {
                    reason: "file is not valid UTF-8; use read for base64 bytes".into(),
                });
            }
        };
        Ok(Response::Text { text })
    }

    fn handle_write(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write(path) {
            return Ok(resp);
        }
        self.write_authorized(path, content_b64)
    }

    fn handle_write_text(&mut self, path: &str, content: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write(path) {
            return Ok(resp);
        }
        self.write_text_authorized(path, content)
    }

    fn handle_write_confirmed(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write_confirmed(path) {
            return Ok(resp);
        }
        self.write_authorized(path, content_b64)
    }

    fn handle_write_text_confirmed(&mut self, path: &str, content: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write_confirmed(path) {
            return Ok(resp);
        }
        // Two-phase write: return diff for human review before applying
        self.review_write(path, content)
    }

    fn review_write(&mut self, path: &str, proposed: &str) -> io::Result<Response> {
        let full_path = self.authority.resolve(path);
        let original = if full_path.exists() {
            fs::read_to_string(&full_path).unwrap_or_default()
        } else {
            String::new()
        };

        // Write proposed content to a temp file for review
        let hash = hex_sha256(proposed.as_bytes());
        let review_path = self.tmp_dir.join(format!("review_{}", &hash[..12]));
        fs::write(&review_path, proposed)?;

        // Generate diff
        let diff = if original.is_empty() {
            // New file: produce a proper diff with headers so the TUI can
            // detect it and apply tree-sitter highlighting + __REVIEW__ split.
            let mut d = format!("diff --git a/{path} b/{path}\n--- /dev/null\n+++ b/{path}\n");
            let line_count = proposed.lines().count();
            d.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
            for line in proposed.lines() {
                d.push_str(&format!("+{line}\n"));
            }
            d.trim_end().to_string()
        } else {
            generate_unified_diff(&original, proposed, path)
        };

        Ok(Response::Review {
            path: path.to_string(),
            original,
            proposed: proposed.to_string(),
            diff,
            review_path: review_path.to_string_lossy().to_string(),
        })
    }

    fn handle_write_apply(&mut self, path: &str, content: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write_confirmed(path) {
            return Ok(resp);
        }
        self.write_text_authorized(path, content)
    }

    fn handle_write_abort(&mut self, path: &str) -> io::Result<Response> {
        // Clean up the review temp file
        let hash = hex_sha256(path.as_bytes());
        let review_path = self.tmp_dir.join(format!("review_{}", &hash[..12]));
        if review_path.exists() {
            fs::remove_file(&review_path)?;
        }
        Ok(Response::Aborted {
            error_type: AuthorityErrorType::FileAccessAbortError,
            message: format!("write to {} aborted by user during review", path),
        })
    }

    fn write_authorized(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        let content = base64_decode(content_b64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad base64"))?;

        self.write_bytes_authorized(path, &content)
    }

    fn write_text_authorized(&mut self, path: &str, content: &str) -> io::Result<Response> {
        self.write_bytes_authorized(path, content.as_bytes())
    }

    fn write_bytes_authorized(&mut self, path: &str, content: &[u8]) -> io::Result<Response> {
        let full_path = self.authority.resolve(path);
        let tmp_dir = self.tmp_dir.clone();
        self.with_recorded_mutations(|_| {
            // Atomic write: tmp → fsync → rename
            atomic_write(&tmp_dir, &full_path, content)?;
            Ok(Response::Ok)
        })
    }

    fn handle_delete(&mut self, path: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write(path) {
            return Ok(resp);
        }
        self.delete_authorized(path)
    }

    fn handle_delete_confirmed(&mut self, path: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write_confirmed(path) {
            return Ok(resp);
        }
        self.delete_authorized(path)
    }

    fn delete_authorized(&mut self, path: &str) -> io::Result<Response> {
        let full_path = self.authority.resolve(path);
        self.with_recorded_mutations(|_| {
            if full_path.exists() {
                fs::remove_file(&full_path)?;
            }
            Ok(Response::Ok)
        })
    }

    fn handle_rename(&mut self, src: &str, dst: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write(src) {
            return Ok(resp);
        }
        self.rename_authorized(src, dst)
    }

    fn handle_rename_confirmed(&mut self, src: &str, dst: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write_confirmed(src) {
            return Ok(resp);
        }
        self.rename_authorized(src, dst)
    }

    fn rename_authorized(&mut self, src: &str, dst: &str) -> io::Result<Response> {
        let src_path = self.authority.resolve(src);
        let dst_path = self.authority.resolve(dst);
        self.with_recorded_mutations(|_| {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&src_path, &dst_path)?;
            Ok(Response::Ok)
        })
    }

    fn handle_query(&self, path: &str) -> Response {
        if let RuleVerdict::Deny = self.authority.check_read(path) {
            return Response::Permission {
                level: AccessLevel::None,
            };
        }
        let level = match self.authority.check_write(path) {
            RuleVerdict::Allow => AccessLevel::Write,
            _ => AccessLevel::Read,
        };
        Response::Permission { level }
    }

    fn handle_fetch(&mut self, url: &str) -> io::Result<Response> {
        let domain = extract_domain(url).unwrap_or_default();
        let allowed = match self.authority.check_web(&domain) {
            RuleVerdict::Allow => true,
            RuleVerdict::Deny => false,
            RuleVerdict::Prompt => match self.authority.editable_approval(
                "Web Access",
                url,
                AuthorityErrorType::WebAccessAbortError,
            ) {
                Approval::Approved(edited_url) => return self.perform_fetch(&edited_url),
                Approval::Aborted {
                    error_type,
                    message,
                } => {
                    return Ok(Response::Aborted {
                        error_type,
                        message,
                    });
                }
            },
        };
        if !allowed {
            return Ok(Response::Denied {
                reason: "web access denied".into(),
            });
        }

        self.perform_fetch(url)
    }

    fn handle_fetch_confirmed(&mut self, url: &str) -> io::Result<Response> {
        let domain = extract_domain(url).unwrap_or_default();
        if let RuleVerdict::Deny = self.authority.check_web(&domain) {
            return Ok(Response::Denied {
                reason: "web access denied".into(),
            });
        }
        self.perform_fetch(url)
    }

    fn perform_fetch(&mut self, url: &str) -> io::Result<Response> {
        let url_hash = hex_sha256(url.as_bytes());
        let cache_file = self.web_cache_dir.join("pages").join(&url_hash);

        if !cache_file.exists() {
            let client = reqwest::blocking::Client::builder()
                .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
                .timeout(std::time::Duration::from_secs(30))
                .danger_accept_invalid_certs(false)
                .build()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("reqwest client: {e}")))?;

            let response = client
                .get(url)
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                )
                .header("Accept-Language", "en-US,en;q=0.9")
                .send()
                .map_err(|e| {
                    io::Error::new(io::ErrorKind::Other, format!("reqwest fetch failed: {e}"))
                })?;

            if !response.status().is_success() {
                return Ok(Response::Denied {
                    reason: format!("HTTP {}", response.status()),
                });
            }

            let body = response
                .bytes()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("reqwest read: {e}")))?;

            fs::write(&cache_file, &body)?;

            // Update web cache index
            let index_path = self.web_cache_dir.join("index.jsonl");
            let mut idx = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&index_path)?;
            writeln!(idx, "{{\"url\":\"{}\",\"file\":\"{}\"}}", url, url_hash)?;
        }

        let bytes = fs::metadata(&cache_file)?.len();
        Ok(Response::Fetched {
            path: format!(".arcana/web_cache/pages/{}", url_hash),
            bytes,
        })
    }

    fn handle_exec(&mut self, cmd: &str, args: &[String]) -> io::Result<Response> {
        let allowed = match self.authority.check_tool(cmd, args) {
            RuleVerdict::Allow => true,
            RuleVerdict::Deny => false,
            RuleVerdict::Prompt => {
                let full_cmd = format!("{} {}", cmd, args.join(" "));
                match self.authority.editable_approval(
                    "Tool Call",
                    &full_cmd,
                    AuthorityErrorType::ToolCallAbortError,
                ) {
                    Approval::Approved(edited) => {
                        if edited == full_cmd {
                            true
                        } else {
                            return self.handle_exec_shell(&edited, false);
                        }
                    }
                    Approval::Aborted {
                        error_type,
                        message,
                    } => {
                        return Ok(Response::Aborted {
                            error_type,
                            message,
                        });
                    }
                }
            }
        };
        if !allowed {
            return Ok(Response::Denied {
                reason: "command not allowed".into(),
            });
        }

        let project_root = self.authority.project_root().to_path_buf();
        self.exec_with_recording(|| {
            Command::new(cmd)
                .args(args)
                .current_dir(&project_root)
                .output()
        })
    }

    fn handle_exec_confirmed(&mut self, cmd: &str, args: &[String]) -> io::Result<Response> {
        if let RuleVerdict::Deny = self.authority.check_tool(cmd, args) {
            return Ok(Response::Denied {
                reason: "command not allowed".into(),
            });
        }

        let project_root = self.authority.project_root().to_path_buf();
        self.exec_with_recording(|| {
            Command::new(cmd)
                .args(args)
                .current_dir(&project_root)
                .output()
        })
    }

    fn handle_exec_shell(&mut self, command: &str, readonly: bool) -> io::Result<Response> {
        let command = match self.authority.editable_approval(
            "Tool Call",
            command,
            AuthorityErrorType::ToolCallAbortError,
        ) {
            Approval::Approved(command) => command,
            Approval::Aborted {
                error_type,
                message,
            } => {
                return Ok(Response::Aborted {
                    error_type,
                    message,
                });
            }
        };

        let project_root = self.authority.project_root().to_path_buf();
        if readonly {
            self.exec_without_recording(|| {
                Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&project_root)
                    .output()
            })
        } else {
            self.exec_with_recording(|| {
                Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&project_root)
                    .output()
            })
        }
    }

    fn handle_exec_shell_confirmed(
        &mut self,
        command: &str,
        readonly: bool,
    ) -> io::Result<Response> {
        let args = vec!["-c".to_string(), command.to_string()];
        if let RuleVerdict::Deny = self.authority.check_tool("sh", &args) {
            return Ok(Response::Denied {
                reason: "command not allowed".into(),
            });
        }

        let project_root = self.authority.project_root().to_path_buf();
        if readonly {
            self.exec_without_recording(|| {
                Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(&project_root)
                    .output()
            })
        } else {
            self.exec_with_recording(|| {
                Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(&project_root)
                    .output()
            })
        }
    }

    fn handle_register_tool(
        &mut self,
        name: &str,
        path: &str,
        args: &[String],
        description: &str,
    ) -> io::Result<Response> {
        if !self.authority.runtime_registration_allowed() {
            return Ok(Response::Denied {
                reason: "runtime registration disabled".into(),
            });
        }
        let command_pattern = if args.is_empty() {
            path.to_string()
        } else {
            format!("{} {}", path, args.join(" "))
        };
        eprintln!("[Arcana] Tool registration requested: {name} ({description})");
        let approved_pattern = match self.authority.editable_approval(
            "Tool Registration",
            &command_pattern,
            AuthorityErrorType::ToolRegistrationAbortError,
        ) {
            Approval::Approved(edited) => edited,
            Approval::Aborted {
                error_type,
                message,
            } => {
                return Ok(Response::Aborted {
                    error_type,
                    message,
                });
            }
        };
        self.with_recorded_mutations(|srv| {
            srv.authority.register_tool_runtime(name);
            srv.authority.register_command(&approved_pattern)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_tool_confirmed(
        &mut self,
        name: &str,
        path: &str,
        args: &[String],
        _description: &str,
    ) -> io::Result<Response> {
        if !self.authority.runtime_registration_allowed() {
            return Ok(Response::Denied {
                reason: "runtime registration disabled".into(),
            });
        }
        let command_pattern = if args.is_empty() {
            path.to_string()
        } else {
            format!("{} {}", path, args.join(" "))
        };
        self.with_recorded_mutations(|srv| {
            srv.authority.register_tool_runtime(name);
            srv.authority.register_command(&command_pattern)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_command(&mut self, pattern: &str) -> io::Result<Response> {
        let approved_pattern = match self.authority.editable_approval(
            "Tool Registration",
            pattern,
            AuthorityErrorType::ToolRegistrationAbortError,
        ) {
            Approval::Approved(pattern) => pattern,
            Approval::Aborted {
                error_type,
                message,
            } => {
                return Ok(Response::Aborted {
                    error_type,
                    message,
                });
            }
        };
        self.with_recorded_mutations(|srv| {
            srv.authority.register_command(&approved_pattern)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_command_confirmed(&mut self, pattern: &str) -> io::Result<Response> {
        self.with_recorded_mutations(|srv| {
            srv.authority.register_command(pattern)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_web(&mut self, domain: &str) -> io::Result<Response> {
        let approved_domain = match self.authority.editable_approval(
            "Web Access Registration",
            domain,
            AuthorityErrorType::WebAccessRegistrationAbortError,
        ) {
            Approval::Approved(domain) => domain,
            Approval::Aborted {
                error_type,
                message,
            } => {
                return Ok(Response::Aborted {
                    error_type,
                    message,
                });
            }
        };
        self.with_recorded_mutations(|srv| {
            srv.authority.register_web(&approved_domain)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_web_confirmed(&mut self, domain: &str) -> io::Result<Response> {
        self.with_recorded_mutations(|srv| {
            srv.authority.register_web(domain)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_filesystem(
        &mut self,
        access: crate::types::FilesystemAccess,
        path: &str,
    ) -> io::Result<Response> {
        let approved_path = match self.authority.editable_approval(
            "File Access Registration",
            path,
            AuthorityErrorType::FileAccessRegistrationAbortError,
        ) {
            Approval::Approved(path) => path,
            Approval::Aborted {
                error_type,
                message,
            } => {
                return Ok(Response::Aborted {
                    error_type,
                    message,
                });
            }
        };
        self.with_recorded_mutations(|srv| {
            srv.authority.register_filesystem(access, &approved_path)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_register_filesystem_confirmed(
        &mut self,
        access: crate::types::FilesystemAccess,
        path: &str,
    ) -> io::Result<Response> {
        self.with_recorded_mutations(|srv| {
            srv.authority.register_filesystem(access, path)?;
            srv.regenerate_prompt()?;
            Ok(Response::Ok)
        })
    }

    fn handle_prompt(&self) -> io::Result<Response> {
        let content = prompt::generate_prompt(&self.authority)?;
        Ok(Response::Prompt { content })
    }

    fn handle_instruction(&self) -> io::Result<Response> {
        let content = prompt::load_instruction()?;
        Ok(Response::Instruction { content })
    }

    fn authorize_write(&self, path: &str) -> Result<(), Response> {
        match self.authority.check_write(path) {
            RuleVerdict::Allow => Ok(()),
            RuleVerdict::Deny => Err(Response::Denied {
                reason: "write access denied".into(),
            }),
            RuleVerdict::Prompt => match self.authority.approval(
                "File Access",
                path,
                AuthorityErrorType::FileAccessAbortError,
            ) {
                Approval::Approved(_) => Ok(()),
                Approval::Aborted {
                    error_type,
                    message,
                } => Err(Response::Aborted {
                    error_type,
                    message,
                }),
            },
        }
    }

    fn authorize_write_confirmed(&self, path: &str) -> Result<(), Response> {
        match self.authority.check_write(path) {
            RuleVerdict::Deny => Err(Response::Denied {
                reason: "write access denied".into(),
            }),
            RuleVerdict::Allow | RuleVerdict::Prompt => Ok(()),
        }
    }

    fn regenerate_prompt(&self) -> io::Result<()> {
        let content = prompt::generate_prompt(&self.authority)?;
        fs::write(&self.prompt_path, &content)
    }

    fn with_recorded_mutations<F>(&mut self, action: F) -> io::Result<Response>
    where
        F: FnOnce(&mut Self) -> io::Result<Response>,
    {
        let before = self.record.scan_project_tree()?;
        let response = action(self)?;
        let after = self.record.scan_project_tree()?;
        let report = self.record.record_project_delta(before, after)?;
        Ok(attach_mutation_report(response, report))
    }

    fn exec_with_recording<F>(&mut self, run: F) -> io::Result<Response>
    where
        F: FnOnce() -> io::Result<Output>,
    {
        let before = self.record.scan_project_tree()?;
        let output = run()?;
        let after = self.record.scan_project_tree()?;
        let report = self.record.record_project_delta(before, after)?;

        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
            records: report.records,
            diff: report.diff,
        })
    }

    /// Run a shell command WITHOUT the before/after project-tree scan.
    /// Used for read-only safe commands (ls, grep, echo, …) where recording
    /// is unnecessary and would be prohibitively slow on large projects.
    fn exec_without_recording<F>(&mut self, run: F) -> io::Result<Response>
    where
        F: FnOnce() -> io::Result<Output>,
    {
        let output = run()?;
        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
            records: Vec::new(),
            diff: String::new(),
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

/// Atomic write: write to tmp, fsync, rename to target, fsync parent dir.
fn atomic_write(tmp_dir: &PathBuf, target: &PathBuf, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write to temp file
    let tmp_path = tmp_dir.join(format!(".tmp_{}", std::process::id()));
    let mut tmp_file = File::create(&tmp_path)?;
    tmp_file.write_all(content)?;
    tmp_file.sync_all()?; // fsync the content

    // Atomic rename
    fs::rename(&tmp_path, target)?;

    // fsync parent directory to ensure rename is durable
    if let Some(parent) = target.parent() {
        if let Ok(dir) = File::open(parent) {
            dir.sync_all().ok();
        }
    }
    Ok(())
}

fn send(writer: &mut impl Write, resp: &Response) -> io::Result<()> {
    let json = serde_json::to_string(resp).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    writeln!(writer, "{}", json)?;
    writer.flush()
}

fn attach_mutation_report(response: Response, report: MutationReport) -> Response {
    if report.records.is_empty() {
        return response;
    }
    match response {
        Response::Ok => Response::Mutation {
            records: report.records,
            diff: report.diff,
        },
        Response::ExecResult {
            stdout,
            stderr,
            code,
            ..
        } => Response::ExecResult {
            stdout,
            stderr,
            code,
            records: report.records,
            diff: report.diff,
        },
        other => other,
    }
}

fn extract_domain(url: &str) -> Option<String> {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let domain = without_scheme.split('/').next()?.split(':').next()?;
    Some(domain.to_string())
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let val = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' => continue,
            _ => return None,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// Web fetch is handled by reqwest (Rust HTTP client with rustls TLS).
// See perform_fetch() above.

/// Generate a unified diff showing only context around changed lines.
fn generate_unified_diff(original: &str, proposed: &str, path: &str) -> String {
    let old_lines: Vec<&str> = original.lines().collect();
    let new_lines: Vec<&str> = proposed.lines().collect();
    let max_len = old_lines.len().max(new_lines.len());

    // Mark changed line indices
    let mut changed = vec![false; max_len];
    for i in 0..old_lines.len().min(new_lines.len()) {
        if old_lines[i] != new_lines[i] {
            changed[i] = true;
        }
    }
    for i in old_lines.len().min(new_lines.len())..max_len {
        changed[i] = true;
    }

    // Include CONTEXT lines of context around each change
    const CONTEXT: usize = 3;
    let mut show = vec![false; max_len];
    for i in 0..max_len {
        if changed[i] {
            let s = i.saturating_sub(CONTEXT);
            let e = (i + CONTEXT + 1).min(max_len);
            for j in s..e {
                show[j] = true;
            }
        }
    }

    // Build hunks
    let mut diff = format!("diff --git a/{0} b/{0}\n--- a/{0}\n+++ b/{0}\n", path);
    let mut i = 0;
    while i < max_len {
        if !show[i] {
            i += 1;
            continue;
        }

        // Start of hunk — find the range of consecutive shown lines
        let hunk_start = i;
        while i < max_len && show[i] {
            i += 1;
        }
        let hunk_end = i;

        // Compute line numbers
        let old_start = (hunk_start + 1).min(old_lines.len().saturating_add(1));
        let new_start = (hunk_start + 1).min(new_lines.len().saturating_add(1));
        let mut old_count = 0u32;
        let mut new_count = 0u32;
        let mut hunk_text = String::new();

        for j in hunk_start..hunk_end {
            let o = old_lines.get(j).copied().unwrap_or("");
            let n = new_lines.get(j).copied().unwrap_or("");
            if j < old_lines.len() && j < new_lines.len() && o == n {
                hunk_text.push_str(&format!(" {}\n", o));
                old_count += 1;
                new_count += 1;
            } else {
                if j < old_lines.len() {
                    hunk_text.push_str(&format!("-{}\n", o));
                    old_count += 1;
                }
                if j < new_lines.len() {
                    hunk_text.push_str(&format!("+{}\n", n));
                    new_count += 1;
                }
            }
        }

        diff.push_str(&format!(
            "@@ -{},{} +{},{} @@\n{}",
            old_start, old_count, new_start, new_count, hunk_text
        ));
    }

    diff
}
