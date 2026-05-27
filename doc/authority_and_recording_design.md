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
| `fetch` | `url`, optional `tag` | `ok { path, bytes }` / `denied` / `aborted` |
| `exec` | `cmd`, `args` | `ok { stdout, stderr, code }` / `denied` / `aborted` |
| `exec_shell` | `command` | `ok { stdout, stderr, code }` / `aborted` |
| `register_tool` | `name`, `path`, `description` | `ok` / `denied` |
| `register_command` | `pattern` | `ok` / `aborted` |
| `register_web` | `domain` | `ok` / `aborted` |
| `register_filesystem` | `access`, `path` | `ok` / `aborted` |
| `instruction` | *(none)* | `{ content: "<markdown>" }` |
| `list_authority` | *(none)* | merged authority snapshot |
| `prompt` | *(none)* | full injected prompt |


### LLM Authority Prompt (`authorized_prompt.md`)

The authority program **auto-generates** a project-level file `.arcana/authorized_prompt.md` that serves as the first-line context exposed to LLMs at the beginning of each session. This file:

1. Includes the human-maintained API instruction.
2. Includes the loaded system-wide authority TOML.
3. Includes the loaded project-level authority TOML when present.
4. Includes the merged machine-readable authority snapshot.
5. Describes typed abort responses that the LLM must report and stop on.
6. Is regenerated on server startup and whenever tools/permissions change at runtime.

#### Generation

- **On server startup:** The authority program writes `.arcana/authorized_prompt.md` before accepting connections.
- **On hot-change:** After successful runtime registration or config reload, the file is regenerated.
- **CLI access:** `authority_and_recording auth instruction [project_root]` prints the instruction text.
- **IPC access:** `{"op": "prompt"}` returns the prompt content directly (not base64 — plain text).

#### Usage by TUI

The TUI (`arcana_tui`) reads `.arcana/authorized_prompt.md` and **mandatorily prepends** it to the system message at the start of each session. When the model emits AAS JSON request lines, Arcana-Agent asks the human to approve/edit/abort privileged operations, relays approved requests to `.arcana/authority.sock`, shows shell execution in an embedded `[Arcana Run]` panel and other authority operations in an `[Arcana Request]` panel, appends the JSON responses back into the conversation, and lets the model continue. This ensures the LLM always knows:
- What tools are available and how to invoke them.
- What permissions it has (so it doesn't attempt denied operations).
- That all operations go through the authority socket (not direct filesystem access).
- That natural-language requests should use any available combination of AAS
  tools, commands, filesystem authority, and network authority that can
  materially improve the answer.


#### Config (`~/.arcana/authority.toml` and `.arcana/authority.toml`)

Authority policy uses the same TOML schema at both levels. The system-wide file supplies global defaults. The project-level file supplies project additions and is created by approved registration APIs when needed. Both files are exposed to the LLM as first-line context, and the authority program enforces the merged view.

```toml
[commands]
safe = ["ls", "cat", "rg", "git status", "git diff"]
allow = ["cargo test"]
confirm = ["git commit", "git push", "rm"]
deny = ["sudo *"]

[network]
allow = ["docs.rs", "crates.io", "github.com"]
deny = ["*"]

[filesystem]
writable = ["src/**", "Cargo.toml"]
readonly = ["/etc", "/usr"]
deny = [".env", "secrets/**"]
```

Rule evaluation order (for all rule types): deny → allow → default.

#### Human Approval and Edit Loop

For prompt-required operations, AAS shows a typed confirmation prompt:

```text
[Tool Call] LLM requires `cargo test`. Confirm Allowance?
    - Yes and Run [y/Enter]
    - No and Edit [e]
    - No and Abort [n/a]:
```

Editing opens `$EDITOR` with the LLM request. Saving returns to the same confirmation prompt with the edited value. Aborting returns a typed response such as `ToolCallAbortError`, `WebAccessAbortError`, or `FileAccessRegistrationAbortError`. The LLM is instructed to surface the error to the user and stop the current operation.


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

Defined in the `[commands]` section of `~/.arcana/authority.toml` and project
`.arcana/authority.toml`. These are available immediately when the agent starts:
- `safe` — read-only commands the agent can run through AAS without human confirmation.
- `allow` — commands permitted by AAS; Arcana-Agent may still show human confirmation unless also listed in `safe`.
- `confirm` — commands that require human approval and optional editing.
- `deny` — commands that are always blocked.
- Unlisted commands → runtime default behavior.

#### Runtime Registration (by SKILLs/MCPs)

The agent (or its skill/MCP plugins) can request to register new tools at runtime:

```json
{"op": "register_tool", "name": "cargo_test", "path": "/usr/bin/cargo", "args": ["test"], "description": "Run cargo tests"}
```

- The authority program prompts the user with the same allow/edit/abort loop used
  for execution requests.
- Once approved, the tool or command authority is added to the session's allowed
  list and **persisted to project `.arcana/authority.toml`**.
- If denied or aborted, the authority program returns a typed abort response such
  as `ToolRegistrationAbortError`.

#### Execution Flow

```
Agent: {"op": "exec", "cmd": "ls", "args": ["-la", "src/"]}
Authority:
  1. Check deny list → not denied
  2. Check safe/allow list → "ls" is allowed
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
  3. Check confirm list or runtime default → ask user
  4. User approves, edits, or aborts
  5. Approved command executes and returns captured stdout/stderr/status
```


### Record System (Git-like Action Tree)

Integrated into the authority program. The recorder treats mutation as an actual project-tree delta, not as a hard-coded command name. Before an approved mutating API or command runs, AAS scans the recoverable project tree and stores all pre-operation blobs. After the operation finishes, AAS scans again, records every added/modified/deleted path, and returns a git-compatible unified diff to the agent/TUI.

This means shell commands are recordable without trying to predict which command names are mutating: if `python`, `cargo`, `sed`, or any other approved command changes files under the project, the before/after tree delta is recorded.

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

Volatile authority runtime data and `.gitignore`-excluded paths are excluded from recovery records:

```
.git/
.arcana/git_record/
.arcana/authority.sock
.arcana/authorized_prompt.md
.arcana/web_cache/
.arcana/tmp/
# plus any paths matching .gitignore patterns (data/, *.h5, log/, etc.)
```

#### .gitignore Integration

The recording system parses `.gitignore` from the project root on startup.  Patterns are converted to glob-matchable rules and applied during every tree scan.  This means large untracked directories (e.g. `data/`, `figures/`, `log/`) are never hashed or stored in `objects/`.

- **Handled syntax:** comments (`#`), blank lines, trailing `/` for directories, leading `/` for root-relative paths, bare filename globs.
- **Not handled (yet):** negation patterns (`!`), nested `.gitignore` files in subdirectories.
- **On workspace creation** the TUI reports how many `.gitignore` patterns were found.
- **If no `.gitignore` exists** all project files are tracked — add one with `data/`, `*.h5`, etc. to speed up the baseline scan.

**Performance impact** (HyperDet_VMC, 43 GB project, 22 GB in `data/`):

| | Without .gitignore | With .gitignore |
|---|---|---|
| Paths tracked | 677 | 29 |
| Baseline scan | 61.4 s | 0.1 s |
| Incremental mutation | 0.02 s | 0.01 s |

Project authority configuration, such as `.arcana/authority.toml`, is recordable because registration changes are meaningful project state.

#### Action Log Entry Format

```json
{"seq":1,"ts":"1760000000s_since_epoch","op":"write","path":"src/main.rs","blob":"abcdef...","prev_blob":"123456..."}
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

**Recovery is a subcommand of the same `authority_and_recording` binary** — no separate program needed. Invoked as:
```
authority_and_recording recovery <project_root> --list
authority_and_recording recovery <project_root> --to-sequence <N>
```

**Algorithm:**
1. Find the latest snapshot with `seq ≤ target_seq`.
2. Load that snapshot's tree state (path → blob hash).
3. Replay `actions.jsonl` entries from `snapshot.seq + 1` through `target_seq`, applying each op to the in-memory tree.
4. For each path in the final tree, read the blob from `objects/` and write it to the project directory.
5. Delete any recoverable project files that exist on disk but not in the recovered tree.
6. Preserve recorder storage itself, so recovery works even if the rest of the project was removed.
7. Record the recovery delta as a new append-only mutation instead of moving `HEAD` backward. This keeps all future states addressable and avoids duplicate sequence numbers after rollback.

**If no snapshot exists** (or recovering to seq < first snapshot): replay all actions from seq 1.

**Full recovery guarantee:** Given `objects/` + `actions.jsonl`, the entire project can be reconstructed at any point in time. Snapshots only accelerate recovery — they are not required for correctness.

The TUI command is:

```
arcana recovery [<project>] --list
arcana recovery [<project>] --to-sequence N
```

Both commands show a boxed warning and ask for confirmation before overwriting
the working tree. `arcana recovery` defaults to the current directory when no
project path is supplied.


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
    ExecShell { command: String },
    RegisterTool { name: String, path: String, args: Vec<String>, description: String },
    RegisterCommand { pattern: String },
    RegisterWeb { domain: String },
    RegisterFilesystem { access: FilesystemAccess, path: String },
    Instruction,
    ListAuthority,
    Prompt,
}

pub enum Response {
    Ok,
    Denied { reason: String },
    Aborted { error_type: AuthorityErrorType, message: String },
    Permission { level: AccessLevel },
    Content { data: String },           // base64 file content
    Fetched { path: String, bytes: u64 },
    ExecResult { stdout: String, stderr: String, code: i32 },
    Instruction { content: String },
    Authority { snapshot: AuthoritySnapshot },
    Prompt { content: String },
}
```


## Open Questions

- [ ] Web cache eviction policy (max size? TTL?).
- [x] Should runtime-registered authority persist across sessions? **YES.** Approved registrations are persisted to project `.arcana/authority.toml`; system-wide policy remains user-owned.
- [ ] Rate limiting on exec requests to prevent abuse.
