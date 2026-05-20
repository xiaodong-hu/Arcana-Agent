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
  <strong>A 100% RUST-written sovereign AI agent that remembers, involves, and operates under RUST-managed authority.</strong>
</p>

<p align="center">
  <em>A full autonomous agent runtime with memory persistence, skill composition, hybrid architectures, sub-agent orchestration, and cryptographic rust-managed authority control — all in your terminal.</em>
</p>

---

## Why Arcana

Every existing coding agent is a **stateless parrot** — it forgets everything the moment you close the terminal. Arcana is different:

| Problem | Arcana's Answer |
|---------|-----------------|
| Agents forget context between sessions | **Multistage memory** — semantic knowledge store survives across sessions, with human-like foget mechanism |
| No control over what agents do | **Authority system** — every file write is recorded, reviewable, recoverable |
| One model fits all | **Hybrid LLM routing** — different models for different agent roles |
| Skills are hardcoded | **Composable skill modules** — trigger-based, hot-loadable, user-extensible |
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
│  │  │ Status: ⚗ deepseek-v4-pro │ [████░░░░░░] 8.2K/1M | Sub-Agents: 0 | Loaded Skills: 3 | Tasks: 2/7 │  │  │
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

### Agent Hierarchy

```
                    ┌──────────────────┐
                    │    Main Agent    │  ← deepseek-v4-pro (configurable)
                    │  plans, reasons  │
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
     ┌────────▼───┐  ┌──────▼─────┐  ┌────▼────────┐
     │Query Agent │  │ Sub-Agent  │  │ Sub-Agent   │  ← deepseek-v4-flash
     │(persistent)│  │ (spawned)  │  │ (spawned)   │    (configurable)
     │ shares ctx │  │ scoped fs  │  │ scoped fs   │
     └────────────┘  └────────────┘  └─────────────┘
```

### Data Flow

```
User Input ──► Skill Triggers ──► Main Agent ──► Authority Gate ──► File System
                                       │                │
                                       │           git_record/
                                       │          (every mutation)
                                       ▼
                                  Memory Store
                              (knowledge, errors,
                               session history)
```

---

## Features

### Hybrid LLM Configuration

Assign different models to different roles. Use your most powerful model where it matters, cheap models where it doesn't:

```toml
[agents.main]
provider = "deepseek"
model = "deepseek-v4-pro"

[agents.main.thinking]
enabled = true
reasoning_effort = "max"

[agents.query]
provider = "deepseek"
model = "deepseek-v4-pro"

[agents.main.thinking]
enabled = true
reasoning_effort = "high"


[agents.sub]
provider = "deepseek"
model = "deepseek-v4-flash"    # Fast & cheap for parallel work

[agents.main.thinking]
enabled = true
reasoning_effort = "high"

```

### Authority & Recording

Every file mutation and system command is gated and recorded. Full git-like history of agent actions:

```
.arcana/git_record/
├── objects/          # Content-addressed blobs
├── actions.jsonl     # Append-only action log
├── snapshots/        # Periodic full snapshots
└── HEAD              # Current sequence number
```

Recover any state: `arcana recover . --to-seq 42`

#### Command Authorization

The agent cannot execute system calls or network commands without explicit authorization. Authorized commands are managed via config or CLI:

```toml
# ~/.arcana/authority.toml — editable before or after onboard
[commands]
# Shell commands the agent is allowed to execute without confirmation
allow = [
    "cargo build",
    "cargo test",
    "cargo clippy",
    "git status",
    "git diff",
    "git log",
    "ls",
    "cat",
    "find",
    "grep",
    "rg",
]

# Commands that always require confirmation (even if pattern-matched above)
confirm = [
    "git push",
    "git commit",
    "rm -rf",
    "sudo *",
]

# Network access rules
[network]
allow = [
    "api.deepseek.com",
    "api.openai.com",
    "api.anthropic.com",
]
deny = ["*"]  # deny all other outbound by default

# File system scope (relative to project root)
[filesystem]
writable = ["."]           # project root
readonly = ["/etc", "/usr"]
deny = ["~/.ssh", "~/.gnupg", "~/.arcana/authority.toml"]
```

Manage at runtime:

```bash
arcana auth status              # Show all authorized commands/network/fs rules
arcana auth allow "cargo fmt"   # Add a command to the allow list
arcana auth deny "rm -rf /"     # Add to deny list
arcana auth revoke "git push"   # Remove from allow list
arcana auth reset               # Reset to defaults
```

The authority config is hot-reloadable — edit `~/.arcana/authority.toml` and changes take effect immediately, just like skill modules.

### Persistent Memory

Knowledge survives across sessions. The agent learns your codebase, your patterns, your mistakes:

- **Knowledge store** — semantic search over accumulated project understanding
- **Error patterns** — never repeat the same mistake twice
- **Session memory** — resume exactly where you left off

### Composable Skills

Hot-loadable, trigger-based skill modules:

```toml
# ~/.arcana/skills/user/my-skill/manifest.toml
[skill]
name = "deploy-checker"
trigger = { pattern = "deploy|ship|release" }
mode = "inject"    # inject context when triggered
```

### Per-Response Telemetry

Every LLM response shows exactly what it cost:

```
Expense: 0.0031 ( 1.2K in / 847 out )
Time: 2.4s
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
arcana -q "explain main.rs"    # Single-shot query
arcana --model deepseek-v4-flash  # Override model
arcana config show              # View configuration
arcana config edit              # Edit config in $EDITOR
arcana --reset                  # Factory reset
arcana check                    # System health check
arcana resume --last            # Resume previous session
```

### Keybindings

| Key | Action |
|-----|--------|
| `Ctrl+T` | Toggle tasks panel |
| `Ctrl+S` | Toggle skills panel |
| `Ctrl+A` | Toggle agents panel |
| `?` | Open query agent overlay |
| `Ctrl+C` | Interrupt / clear |
| `Ctrl+D` | End session |
| `Ctrl+Shift+P` | Freeze all agents |

---

## Configuration

Config lives at `~/.arcana/config.toml`. Created automatically on first launch.

```bash
arcana config show    # Print current config
arcana config edit    # Open in $EDITOR
arcana config path    # Print file path
```

See [doc/agent_usage.md](doc/agent_usage.md) §7 for the full configuration reference.

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
    ├── agent_running_design.md        # Agent runtime design
    └── authority_and_recording_design.md  # Authority system design
```

---

## Design Philosophy

1. **The agent works for you, not the other way around.** Authority is non-negotiable — every destructive action requires explicit approval or pre-configured trust.

2. **Memory is not optional.** An agent that forgets is just an expensive autocomplete. Arcana accumulates understanding over time.

3. **Composition over monoliths.** Skills, sub-agents, and memory layers are independent, hot-swappable modules communicating over unix sockets.

4. **Transparency over magic.** Every token spent, every file touched, every decision made — visible, recorded, recoverable.

---

## Documentation

| Document | Contents |
|----------|----------|
| [Agent Usage Manual](doc/agent_usage.md) | CLI commands, keybindings, configuration, workflows |
| [TUI Design](doc/tui_design.md) | Terminal interface architecture, rendering, streaming |
| [Agent Runtime](doc/agent_running_design.md) | Agent lifecycle, context management, LLM integration |
| [Authority & Recording](doc/authority_and_recording_design.md) | Permission system, mutation recording, crash recovery |

---

## ⚠️ Current Status — Unimplemented Features

> **Warning:** Arcana is in early development. The following features are designed but not yet implemented.

### Working Now
- [x] `arcana onboard` — interactive & non-interactive setup wizard
- [x] `arcana -q "..."` — single-shot LLM query (DeepSeek API, with thinking mode)
- [x] `arcana version` / `arcana check` / `arcana config show|path|edit`
- [x] `arcana --reset` — factory reset
- [x] Interactive TUI shell (viewport, composer, status bar, keybindings)
- [x] Collapsible task panel (Ctrl+T) with tree-style indicators
- [x] Interactive LLM streaming — TUI session sends messages to DeepSeek with SSE streaming
- [x] Thinking chain panel — collapsed by default, Ctrl+O to expand/collapse
- [x] `arcana auth status|allow|deny|revoke|reset` — command authorization management
- [x] `~/.arcana/authority.toml` — hot-reloadable authority config (created on onboard)

### Not Yet Implemented
- [ ] **Session management** — `arcana session list|resume|rename|delete|export|import`
- [ ] **Session resume** — `arcana resume --last` / `arcana resume <id>`
- [ ] **Sub-agent orchestration** — spawning, checkpointing, freezing, resuming sub-agents
- [ ] **Skills daemon** — trigger-based skill loading, hot-reload, manifest parsing
- [ ] **Memory system** — knowledge DB, semantic search, error patterns, session recall
- [ ] **Embedding model download** — `arcana onboard` does not yet download `all-MiniLM-L6-v2.onnx`
- [ ] **Authority & recording** — permission gate, git-like mutation recording, crash recovery
- [ ] **`arcana recover`** — restore project state from `git_record`
- [ ] **Query agent overlay** — `?` overlay sends queries to a persistent sub-agent
- [ ] **Tool calls** — shell execution, file read/write, search, web fetch
- [ ] **Diff review panel** — interactive accept/reject of file mutations
- [ ] **OpenAI / Anthropic provider support** — only DeepSeek is wired up
- [ ] **Context caching** — leveraging DeepSeek's prefix caching for long contexts
- [ ] **Desktop notifications** — bell/notification on response complete

---

## License

Apache-2.0
