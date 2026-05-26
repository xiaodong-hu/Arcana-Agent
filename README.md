<p align="center">
  <pre>
     ╔═══════════════════════════════════════════════════════════════════════════════════╗
     ║                                                                                   ║
     ║                           ░█▀█░█▀▄░█▀▀░█▀█░█▀█░█▀█                                ║
     ║                           ░█▀█░█▀▄░█░░░█▀█░█░█░█▀█                                ║
     ║                           ░▀░▀░▀░▀░▀▀▀░▀░▀░▀░▀░▀░▀                                ║
     ║                                                                                   ║
     ║   Authority & Recording · Multistage Memory · Skill Modules · Sub-Agents Support  ║
     ║                                                                                   ║
     ╚═══════════════════════════════════════════════════════════════════════════════════╝
  </pre>
</p>

<p align="center">
  <strong>A 100% RUST-written AI agent that memorize, involves, collaborates, and operates under strict RUST-managed authority and recording system.</strong>
</p>

<p align="center">
  <em>A full autonomous agent runtime with memory persistence, skill composition, hybrid LLM combinations, sub-agent orchestration, and cryptographic rust-managed authority/recording control — all in your terminal.</em>
</p>

---

## Why Arcana

Every existing coding agent is a **stateless parrot** — it forgets everything the moment you close the terminal. Arcana is different:

| Problem | Arcana's Answer |
|---------|-----------------|
| Agents forget context between sessions | **Multistage memory** — semantic knowledge store survives across sessions, with human-like foget mechanism |
| No control over what agents can access | **Authority system** — every file write is recorded, reviewable, recoverable |
| One model fits all | **Hybrid LLM routing** — different models for different agent roles |
| Skills are hardcoded | **Composable skill modules** — multilevel, trigger-based, hot-loadable, user-extensible |
| Sub-agents are fire-and-forget | **Orchestrated sub-agents** — checkpointed, freezable, resumable |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              TERMINAL                                    │
│                                                                         │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │                        arcana (TUI)                               │  │
│  │  ┌─────────────────────────────────────────────────────────────┐  │  │
│  │  │ Status: deepseek-v4-pro │ [████░░░░░░] 2k/1M │ Tasks: 2/7 | Sub-Agents: 3 │ Skills (System/User): 9/2 │  │  │
│  │  ├─────────────────────────────────────────────────────────────┤  │  │
│  │  │ Viewport (streaming responses, thinking blocks, diffs)      │  │  │
│  │  ├─────────────────────────────────────────────────────────────┤  │  │
│  │  │ Composer (multiline input)                                  │  │  │
│  │  └─────────────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                              │                                           │
│                    Unix Socket IPC                                       │
│                              │                                           │
│  ┌───────────┐  ┌───────────┴───────────┐  ┌─────────────────────────┐ │
│  │  Skills   │  │  Authority & Record   │  │  Sub-Agent Orchestrator │ │
│  │  Daemon   │  │  Daemon               │  │                         │ │
│  │           │  │                       │  │  ┌─────┐ ┌─────┐       │ │
│  │ triggers  │  │  • Permission gate    │  │  │Agent│ │Agent│ ...   │ │
│  │ manifests │  │  • Git-like recording │  │  │  1  │ │  2  │       │ │
│  │ hot-load  │  │  • Crash recovery     │  │  └─────┘ └─────┘       │ │
│  └───────────┘  └───────────────────────┘  └─────────────────────────┘ │
│                              │                                           │
│                    ┌─────────┴─────────┐                                │
│                    │   Memory System   │                                 │
│                    │                   │                                 │
│                    │  • Knowledge DB   │                                 │
│                    │  • Error patterns │                                 │
│                    │  • Session recall │                                 │
│                    │  • Embeddings     │                                 │
│                    └───────────────────┘                                 │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Features

### 1. Rust-Hosted Strict Authority & Recording System

Every privileged operation is gated by AAS. File writes, deletes, renames, authority registrations, and any project-tree changes caused by approved shell commands are recorded by comparing the project tree before and after the operation. Each recorded mutation returns a git-compatible diff for review. **The agent cannot silently overwrite your project** — every recorded change is recoverable from `.arcana/git_record`.

```
.arcana/git_record/
├── objects/          # Content-addressed blobs
├── actions.jsonl     # Append-only action log
├── snapshots/        # Periodic full snapshots
└── HEAD              # Current sequence number
```

Inspect recorded mutations before recovering: `arcana recovery --list`. Recover any recorded state, even after project files were deleted as long as `.arcana/git_record` remains: `arcana recovery --to-sequence 42`

#### Command Authorization

```toml
# ~/.arcana/authority.toml — editable before or after onboard
[commands]
safe = [
    "git status", "git diff", "git log",
    "ls", "cat", "find", "grep", "rg",
    "head", "tail", "wc", "sort", "uniq", "tree",
]
allow = [
    "cargo build", "cargo test", "cargo clippy", "cargo fmt",
    "python3", "node",
]

confirm = ["git push", "git commit", "rm -rf", "sudo *"]

[network]
allow = [
    "api.deepseek.com", "api.openai.com", "api.anthropic.com",
    "scholar.google.com", "arxiv.org", "*.arxiv.org",
    "en.wikipedia.org", "*.wikipedia.org", "wiki.archlinux.org",
    "stackoverflow.com", "*.stackoverflow.com", "*.stackexchange.com",
    "docs.rs", "crates.io", "github.com", "gitlab.com",
    "zhihu.com", "*.zhihu.com",
]
deny = ["*"]

[filesystem]
writable = ["."]
readonly = ["/etc", "/usr"]
deny = ["~/.ssh", "~/.gnupg", "~/.arcana/authority.toml"]
```

Runtime management: `\authorization list|add|remove|edit`

`[commands.safe]` is the no-confirmation pool for read-only commands such as
`ls` and `cat`. These commands still execute through AAS; they simply skip the
human confirmation panel. `[commands.allow]` remains the broader AAS permission
pool; Arcana-Agent may still ask before running those commands. Project file
reads are also no-confirmation by default for paths inside the current
workspace, except project `.arcana/`.

#### LLM Authority Instruction

The authority program reads the human-maintained authority instruction and auto-generates `.arcana/authorized_prompt.md` as mandatory first-line LLM context. The generated prompt includes:

- the API-only `~/.arcana/INSTRUCTION.md`,
- the loaded system-wide authority TOML,
- the loaded project-level `.arcana/authority.toml` when present,
- the merged machine-readable authority snapshot.

Agents interact with authority by emitting JSONL requests. Arcana-Agent detects those request lines, asks the human to approve/edit/abort privileged operations in the TUI, relays approved requests to the session authority socket, shows shell execution in an expandable `[Arcana Run]` panel and other authority operations as compact `[Arcana Request]` lines, and returns the JSON responses to the model:

```bash
# View the instruction text:
arcana auth instruction
```

```json
{"op":"instruction"}
{"op":"list_authority"}
{"op":"query","path":"README.md"}
{"op":"fetch","url":"https://example.com","tag":null}
{"op":"exec_shell","command":"cargo test\ncargo clippy"}
{"op":"register_command","pattern":"cargo test"}
{"op":"register_web","domain":"example.com"}
{"op":"register_filesystem","access":"writable","path":"src/**"}
```

Unlisted operations can be approved, edited, or aborted by the human. Abort responses are typed, for example `ToolCallAbortError`, `WebAccessAbortError`, and `FileAccessRegistrationAbortError`; the agent must report them and stop that operation. Approved registrations are persisted to project-level `.arcana/authority.toml`, creating it if needed. The generated prompt is refreshed on server startup and after runtime authority changes.

For natural-language requests, the injected instruction tells the model to use any available combination of AAS tools, commands, filesystem authority, and network authority that can materially improve the answer. Temporary scripts should be written under project `.arcana/tmp/`; persistent files should use the recorded `write` API.

---

### 2. Hot-Plug Multilayer Skill Module System

Skills operate at three levels, all hot-loadable:

| Level | Scope | Modifiable by | Description |
|-------|-------|---------------|-------------|
| **System (immutable)** | All projects, all sessions | Nobody (hardcoded) | Core agent behavior, safety constraints |
| **System (evolvable)** | All projects, all sessions | LLM + Human | Self-improving skills the agent updates over time |
| **Project (user)** | Per-project | Human | Custom triggers, workflows, domain knowledge |

```toml
# ~/.arcana/skills/user/my-skill/manifest.toml
[skill]
name = "deploy-checker"
trigger = { pattern = "deploy|ship|release" }
mode = "inject"    # inject context when triggered
```

All skills are hot-reloadable — add/remove/modify without restarting.

---

### 3. Multistage Memory System

Memory persists across sessions with multiple layers:

```
┌─────────────────────────────────────────────────────┐
│  Long-term Persistent Vector DB (cross-project)     │
│  • Knowledge store (semantic search)                │
│  • Error patterns (never repeat mistakes)           │
│  • Thinking chain archive (cross-session recall)    │
│  • Queryable and editable by users                  │
├─────────────────────────────────────────────────────┤
│  Session-level Short-term Vector DB                 │
│  • Current conversation context                     │
│  • Reasoning chains (for DeepSeek cache hits)       │
│  • Tool call results                                │
├─────────────────────────────────────────────────────┤
│  Project-level Squeezed Markdown Memory             │
│  • PROJECT.md (editable by users)                   │
│  • Auto-generated summaries                         │
│  • Codebase understanding                           │
└─────────────────────────────────────────────────────┘
```

---

### 4. Human-in-the-Loop Interaction

The agent never acts alone on destructive operations. Every mutation goes through a human review cycle:

```
┌─────────────────────────────────────────────────────────┐
│  Editor ↔ Prompt Panel                                   │
│  • Ctrl+e opens $EDITOR with current prompt              │
│  • Full vim/neovim editing power (motions, LSP, plugins) │
│  • :wq flushes content back to prompt panel              │
│  • Seamless two-way: prompt → editor → prompt → send     │
│                                                          │
│  Diff Review (on file mutations):                        │
│  • Full unified diff with syntax coloring                │
│  • Accept / Edit in $EDITOR / Reject                     │
│  • Human can modify LLM's proposed changes before apply  │
│                                                          │
│  Authority Approval (on restricted operations):          │
│  • Single permission — approve this one mutation         │
│  • Trust session — approve all (dangerous)               │
│  • Human interrupt — pause, edit, then resume            │
│  • Reject and abort — agent must find alternative        │
└─────────────────────────────────────────────────────────┘
```

---

### 5. Orchestrated Sub-Agents

Checkpointed, freezable, resumable sub-agents to save tokens:

```
                    ┌──────────────────┐
                    │    Main Agent    │  ← deepseek-v4-pro
                    │  plans, reasons  │
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
     ┌────────▼───┐  ┌──────▼─────┐  ┌────▼────────┐
     │Query Agent │  │ Sub-Agent  │  │ Sub-Agent   │  ← deepseek-v4-flash
     │(persistent)│  │ (spawned)  │  │ (spawned)   │
     │ shares ctx │  │ scoped fs  │  │ scoped fs   │
     └────────────┘  └────────────┘  └─────────────┘
```

- Sub-agents cannot spawn further sub-agents (authority constraint)
- Each sub-agent has scoped filesystem access
- Checkpointable: freeze mid-task, resume later
- Query agent: persistent overlay via `Ctrl+/`

---

### 6. Hybrid LLM Configuration

Assign different models to different roles:

```toml
[agents.main]
provider = "deepseek"
model = "deepseek-v4-pro"

[agents.main.thinking]
enabled = true
reasoning_effort = "high"

[agents.query]
provider = "deepseek"
model = "deepseek-v4-pro"

[agents.sub]
provider = "deepseek"
model = "deepseek-v4-flash"    # Fast & cheap for parallel work
```

---

### 7. Per-Response Telemetry

Every LLM response shows exactly what it cost:

```
Cost: 0.0031 ( 1.2K in / 847 out )
Time: 2.4s
```

---

## Editor ↔ Agent Operation Flow

Arcana integrates seamlessly with your `$EDITOR` (neovim, vim, vscode):

```
┌─────────────────────────────────────────────────────────┐
│  Prompt Panel (TUI)                                      │
│  ❯ type here, or press Ctrl+e to open $EDITOR           │
│                                                          │
│  ┌─── Ctrl+e ───►  $EDITOR (full editing power)         │
│  │                  • vim motions, plugins, LSP          │
│  │                  • paste large code blocks            │
│  │                  • :wq to return                      │
│  │◄── :wq ──────   content flushed back to prompt       │
│  │                                                       │
│  │  Continue editing in prompt, or Enter to send         │
│  └───────────────────────────────────────────────────────│
│                                                          │
│  Diff Review (on file mutations):                        │
│  • Full unified diff display                             │
│  • Accept / Edit in $EDITOR / Reject                     │
│  • Human can modify LLM's proposed changes               │
│                                                          │
│  Authority Approval (on restricted operations):          │
│  • Single permission / Trust session / Interrupt / Reject│
└─────────────────────────────────────────────────────────┘
```

---

## Quick Start

```bash
# Install (from source)
cd arcana_tui && cargo build --release
cp target/release/arcana ~/.local/bin/

# First-time setup
arcana onboard

# Start working
cd your-project
arcana
```

### Key Commands

```bash
arcana                          # Interactive session
arcana <project-path>           # Launch in specific project directory
arcana -q "explain main.rs"    # Single-shot query
arcana --model deepseek-v4-flash  # Override model
arcana config show              # View configuration
arcana config edit              # Edit config in $EDITOR
arcana --reset [<project>]      # Reset project workspace (confirmation required)
arcana --reset --factory        # Reset global ~/.arcana/ (extra warning + confirmation)
arcana check                    # System health check
arcana resume --last            # Resume previous session
```

### Keybindings

| Key | Action |
|-----|--------|
| `Ctrl+/` | Toggle query agent overlay |
| `Ctrl+e` | Open `$EDITOR` for prompt editing |
| `Ctrl+b` | Stop LLM generation immediately |
| `Ctrl+o` | Toggle thinking chain expand/collapse |
| `Ctrl+x` | Toggle `[Arcana Run]` shell panel + diff truncation expand/collapse |
| `Ctrl+y` | Toggle mouse capture — release mouse for native text selection & copy |
| `Ctrl+j` / `Ctrl+k` | Scroll viewport down/up |
| `Ctrl+Enter` | Newline in composer (also `Shift+Enter`) |
| `Ctrl+w` | Delete word left |
| `Ctrl+h` / `Ctrl+l` | Move cursor word left/right |
| `Ctrl+Up` / `Ctrl+Down` | Jump to start/end of input |
| `Home` / `End` | Start/end of current line |
| `Tab` | Autocomplete command / insert spaces |
| `Ctrl+c` | Interrupt / clear composer |

> **Text selection:** `Ctrl+y` releases the mouse to the terminal for native
> text selection — select with mouse, copy with `Ctrl+Shift+C`. Press `Ctrl+y`
> again to restore mouse scrolling. The TUI stays visible throughout.

### TUI Commands (prefix: `\`)

Type `\` then press `↓` to browse all commands with arrow keys. Press `Esc` to exit selection.

| Command | Action |
|---------|--------|
| `\quit` | Exit session |
| `\help` | Show all commands and hotkeys |
| `\clear` | Clear viewport |
| `\mode` | Switch agent mode (Ask / Agent) |
| `\status` | Show model/token info |
| `\usage` | Session token/cost statistics |
| `\working_dir` | Show current working directory |
| `\check` | System health check |
| `\config list` | Show `~/.arcana/config.toml` |
| `\config edit` | Open config.toml in `$EDITOR` |
| `\authorization list` | Show authorized commands |
| `\authorization add <cmd>` | Add to allow list |
| `\authorization remove <cmd>` | Remove from allow list |
| `\authorization edit` | Open authority.toml in `$EDITOR` |
| `\instruction show` | Show `~/.arcana/INSTRUCTION.md` |
| `\instruction edit` | Open INSTRUCTION.md in `$EDITOR` |
| `\behavioral show` | Show `~/.arcana/BEHAVIORAL.md` |
| `\behavioral edit` | Edit behavioral line in `$EDITOR` |

---

Shell startup completions are available with `arcana completions bash`, `arcana completions zsh`, or `arcana completions fish`.

## Configuration

Config lives at `~/.arcana/config.toml`. Arcana prompts to create the global
workspace on first launch if it does not exist. Run `arcana onboard` for guided setup.

```bash
arcana config show    # Print current config
arcana config edit    # Open in $EDITOR
arcana config path    # Print file path
```

---

## System Prompt Architecture

Arcana dispatches the system prompt based on the current agent mode:

```
┌──────────────────────────────────────────────────────────┐
│  Ask Mode           Agent Mode                           │
│  ┌──────────┐       ┌───────────────────────────────┐   │
│  │ Simple   │       │ 1. authorized_prompt.md        │   │
│  │ research │       │    (structured authority.toml) │   │
│  │ prompt   │       │ 2. INSTRUCTION.md              │   │
│  └──────────┘       │    (pure AAS API reference)    │   │
│                     │ 3. BEHAVIORAL.md                │   │
│                     │    (user-editable "when to      │   │
│                     │     call tools" directive)      │   │
│                     └───────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
```

| File | Purpose | Editable |
|------|---------|----------|
| `INSTRUCTION.md` | AAS API reference (JSON ops, conventions) | `\instruction edit` |
| `BEHAVIORAL.md` | Behavioral line — tells LLM *when* to use tools | `\behavioral edit` |
| `authority.toml` | Structured allow/deny lists (fed via `authorized_prompt.md`) | `\authorization edit` |

Switch modes with `\mode` (↑↓ browse, Enter select). The system prompt
refreshes immediately on mode change or config edit.

---

## Project Structure

```
Arcana-Agent/
├── arcana_tui/              # Terminal UI (ratatui + crossterm)
├── authority_and_recording/ # Permission gate + mutation recording
├── human_in_loop_interaction/ # Diff review, session management
├── subagent_system/         # Sub-agent orchestration + checkpointing
├── skills_modules/          # Skill daemon, triggers, manifests
├── memory_system/           # Knowledge DB, embeddings, semantic search
└── doc/                     # Design documents
    ├── agent_usage.md       # User manual
    ├── tui_design.md        # TUI architecture
    ├── context_design.md    # LLM context harness design
    ├── human_in_loop_design.md  # Human-in-loop interaction design
    └── authority_and_recording_design.md  # Authority system design
```

---

## Design Philosophy

1. **The agent works for you, not the other way around.** Authority is non-negotiable — every destructive action requires explicit approval or pre-configured trust.

2. **Memory is not optional.** An agent that forgets is just an expensive autocomplete. Arcana accumulates understanding over time.

3. **Composition over monoliths.** Skills, sub-agents, and memory layers are independent, hot-swappable modules communicating over unix sockets.

4. **Transparency over magic.** Every token spent, every file touched, every decision made — visible, recorded, recoverable.

---

## ⚠️ Current Status — Unimplemented Features

> **Warning:** Arcana is in early development. The following features are designed but not yet implemented.

### Working Now
- [x] `arcana onboard` — interactive & non-interactive setup wizard
- [x] `arcana -q "..."` — single-shot LLM query (DeepSeek API, with thinking mode)
- [x] `arcana version` / `arcana check` / `arcana config show|path|edit`
- [x] `arcana --reset [<project>]` — reset project workspace (with confirmation)
- [x] `arcana --reset --factory` — reset global config (with extra warning)
- [x] Interactive TUI shell (viewport, composer, status bar, keybindings)
- [x] Collapsible task panel (Ctrl+T) with tree-style indicators
- [x] Interactive LLM streaming — TUI session sends messages to DeepSeek with SSE streaming
- [x] Thinking chain panel — collapsed by default, Ctrl+O to expand/collapse (works during streaming)
- [x] `arcana auth status|allow|deny|revoke|reset` — command authorization management
- [x] `~/.arcana/authority.toml` — hot-reloadable authority config (created on onboard)
- [x] Query agent overlay — `Ctrl+/` toggles persistent query sub-agent with full streaming support
- [x] Kitty keyboard protocol — reliable Ctrl+/, Ctrl+Enter, Shift+Enter detection
- [x] Auto-scroll with cursor-tracking threshold algorithm (adapts to window resize)
- [x] Editor integration — `Ctrl+E` opens `$EDITOR`, content flushed back to composer
- [x] Multiline composer — `Ctrl+Enter`/`Shift+Enter` for newlines, faithful formatting
- [x] History recall — `Up`/`Down` from empty prompt, breaks on any edit action
- [x] Markdown rendering — syntax highlighting, inline code, compact newlines
- [x] Welcome banner — gradient-colored ASCII art, scrollable in viewport history

### Not Yet Implemented
- [ ] **Session management** — `arcana session list|resume|rename|delete|export|import`
- [ ] **Session resume** — `arcana resume --last` / `arcana resume <id>`
- [ ] **Sub-agent orchestration** — spawning, checkpointing, freezing, resuming sub-agents
- [ ] **Skills daemon** — trigger-based skill loading, hot-reload, manifest parsing
- [ ] **Memory system** — knowledge DB, semantic search, error patterns, session recall
- [ ] **Embedding model download** — `arcana onboard` does not yet download `all-MiniLM-L6-v2.onnx`
- [x] **Authority & recording** — permission gate, git-like mutation recording, command-delta recording, and diff reporting
- [x] **`arcana recovery`** — inspect and restore project state from `git_record`
- [ ] **Tool calls** — shell execution, file read/write, search, web fetch (IPC protocol implemented)
- [ ] **Diff review panel** — interactive accept/reject of file mutations
- [ ] **OpenAI / Anthropic provider support** — only DeepSeek is wired up
- [ ] **Context caching** — leveraging DeepSeek's prefix caching for long contexts
- [ ] **Desktop notifications** — bell/notification on response complete

---

## License

Apache-2.0
