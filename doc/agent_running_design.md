# Agent Running Design

This document covers the runtime behavior of the Arcana agent process — how it remembers, learns, manages skills, interacts with humans mid-generation, and coordinates sub-agents.

---

## 1. Memory System

### 1.1 Overview

The memory system is a multi-tier architecture spanning three scopes: **global** (cross-project, persistent forever), **project** (per-project, persistent across sessions), and **session** (per-session, ephemeral until promoted).

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Agent Context Window                         │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │ SOUL.md  │  │ USER.md  │  │ Top-K    │  │ Session rolling   │  │
│  │ (always) │  │ (always) │  │ memories │  │ context           │  │
│  └──────────┘  └──────────┘  └──────────┘  └───────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
        ▲               ▲              ▲
        │               │              │
┌───────┴───────┐ ┌─────┴─────┐ ┌─────┴──────────────────────────┐
│ ~/.arcana/    │ │ ~/.arcana/ │ │ .arcana/memory/ (project)      │
│ SOUL.md       │ │ USER.md    │ │ + ~/.arcana/knowledge.db       │
│               │ │            │ │ + ~/.arcana/errors.db           │
└───────────────┘ └────────────┘ │ + .arcana/memory/session.db    │
                                 │ + .arcana/memory/project.md     │
                                 └─────────────────────────────────┘
```

### 1.2 Tier A: Global Long-Term Memory (`~/.arcana/`)

Located in the user's home directory. Persists forever across all projects and sessions. The agent must request write permission on first startup.

#### 1.2.1 `SOUL.md` — Agent Personality (< 100 lines)

Defines the agent's tone, response style, knowledge level, and behavioral preferences.

- **Written by**: AI self-summarization + user edits.
- **Loaded**: Always, in full, at the start of every context window.
- **Update trigger**: When the error/complaint store accumulates repeated patterns (≥ 3 similar complaints), the agent proposes a `SOUL.md` amendment to the user.
- **Format**: Plain markdown, human-readable and editable.

```markdown
# SOUL.md — Arcana Agent Personality

## Tone
- Direct, concise, no filler
- Match user's technical level (currently: advanced systems programmer)

## Preferences
- Prefer Rust over C/C++ unless user specifies
- Always show reasoning before conclusions
- Use code examples over prose explanations

## Constraints
- Never apologize unnecessarily
- Do not repeat information already stated
```

#### 1.2.2 `USER.md` — User Portrait (< 100 lines)

A profile of the user built from interaction history.

- **Written by**: AI summarization from session interactions.
- **Loaded**: Always, in full.
- **Update trigger**: End of each session, the agent reviews if new information about the user was revealed and proposes updates.
- **Format**: Plain markdown.

```markdown
# USER.md — User Portrait

## Background
- Systems programmer, strong in Rust, Haskell, category theory
- Works on formal verification and AI agent systems

## Preferences
- Prefers mathematical rigor in explanations
- Likes designs documented before implementation
- Uses Neovim as primary editor

## Communication Style
- Concise, expects direct answers
- Appreciates when AI challenges assumptions
```

#### 1.2.3 `knowledge.db` — Global Knowledge Store (Vectorized)

A vector database storing compressed knowledge summaries that the agent has learned across all sessions and projects.

- **Content**: Factual knowledge, techniques, patterns, solutions discovered during interactions.
- **Written by**: AI summarization at session end or on explicit user command.
- **Queried**: At session start (using initial user message as query) and on each turn (using current context as query).
- **Eviction**: Based on activation ranking (see §1.5).

#### 1.2.4 `errors.db` — Error & Complaint Store (Vectorized)

A vector database storing records of AI mistakes, user complaints, and unsatisfactory responses.

- **Content**: What went wrong, what the user expected, the correction applied.
- **Written by**: AI self-reflection when user expresses dissatisfaction, or when the agent detects its own error.
- **Queried**: On every turn — before generating a response, the agent checks if the current task resembles a past mistake.
- **Eviction**: Based on activation ranking, but with a higher threshold (errors are retained longer).
- **Feedback loop**: Repeated error patterns (≥ 3 similar entries) trigger a `SOUL.md` update proposal.

### 1.3 Tier B: Session-Level Memory (`.arcana/memory/session.db`)

Located in the project folder. One database per session (named by session ID or timestamp). Contains vectorized summaries of the current session's interactions.

- **Content**: Compressed summaries of prompt-response pairs, decisions made, errors encountered.
- **Written by**: AI, buffered in-memory and flushed every N turns (default: 5) + at session end.
- **Queried**: Automatically on each turn to maintain coherence within the session.
- **Eviction**: Based on activation ranking within the session.
- **Promotion**: Entries queried ≥ 2 times across different sessions are candidates for promotion to project-level memory.
- **Human access**: Users can inspect and edit via SQLite tools or a CLI subcommand (`arcana memory list`, `arcana memory edit`).

### 1.4 Tier C: Project-Level Memory (`.arcana/memory/project.md`)

A markdown file (or set of files) summarizing durable project knowledge. Unlike the vector stores, this is fully human-readable and version-controllable.

- **Sources** (summarized from):
  1. Session-level entries that have been queried/activated at least once.
  2. Project documentation: markdown, Jupyter notebooks, LaTeX, Typst files found in the project tree.
  3. Source code structure and key architectural decisions.
- **Written by**: AI summarization, triggered at session end or on explicit command.
- **Loaded**: Relevant sections retrieved via keyword/semantic search at each turn.
- **Format**: Structured markdown with sections for architecture, conventions, key decisions, known issues.
- **Human access**: Directly editable as a markdown file. Changes are picked up on next session start.

### 1.5 Activation-Based Ranking & Eviction

Every entry in the vectorized stores (`knowledge.db`, `errors.db`, `session.db`) carries metadata:

```json
{
  "id": "uuid",
  "text": "compressed summary",
  "embedding": [0.1, -0.3, ...],
  "created_at": "2026-05-19T22:00:00Z",
  "last_accessed": "2026-05-19T22:30:00Z",
  "access_count": 7,
  "activation_score": 0.85,
  "source_session": "session-id",
  "tags": ["rust", "error-handling"]
}
```

**Activation score** is computed as:

```
activation_score = access_count * recency_weight(last_accessed)
recency_weight(t) = exp(-λ * (now - t))  // exponential decay, λ configurable
```

**Eviction policy**:
- When a store exceeds its capacity (configurable, default: 10,000 entries for knowledge, 5,000 for errors, 1,000 per session), entries with the lowest activation scores are candidates for removal.
- Before removal, the agent generates a "consolidation summary" — merging low-activation entries into fewer, more general entries. This prevents total information loss.
- Eviction runs at session end, not mid-session.

**Purpose**: This mechanism lets the AI "forget" stale, unused knowledge naturally, while frequently-accessed information persists indefinitely.

### 1.6 Memory Retrieval (Context Loading)

The agent has a finite context window. Memory loading follows a budget:

| Source | Budget | Loading Strategy |
|--------|--------|-----------------|
| `SOUL.md` | Always loaded in full | Static |
| `USER.md` | Always loaded in full | Static |
| `errors.db` | Top-3 relevant entries per turn | Query with current task context |
| `knowledge.db` | Top-5 relevant entries per turn | Query with current task + user message |
| `project.md` | Relevant sections (≤ 500 tokens) | Keyword + semantic search |
| `session.db` | Top-5 recent + top-3 relevant | Recency + semantic relevance |

**Bootstrap (first message of session)**:
1. Load `SOUL.md` + `USER.md` (always).
2. Use the user's first message as a query against all stores.
3. Load top-K results within budget.
4. If the project has `project.md`, load its summary section.

**Subsequent turns**:
1. Use the current turn's context (user message + recent conversation) as query.
2. Retrieve top-K from each store within budget.
3. Entries that are retrieved have their `access_count` incremented and `last_accessed` updated.

### 1.7 Memory Write-Back

**When does the AI write to memory stores?**

| Event | Action |
|-------|--------|
| Every 5 turns (configurable) | Flush buffered session entries to `session.db` |
| User expresses dissatisfaction | Write to `errors.db` immediately |
| AI detects own error | Write to `errors.db` immediately |
| Session end (graceful) | Flush all buffers, run eviction, update `USER.md` if needed, promote session entries to `project.md` |
| Session end (crash) | On next startup, recover from last flush point (buffered entries since last flush are lost) |
| Explicit user command | `arcana memory flush` forces immediate write-back |

### 1.8 Human Access & Correction

All memory stores are designed for human inspection and modification:

| Store | Access Method |
|-------|--------------|
| `SOUL.md` | Direct file edit |
| `USER.md` | Direct file edit |
| `project.md` | Direct file edit |
| `knowledge.db` | CLI: `arcana memory knowledge list/search/edit/delete` |
| `errors.db` | CLI: `arcana memory errors list/search/edit/delete` |
| `session.db` | CLI: `arcana memory session list/search/edit/delete` |

The CLI provides:
- `list` — show all entries sorted by activation score.
- `search <query>` — semantic search against the store.
- `edit <id>` — open entry in `$EDITOR` for modification.
- `delete <id>` — remove an entry.
- `export` — dump store to JSON for bulk editing.
- `import` — load entries from JSON.

Changes made by humans are picked up on the next query (no restart needed for vector stores; `SOUL.md`/`USER.md`/`project.md` are re-read at session start).

### 1.9 Vector Store Backend

**Chosen: SQLite (rusqlite + sqlite-vec + FTS5)**

Rationale:
- **>1M entries** — disk-backed via SQLite, not memory-limited. The `vec0` virtual table uses partition-based indexing that scales well.
- **Persistence + ACID** — single `.db` file, crash-safe, aligns with Arcana's atomic-write philosophy.
- **Human-editable** — standard SQL queries. Users can inspect/edit with `sqlite3` CLI, DB Browser, or any SQLite tool.
- **Hybrid search** — `vec0` for vector KNN + FTS5 for full-text BM25, in the same database file.
- **Minimal dependencies** — `rusqlite` (bundled SQLite) + `sqlite-vec` (single C file, statically linked at build time). No external processes.
- **Actively maintained** — sqlite-vec by Alex Garcia (Mozilla Builders project, 2024+), rusqlite is the standard Rust SQLite binding.
- **One backend for everything** — replaces the need for separate text search + vector search libraries.

**Alternatives considered:**

| System | Why not |
|--------|---------|
| Tantivy | No native vector search; would need brute-force cosine on top (ugly, limited to ~100K) |
| hora | Abandoned since 2021, no persistence |
| memvdb | Brute-force only, no persistence, 2 stars |
| Qdrant | Requires separate process |
| LanceDB | Python-first, Rust bindings less mature |

**Schema (per store):**

```sql
-- Metadata table (standard SQLite)
CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_accessed TEXT NOT NULL,
    access_count INTEGER DEFAULT 0,
    activation_score REAL DEFAULT 0.0,
    source_session TEXT,
    tags TEXT,           -- JSON array
    metadata TEXT        -- JSON object
);

-- Vector index (sqlite-vec virtual table)
CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(
    id TEXT PRIMARY KEY,
    embedding float[384]
);

-- Full-text search (SQLite FTS5)
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    id UNINDEXED,
    text,
    tags
);
```

**Query patterns:**

```sql
-- KNN vector search (top-K nearest neighbors)
SELECT id, distance FROM memories_vec
WHERE embedding MATCH ?1
ORDER BY distance
LIMIT ?2;

-- Full-text BM25 search
SELECT id, rank FROM memories_fts
WHERE memories_fts MATCH ?1
ORDER BY rank
LIMIT ?2;

-- Hybrid: retrieve candidates from both, fuse with RRF in application code.
```

**Embedding model**: `all-MiniLM-L6-v2` (384 dimensions) via ONNX Runtime (`ort` crate in Rust).
- Runs locally, no network dependency.
- ~80MB model file, stored in `~/.arcana/models/`.
- Fast inference (~5ms per embedding on CPU).

### 1.10 Configuration (`~/.arcana/memory.toml`)

```toml
[global]
knowledge_capacity = 10000
errors_capacity = 5000
eviction_decay_lambda = 0.01        # exponential decay rate
consolidation_on_evict = true       # merge low-score entries before removing

[session]
capacity = 1000
flush_interval_turns = 5
promotion_threshold = 2             # access_count needed for project promotion

[project]
auto_summarize_docs = true          # scan project docs on first session
doc_extensions = ["md", "ipynb", "tex", "typ", "rst", "txt"]
max_project_md_lines = 500

[retrieval]
errors_top_k = 3
knowledge_top_k = 5
project_max_tokens = 500
session_recent = 5
session_relevant = 3

[embedding]
model = "all-MiniLM-L6-v2"
model_path = "~/.arcana/models/all-MiniLM-L6-v2.onnx"
dimensions = 384
```

### 1.11 First-Startup Flow

```
1. Agent starts for the first time.
2. Detects ~/.arcana/ does not exist.
3. Prompts user: "Arcana needs to create ~/.arcana/ for persistent memory. Allow? [y/N]"
4. If approved:
   a. Creates ~/.arcana/ directory structure.
   b. Downloads embedding model to ~/.arcana/models/ (or prompts user to provide).
   c. Creates empty knowledge.db and errors.db.
   d. Generates initial SOUL.md from a default template.
   e. Creates empty USER.md.
5. If denied:
   a. Operates in "stateless mode" — no global memory, session-only.
   b. Reminds user periodically that memory is disabled.
```

### 1.12 Directory Layout Summary

```
~/.arcana/                          # Global (cross-project)
├── SOUL.md                         # Agent personality
├── USER.md                         # User portrait
├── knowledge.db                    # SQLite+vss: global knowledge
├── errors.db                       # SQLite+vss: errors & complaints
├── memory.toml                     # Memory system configuration
├── models/
│   └── all-MiniLM-L6-v2.onnx      # Local embedding model
├── INSTRUCTION.md                  # AAS JSONL API guidance
└── authority.toml                  # System-wide authority policy

<project>/.arcana/memory/           # Project-level
├── project.md                      # Human-readable project knowledge
├── sessions/
│   ├── <session-id-1>.db           # Session vector store
│   ├── <session-id-2>.db
│   └── ...
└── project.db                      # (optional) vectorized project knowledge
```

---

## 2. SKILL Management: Context and Hook Design

### 2.1 Overview

SKILLs are modular capabilities that extend the agent's behavior. Unlike tools (which are single commands), SKILLs bundle context, triggers, hooks, and execution logic into a coherent unit.

The key design constraint: **the AI cannot be trusted to self-trigger mandatory hooks** (formatting, tests, etc.). A background daemon independently evaluates every AI-parsed prompt against a trigger database and forces skill execution when matched.

```
┌──────────────────┐         ┌─────────────────────┐
│  Agent Process   │────────►│  Skill Daemon       │
│  (every prompt)  │ prompt  │  (trigger evaluator) │
└──────────────────┘         └──────────┬──────────┘
                                        │ trigger match?
                                        ▼
                              ┌─────────────────────┐
                              │  skill_trigger.db   │
                              │  (sqlite-vec)       │
                              └──────────┬──────────┘
                                        │ yes: skill X
                                        ▼
                              ┌─────────────────────┐
                              │  Skill Execution    │
                              │  (via Authority)    │
                              └─────────────────────┘
```

### 2.2 Skill Pools (Storage Layout)

```
~/.arcana/skills/
├── system/                     # System SKILLs (all projects)
│   ├── formatter/
│   │   ├── skill.toml
│   │   └── prompt.md
│   ├── test-runner/
│   │   ├── skill.toml
│   │   └── prompt.md
│   └── ...
├── user/                       # User-installed SKILLs (all projects)
│   ├── my-custom-skill/
│   │   ├── skill.toml
│   │   └── ...
│   └── ...
└── skill_trigger.db            # Trigger vector database (sqlite-vec)

<project>/.arcana/skills/       # Project-specific SKILLs
├── lint-check/
│   ├── skill.toml
│   └── prompt.md
└── ...
```

**Resolution order** (later overrides earlier): system → user → project.

### 2.3 Skill Manifest (`skill.toml`)

Each skill is a directory containing a `skill.toml`:

```toml
[skill]
name = "rust-formatter"
description = "Run cargo fmt after any Rust file modification"
version = "0.1.0"
enabled = true
priority = 100              # Higher = runs first (for ordering)
mode = "action"             # "context" | "action" | "hybrid"

[context]
# Injected into agent prompt when skill is active
prompt_file = "prompt.md"   # Relative path to context injection file
memory_access = true        # Can read all memory stores (default: true)
memory_write = false        # Cannot modify memories (default: false)

[triggers]
# Rule-based exact triggers (deterministic, checked first)
on_events = ["post-write"]                  # Event types
file_patterns = ["**/*.rs"]                 # Glob patterns on affected files
# Semantic triggers (vector-matched against AI prompts)
descriptions = [
    "modifying rust source code",
    "writing or editing .rs files",
    "refactoring rust code",
]

[hooks]
# What to execute when triggered
pre = []                                    # Commands before agent action
post = ["cargo fmt -- {files}"]             # Commands after agent action
inject_output = true                        # Feed hook output back to agent

[permissions]
# What this skill is allowed to do (enforced by authority)
read_paths = ["**/*.rs", "Cargo.toml"]
write_paths = []                            # Formatter writes via cargo fmt
exec_commands = ["cargo fmt"]
```

### 2.4 Skill Modes

| Mode | Behavior |
|------|----------|
| `context` | Injects `prompt_file` content into agent's context window. No execution. |
| `action` | Runs hook commands via authority program. No context injection. |
| `hybrid` | Both: injects context AND runs hooks. |

### 2.5 Trigger System (Two-Tier)

#### Tier 1: Rule-Based Exact Triggers (Deterministic)

Checked on every agent action. These **cannot be skipped by the AI**.

| Trigger Field | Matches On |
|---------------|-----------|
| `on_events` | Agent action type: `pre-write`, `post-write`, `pre-exec`, `post-exec`, `pre-commit`, `session-start`, `session-end` |
| `file_patterns` | Glob match on files affected by the action |

If both `on_events` and `file_patterns` are specified, both must match (AND logic).

#### Tier 2: Semantic Vector Triggers (Fuzzy)

The `descriptions` field in `skill.toml` is embedded and stored in `skill_trigger.db`. On every AI-parsed prompt, the daemon:

1. Embeds the prompt text.
2. Queries `skill_trigger.db` for nearest neighbors.
3. If similarity > threshold (configurable, default 0.75), the skill is triggered.

This catches intent-based triggers that can't be expressed as simple glob/event rules.

### 2.6 Trigger Database (`skill_trigger.db`)

SQLite + sqlite-vec database storing skill trigger embeddings:

```sql
CREATE TABLE IF NOT EXISTS triggers (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    skill_pool TEXT NOT NULL,        -- "system" | "user" | "project"
    description TEXT NOT NULL,       -- Original trigger description
    threshold REAL DEFAULT 0.75,     -- Similarity threshold for activation
    enabled INTEGER DEFAULT 1
);

CREATE VIRTUAL TABLE IF NOT EXISTS triggers_vec USING vec0(
    id TEXT PRIMARY KEY,
    embedding float[384]
);
```

**Human-editable**: Users can directly query/modify this database:
```bash
# List all triggers
sqlite3 ~/.arcana/skills/skill_trigger.db "SELECT skill_name, description, threshold FROM triggers"

# Disable a trigger
sqlite3 ~/.arcana/skills/skill_trigger.db "UPDATE triggers SET enabled=0 WHERE skill_name='formatter'"

# Adjust threshold
sqlite3 ~/.arcana/skills/skill_trigger.db "UPDATE triggers SET threshold=0.9 WHERE skill_name='test-runner'"
```

### 2.7 Skill Daemon (Trigger Evaluator)

A background Rust process started at agent launch. Communicates with the agent via a unix socket at `.arcana/skill_daemon.sock`.

**Responsibilities:**
1. Load all skill manifests from system/user/project pools.
2. Build/update `skill_trigger.db` from skill `descriptions`.
3. Listen for prompts from the agent process.
4. Evaluate triggers (rule-based first, then semantic).
5. Return list of triggered skills to the agent.
6. The agent (or daemon directly) executes skill hooks via the authority program.

**Protocol (JSON over unix socket):**

```json
// Agent → Daemon: evaluate triggers for this prompt
{"op": "evaluate", "prompt": "I'll refactor the parser in src/parser.rs", "event": "pre-write", "files": ["src/parser.rs"]}

// Daemon → Agent: triggered skills
{"triggered": [
    {"skill": "rust-formatter", "mode": "action", "reason": "file_pattern match: **/*.rs"},
    {"skill": "test-runner", "mode": "hybrid", "reason": "semantic match (0.82): modifying rust source code"}
]}

// Agent → Daemon: register/reload skills
{"op": "reload"}
{"op": "register", "path": "/path/to/skill/dir"}

// Agent → Daemon: query triggers (for debugging)
{"op": "query", "prompt": "writing python code", "top_k": 5}
```

**Lifecycle:**
- Started by the authority program at session begin.
- Runs as the same user (not sandboxed — it only reads skill configs and the trigger DB).
- Shuts down at session end.

### 2.8 Skill Context Injection

For skills with `mode = "context"` or `mode = "hybrid"`:

1. The skill's `prompt_file` (e.g., `prompt.md`) is read via the authority program.
2. Its content is injected into the agent's system prompt or context window.
3. Injection happens at session start (for always-active skills) or on trigger (for event-driven skills).

**Memory access**: Skills with `memory_access = true` can read from all memory stores (knowledge, errors, session, project). They receive a read-only view. This allows skills to be context-aware (e.g., a "code style" skill can read past style complaints from `errors.db`).

### 2.9 Skill Execution Flow

```
1. Agent generates a response / plans an action.
2. Agent sends prompt + planned action to skill daemon.
3. Daemon evaluates:
   a. Rule-based triggers (exact match on event + file patterns)
   b. Semantic triggers (vector search in skill_trigger.db)
4. Daemon returns triggered skills (sorted by priority).
5. For each triggered skill:
   a. If mode=context/hybrid: inject prompt_file into agent context.
   b. If mode=action/hybrid AND has pre-hooks: execute pre-hooks via authority.
6. Agent proceeds with its action.
7. After action completes:
   a. For each triggered skill with post-hooks: execute post-hooks via authority.
   b. If inject_output=true: feed hook stdout/stderr back into agent context.
8. Agent incorporates hook output in its next response.
```

### 2.10 Priority & Conflict Resolution

- Skills are sorted by `priority` (descending: higher number runs first).
- If two skills have the same priority, system < user < project order applies.
- **Mutual exclusion**: A skill can declare `conflicts_with = ["other-skill"]` in its manifest. If both trigger, only the higher-priority one runs.

### 2.11 Skill Lifecycle Commands

```bash
arcana skill list                    # List all installed skills + status
arcana skill install <path|url>      # Install a skill to user pool
arcana skill enable <name>           # Enable a skill
arcana skill disable <name>          # Disable a skill
arcana skill remove <name>           # Remove from user pool
arcana skill triggers                # Show all registered triggers
arcana skill test <prompt>           # Test which skills would trigger for a prompt
```

### 2.12 Persistence Across Sessions

- Skill installations persist in `~/.arcana/skills/system/` and `~/.arcana/skills/user/`.
- `skill_trigger.db` is rebuilt on daemon start from all active skill manifests (source of truth is always `skill.toml`).
- Runtime-registered command authority is approved by AAS and persisted to the
  project-level `.arcana/authority.toml`. Skills remain described by their own
  manifests; AAS decides whether their requested commands, paths, and network
  domains are permitted.

### 2.13 Security (Integration with Authority)

- All skill file reads go through the authority program (respects `deny_read` rules).
- All skill command executions go through the authority program (respects `deny`/`allow` tool lists).
- Skills cannot bypass the authority program — the daemon only *triggers*, it doesn't *execute*.
- A skill's `[permissions]` section is checked against the authority config. If a skill requests permissions not granted, the user is prompted on first trigger.

---

## 3. Sub-Agent Management

### 3.1 Overview

Arcana uses a **single dominant agent** (main agent) that can spawn and govern multiple sub-agents. There is no peer-to-peer multi-agent communication — all coordination flows through the main agent.

The architecture is a **blackboard pattern**: the main agent is the blackboard, sub-agents are knowledge sources that read from a passed context snapshot and write results back. Sub-agents cannot see each other's work until the main agent collects, summarizes, and redistributes.

```
┌─────────────────────────────────────────────────────────┐
│                     Main Agent                          │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────────┐   │
│  │ Context     │  │ Sub-Agent   │  │ Checkpoint   │   │
│  │ Summarizer  │  │ Orchestrator│  │ Manager      │   │
│  └─────────────┘  └──────┬──────┘  └──────────────┘   │
└───────────────────────────┼─────────────────────────────┘
                            │ spawn / freeze / collect
              ┌─────────────┼─────────────┐
              ▼             ▼             ▼
     ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
     │ Sub-Agent A  │ │ Sub-Agent B  │ │ Sub-Agent C  │
     │ (scope: src/)│ │ (scope: docs)│ │ (scope: test)│
     └──────────────┘ └──────────────┘ └──────────────┘
```

### 3.2 Spawning via System Skill

Sub-agent spawning is a system skill (`spawn_agent`) registered at startup:

```toml
# ~/.arcana/skills/system/spawn_agent/skill.toml
[skill]
name = "spawn_agent"
description = "Spawn a sub-agent with scoped authority"
mode = "action"
priority = 200

[triggers]
on_events = []
file_patterns = []
descriptions = ["delegate task to sub-agent", "spawn worker for subtask", "parallelize work"]
```

The main agent invokes spawning by sending a structured request to the sub-agent orchestrator daemon.

### 3.3 Sub-Agent Scope & Isolation

Each sub-agent receives:
- **A scoped authority config** — subset of file paths and commands it can access.
- **A context snapshot** — the main agent's summarized context for the task.
- **Read access to memory stores** — can query knowledge/errors/session DBs (read-only).
- **Its own conversation history** — independent from main agent and other sub-agents.

Each sub-agent **cannot**:
- Access other sub-agents' conversation or output (until main agent shares it).
- Write outside its declared scope.
- Spawn further sub-agents (single level only, to prevent runaway recursion).
- Modify memory stores (read-only access).

**Scope declaration** (passed at spawn time):

```json
{
    "id": "subagent-abc123",
    "task": "Implement the parser module",
    "context": "We are building a Rust CLI tool. The parser should...",
    "scope": {
        "read_paths": ["src/**", "Cargo.toml", "docs/parser.md"],
        "write_paths": ["src/parser.rs", "src/parser/**"],
        "exec_commands": ["cargo check", "cargo test -- parser"]
    },
    "model": "same-as-main",
    "max_turns": 50
}
```

### 3.4 Orchestrator Daemon

A Rust daemon (can be the same process as the skill daemon) that manages sub-agent lifecycle. Communicates with the main agent via unix socket at `.arcana/subagent.sock`.

**Responsibilities:**
1. Spawn sub-agent processes with scoped authority configs.
2. Track sub-agent status: `running`, `frozen`, `completed`, `failed`.
3. Enforce completion criteria (not trusting LLM to self-report "done").
4. Collect sub-agent outputs when they complete or are frozen.
5. Manage freeze/unfreeze lifecycle.

**Protocol:**

```json
// Main Agent → Orchestrator: spawn
{"op": "spawn", "task": "...", "context": "...", "scope": {...}}
// Response: {"id": "subagent-abc123", "status": "running"}

// Main Agent → Orchestrator: status
{"op": "status"}
// Response: {"agents": [{"id": "...", "status": "running", "turns": 12}, ...]}

// Main Agent → Orchestrator: collect (get output from a sub-agent)
{"op": "collect", "id": "subagent-abc123"}
// Response: {"id": "...", "output": "summary of work done", "files_modified": [...], "status": "completed"}

// Main Agent → Orchestrator: freeze all / freeze one
{"op": "freeze", "id": "subagent-abc123"}
{"op": "freeze_all"}

// Main Agent → Orchestrator: unfreeze
{"op": "unfreeze", "id": "subagent-abc123", "new_context": "updated context..."}

// Main Agent → Orchestrator: kill
{"op": "kill", "id": "subagent-abc123"}
```

### 3.5 Looping Execution Pattern

The main agent runs sub-agents in a **monitor-collect-redistribute loop**:

```
1. Main agent spawns N sub-agents with tasks + context.
2. Loop:
   a. Orchestrator monitors all sub-agents.
   b. When a sub-agent completes (or hits max_turns):
      - Orchestrator marks it "completed" or "stalled".
      - Main agent collects its output.
   c. Main agent summarizes collected outputs.
   d. Main agent redistributes updated context to remaining sub-agents
      (via unfreeze with new_context, or by spawning replacements).
   e. If all tasks done → exit loop.
   f. If user sends freeze → freeze all, serialize state.
```

### 3.6 Freeze & Checkpoint

**Freeze** means: "finish current atomic step (complete the current LLM response), then serialize state to disk." This avoids corrupted partial outputs.

**What gets serialized (per agent):**

```json
{
    "agent_id": "subagent-abc123",
    "status": "frozen",
    "conversation_history": [...],   // Full message array
    "task": "...",
    "context_snapshot": "...",
    "scope": {...},
    "turn_count": 12,
    "files_modified": ["src/parser.rs"],
    "memory_retrieval_state": {...}, // Last retrieved memories
    "active_skills": ["rust-formatter"],
    "frozen_at": "2026-05-20T00:05:00Z"
}
```

**Storage:**

```
.arcana/checkpoints/
├── main_agent.json              # Main agent checkpoint
├── subagent-abc123.json         # Sub-agent checkpoints
├── subagent-def456.json
└── orchestrator_state.json      # Which agents exist, their status
```

**Resume from checkpoint:**
1. Load `orchestrator_state.json` to reconstruct the agent tree.
2. For each frozen agent, load its checkpoint.
3. Resume the main agent's conversation from its last message.
4. Unfreeze sub-agents as needed (with optional updated context).

**Token savings**: Instead of replaying the entire conversation to the LLM, the checkpoint stores the full message history. On resume, this is sent as-is to the LLM API — no re-generation needed. The LLM sees it as a continuation of the same conversation.

### 3.7 Completion Criteria (Daemon-Enforced)

The orchestrator does NOT trust the LLM to self-report completion. Instead:

| Criterion | How Enforced |
|-----------|-------------|
| Max turns reached | Orchestrator counts turns, force-freezes at limit |
| Explicit done signal | Sub-agent must call a `done` tool (registered as a skill) |
| Post-hooks pass | If the sub-agent's scope has required post-hooks (tests, lint), they must pass |
| Stall detection | If N consecutive turns produce no file modifications, mark as stalled |
| Main agent recall | Main agent can force-collect at any time |

The `done` tool is a special skill that the sub-agent calls when it believes its task is complete. The orchestrator then runs any post-hooks (tests, etc.) and only marks "completed" if they pass. Otherwise, it feeds the failure back to the sub-agent for another attempt.

### 3.8 Memory Access Model

| Memory Tier | Sub-Agent Access |
|-------------|-----------------|
| `SOUL.md` | Read (inherits main agent personality) |
| `USER.md` | Read |
| `knowledge.db` | Read (query) |
| `errors.db` | Read (query) |
| `session.db` | No access (main agent's session is private) |
| `project.md` | Read |
| Sub-agent's own session | Read + Write (its own conversation summaries) |

Sub-agents share the same embedding model and can query the global knowledge/error stores to avoid repeating known mistakes. But they cannot write to them — only the main agent promotes knowledge after collecting sub-agent outputs.

### 3.9 Authority Integration

When a sub-agent is spawned, the orchestrator:
1. Creates a temporary scoped authority config at `.arcana/subagent_configs/<id>.toml`.
2. Starts a sub-agent process connected to the main authority program but with restricted rules.
3. The authority program checks the sub-agent's config for every request (using the agent's ID to look up its scope).

```toml
# .arcana/subagent_configs/subagent-abc123.toml
[rules]
allow_write = ["src/parser.rs", "src/parser/**"]
deny_write = ["*"]
allow_read = ["src/**", "Cargo.toml", "docs/parser.md"]

[tools]
allow = ["cargo check", "cargo test -- parser"]
deny = ["*"]
```

### 3.10 Limitations (By Design)

- **Single level only**: Sub-agents cannot spawn further sub-agents.
- **No peer communication**: Sub-agents are isolated from each other.
- **Main agent bottleneck**: All coordination flows through the main agent. This is intentional — it keeps the system predictable and auditable.
- **Shared model**: Sub-agents use the same LLM model as the main agent (configurable per-spawn for cost optimization in the future).

---

## 4. Human-in-the-Loop Design

### 4.1 Overview

The human-in-the-loop (HITL) system ensures the user retains full control over the agent at all times — able to interrupt, correct, edit, and resume without wasting tokens on regeneration.

**Core principles:**
- The agent is always interruptible (freeze at any point).
- Every file mutation requires explicit human approval (unless session-approved).
- Human edits feed back into the LLM context as corrections.
- Long-running loops persist state to disk so humans can leave and return.
- Session history is managed, named, and recoverable.

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Terminal                             │
│                                                                 │
│  Ctrl+Shift+P  → Freeze (backup state, pause all agents)       │
│  Ctrl+Shift+M  → Modify last prompt (re-submit corrected)      │
│  Ctrl+O        → Expand diff view                               │
│                                                                 │
│  On file write: show diff → [A]ccept / [S]ession-accept /      │
│                              [E]dit / [X] Abort                 │
│                                                                 │
│  On long-loop: [D]one / [U]nfinished / [A]bort                 │
└─────────────────────────────────────────────────────────────────┘
```

### 4.2 Interrupt Commands

| Keybinding | Action | Effect |
|------------|--------|--------|
| `Ctrl+Shift+P` | **Freeze & Backup** | Freeze main agent + all sub-agents. Serialize full state to disk. User can inspect/edit checkpoints, then resume later. |
| `Ctrl+Shift+M` | **Modify Last Prompt** | Discard the agent's current/last response. Open the user's last message in `$EDITOR` for correction. Re-submit the edited prompt. |
| `Ctrl+C` | **Abort Current** | Cancel current LLM generation. Agent remains alive, awaits next input. |
| `Ctrl+D` | **End Session** | Graceful session end. Flush memories, run eviction, save session summary. |

**Freeze behavior:**
1. Signal the LLM to stop generating (cancel in-flight request).
2. Wait for any in-progress authority operations to complete (atomic — don't corrupt files).
3. Serialize: main agent checkpoint + all sub-agent checkpoints + orchestrator state.
4. Print: "Session frozen. Resume with `arcana resume <session-id>`."

### 4.3 Diff Review (File Mutation Approval)

Every file write requested by the agent triggers a diff review flow:

```
┌─────────────────────────────────────────────────────┐
│ Agent wants to write: src/parser.rs                  │
├─────────────────────────────────────────────────────┤
│  @@ -12,3 +12,5 @@                                  │
│   fn parse(input: &str) -> Result<Ast> {             │
│  -    todo!()                                        │
│  +    let tokens = tokenize(input)?;                 │
│  +    build_ast(&tokens)                             │
│   }                                                  │
│                                                      │
│  [A]ccept  [S]ession-accept  [E]dit  [X]Abort       │
└─────────────────────────────────────────────────────┘
```

**Options:**

| Key | Action | Description |
|-----|--------|-------------|
| `A` | Accept & Continue | Apply this write, continue agent execution. |
| `S` | Session-Accept | Accept this write AND auto-accept all future writes to this file for this session. ⚠️ Dangerous — shown with warning. |
| `E` | Edit | Open the modified file in user's configured editor. After editor closes, compute diff between LLM's version and user's version. Inject correction into agent context. |
| `X` | Abort | Reject the write. Agent is informed the write was denied. |

**Diff display rules:**
- Default: show max 20 lines of context around changes.
- `Ctrl+O`: expand to full diff.
- If diff is empty (no actual change): auto-accept silently.

### 4.4 Editor Integration

**Configuration** (`~/.arcana/hitl.toml`):

```toml
[editor]
command = "nvim"                    # or "vim", "code --wait", "emacs"
diff_command = "delta"              # Diff renderer (optional, falls back to built-in)
wait_for_close = true               # Wait for editor process to exit

[diff]
max_lines = 20                      # Default collapsed diff lines
auto_accept_empty = true            # Auto-accept if diff is empty

[approval]
default_timeout_secs = 0            # 0 = wait forever (no auto-accept)
session_accept_warning = true       # Show warning for session-accept
```

**Editor flow:**
1. Write the LLM's proposed content to a temp file.
2. Open `$editor <temp_file>` (or `$editor --diff <original> <proposed>` if supported).
3. Wait for editor process to exit (for terminal editors) or for user to press Enter (for GUI editors with `wait_for_close = false`).
4. Read the temp file back.
5. If user modified it: compute diff between LLM's version and user's version.
6. Apply user's version to the actual file (via authority).
7. Inject a context message: `"User edited your proposed change to {file}: {diff}"`.

### 4.5 Context Update on Human Edit

When the user modifies an LLM-proposed file change, the agent needs to learn from the correction. The system injects a synthetic message into the conversation:

```json
{
    "role": "system",
    "content": "User modified your proposed change to src/parser.rs:\n--- Your version\n+++ User's version\n@@ -1,2 +1,3 @@\n fn parse(input: &str) -> Result<Ast> {\n-    let tokens = tokenize(input)?;\n+    let tokens = tokenize(input).map_err(ParseError::Lex)?;\n+    validate_tokens(&tokens)?;\n     build_ast(&tokens)\n }"
}
```

This teaches the LLM the user's preferences in-context without regenerating the entire conversation.

### 4.6 Long-Loop Human Interaction

For skill-driven loops (e.g., multi-turn sub-agent coordination where human input is required each iteration):

```
┌─────────────────────────────────────────────────────┐
│ Loop iteration 3/∞ complete.                         │
│ Sub-agents produced: [parser.rs, lexer.rs updated]   │
│ Main agent summary: "Parser now handles nested..."   │
│                                                      │
│ Your turn: review/edit notebooks, then signal:       │
│  [D]one  [U]nfinished (save & exit)  [A]bort        │
└─────────────────────────────────────────────────────┘
```

| Signal | Effect |
|--------|--------|
| `D` (Done) | Human is satisfied. Main agent proceeds to next phase or ends loop. |
| `U` (Unfinished) | Save full state (main agent + sub-agents + orchestrator + loop iteration counter) to disk. User can leave. Resume later with `arcana resume`. |
| `A` (Abort) | Discard loop progress. Revert to state before loop started (if checkpoint exists). |

**Daemon management**: The main agent daemon listens for these signals on stdin. On `U`, it:
1. Freezes all agents (main + sub).
2. Saves loop state (iteration count, accumulated context, sub-agent outputs so far).
3. Exits cleanly.

On next `arcana resume <session>`:
1. Loads orchestrator state + all agent checkpoints.
2. Resumes the loop from where it left off.
3. Presents the same `[D]/[U]/[A]` prompt to the user.

### 4.7 Session Management

Every interaction with Arcana is a **session**. Sessions are named, checkpointed, and recoverable.

**Session storage:**

```
.arcana/sessions/
├── index.json                      # Session index (id, name, status, timestamps)
├── <session-id>/
│   ├── meta.json                   # Session metadata (name, model, start time)
│   ├── conversation.json           # Full message history
│   ├── checkpoints/                # Agent checkpoints (same as §3.6)
│   │   ├── main_agent.json
│   │   └── subagent-*.json
│   └── loop_state.json             # Long-loop state (if applicable)
└── <session-id-2>/
    └── ...
```

**Session index (`index.json`):**

```json
[
    {
        "id": "sess-a1b2c3d4",
        "name": "Implement parser module",
        "status": "frozen",
        "created_at": "2026-05-20T00:30:00Z",
        "last_active": "2026-05-20T00:40:00Z",
        "turn_count": 23,
        "model": "claude-sonnet-4-20250514"
    }
]
```

**Session naming:**
- On first meaningful exchange, the LLM generates a short summary name (< 60 chars).
- User can rename: `arcana session rename <id> "New Name"`.
- The `id` is a stable UUID-based identifier (never changes).

### 4.8 Session Recovery

**Two recovery modes:**

| Mode | Trigger | Behavior |
|------|---------|----------|
| Clean resume | User ran `arcana resume <id>` | Load full checkpoint, continue conversation. |
| Crash recovery | Process died unexpectedly | On next startup, detect incomplete session. Offer: "Last session was interrupted. Resume? [Y/n]" |

**Crash detection:**
- On session start, write a lock file: `.arcana/sessions/<id>/lock`.
- On clean exit, remove the lock file.
- On next startup, if a lock file exists without a corresponding "completed" or "frozen" status → crash detected.

**Crash recovery strategy:**
- Load the last flushed conversation state (may be missing the last 1-2 turns if buffer wasn't flushed).
- Inform the user: "Recovered session. Last N turns may be incomplete."
- Resume from the last known good state.

### 4.9 Session Lifecycle Commands

```bash
arcana session list                  # List all sessions (name, status, date)
arcana session resume <id|name>      # Resume a frozen/crashed session
arcana session rename <id> "name"    # Rename a session
arcana session delete <id>           # Delete a session and its checkpoints
arcana session export <id>           # Export session as JSON (for sharing/backup)
arcana session import <file>         # Import a session from JSON
```

### 4.10 Configuration Summary (`~/.arcana/hitl.toml`)

```toml
[editor]
command = "nvim"
diff_command = "delta"
wait_for_close = true

[diff]
max_lines = 20
auto_accept_empty = true

[approval]
default_timeout_secs = 0
session_accept_warning = true

[session]
max_sessions_kept = 50              # Auto-prune oldest sessions beyond this
auto_name = true                    # LLM generates session name
crash_recovery_prompt = true        # Ask before recovering crashed session

[freeze]
auto_freeze_on_disconnect = true    # Freeze if terminal disconnects (SIGHUP)
checkpoint_conversation = true      # Include full conversation in checkpoint
```

### 4.11 Integration with Other Systems

| System | HITL Integration |
|--------|-----------------|
| Authority | Diff review happens *before* authority commits the write. Rejected writes never reach disk. |
| Memory | Session conversation is flushed to `session.db` on freeze/end. Human edits are recorded as corrections in `errors.db`. |
| Skills | Skills with `inject_output=true` show their output in the diff review flow if they modify files. |
| Sub-agents | `Ctrl+Shift+P` freezes the entire agent tree. Long-loop `[U]` saves all sub-agent state. |
| Checkpoints | HITL freeze uses the same checkpoint system as §3.6. Sessions are the top-level container. |
