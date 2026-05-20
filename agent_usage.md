# Agent Usage

This document covers the end-user experience of Arcana — onboarding, daily usage, CLI commands, and session workflows. It is the user-facing counterpart to the internal design documents.

---

## 1. Installation & Onboarding

### 1.1 First-Time Setup: `arcana onboard`

On first run, the user executes:

```bash
arcana onboard
```

This launches an interactive onboarding wizard in the terminal:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                 │
│   Welcome to Arcana — The Arcane Agent                          │
│                                                                 │
│   Let's set up your environment.                                │
│                                                                 │
│   Step 1/4: Model Provider                                      │
│                                                                 │
│   Which provider do you want to use?                            │
│                                                                 │
│   > [x] DeepSeek (deepseek-v4-pro, deepseek-v4-flash)          │
│     [ ] OpenAI (gpt-4o, o3)                                     │
│     [ ] Anthropic (claude-sonnet-4, opus)                       │
│     [ ] OpenRouter (any model)                                  │
│     [ ] Local (Ollama, vLLM, SGLang)                            │
│     [ ] Custom OpenAI-compatible endpoint                       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

#### Onboarding Steps

| Step | Action |
|------|--------|
| 1. Provider | Select model provider(s). Multiple allowed. |
| 2. API Key | Enter API key interactively, OR detect from environment variables (see §1.2). |
| 3. Model Selection | Choose default model for the selected provider. |
| 4. Global Directory | Create `~/.arcana/` with initial files (SOUL.md, USER.md, configs). |

#### What Gets Created

```
~/.arcana/
├── SOUL.md                     # Default agent personality (editable)
├── USER.md                     # Empty user portrait (populated over time)
├── config.toml                 # Global configuration
├── knowledge.db                # Empty global knowledge store
├── errors.db                   # Empty error store
├── models/
│   └── all-MiniLM-L6-v2.onnx  # Embedding model (downloaded during onboard)
├── skills/
│   ├── system/                 # Built-in skills
│   └── user/                   # User-installed skills
└── tools.toml                  # Runtime-registered tools (empty)
```

**Non-interactive mode** (for scripting/CI):

```bash
arcana onboard --provider deepseek --model deepseek-v4-pro --non-interactive
```

This skips the wizard and uses environment variables for API keys.

### 1.2 API Key Detection

Arcana checks for API keys in this order (first found wins):

1. **Config file**: `~/.arcana/config.toml` → `[providers.deepseek] api_key = "..."`.
2. **Environment variable**: Standard names per provider.
3. **Interactive prompt**: Asked during `arcana onboard` or on first use if missing.

| Provider | Environment Variable |
|----------|---------------------|
| DeepSeek | `DEEPSEEK_API_KEY` |
| OpenAI | `OPENAI_API_KEY` |
| Anthropic | `ANTHROPIC_API_KEY` |
| OpenRouter | `OPENROUTER_API_KEY` |

Environment variables are typically set in `~/.zshrc`, `~/.bashrc`, or `~/.zshenv`:

```bash
# ~/.zshrc
export DEEPSEEK_API_KEY="sk-..."
```

Arcana reads these at process start. If the key is found in the environment, it is NOT written to `config.toml` (the env var remains the source of truth). This allows users to manage secrets via their shell profile or a secrets manager.

**Key rotation**: `arcana auth set --provider deepseek` prompts for a new key and saves to `config.toml`. `arcana auth clear --provider deepseek` removes the saved key (falls back to env var).

### 1.3 Workspace Initialization

When Arcana is first run inside a project directory (any directory without an existing `.arcana/` folder):

```bash
cd ~/projects/my-app
arcana
```

Arcana detects no `.arcana/` workspace and offers to create one:

```
No Arcana workspace found in ~/projects/my-app.
Create .arcana/ workspace? [Y/n]
```

If accepted, creates:

```
my-app/.arcana/
├── access.toml                 # Authority rules (default: prompt for writes)
├── authority.sock              # Unix socket (created at runtime)
├── memory/
│   ├── project.md              # Project knowledge (auto-populated on first scan)
│   └── sessions/               # Session databases
├── skills/                     # Project-specific skills
├── sessions/
│   └── index.json              # Session index
├── checkpoints/                # Agent checkpoints
├── git_record/                 # Mutation recording (see authority_and_recording_design.md)
│   ├── objects/
│   ├── actions.jsonl
│   ├── snapshots/
│   └── HEAD
└── web_cache/                  # Fetched web pages
    ├── index.jsonl
    └── pages/
```

**Auto-scan**: On first workspace creation, Arcana scans the project tree for documentation files (`.md`, `.rst`, `.txt`, `Cargo.toml`, `package.json`, etc.) and generates an initial `project.md` summary. This gives the agent immediate project context.

### 1.4 Migration from Other Agents

Arcana detects existing agent configurations and offers to import relevant settings:

| Source | Detection | What's Imported |
|--------|-----------|-----------------|
| Claude Code | `.claude/` directory, `CLAUDE.md` | System prompt → SOUL.md inspiration, settings → config.toml |
| Cursor | `.cursor/` directory | Rules → skills, settings → config.toml |
| Hermes | `~/.hermes/` directory | Skills → `~/.arcana/skills/user/`, config → config.toml |
| DeepSeek TUI | `~/.deepseek/` directory | Config → config.toml, API keys |
| Aider | `.aider.conf.yml` | Model settings, conventions |

Migration is opt-in and non-destructive (never modifies the source agent's files):

```
Detected existing agent configurations:
  • Claude Code (.claude/) — system prompt, settings
  • Hermes (~/.hermes/) — 5 skills, model config

Import settings? [Y/n/select]
```

---

## 2. Daily Usage

### 2.1 Starting a Session

```bash
# Start interactive session in current directory
arcana

# Start with a specific model
arcana --model deepseek-v4-flash

# Start with a specific provider
arcana --provider openai --model gpt-4o

# Single-shot query (non-interactive, prints response and exits)
arcana -q "explain the parse function in src/parser.rs"

# Resume last session
arcana resume --last

# Resume specific session
arcana resume <session-id>
arcana resume "implement parser"    # by name
```

### 2.2 Session Startup Sequence

When `arcana` is invoked:

```
1. Start authority_and_record daemon (owns all writes, records mutations).
2. Start skill daemon (trigger evaluator).
3. Start sub-agent orchestrator daemon.
4. Load global memory (SOUL.md, USER.md).
5. Load project memory (project.md, relevant knowledge).
6. Spawn query sub-agent (always-on, shares main context).
7. Display welcome banner + status bar.
8. Ready for input.
```

All daemons communicate via unix sockets in `.arcana/`. The agent process itself is sandboxed (no write access — see `authority_and_recording_design.md`).

### 2.3 The Two-Agent Model

Arcana always runs with **two agents** from the start:

| Agent | Role | Lifecycle |
|-------|------|-----------|
| **Main Agent** | Primary work agent. Plans, executes, spawns sub-agents, modifies files. | Starts on session begin, ends on session end. |
| **Query Agent** | Fast Q&A. Answers quick questions without disrupting main agent's flow. | Spawned at session start, never killed (only hidden/shown). |

The query agent is NOT a separate LLM session — it shares the main agent's context window exactly. Each query constructs a one-shot prompt from the current context + the user's question. This means:
- **Zero extra token storage cost** (no separate conversation to maintain).
- **Always up-to-date** (sees whatever the main agent sees right now).
- **Stateless between queries** (each question is independent).

### 2.4 Interaction Flow

```
User types message
    │
    ├─► Main agent receives prompt
    │       │
    │       ├─► Skill daemon evaluates triggers
    │       │       └─► Triggered skills inject context / run hooks
    │       │
    │       ├─► Agent generates response (streamed to TUI)
    │       │       ├─► Thinking block (collapsible, dimmed)
    │       │       └─► Final response (normal text)
    │       │
    │       ├─► If file write proposed → diff review panel
    │       │       └─► User: [A]ccept / [S]ession-accept / [E]dit / [X]Abort
    │       │
    │       ├─► If tool execution needed → authority check
    │       │       └─► Allowed / Denied / Prompt user
    │       │
    │       └─► Response complete. Ready for next input.
    │
    └─► User presses `?` (composer empty)
            │
            └─► Query overlay opens
                    │
                    ├─► User types question
                    ├─► Query agent responds (streamed in overlay)
                    └─► User presses `q` or `Esc` → back to main viewport
```

---

## 3. Query Sub-Agent (`?` Overlay)

### 3.1 Activation

Press `?` when the input composer is empty. This opens a floating overlay panel covering most of the viewport.

**Cannot be activated when:**
- The composer has text (inserts literal `?` instead).
- The overlay is already open (no nesting — single layer only).

### 3.2 Usage

Inside the overlay, type questions and get immediate responses:

```
┌─ Query Agent ─────────────────────────────────────────────────┐
│                                                               │
│  ❯ what does the TokenKind enum look like?                    │
│                                                               │
│  Based on the current context, `TokenKind` is defined in      │
│  src/lexer.rs:                                                │
│                                                               │
│  ```rust                                                      │
│  pub enum TokenKind {                                         │
│      Ident(String),                                           │
│      Number(i64),                                             │
│      Plus, Minus, Star, Slash,                                │
│      LParen, RParen,                                          │
│      Eof,                                                     │
│  }                                                            │
│  ```                                                          │
│                                                               │
├───────────────────────────────────────────────────────────────┤
│  ❯ |                                          [q to go back]  │
└───────────────────────────────────────────────────────────────┘
```

### 3.3 Dismissal

| Key | Action |
|-----|--------|
| `q` (composer empty) | Close overlay, return to main viewport |
| `Esc` | Close overlay (always, even if composer has text — text is discarded) |

The query agent remains alive after dismissal. Its overlay conversation is kept in memory for the session (scrollable if re-opened) but is NOT persisted to disk or memory stores.

### 3.4 Interaction with Main Agent

- The main agent **continues running** while the overlay is open. If it produces output (e.g., a sub-agent completes), a notification badge appears on the overlay border.
- The query agent's responses are **invisible to the main agent**. They do not pollute the main conversation history.
- If the main agent is waiting for user input (e.g., diff review), the overlay can still be opened. The diff review prompt remains pending until the user dismisses the overlay and responds.

---

## 4. CLI Commands

### 4.1 Top-Level Commands

```bash
arcana                          # Start interactive session
arcana onboard                  # First-time setup wizard
arcana -q "prompt"              # Single-shot query
arcana --model <model>          # Override model for this session
arcana --provider <provider>    # Override provider for this session
arcana --reset                  # Remove ~/.arcana and recreate (factory reset)
arcana resume [--last | <id>]   # Resume a session
arcana recover <project> [--to-seq N]  # Recover project state (see authority design)
arcana check                    # Check setup & connectivity
arcana version                  # Print version
```

### 4.2 Session Commands

```bash
arcana session list             # List all sessions (name, status, date)
arcana session resume <id>      # Resume a frozen/crashed session
arcana session rename <id> "n"  # Rename a session
arcana session delete <id>      # Delete session and checkpoints
arcana session export <id>      # Export as JSON
arcana session import <file>    # Import from JSON
```

### 4.3 Memory Commands

```bash
arcana memory knowledge list    # List knowledge entries by activation score
arcana memory knowledge search "query"  # Semantic search
arcana memory knowledge edit <id>       # Edit in $EDITOR
arcana memory knowledge delete <id>     # Remove entry
arcana memory knowledge export          # Dump to JSON
arcana memory knowledge import <file>   # Load from JSON

arcana memory errors list       # Same interface for error store
arcana memory errors search "query"
# ... (same subcommands)

arcana memory session list      # List session memory entries
arcana memory flush             # Force immediate write-back
```

### 4.4 Skill Commands

```bash
arcana skill list               # List all installed skills + status
arcana skill install <path|url> # Install to user pool
arcana skill enable <name>      # Enable a skill
arcana skill disable <name>     # Disable a skill
arcana skill remove <name>      # Remove from user pool
arcana skill triggers           # Show all registered triggers
arcana skill test "prompt"      # Test which skills would trigger
```

### 4.5 Auth Commands

```bash
arcana auth set --provider deepseek     # Save API key interactively
arcana auth clear --provider deepseek   # Remove saved key
arcana auth status                      # Show credential sources (without printing keys)
```

### 4.6 Configuration

```bash
arcana config                   # Show current configuration (same as `arcana config show`)
arcana config show              # Print effective config as TOML
arcana config edit              # Open ~/.arcana/config.toml in $EDITOR
arcana config path              # Print config file path
arcana --reset                  # Remove ~/.arcana entirely and recreate (factory reset)
```

Note: `~/.arcana/` is automatically created on every launch if it doesn't exist.

---

## 5. In-Session Slash Commands

These are available inside the interactive TUI session (typed in the composer):

| Command | Description |
|---------|-------------|
| `/help` | Show all available commands |
| `/model [name]` | Show or change active model |
| `/provider [name]` | Show or change provider |
| `/skills` | List active skills |
| `/skill <name>` | Activate/deactivate a skill |
| `/agents` | Show sub-agent tree (status, turns, scope) |
| `/tasks` | Show task progress |
| `/freeze` | Freeze all agents, save state |
| `/resume` | Resume from last freeze |
| `/memory` | Show memory stats |
| `/usage` | Token usage breakdown (input/output, cache hits, cost) |
| `/compress` | Force context compression |
| `/status` | Session info (model, tokens, duration, files touched) |
| `/title <name>` | Name the current session |
| `/theme [name]` | Show or change color theme |
| `/verbose` | Cycle detail level: off → collapsed → expanded |
| `/clear` | Clear viewport (history preserved, just visual reset) |
| `/export` | Export current session to file |

---

## 6. Keybindings Reference

### 6.1 Global (Always Active)

| Key | Action |
|-----|--------|
| `Ctrl+C` | Interrupt agent / clear input |
| `Ctrl+D` | End session (graceful) |
| `Ctrl+T` | Toggle tasks panel (expand/fold) |
| `Ctrl+S` | Toggle skills panel (expand/fold) |
| `Ctrl+A` | Toggle agents panel (expand/fold) |
| `Ctrl+Shift+P` | Freeze & backup all agents |
| `Ctrl+Shift+M` | Modify last prompt |
| `?` | Open query overlay (when composer empty) |

### 6.2 Composer

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Alt+Enter` / `Ctrl+J` | Insert newline |
| `Tab` | Autocomplete slash commands |
| `↑` (empty composer) | Recall previous message |
| `Ctrl+G` | Open input in `$EDITOR` |

### 6.3 Viewport

| Key | Action |
|-----|--------|
| `j`/`k` or `↑`/`↓` | Scroll line-by-line |
| `PgUp`/`PgDn` | Scroll by page |
| `g`/`G` or `Home`/`End` | Jump to top/bottom |
| `Ctrl+U`/`Ctrl+D` | Half-page scroll |
| `Enter` (on collapsed block) | Expand thinking/tool block |

### 6.4 Diff Review

| Key | Action |
|-----|--------|
| `A` | Accept write |
| `S` | Session-accept (auto-approve this file for session) |
| `E` | Edit in `$EDITOR` |
| `X` | Abort (reject write) |
| `O` / `Ctrl+O` | Expand full diff |

### 6.5 Query Overlay

| Key | Action |
|-----|--------|
| `q` (composer empty) | Dismiss overlay |
| `Esc` | Dismiss overlay (always) |
| `Enter` | Send query |
| `Alt+Enter` / `Ctrl+J` | Insert newline |

---

## 7. Configuration Reference (`~/.arcana/config.toml`)

```toml
# ─── Agent LLM Configuration ───────────────────────────────────────────────
# Split into three agent roles: main, query (persistent), and sub (spawned).
# This enables hybrid LLM structures — e.g., powerful model for main agent,
# fast/cheap model for sub-agents.

[agents.main]
provider = "deepseek"
model = "deepseek-v4-pro"
max_tokens = 8192                   # Optional: max output tokens per response
temperature = 0.0                   # Optional: sampling temperature

[agents.main.thinking]
enabled = true                      # Maps to {"thinking": {"type": "enabled"}}
reasoning_effort = "high"           # "high" or "max" (OpenAI: reasoning_effort, Anthropic: output_config.effort)

[agents.query]
provider = "deepseek"
model = "deepseek-v4-pro"

[agents.query.thinking]
enabled = true
reasoning_effort = "high"

[agents.sub]
provider = "deepseek"
model = "deepseek-v4-flash"         # Cheaper/faster model for spawned sub-agents

[agents.sub.thinking]
enabled = true
reasoning_effort = "high"

# ─── Provider Credentials ──────────────────────────────────────────────────

[providers.deepseek]
api_key = ""                        # Leave empty to use DEEPSEEK_API_KEY env var
base_url = "https://api.deepseek.com/beta"
models = ["deepseek-v4-pro", "deepseek-v4-flash"]

[providers.openai]
api_key = ""
base_url = "https://api.openai.com/v1"
models = ["gpt-4o", "o3"]

[providers.anthropic]
api_key = ""
base_url = "https://api.anthropic.com"
models = ["claude-sonnet-4-20250514"]

[providers.local]
base_url = "http://localhost:11434/v1"  # Ollama default
models = []                             # Auto-detected

# ─── Display ───────────────────────────────────────────────────────────────

[display]
theme = "arcane"                    # arcane | light | dracula | gruvbox
animations = true                   # false for accessibility
bell_on_complete = true             # Terminal bell when sub-agent completes
thinking_default = "collapsed"      # collapsed | expanded | hidden
tool_detail = "collapsed"           # collapsed | expanded | hidden

# ─── Editor ───────────────────────────────────────────────────────────────

[editor]
command = "nvim"                    # For diff review [E]dit and Ctrl+G
diff_command = "delta"              # External diff renderer (optional)

# ─── Notifications ─────────────────────────────────────────────────────────

[notifications]
desktop = true                      # OSC 9/99 desktop notifications
bell = true                         # Terminal bell fallback
toast_duration_secs = 5             # In-TUI toast auto-dismiss

# ─── Session ──────────────────────────────────────────────────────────────

[session]
max_sessions_kept = 50              # Auto-prune oldest beyond this
auto_freeze_on_disconnect = true    # Freeze on SIGHUP
crash_recovery_prompt = true        # Ask before recovering crashed session
```

### 7.1 DeepSeek Thinking Mode Options

The `[agents.*.thinking]` section maps directly to DeepSeek's API parameters:

| Config Field | API Parameter (OpenAI format) | API Parameter (Anthropic format) |
|---|---|---|
| `enabled = true/false` | `{"thinking": {"type": "enabled/disabled"}}` | Same |
| `reasoning_effort = "high"` | `{"reasoning_effort": "high"}` | `{"output_config": {"effort": "high"}}` |
| `reasoning_effort = "max"` | `{"reasoning_effort": "max"}` | `{"output_config": {"effort": "max"}}` |

Notes:
- Default effort is `high` for regular requests; `max` is for complex agent tasks.
- For compatibility, `low` and `medium` are mapped to `high`, and `xhigh` is mapped to `max`.
- When thinking is enabled, `temperature`, `top_p`, `presence_penalty`, and `frequency_penalty` have no effect.

### 7.2 Hybrid LLM Configuration Examples

**Example: DeepSeek Pro for main, Flash for sub-agents**
```toml
[agents.main]
provider = "deepseek"
model = "deepseek-v4-pro"

[agents.sub]
provider = "deepseek"
model = "deepseek-v4-flash"
```

**Example: Anthropic for main, DeepSeek for sub-agents**
```toml
[agents.main]
provider = "anthropic"
model = "claude-sonnet-4-20250514"

[agents.main.thinking]
enabled = false                     # Anthropic uses extended thinking differently

[agents.sub]
provider = "deepseek"
model = "deepseek-v4-flash"

[agents.sub.thinking]
enabled = true
reasoning_effort = "high"
```

---

## 8. Model Support

### 8.1 DeepSeek V4 (Primary Target)

Arcana is optimized for DeepSeek V4 models:

| Model | Context | Best For |
|-------|---------|----------|
| `deepseek-v4-pro` | 1M tokens | Complex reasoning, architecture, multi-file changes |
| `deepseek-v4-flash` | 1M tokens | Fast queries, simple edits, high throughput |

**Thinking mode**: DeepSeek V4 produces `<think>...</think>` blocks containing chain-of-thought reasoning. Arcana:
- Streams thinking tokens into a collapsible panel (see TUI design §4.2).
- Tracks thinking token cost separately in `/usage`.
- Supports reasoning effort tiers: `off` → `high` → `max` (cycled with `Shift+Tab` or `/reasoning <level>`).

**Auto mode** (`--model auto`): A lightweight routing call (using `deepseek-v4-flash` with thinking off) decides which model and thinking level to use for each turn. Simple questions stay on Flash; complex tasks escalate to Pro with high/max thinking.

### 8.2 Other Providers

Arcana works with any OpenAI-compatible API. Provider-specific features:

| Provider | Special Handling |
|----------|-----------------|
| OpenAI | Tool-use format, streaming |
| Anthropic | Extended thinking blocks, tool-use format |
| OpenRouter | Model routing, fallback chains |
| Local (Ollama/vLLM) | No cost tracking, local inference |

### 8.3 Cost Tracking

After every LLM response, a stats line is appended to the conversation showing per-response usage:

```
Expense: 0.0031 ( 1.2K in / 847 out )
Time: 2.4s
```

This line appears inline after each agent response (rendered as a dim system message). It shows:
- **Expense**: Cost in USD for that single response.
- **Tokens**: Input tokens consumed / output tokens generated.
- **Time**: Wall-clock time for the LLM to generate the response.

The `/usage` command shows cumulative session totals:

```
/usage
  Input tokens:   8,234  (cached: 6,100 — 74% hit rate)
  Output tokens:  1,456
  Thinking tokens: 3,200
```

The status bar shows context window utilization (not cost or time):
```
 ⚗ deepseek-v4-pro │ [████░░░░░░] 8.2K/1M
```

---

## 9. Workflow Examples

### 9.1 Typical Development Session

```bash
$ cd ~/projects/my-parser
$ arcana

# Banner displays, status bar shows model + skills
# Main agent ready, query agent spawned

❯ I want to add operator precedence to the parser. Currently it's
  a flat recursive descent. Let's use a Pratt parser approach.

# Agent thinks (collapsible panel streams reasoning)
# Agent proposes changes to src/parser.rs
# Diff review panel appears
# User presses [A] to accept

# Agent continues, proposes test changes
# User presses [A]

# Mid-task, user has a quick question:
# User presses `?` (composer is empty)

┌─ Query Agent ─────────────────────────────────────────────────┐
│  ❯ what's the binding power for multiplication?               │
│                                                               │
│  In the current implementation, multiplication has binding    │
│  power (5, 6) — left-associative with precedence 5.          │
└───────────────────────────────────────────────────────────────┘

# User presses `q` to go back
# Main agent's work continues uninterrupted
```

### 9.2 Resuming After Interruption

```bash
# Session was frozen (Ctrl+Shift+P or terminal disconnect)

$ arcana resume --last

# Or by name:
$ arcana resume "pratt parser"

# Full state restored: main agent + sub-agents + task progress
# Conversation continues from where it left off
```

### 9.3 Multi-Agent Task Delegation

```bash
❯ Implement the full compiler pipeline: lexer, parser, codegen, and tests.
  Delegate to sub-agents where possible.

# Main agent plans tasks, spawns sub-agents:
#   parser-impl → src/parser/**
#   codegen-impl → src/codegen/**
#   test-writer → tests/**

# Status bar updates: Agents: 3/0 │ Tasks: 0/7

# Sub-agents work in parallel. User can:
#   - Watch progress in /agents panel
#   - Use `?` overlay for questions
#   - Interrupt with Ctrl+C if needed
#   - Freeze everything with Ctrl+Shift+P

# As sub-agents complete, notifications appear:
#   ✓ parser-impl completed (3 files modified)
#   ✓ test-writer completed (2 files modified)

# Main agent collects results, summarizes, continues
```

---

## 10. Troubleshooting

### 10.1 `arcana check`

Checks system health:

```bash
$ arcana check

  ✓ Global config (~/.arcana/config.toml)
  ✓ Embedding model (all-MiniLM-L6-v2.onnx, 80MB)
  ✓ DeepSeek API key (from DEEPSEEK_API_KEY env var)
  ✓ API connectivity (deepseek-v4-pro: 200 OK, 142ms)
  ✓ Workspace (.arcana/ exists)
  ✓ Authority binary (authority_and_record v0.1.0)
  ✓ Unix socket permissions (group-writable)
  ✗ Agent user (arcana-agent not found — sandbox disabled)
    → Run: sudo useradd -r -s /usr/sbin/nologin arcana-agent
```

### 10.2 Common Issues

| Issue | Solution |
|-------|----------|
| "API key not found" | Set `DEEPSEEK_API_KEY` in `~/.zshrc` or run `arcana auth set --provider deepseek` |
| "Workspace not initialized" | Run `arcana` in the project directory and accept workspace creation |
| "Authority socket not found" | The authority daemon failed to start. Check `~/.arcana/logs/authority.log` |
| "Embedding model missing" | Run `arcana onboard` again or manually download to `~/.arcana/models/` |
| "Sandbox disabled" | Create the `arcana-agent` system user (see `authority_and_recording_design.md` §OS-Level Confinement) |
| "Session corrupted" | Run `arcana session delete <id>` and start fresh |

---

## 11. Environment Variables

| Variable | Purpose |
|----------|---------|
| `DEEPSEEK_API_KEY` | DeepSeek API key |
| `OPENAI_API_KEY` | OpenAI API key |
| `ANTHROPIC_API_KEY` | Anthropic API key |
| `OPENROUTER_API_KEY` | OpenRouter API key |
| `ARCANA_MODEL` | Override default model |
| `ARCANA_PROVIDER` | Override default provider |
| `ARCANA_HOME` | Override global config directory (default: `~/.arcana`) |
| `ARCANA_LOG_LEVEL` | Logging verbosity: `error`, `warn`, `info`, `debug`, `trace` |
| `NO_COLOR` | Disable all colors (accessibility) |
| `NO_ANIMATIONS` | Disable animations (accessibility) |
| `EDITOR` | Editor for diff review and `Ctrl+G` (fallback if `config.toml` editor not set) |
