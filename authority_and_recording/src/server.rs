use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::authority::{Approval, Authority};
use crate::prompt;
use crate::record::Record;
use crate::types::{AccessLevel, AuthorityErrorType, Request, Response, RuleVerdict};

pub struct Server {
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
        let authority = Authority::load(project_root.clone())?;
        let record = Record::open(&project_root)?;

        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }
        fs::create_dir_all(socket_path.parent().unwrap())?;
        fs::create_dir_all(web_cache_dir.join("pages"))?;
        fs::create_dir_all(&tmp_dir)?;

        // Generate authorized_prompt.md on startup
        let prompt_content = prompt::generate_prompt(&authority)?;
        fs::write(&prompt_path, &prompt_content)?;
        eprintln!("[arcana] Generated {:?}", prompt_path);

        Ok(Self {
            socket_path,
            authority,
            record,
            web_cache_dir,
            tmp_dir,
            prompt_path,
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        let listener = UnixListener::bind(&self.socket_path)?;
        eprintln!("[arcana] Listening on {:?}", self.socket_path);
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    if let Err(e) = self.handle_connection(s) {
                        eprintln!("[arcana] Error: {}", e);
                    }
                }
                Err(e) => eprintln!("[arcana] Accept error: {}", e),
            }
        }
        Ok(())
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
            Request::Write { path, content } => self.handle_write(&path, &content),
            Request::WriteConfirmed { path, content } => {
                self.handle_write_confirmed(&path, &content)
            }
            Request::Delete { path } => self.handle_delete(&path),
            Request::DeleteConfirmed { path } => self.handle_delete_confirmed(&path),
            Request::Rename { src, dst } => self.handle_rename(&src, &dst),
            Request::RenameConfirmed { src, dst } => self.handle_rename_confirmed(&src, &dst),
            Request::Query { path } => Ok(self.handle_query(&path)),
            Request::Fetch { url, tag: _ } => self.handle_fetch(&url),
            Request::FetchConfirmed { url, tag: _ } => self.handle_fetch_confirmed(&url),
            Request::Exec { cmd, args } => self.handle_exec(&cmd, &args),
            Request::ExecConfirmed { cmd, args } => self.handle_exec_confirmed(&cmd, &args),
            Request::ExecShell { command } => self.handle_exec_shell(&command),
            Request::ExecShellConfirmed { command } => self.handle_exec_shell_confirmed(&command),
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

    fn handle_write(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write(path) {
            return Ok(resp);
        }
        self.write_authorized(path, content_b64)
    }

    fn handle_write_confirmed(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        if let Err(resp) = self.authorize_write_confirmed(path) {
            return Ok(resp);
        }
        self.write_authorized(path, content_b64)
    }

    fn write_authorized(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        let content = base64_decode(content_b64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad base64"))?;

        let full_path = self.authority.resolve(path);
        let prev_blob = self.record.hash_file(&full_path)?;
        let blob = self.record.store_blob(&content)?;
        self.record
            .append("write", path, Some(blob), prev_blob, None)?;

        // Atomic write: tmp → fsync → rename
        atomic_write(&self.tmp_dir, &full_path, &content)?;
        Ok(Response::Ok)
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
        let prev_blob = self.record.hash_file(&full_path)?;
        self.record.append("delete", path, None, prev_blob, None)?;
        if full_path.exists() {
            fs::remove_file(&full_path)?;
        }
        Ok(Response::Ok)
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
        let prev_blob = self.record.hash_file(&src_path)?;
        self.record
            .append("rename", src, None, prev_blob, Some(dst.to_string()))?;
        if let Some(parent) = dst_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&src_path, &dst_path)?;
        Ok(Response::Ok)
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
            // Try curl first (most robust), then wget, then w3m -dump.
            let fetched = try_curl_fetch(url, &cache_file)
                .or_else(|e| {
                    eprintln!("[arcana] curl failed for {url}: {e}; trying wget...");
                    try_wget_fetch(url, &cache_file)
                })
                .or_else(|e| {
                    eprintln!("[arcana] wget failed for {url}: {e}; trying w3m -dump...");
                    try_w3m_fetch(url, &cache_file)
                });

            if let Err(e) = fetched {
                eprintln!("[arcana] All fetch methods failed for {url}: {e}");
                return Ok(Response::Denied {
                    reason: format!("fetch failed: {e}"),
                });
            }

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

    fn handle_exec(&self, cmd: &str, args: &[String]) -> io::Result<Response> {
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
                            return self.handle_exec_shell(&edited);
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

        let output = Command::new(cmd)
            .args(args)
            .current_dir(self.authority.project_root())
            .output()?;

        let code = output.status.code().unwrap_or(-1);
        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code,
        })
    }

    fn handle_exec_confirmed(&self, cmd: &str, args: &[String]) -> io::Result<Response> {
        if let RuleVerdict::Deny = self.authority.check_tool(cmd, args) {
            return Ok(Response::Denied {
                reason: "command not allowed".into(),
            });
        }

        let output = Command::new(cmd)
            .args(args)
            .current_dir(self.authority.project_root())
            .output()?;

        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        })
    }

    fn handle_exec_shell(&self, command: &str) -> io::Result<Response> {
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

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(self.authority.project_root())
            .output()?;

        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        })
    }

    fn handle_exec_shell_confirmed(&self, command: &str) -> io::Result<Response> {
        let args = vec!["-c".to_string(), command.to_string()];
        if let RuleVerdict::Deny = self.authority.check_tool("sh", &args) {
            return Ok(Response::Denied {
                reason: "command not allowed".into(),
            });
        }

        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(self.authority.project_root())
            .output()?;

        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        })
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
        eprintln!("[arcana] Tool registration requested: {name} ({description})");
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
        self.authority.register_tool_runtime(name);
        self.authority.register_command(&approved_pattern)?;
        self.regenerate_prompt()?;
        Ok(Response::Ok)
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
        self.authority.register_tool_runtime(name);
        self.authority.register_command(&command_pattern)?;
        self.regenerate_prompt()?;
        Ok(Response::Ok)
    }

    fn handle_register_command(&mut self, pattern: &str) -> io::Result<Response> {
        match self.authority.editable_approval(
            "Tool Registration",
            pattern,
            AuthorityErrorType::ToolRegistrationAbortError,
        ) {
            Approval::Approved(pattern) => self.authority.register_command(&pattern)?,
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
        self.regenerate_prompt()?;
        Ok(Response::Ok)
    }

    fn handle_register_command_confirmed(&mut self, pattern: &str) -> io::Result<Response> {
        self.authority.register_command(pattern)?;
        self.regenerate_prompt()?;
        Ok(Response::Ok)
    }

    fn handle_register_web(&mut self, domain: &str) -> io::Result<Response> {
        match self.authority.editable_approval(
            "Web Access Registration",
            domain,
            AuthorityErrorType::WebAccessRegistrationAbortError,
        ) {
            Approval::Approved(domain) => self.authority.register_web(&domain)?,
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
        self.regenerate_prompt()?;
        Ok(Response::Ok)
    }

    fn handle_register_web_confirmed(&mut self, domain: &str) -> io::Result<Response> {
        self.authority.register_web(domain)?;
        self.regenerate_prompt()?;
        Ok(Response::Ok)
    }

    fn handle_register_filesystem(
        &mut self,
        access: crate::types::FilesystemAccess,
        path: &str,
    ) -> io::Result<Response> {
        match self.authority.editable_approval(
            "File Access Registration",
            path,
            AuthorityErrorType::FileAccessRegistrationAbortError,
        ) {
            Approval::Approved(path) => self.authority.register_filesystem(access, &path)?,
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
        self.regenerate_prompt()?;
        Ok(Response::Ok)
    }

    fn handle_register_filesystem_confirmed(
        &mut self,
        access: crate::types::FilesystemAccess,
        path: &str,
    ) -> io::Result<Response> {
        self.authority.register_filesystem(access, path)?;
        self.regenerate_prompt()?;
        Ok(Response::Ok)
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

// ---------------------------------------------------------------------------
// Multi-tool fetch helpers (tried in order)
// ---------------------------------------------------------------------------

/// Common browser-emulation headers shared by all fetch methods.
const BROWSER_HEADERS: &[&str] = &[
    "-H",
    "User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "-H",
    "Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
    "-H",
    "Accept-Language: en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7",
    "-H",
    "Accept-Encoding: gzip, deflate, br",
    "-H",
    "DNT: 1",
    "-H",
    "Upgrade-Insecure-Requests: 1",
];

/// Try curl with retries and browser headers.
fn try_curl_fetch(url: &str, out_path: &PathBuf) -> Result<(), String> {
    for attempt in 1..=3 {
        let status = Command::new("curl")
            .args([
                "-sL",
                "--compressed",
                "--max-time",
                "30",
                "--retry",
                "0",
                "--http2",
            ])
            .args(BROWSER_HEADERS)
            .args(["-o"])
            .arg(out_path)
            .arg(url)
            .status()
            .map_err(|e| format!("curl spawn failed: {e}"))?;

        if status.success() {
            // Verify we got actual content, not an empty file
            if let Ok(meta) = std::fs::metadata(out_path) {
                if meta.len() > 0 {
                    return Ok(());
                }
            }
        }

        if attempt < 3 {
            std::thread::sleep(std::time::Duration::from_millis(500 * attempt));
        }
    }
    Err("curl: all attempts failed".into())
}

/// Try wget with browser headers.
fn try_wget_fetch(url: &str, out_path: &PathBuf) -> Result<(), String> {
    let status = Command::new("wget")
        .args(["-q", "--timeout=30", "--tries=2"])
        .args(
            BROWSER_HEADERS
                .iter()
                .flat_map(|h| ["--header", h.strip_prefix("-H ").unwrap_or(h)]),
        )
        .args(["-O"])
        .arg(out_path)
        .arg(url)
        .status()
        .map_err(|e| format!("wget spawn failed: {e}"))?;

    if status.success() {
        if let Ok(meta) = std::fs::metadata(out_path) {
            if meta.len() > 0 {
                return Ok(());
            }
        }
    }
    Err("wget: fetch failed".into())
}

/// Try w3m -dump (text-mode browser, no JS execution, safe).
fn try_w3m_fetch(url: &str, out_path: &PathBuf) -> Result<(), String> {
    let output = Command::new("w3m")
        .args(["-dump", "-no-graph", "-cols", "120"])
        .arg(url)
        .output()
        .map_err(|e| format!("w3m spawn failed: {e}"))?;

    if output.status.success() && !output.stdout.is_empty() {
        // w3m -dump outputs rendered plain text to stdout — write it to the cache file.
        std::fs::write(out_path, &output.stdout).map_err(|e| format!("w3m write failed: {e}"))?;
        return Ok(());
    }
    // w3m may also have useful stderr for debugging
    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            eprintln!("[arcana] w3m stderr for {url}: {stderr}");
        }
    }
    Err("w3m: fetch failed".into())
}
