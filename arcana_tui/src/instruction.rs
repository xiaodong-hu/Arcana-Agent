use std::path::PathBuf;

const DEFAULT_INSTRUCTION: &str = r#"# Arcana Authority System (AAS)
AAS can be called to communicate with Arcana-Agent for command execution, filesystem access, web access, and authority registration. 

To call AAS, output ONE SINGLE JSON object per line using the AAS API below, with NO markdown wrapper. Arcana-Agent will take action from those JSON lines passed via AAS, return JSON responses to you, and then you MUST continue from the returned results. 

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
For temporary scripts, use `.arcana/tmp/`. If AAS returns `{"status":"aborted",...}` or `{"status":"denied",...}`, report it and stop that operation. Do not retry or route around AAS.
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
