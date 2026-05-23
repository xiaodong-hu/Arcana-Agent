use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::authority::Authority;

const DEFAULT_INSTRUCTION: &str = r#"# Interface for `Arcana Authority System (AAS)`

`Arcana Authority System` is used for every filesystem mutation, command execution, network request, and runtime tool change. Ask AAS when permission is unclear.

Communicate with the authority process over the session IPC channel by sending one JSON object per line. Each request returns one JSON object on one line.

## Discovery

```json
{"op":"instruction"}
{"op":"list_authority"}
{"op":"query","path":"README.md"}
```

## Operations

```json
{"op":"read","path":"README.md"}
{"op":"write","path":"notes.md","content":"<base64-bytes>"}
{"op":"delete","path":"notes.md"}
{"op":"rename","src":"old.md","dst":"new.md"}
{"op":"exec","cmd":"cargo","args":["test"]}
{"op":"fetch","url":"https://example.com","tag":null}
{"op":"register_tool","name":"tool-name","path":"binary-or-script","args":[],"description":"what it does"}
```

`read` returns base64 file content. `write` requires base64 file content. If a request is denied, stop and report the denial to the user.
"#;

/// Read the human-maintained agent instruction, creating a compact default
/// only when the user has not created one yet.
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

/// Generate the mandatory first-line LLM context. The user instruction explains
/// how to call authority APIs; the snapshot is machine-readable current policy.
pub fn generate_prompt(authority: &Authority) -> io::Result<String> {
    let instruction = load_or_create_instruction()?;
    let snapshot = serde_json::to_string_pretty(&authority.snapshot())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut out = String::new();
    out.push_str(instruction.trim_end());
    out.push_str("\n\n## Session Authority Channel\n\n");
    out.push_str("Send JSONL requests to the Unix socket `.arcana/authority.sock`.\n");
    out.push_str("Use `instruction` to reload this interface and `list_authority` to reload current policy.\n");
    out.push_str("\n## Current Authority Snapshot\n\n```json\n");
    out.push_str(&snapshot);
    out.push_str("\n```\n");
    out.push_str("\nThis snapshot is informational. Enforce decisions by sending requests to the authority system.\n");
    Ok(out)
}

fn instruction_path() -> io::Result<PathBuf> {
    let home = env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".arcana").join("INSTRUCTION.md"))
}
