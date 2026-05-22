use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::authority::Authority;
use crate::prompt;
use crate::record::Record;
use crate::types::{AccessLevel, Request, Response, RuleVerdict};

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

        if socket_path.exists() { fs::remove_file(&socket_path)?; }
        fs::create_dir_all(socket_path.parent().unwrap())?;
        fs::create_dir_all(web_cache_dir.join("pages"))?;
        fs::create_dir_all(&tmp_dir)?;

        // Generate authorized_prompt.md on startup
        let prompt_content = prompt::generate_prompt(&authority);
        fs::write(&prompt_path, &prompt_content)?;
        eprintln!("[arcana] Generated {:?}", prompt_path);

        Ok(Self { socket_path, authority, record, web_cache_dir, tmp_dir, prompt_path })
    }

    pub fn run(&mut self) -> io::Result<()> {
        let listener = UnixListener::bind(&self.socket_path)?;
        eprintln!("[arcana] Listening on {:?}", self.socket_path);
        for stream in listener.incoming() {
            match stream {
                Ok(s) => { if let Err(e) = self.handle_connection(s) { eprintln!("[arcana] Error: {}", e); } }
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
            if line.is_empty() { continue; }
            let req: Request = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => { send(&mut writer, &Response::Denied { reason: format!("bad request: {}", e) })?; continue; }
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
            Request::Delete { path } => self.handle_delete(&path),
            Request::Rename { src, dst } => self.handle_rename(&src, &dst),
            Request::Query { path } => Ok(self.handle_query(&path)),
            Request::Fetch { url, tag: _ } => self.handle_fetch(&url),
            Request::Exec { cmd, args } => self.handle_exec(&cmd, &args),
            Request::RegisterTool { name, path, args, description } => self.handle_register_tool(&name, &path, &args, &description),
            Request::Prompt => self.handle_prompt(),
        }
    }

    fn handle_read(&self, path: &str) -> io::Result<Response> {
        if let RuleVerdict::Deny = self.authority.check_read(path) {
            return Ok(Response::Denied { reason: "read access denied".into() });
        }
        let full_path = self.authority.resolve(path);
        if !full_path.exists() {
            return Ok(Response::Denied { reason: "file not found".into() });
        }
        let content = fs::read(&full_path)?;
        Ok(Response::Content { data: base64_encode(&content) })
    }

    fn handle_write(&mut self, path: &str, content_b64: &str) -> io::Result<Response> {
        if !self.authorize_write(path) {
            return Ok(Response::Denied { reason: "write access denied".into() });
        }
        let content = base64_decode(content_b64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad base64"))?;

        let full_path = self.authority.resolve(path);
        let prev_blob = self.record.hash_file(&full_path)?;
        let blob = self.record.store_blob(&content)?;
        self.record.append("write", path, Some(blob), prev_blob, None)?;

        // Atomic write: tmp → fsync → rename
        atomic_write(&self.tmp_dir, &full_path, &content)?;
        Ok(Response::Ok)
    }

    fn handle_delete(&mut self, path: &str) -> io::Result<Response> {
        if !self.authorize_write(path) {
            return Ok(Response::Denied { reason: "write access denied".into() });
        }
        let full_path = self.authority.resolve(path);
        let prev_blob = self.record.hash_file(&full_path)?;
        self.record.append("delete", path, None, prev_blob, None)?;
        if full_path.exists() { fs::remove_file(&full_path)?; }
        Ok(Response::Ok)
    }

    fn handle_rename(&mut self, src: &str, dst: &str) -> io::Result<Response> {
        if !self.authorize_write(src) {
            return Ok(Response::Denied { reason: "write access denied".into() });
        }
        let src_path = self.authority.resolve(src);
        let dst_path = self.authority.resolve(dst);
        let prev_blob = self.record.hash_file(&src_path)?;
        self.record.append("rename", src, None, prev_blob, Some(dst.to_string()))?;
        if let Some(parent) = dst_path.parent() { fs::create_dir_all(parent)?; }
        fs::rename(&src_path, &dst_path)?;
        Ok(Response::Ok)
    }

    fn handle_query(&self, path: &str) -> Response {
        if let RuleVerdict::Deny = self.authority.check_read(path) {
            return Response::Permission { level: AccessLevel::None };
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
            RuleVerdict::Prompt => self.authority.prompt_user("fetch", url),
        };
        if !allowed { return Ok(Response::Denied { reason: "web access denied".into() }); }

        let url_hash = hex_sha256(url.as_bytes());
        let cache_file = self.web_cache_dir.join("pages").join(&url_hash);

        if !cache_file.exists() {
            let output = Command::new("curl")
                .args(["-sL", "--max-time", "30", "-o"])
                .arg(&cache_file).arg(url).output()?;
            if !output.status.success() {
                return Ok(Response::Denied { reason: "fetch failed".into() });
            }
            let index_path = self.web_cache_dir.join("index.jsonl");
            let mut idx = OpenOptions::new().create(true).append(true).open(&index_path)?;
            writeln!(idx, "{{\"url\":\"{}\",\"file\":\"{}\"}}", url, url_hash)?;
        }

        let bytes = fs::metadata(&cache_file)?.len();
        Ok(Response::Fetched { path: format!(".arcana/web_cache/pages/{}", url_hash), bytes })
    }

    fn handle_exec(&self, cmd: &str, args: &[String]) -> io::Result<Response> {
        let allowed = match self.authority.check_tool(cmd) {
            RuleVerdict::Allow => true,
            RuleVerdict::Deny => false,
            RuleVerdict::Prompt => {
                let full_cmd = format!("{} {}", cmd, args.join(" "));
                self.authority.prompt_user("exec", &full_cmd)
            }
        };
        if !allowed { return Ok(Response::Denied { reason: "command not allowed".into() }); }

        let output = Command::new(cmd).args(args)
            .current_dir(self.authority.project_root())
            .output()?;

        let code = output.status.code().unwrap_or(-1);
        Ok(Response::ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code,
        })
    }

    fn handle_register_tool(&mut self, name: &str, path: &str, _args: &[String], description: &str) -> io::Result<Response> {
        if !self.authority.register_tool(name) {
            return Ok(Response::Denied { reason: "runtime registration disabled".into() });
        }
        // Prompt user for approval
        let msg = format!("register tool '{}' ({}): {}", name, path, description);
        if !self.authority.prompt_user("register_tool", &msg) {
            return Ok(Response::Denied { reason: "user denied registration".into() });
        }
        // Regenerate prompt after tool registration
        let content = prompt::generate_prompt(&self.authority);
        fs::write(&self.prompt_path, &content)?;
        Ok(Response::Ok)
    }

    fn handle_prompt(&self) -> io::Result<Response> {
        let content = prompt::generate_prompt(&self.authority);
        Ok(Response::Content { data: content })
    }

    fn authorize_write(&self, path: &str) -> bool {
        match self.authority.check_write(path) {
            RuleVerdict::Allow => true,
            RuleVerdict::Deny => false,
            RuleVerdict::Prompt => self.authority.prompt_user("write", path),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) { let _ = fs::remove_file(&self.socket_path); }
}

/// Atomic write: write to tmp, fsync, rename to target, fsync parent dir.
fn atomic_write(tmp_dir: &PathBuf, target: &PathBuf, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }

    // Write to temp file
    let tmp_path = tmp_dir.join(format!(".tmp_{}", std::process::id()));
    let mut tmp_file = File::create(&tmp_path)?;
    tmp_file.write_all(content)?;
    tmp_file.sync_all()?; // fsync the content

    // Atomic rename
    fs::rename(&tmp_path, target)?;

    // fsync parent directory to ensure rename is durable
    if let Some(parent) = target.parent() {
        if let Ok(dir) = File::open(parent) { dir.sync_all().ok(); }
    }
    Ok(())
}

fn send(writer: &mut impl Write, resp: &Response) -> io::Result<()> {
    let json = serde_json::to_string(resp).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    writeln!(writer, "{}", json)?;
    writer.flush()
}

fn extract_domain(url: &str) -> Option<String> {
    let without_scheme = url.strip_prefix("https://")
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
        if chunk.len() > 1 { out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char); } else { out.push('='); }
        if chunk.len() > 2 { out.push(CHARS[(triple & 0x3F) as usize] as char); } else { out.push('='); }
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
            b'+' => 62, b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' => continue,
            _ => return None,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8); buf &= (1 << bits) - 1; }
    }
    Some(out)
}
