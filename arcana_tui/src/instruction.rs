use std::path::PathBuf;

const DEFAULT_INSTRUCTION: &str = r#"# Interface for `Arcana Authority System (AAS)`

`Arcana Authority System` is used for every filesystem mutation, command
execution, network request, and runtime authority change. Ask AAS when
permission is unclear.

Communicate with the authority process through the Arcana-Agent AAS bridge by
emitting one JSON object per line. Arcana-Agent relays each request to the
session IPC channel and returns one JSON object per line back to you.
The request schema is exactly `{"op":"..."}`. Do not use wrapper schemas such as
`{"command":"run_terminal_cmd","params":...}`.

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
{"op":"exec_shell","command":"cargo test --all"}
{"op":"fetch","url":"https://example.com","tag":null}
```

## Registration
```json
{"op":"register_tool","name":"tool-name","path":"binary-or-script","args":[],"description":"what it does"}
{"op":"register_command","pattern":"cargo test --all"}
{"op":"register_web","domain":"example.com"}
{"op":"register_filesystem","access":"writable","path":"generated/**"}
```

`read` returns base64 file content. `write` requires base64 file content. If a
request returns `{"status":"aborted","error_type":"..."}`, report that error to
the user and stop the current operation. Do not retry or route around AAS.

When you need AAS to do work, output only the JSON request lines first. Do not
wrap them in markdown. After Arcana-Agent returns AAS responses, continue the
user task from those results.

## Work Rule
Always try your best to use any available combination of AAS tools, commands,
filesystem authority, and network authority that can materially improve the
answer to the user's request. Do the work through AAS first, then answer from
the returned results. For temporary scripts, use `.arcana/tmp/`. For persistent
project files, prefer `write` so AAS records the mutation. If AAS denies or
aborts an operation, report that response and stop that operation.
"#;

pub fn path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = dirs::home_dir().ok_or("cannot find home directory")?;
    Ok(home.join(".arcana").join("INSTRUCTION.md"))
}

pub fn load_or_create() -> Result<String, Box<dyn std::error::Error>> {
    let path = path()?;
    if path.exists() {
        return Ok(std::fs::read_to_string(path)?);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, DEFAULT_INSTRUCTION)?;
    Ok(DEFAULT_INSTRUCTION.to_string())
}
