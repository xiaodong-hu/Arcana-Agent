use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::authority::Authority;

const DEFAULT_INSTRUCTION: &str = r#"# Interface for `Arcana Authority System (AAS)`
`Arcana Authority System` is used for every filesystem mutation, command execution, network request, and runtime tool change. Ask AAS when permission is unclear.

Communicate with the authority process through the Arcana-Agent AAS bridge by emitting one JSON object per line. Arcana-Agent relays each request to the session IPC channel and returns one JSON object per line back to you.
The request schema is exactly `{"op":"..."}`. Do not use wrapper schemas such as `{"command":"run_terminal_cmd","params":...}`.

## Discovery
```json
{"op":"instruction"}
{"op":"list_authority"}
{"op":"query","path":"README.md"}
{"op":"prompt"}
```

## Operations
```json
{"op":"read","path":"README.md"}
{"op":"write","path":"notes.md","content":"<base64-bytes>"}
{"op":"delete","path":"notes.md"}
{"op":"rename","src":"old.md","dst":"new.md"}
{"op":"exec","cmd":"cargo","args":["test"]}
{"op":"exec_shell","command":"cargo test\ncargo clippy"}
{"op":"fetch","url":"https://example.com","tag":null}
{"op":"register_tool","name":"tool-name","path":"binary-or-script","args":[],"description":"what it does"}
{"op":"register_command","pattern":"cargo test"}
{"op":"register_web","domain":"example.com"}
{"op":"register_filesystem","access":"writable","path":"src/**"}
```

`read` returns base64 file content. `write` requires base64 file content. `fetch` returns the project cache path and byte length; request `read` on that cache path if page content is needed. `exec_shell` is for multi-line executable command strings and always asks the human to approve, edit, or abort before execution.

Registration requests write approved entries to the project-level authority policy. Use registration only when an operation is not already allowed or denied by the supplied authority policies.

When you need AAS to do work, output only the JSON request lines first. Do not wrap them in markdown. After Arcana-Agent returns AAS responses, continue the user task from those results.

## Work Rule
Always try your best to use any available combination of AAS tools, commands,
filesystem authority, and network authority that can materially improve the
answer to the user's request. Do the work through AAS first, then answer from
the returned results. For temporary scripts, use `.arcana/tmp/`. For persistent
project files, prefer `write` so AAS records the mutation. If AAS denies or
aborts an operation, report that response and stop that operation.

## Abort Responses
If AAS returns `{"status":"aborted","error_type":"...","message":"..."}`, immediately report that error to the user and stop generation. Do not retry the same operation unless the user explicitly asks.

Known abort error types:
- `ToolCallAbortError`
- `FileAccessAbortError`
- `WebAccessAbortError`
- `ToolRegistrationAbortError`
- `FileAccessRegistrationAbortError`
- `WebAccessRegistrationAbortError`
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
    out.push_str("Send JSONL requests to the Unix socket `.arcana/authority.sock`.\n");
    out.push_str("Use `instruction` to reload this interface and `list_authority` to reload current policy.\n");

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
