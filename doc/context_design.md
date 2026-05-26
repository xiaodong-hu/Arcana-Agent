# Harness Design — Agent to LLM Interactive

## 1. Context Management

### 1.1 Interface Exposed to LLMs

To reduce the waste of context for LLM to "guess" what is on hand, what is doable, and to avoid many trials blocked by the rust-managed authority program, the agent always starts dialogue by telling what kind of registered and authorized tools/network commands etc are doable for current dialogue. LLM should also be able to send a query of list of what is loaded/registered/authorized.

The generated authority prompt includes:
- the API-only `~/.arcana/INSTRUCTION.md`
- the system-wide authority policy from `~/.arcana/authority.toml`
- the project-level authority policy from `.arcana/authority.toml`, when present
- the merged command, network, and filesystem authority snapshot

### 1.2 Long-term Memory Exposed to LLMs

All long-term system memory is appended to the context at the head of session (only once):
- `SOUL.md` — agent personality and behavior constraints
- `USER.md` — user portrait (auto-populated from interactions)
- Project-level `.arcana/PROJECT.md` — project-specific context

An explicit API interface to query and write the long-term knowledge/error vector database is also exposed to LLMs.

### 1.3 Context Window Management

To reduce token waste:
- Project memory (markdown files) and interface to query/write session vector memory are explicitly exposed
- Thinking chains are maintained as a `thinking_chain` vector memory database per project
- LLM can decide which thinking chains to write and query (requires change to memory architecture)
- Conversation history includes `reasoning_content` for DeepSeek prefix cache hits

### 1.4 Thinking Chain Memory

The thinking chain is critical for DeepSeek's context caching:
- All `reasoning_content` from responses is stored in conversation history
- This enables prefix cache hits on subsequent requests (same thinking prefix = cached)
- Thinking chains are also persisted to a per-project vector DB for cross-session recall
- LLM can query previous thinking chains for similar problems

---

## 2. Mode Design (toggled with `\mode`)

### Ask Mode
- Authority program enforces: **NO mutations** can be made by LLMs
- Can only read projects, extra files, and use web tools
- Project and session memory are loaded
- Useful for code review, explanation, and research

### Agent Mode (default)
- Full agent capabilities within authority constraints
- Can read/write files, execute authorized commands, spawn sub-agents
- All mutations are recorded in `git_record`

### Plan Mode
- LLM produces a structured plan (task list) without executing
- Plan can be reviewed, edited, then executed in Agent mode
- Useful for complex multi-step tasks where user wants oversight

---

## 3. Authority Constraints

### 3.1 Command Authorization (`~/.arcana/authority.toml` and `.arcana/authority.toml`)

```toml
[commands]
safe = ["git status", "git diff", "ls", "cat", "rg", ...]
allow = ["cargo build", "cargo test", ...]
confirm = ["git push", "git commit", "rm -rf", "sudo *"]

[network]
allow = ["api.deepseek.com", "api.openai.com"]
deny = ["*"]

[filesystem]
writable = ["."]
readonly = ["/etc", "/usr"]
deny = ["~/.ssh", "~/.gnupg"]
```

### 3.2 Sub-agent Constraints

- Sub-agents inherit the parent's authority scope (cannot escalate)
- Query agent (overlay) is **forbidden from spawning further sub-agents**
- Sub-agents have scoped filesystem access (only their assigned directory)
- Sub-agent spawning from within a sub-agent is forbidden by the authority program

### 3.3 Hot-reload

Authority config is hot-reloadable:
- Edit `~/.arcana/authority.toml` or project `.arcana/authority.toml` and changes take effect immediately
- Approved registration requests are appended to project `.arcana/authority.toml`
- Skills are similarly hot-loadable from `~/.arcana/skills/`

---

## 4. Agent Hierarchy & Context Isolation

```
Main Agent (deepseek-v4-pro)
├── Full context: SOUL.md + USER.md + PROJECT.md + conversation + memory
├── Can spawn sub-agents
├── Can use query agent overlay
│
├── Query Agent (persistent, overlay via ?)
│   ├── Shares main agent's context window (read-only)
│   ├── Has own conversation history
│   ├── Thinking chain with Ctrl+O expand/collapse
│   ├── CANNOT spawn sub-agents (authority constraint)
│   └── Uses agents.query config (model/thinking)
│
└── Sub-Agents (spawned, scoped)
    ├── Scoped filesystem access
    ├── Own conversation context
    ├── Checkpointable, freezable, resumable
    ├── CANNOT spawn further sub-agents (authority constraint)
    └── Use agents.sub config (model/thinking)
```

---

## 5. Session & Thinking Chain Persistence

### Session Memory
- Full conversation history (user + assistant + reasoning_content)
- Stored per-session with session ID
- Resumable via `arcana resume --last` or `arcana resume <id>`

### Thinking Chain Database
- Per-project vector DB of thinking chains
- Indexed by: problem description, file paths involved, outcome
- LLM can query: "have I solved something similar before?"
- Enables cross-session learning without full context replay

### Error Pattern Memory
- Failed approaches are recorded with context
- LLM is informed of previous failures for similar problems
- Prevents repeating the same mistake across sessions
