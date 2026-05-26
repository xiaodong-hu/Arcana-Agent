use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::authority::Authority;

const DEFAULT_INSTRUCTION: &str = r#"# Arcana Authority System (AAS)

AAS is the only interface for command execution, filesystem access, web access,
and authority registration.

## Bridge Protocol
When an operation requires AAS, put the AAS request in the visible assistant
message, not in hidden reasoning/thinking. Output exactly one JSON object per
line, with no markdown wrapper, prose, or code fence, then stop the message.

Arcana-Agent will relay those JSON lines to AAS, run approved requests, and send
returned JSON back to you as the next user message. Continue only from returned
AAS JSON. Never invent stdout, stderr, status, files, web content, or tool
results before AAS returns them.

If AAS returns `{"status":"aborted",...}` or `{"status":"denied",...}`, report
that result and stop that operation. Do not retry, bypass, or simulate AAS.

## Common Operations
```json
{"op":"list_authority"}
{"op":"query","path":"README.md"}
{"op":"read_text","path":"README.md"}
{"op":"write_text","path":"notes.md","content":"plain UTF-8 text"}
{"op":"exec_shell","command":"cargo test --all"}
{"op":"fetch","url":"https://example.com","tag":null}
```

Use `read_text` and `write_text` for normal text files. Use byte-level
`read`/`write` with base64 content only when exact binary bytes are required.

## Other Operations
```json
{"op":"delete","path":"notes.md"}
{"op":"rename","src":"old.md","dst":"new.md"}
{"op":"exec","cmd":"cargo","args":["test"]}
{"op":"register_command","pattern":"cargo test --all"}
{"op":"register_web","domain":"example.com"}
{"op":"register_filesystem","access":"writable","path":"generated/**"}
```

Use `exec_shell` for ordinary shell commands. Use `.arcana/tmp/` for temporary
scripts. Even safe/no-confirmation operations still go through AAS using this
protocol.
"#;

pub fn load_or_create_instruction() -> io::Result<String> {
    let path = instruction_path()?;
    if path.exists() {
        return fs::read_to_string(path);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, DEFAULT_INSTRUCTION)?;
    Ok(DEFAULT_INSTRUCTION.to_string())
}

pub fn load_instruction() -> io::Result<String> {
    let path = instruction_path()?;
    fs::read_to_string(path)
}

pub fn generate_prompt(authority: &Authority) -> io::Result<String> {
    let instruction = load_or_create_instruction()?;
    let snapshot = authority.snapshot();
    let snapshot_json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut out = String::new();
    out.push_str(instruction.trim_end());
    out.push_str("\n\n## Session Authority Channel\n\n");
    out.push_str("AAS requests must appear in visible assistant content, never only in reasoning/thinking.\n");
    out.push_str("When you need AAS, output only JSONL request objects and stop; wait for Arcana-Agent to return AAS JSON before answering.\n");
    out.push_str("Do not fabricate command output, file content, web content, or AAS status.\n");
    out.push_str("Use `{\"op\":\"instruction\"}` to reload this interface and `{\"op\":\"list_authority\"}` to reload current policy.\n");

    out.push_str("\n## System-Wide Authority Policy\n\n```toml\n");
    match snapshot.configs.system_toml.as_deref() {
        Some(text) => out.push_str(text.trim_end()),
        None => out.push_str("# No system-wide authority policy was loaded."),
    }
    out.push_str("\n```\n");

    out.push_str("\n## Project-Level Authority Policy\n\n```toml\n");
    match snapshot.configs.project_toml.as_deref() {
        Some(text) => out.push_str(text.trim_end()),
        None => out.push_str("# No project-level authority policy exists yet. Approved registration APIs create `.arcana/authority.toml`."),
    }
    out.push_str("\n```\n");

    out.push_str("\n## Merged Authority Snapshot\n\n```json\n");
    out.push_str(&snapshot_json);
    out.push_str("\n```\n");
    out.push_str("\nThe TOML policies are the first source of truth for what is allowed or denied. The merged snapshot is an exact machine-readable view of both policies after merging. Enforce decisions by sending requests to AAS.\n");
    Ok(out)
}

fn instruction_path() -> io::Result<PathBuf> {
    let home = env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".arcana").join("INSTRUCTION.md"))
}
