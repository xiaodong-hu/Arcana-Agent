use std::path::PathBuf;

const DEFAULT_INSTRUCTION: &str = r#"# Interface for `Arcana Authority System (AAS)`

`Arcana Authority System` is used for every filesystem mutation, command
execution, network request, and runtime authority change. Ask AAS when
permission is unclear.

Communicate with the authority process through the Arcana-Agent AAS bridge by
emitting one JSON object per line. Arcana-Agent relays each request to the
session IPC channel and returns one JSON object per line back to you.

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

## Natural-Language Tool Use
For ordinary user requests, decide when AAS is needed:
- If the task asks you to run code, test code, compute using a program, inspect
  local files, modify files, fetch a URL, or use an external command, call AAS
  first.
- For quick temporary scripts, prefer `exec_shell` and write temporary files
  under `.arcana/tmp/`, not `/tmp` or another absolute path.
- For persistent project files, prefer `write` so AAS records the mutation.
- If an allowed operation already exists, use it directly. If authority is
  missing and not denied, request registration.
- Do not merely describe a script when the user asks you to run or compute with
  it; call AAS, wait for the result, then answer.
- If the user asks you to write a script for a concrete input, such as factoring
  a specific integer, treat that as a request to verify the script. Use AAS to
  run the script unless the user explicitly says not to run it.
- Do not ask the user whether to run an allowed verification command. Emit the
  AAS request first; AAS will handle human approval when required.
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
