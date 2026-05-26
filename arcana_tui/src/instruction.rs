use std::path::PathBuf;

const DEFAULT_INSTRUCTION: &str = r#"# Arcana Authority System (AAS)
AAS is the only interface for command execution, filesystem access, web access, and authority registration.

## Bridge Protocol
When an operation requires AAS, put the AAS request in the visible assistant message, not in hidden reasoning/thinking. Output exactly one JSON object per line, with no markdown wrapper, prose, or code fence, then stop the message.

Arcana-Agent will relay those JSON lines to AAS, run approved requests, and send returned JSON back to you as the next user message. Continue only from returned AAS JSON. Never invent stdout, stderr, status, files, web content, or tool results before AAS returns them.

If AAS returns `{"status":"aborted",...}` or `{"status":"denied",...}`, report that result and stop that operation. Do not retry, bypass, or simulate AAS.

## Common Operations
```json
{"op":"list_authority"}
{"op":"query","path":"README.md"}
{"op":"read_text","path":"README.md"}
{"op":"write_text","path":"notes.md","content":"plain UTF-8 text"}
{"op":"exec_shell","command":"cargo test --all"}
{"op":"fetch","url":"https://example.com","tag":null}
```
Use `read_text` and `write_text` for normal text files. Use byte-level `read`/`write` with base64 content only when exact binary bytes are required.

## Other Operations
```json
{"op":"delete","path":"notes.md"}
{"op":"rename","src":"old.md","dst":"new.md"}
{"op":"exec","cmd":"cargo","args":["test"]}
{"op":"register_command","pattern":"cargo test --all"}
{"op":"register_web","domain":"example.com"}
{"op":"register_filesystem","access":"writable","path":"generated/**"}
```

Use `exec_shell` for ordinary shell commands. Use `.arcana/tmp/` for temporary scripts. Even safe/no-confirmation operations still go through AAS using this protocol.
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
