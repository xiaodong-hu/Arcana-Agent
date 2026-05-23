# High-level Design for Agent `Arcana`

By default, the agent should only have read access to the given project directory. A `.arcana` folder is created as its workspace.

Security is enforced at **two layers**:
1. **Rust ownership (compile-time):** The agent never holds writable file handles. All mutations flow through a single gatekeeper process.
2. **OS-level confinement (runtime):** The agent process is sandboxed so it *cannot* bypass the gatekeeper via raw syscalls.


## The Authority & Record Mechanism (Single Process)
### Authority Program
A single rust binary `authority_and_record` **owns all write access**, **records every mutation**, **manages web fetches**, and **controls tool/command execution**.

There is no separate "record program"; recording is an integral step of every authorized operation.
```
┌─────────────────┐     IPC (unix socket)      ┌──────────────────────────────┐
│  Agent Process  │  ── request(op, path) ──►  │  Authority & Record Program  │
│  (sandboxed,    │  ◄── result / denied ────  │  (user privilege)            │
│   no write fds) │                            │                              │
└─────────────────┘                            │  1. Check access rules       │
                                               │  2. Prompt user if needed    │
                                               │  3. Record action to log     │
                                               │  4. Execute filesystem op    │
                                               │     (atomic write + fsync)   │
                                               │  5. ACK back to agent        │
                                               └──────────────────────────────┘
```

Why Merged? Every mutable operation MUST be recorded. By placing recording inside the authority gate, we guarantee:
- **No write can happen without a record** (they share the same code path).
- **Atomic**: if recording fails, the write is not performed.
- **Crash-safe**: writes use tmp+rename+fsync — no partial writes on crash.
- Single process = simpler failure modes, no IPC between authority and record.


#### Core API (over unix socket, JSON messages)

| Request | Fields | Response |
|---------|--------|----------|
| `read` | `path` | `ok { content }` / `denied` |
| `write` | `path`, `content` (base64) | `ok` / `denied { reason }` |
| `delete` | `path` | `ok` / `denied` |
| `rename` | `src`, `dst` | `ok` / `denied` |
| `query` | `path` | `{ permission: "none" \| "read" \| "write" }` |
| `fetch` | `url`, optional `tag` | `ok { path, bytes }` / `denied` |
| `exec` | `cmd`, `args` | `ok { stdout, stderr, code }` / `denied` |
| `register_tool` | `name`, `path`, `description` | `ok` / `denied` |
| `prompt` | *(none)* | `{ content: "<markdown>" }` |


### LLM Authority Prompt (`authorized_prompt.md`)

The authority program **auto-generates** a project-level file `.arcana/authorized_prompt.md` that serves as the first-line context exposed to LLMs at the beginning of each session. This file:

1. **Describes all available tools** with their exact JSON request format.
2. **Lists current permissions** (allowed/denied write paths, read-deny rules, web domains, command whitelist).
3. **Explains the IPC protocol** (unix socket, one JSON per line, one response per line).
4. **Is regenerated** on server startup and whenever tools/permissions change at runtime (e.g., after `register_tool`).

#### Generation

- **On server startup:** The authority program writes `.arcana/authorized_prompt.md` before accepting connections.
- **On hot-change:** After a successful `register_tool` or config reload, the file is regenerated.
- **CLI access:** `authority_and_recording auth prompt [project_root]` prints the prompt to stdout and writes the file.
- **IPC access:** `{"op": "prompt"}` returns the prompt content directly (not base64 — plain text).

#### Usage by TUI

The TUI (`arcana_tui`) reads `.arcana/authorized_prompt.md` and **mandatorily prepends** it to the system message at the start of each session. This ensures the LLM always knows:
- What tools are available and how to invoke them.
- What permissions it has (so it doesn't attempt denied operations).
- That all operations go through the authority socket (not direct filesystem access).


#### Config (`.arcana/access.toml`)

```toml
[rules]
allow_write = ["src/**/*.rs", ".arcana/scratch/**"]
deny_write  = [".env", "secrets/**", ".arcana/git_record/**"]
deny_read   = [".env", "secrets/**", ".git/config"]
default = "prompt"

[web]
default = "prompt"
allow_domains = ["docs.rs", "crates.io", "github.com"]
deny_domains  = []

[tools]
# Static whitelist: commands the agent can always execute
allow = ["ls", "cat", "find", "grep", "head", "tail", "wc", "diff", "tree"]
# Commands that are always denied (even if agent tries to register them)
deny  = ["rm -rf /", "sudo", "su", "chmod", "chown"]
# Whether the agent can register new tools at runtime
allow_runtime_registration = true
# Default for unlisted commands
default = "prompt"
```

Rule evaluation order (for all rule types): deny → allow → default.


### Read-Deny Rules

By default the agent can read all project files. But sensitive files (secrets, credentials, private keys) should be blocked:

- `deny_read` patterns are checked on every `read` and `query` request.
- A denied read returns `permission: "none"` (file appears non-existent to the agent).
- Read-deny is **also enforced for `exec` output** — if a command would output a denied file's content, the authority program redacts it.

Note: OS-level user isolation already gives the agent read access to project files (group-readable). Read-deny is an **application-level** restriction on top — the authority program simply refuses to relay the content. The agent *could* technically read the raw file via group permission, but since all its I/O goes through the authority socket, it has no practical way to do so without a tool.


### Atomic Write Guarantee

All filesystem writes follow this sequence to prevent corruption on crash:

```
1. Write content to temporary file: .arcana/tmp/<random>
2. fsync() the temporary file
3. fsync() the parent directory of the target
4. rename() tmp file → target path (atomic on POSIX)
5. fsync() the target's parent directory
6. ACK to agent
```

If the process crashes at any point before step 4, the target file is untouched. If it crashes during/after step 4, the write is complete (rename is atomic). This guarantees the agent never sees a partial write acknowledged.


### Tool/Command Execution Whitelist

The agent cannot execute arbitrary system commands. All execution goes through the authority program.

#### Static Whitelist (config-time)

Defined in `[tools]` section of `access.toml`. These are available immediately when the agent starts:
- `allow` — commands the agent can run without prompting.
- `deny` — commands that are always blocked.
- Unlisted commands → `default` behavior (prompt/allow/deny).

#### Runtime Registration (by SKILLs/MCPs)

The agent (or its skill/MCP plugins) can request to register new tools at runtime:

```json
{"op": "register_tool", "name": "cargo_test", "path": "/usr/bin/cargo", "args": ["test"], "description": "Run cargo tests"}
```

- If `allow_runtime_registration = true`, the authority program prompts the user once: `"Agent wants to register tool 'cargo_test' (/usr/bin/cargo test). Allow? [y/N]"`
- Once approved, the tool is added to the session's allowed list and **persisted to `~/.arcana/tools.toml`** for future sessions.
- If `allow_runtime_registration = false`, all registration requests are denied.

#### Execution Flow

```
Agent: {"op": "exec", "cmd": "ls", "args": ["-la", "src/"]}
Authority:
  1. Check deny list → not denied
  2. Check allow list → "ls" is allowed
  3. Execute: spawn "ls -la src/" as real user
  4. Capture stdout/stderr
  5. Return: {"status": "ok", "stdout": "...", "stderr": "", "code": 0}
```

For commands not in allow list:
```
Agent: {"op": "exec", "cmd": "cargo", "args": ["build"]}
Authority:
  1. Check deny → no
  2. Check allow → no
  3. Check runtime registered → no
  4. default = "prompt" → ask user
  5. User approves → execute and return result
```


### Record System (Git-like Action Tree)

Integrated into the authority program. Every approved mutation is recorded *before* the filesystem write is committed.

#### Recording Storage Layout

```
.arcana/git_record/
├── objects/            # content-addressed blobs (sha256)
│   ├── ab/
│   │   └── cdef1234…
│   └── …
├── actions.jsonl       # append-only action log (one JSON line per action)
├── snapshots/          # periodic full-tree snapshots (kept: last 50)
│   ├── 000050.json
│   ├── 000100.json
│   └── …
└── HEAD                # latest action sequence number
```

#### Action Log Entry Format

```json
{"seq": 1, "ts": "2026-05-19T21:30:00Z", "op": "write", "path": "src/main.rs", "blob": "abcdef…", "prev_blob": "null"}
```

#### Snapshot Policy

Snapshots are **mutation-count-based**: a snapshot is taken every **N mutations** (default N=50, configurable). Each snapshot captures the full tree state (path → blob hash mapping).

**Retention:** Only the last **50 snapshots** are kept. Older snapshots are deleted. This bounds storage while still allowing fast recovery to any of the last 2500 mutations (50 snapshots × 50 mutations each) without replaying from the beginning.

Snapshot format:
```json
{"seq": 100, "ts": "…", "tree": {"src/main.rs": "abcdef…", "src/lib.rs": "123456…"}}
```

Note: `objects/` blobs are **never** garbage-collected — they are needed for full recovery from `actions.jsonl`. Only snapshot metadata files are pruned.


#### Recovery

Recovery reconstructs the project state at any given sequence number.

**Recovery is a subcommand of the same `authority_and_record` binary** — no separate program needed. Invoked as:
```
authority_and_record recover <project_root> [--to-seq <N>]
```

**Algorithm:**
1. Find the latest snapshot with `seq ≤ target_seq`.
2. Load that snapshot's tree state (path → blob hash).
3. Replay `actions.jsonl` entries from `snapshot.seq + 1` through `target_seq`, applying each op to the in-memory tree.
4. For each path in the final tree, read the blob from `objects/` and write it to the project directory.
5. Delete any files that exist on disk but not in the recovered tree.

**If no snapshot exists** (or recovering to seq < first snapshot): replay all actions from seq 1.

**Full recovery guarantee:** Given `objects/` + `actions.jsonl`, the entire project can be reconstructed at any point in time. Snapshots only accelerate recovery — they are not required for correctness.


### Web Fetch Management

The authority program also manages the agent's web access. Fetched content is stored **separately** from the recording system — web pages are context for the AI, not project mutations.

#### Storage

```
.arcana/web_cache/
├── index.jsonl         # log of all fetches: {"ts", "url", "tag", "file"}
└── pages/
    ├── <sha256_of_url>.html
    └── …
```

- Fetched pages are stored by URL hash in `pages/`.
- `index.jsonl` maps URLs to cached files for lookup.
- The agent can re-read cached pages without re-fetching.
- Web cache is **not** recorded in `git_record/` — it's ephemeral context, not project state.

#### Access Control

Web fetch requests go through the same authority check:
- `deny_domains` → blocked immediately.
- `allow_domains` → fetched without prompt.
- Otherwise → prompt user: `"Agent wants to fetch https://example.com. Allow? [y/N]"`

#### Why Separate from Recording?

The recording system exists for **project recovery** — reconstructing your source code if the AI damages it. Web fetches are:
- Read-only context (not project mutations).
- Potentially large (megabytes of HTML).
- Rarely needed for recovery (the project doesn't depend on them).

Mixing them would bloat `objects/` and `actions.jsonl` with irrelevant data.


## OS-Level Confinement

Rust's type system prevents capability leaks *within* the compiled system. But the agent process could **theoretically issue raw syscalls to bypass the authority program**. OS-level confinement closes this gap.

### Decision: Unix User Isolation (Cross-Platform)

Since Arcana targets **multiple Unix-like systems** (Linux, macOS, BSDs), we use **Unix User Isolation** — the only confinement mechanism that works identically across all POSIX systems.

#### How It Works

1. A dedicated system user `arcana-agent` is created (no login shell, no home directory needed).
2. Project files are owned by the real user with permissions `rwxr-x---` (group readable, not world-writable).
3. The `arcana-agent` user is added to the project owner's group → gains **read access**.
4. The `arcana-agent` user has **no write permission** on any project file.
5. The `.arcana/authority.sock` socket is group-writable → agent can connect.
6. The authority program runs as the **real user** and performs writes on the agent's behalf.

#### Setup (one-time)

```bash
# Create the agent user (Linux)
sudo useradd -r -s /usr/sbin/nologin arcana-agent

# Create the agent user (macOS)
sudo dscl . -create /Users/arcana-agent
sudo dscl . -create /Users/arcana-agent UserShell /usr/bin/false

# Add to user's group
sudo usermod -aG $(id -gn) arcana-agent   # Linux
sudo dscl . -append /Groups/staff GroupMembership arcana-agent  # macOS
```

#### Runtime

The `authority_and_record` binary:
1. Starts as the real user.
2. Creates `.arcana/` directory and unix socket (group-writable).
3. Spawns the agent process as `arcana-agent` (via `setuid`/`sudo -u`).
4. Agent inherits: read access to project + connect access to socket. **Nothing else.**


### Network Confinement

**Network access does NOT need a separate program.** The authority program already mediates all web fetches — the agent sends `fetch` requests over the unix socket, and the authority program performs the actual HTTP call.

The agent process (running as `arcana-agent`) has no reason to make direct network connections. As defense-in-depth, you can optionally block outbound network for the agent user at the OS level:

```bash
# Linux (iptables owner match)
sudo iptables -A OUTPUT -m owner --uid-owner arcana-agent -j DROP

# macOS (pf)
echo "block out quick on any from any to any user arcana-agent" | sudo pfctl -ef -
```

This is **optional hardening** — the primary enforcement is that the agent only communicates through the authority socket, and the authority program controls what gets fetched. The `iptables/pf` rules just ensure that even if the agent somehow opens a raw socket, the packets are dropped.


## Rust Ownership Design

```rust
// The agent side CANNOT construct this — it lives only in authority_and_record
pub struct WriteCapability {
    fd: std::fs::File,
    path: PathBuf,
}

pub enum Request {
    Write { path: String, content: Vec<u8> },
    Delete { path: String },
    Rename { src: String, dst: String },
    Read { path: String },
    Query { path: String },
    Fetch { url: String, tag: Option<String> },
    Exec { cmd: String, args: Vec<String> },
    RegisterTool { name: String, path: String, args: Vec<String>, description: String },
}

pub enum Response {
    Ok,
    Denied { reason: String },
    Permission { level: AccessLevel },
    Content { data: String },           // base64 file content
    Fetched { path: String, bytes: u64 },
    ExecResult { stdout: String, stderr: String, code: i32 },
}
```


## Open Questions

- [ ] Web cache eviction policy (max size? TTL?).
- [x] Should runtime-registered tools persist across sessions? **YES.** Approved tools are persisted to `~/.arcana/tools.toml` so users can "hot-plug" SKILLs that survive restarts. See `agent_running_design.md` §2 for full SKILL lifecycle design.
- [ ] Rate limiting on exec requests to prevent abuse.
