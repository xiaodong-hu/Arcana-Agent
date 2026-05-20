# TUI Design

Arcana's terminal user interface is built with **ratatui** (Rust) and runs in the alternate screen. It streams LLM output token-by-token, renders reasoning/thinking blocks in a collapsible panel, and manages multiple workspace views (main agent + query sub-agent overlay).

The design borrows structural ideas from [Hermes Agent TUI](https://github.com/nousresearch/hermes-agent) (status lines, skill/sub-agent panels, collapsible sections) and streaming patterns from [DeepSeek-TUI](https://github.com/Hmbown/DeepSeek-TUI) (thinking-mode streaming, long-output handling, ratatui architecture).

---

## 1. Architecture Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                        Terminal (alternate screen)                │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Welcome Banner (ASCII art, one-time on session start)     │  │
│  ├────────────────────────────────────────────────────────────┤  │
│  │  Status Bar (model, tokens, cost, skills, sub-agents)      │  │
│  ├────────────────────────────────────────────────────────────┤  │
│  │  Main Viewport                                             │  │
│  │  ┌──────────────────────────────────────────────────────┐  │  │
│  │  │  Conversation Stream (scrollable)                    │  │  │
│  │  │  - User messages                                     │  │  │
│  │  │  - Agent responses (streamed token-by-token)         │  │  │
│  │  │  - Thinking blocks (collapsible, dimmed)             │  │  │
│  │  │  - Tool calls & results                              │  │  │
│  │  │  - Diff review panels                                │  │  │
│  │  └──────────────────────────────────────────────────────┘  │  │
│  ├────────────────────────────────────────────────────────────┤  │
│  │  Input Composer (multi-line, with prompt glyph)            │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  [OVERLAY] Query Sub-Agent (toggled with `?`)              │  │
│  │  - Shares main agent context (zero extra tokens)           │  │
│  │  - Dismissed with `q` (agent stays alive)                  │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

**Rendering engine**: ratatui with crossterm backend. Differential rendering (only repaint changed cells) to avoid flicker during streaming.

**Event loop**: Tokio async runtime. Three concurrent tasks:
1. **Input handler** — reads terminal events (keys, mouse, resize).
2. **LLM stream consumer** — receives tokens from the async streaming client, appends to viewport.
3. **Daemon listener** — receives events from authority/skill/sub-agent daemons via unix sockets.

---

## 2. Welcome Banner (ASCII Art)

Displayed once at session start, before the first prompt. Fades after the user types their first message (scrolls up into history).

```
    ╔═══════════════════════════════════════════════════════════╗
    ║                                                           ║
    ║      ░█████╗░██████╗░░█████╗░░█████╗░███╗░░██╗░█████╗░  ║
    ║      ██╔══██╗██╔══██╗██╔══██╗██╔══██╗████╗░██║██╔══██╗  ║
    ║      ███████║██████╔╝██║░░╚═╝███████║██╔██╗██║███████║  ║
    ║      ██╔══██║██╔══██╗██║░░██╗██╔══██║██║╚████║██╔══██║  ║
    ║      ██║░░██║██║░░██║╚█████╔╝██║░░██║██║░╚███║██║░░██║  ║
    ║      ╚═╝░░╚═╝╚═╝░░╚═╝░╚════╝░╚═╝░░╚═╝╚═╝░░╚══╝╚═╝░░╚═╝  ║
    ║                                                           ║
    ║          The Arcane Agent — Memory · Skills · Authority   ║
    ║                                                           ║
    ╚═══════════════════════════════════════════════════════════╝

      Model: deepseek-v4-pro          Session: new
      Skills: 3 active                Sub-agents: 1 (query)
      Memory: SOUL.md ✓  USER.md ✓   Project: my-project
```

**Design notes:**
- The ASCII block letters use the "ANSI Shadow" font style (Unicode box-drawing + block elements).
- Colors: gradient from deep purple (`#7B2FBE`) to electric blue (`#00D4FF`) across the letters (256-color/truecolor terminals). Falls back to bold white on 16-color terminals.
- The metadata lines below the banner are rendered in dim text and update live as skills/daemons come online (same pattern as Hermes TUI's progressive banner fill).

---

## 3. Status Bar

A persistent bar between the banner area and the conversation viewport. Supports multiline expansion (default: expanded) and single-line folding via keyboard shortcuts.

### 3.1 Main Line (Always Visible)

```
 ⚗ deepseek-v4-pro │ [████░░░░░░] 8.2K/1M
```

| Segment | Content |
|---------|---------|
| Model glyph + name | `⚗` (alchemist flask) + active model name |
| Context bar + tokens | Visual fill `[████░░░░░░]` with color thresholds (green < 50%, yellow 50–80%, orange 80–95%, red ≥ 95%) followed by `used/max` |

### 3.2 Expandable Panels

Additional lines appear when toggled on:

| Shortcut | Panel | Content |
|----------|-------|---------|
| `Ctrl+T` | Tasks | `Tasks 2/7: ✓ parse │ ▶ codegen │ ○ tests │ ...` |
| `Ctrl+S` | Skills | `Skills (3): shell, file_ops, web_fetch` |
| `Ctrl+A` | Agents | `Agents 2/1: parser(running), codegen(running), test(frozen)` |

Default state: tasks expanded, skills and agents folded. Each toggle flips between expanded/folded.

### 3.3 Error Display

When the LLM returns an error (rate limit, API error, timeout, network failure), it is displayed as:
- A **red-bordered toast** in the top-right corner (auto-dismisses after 5s).
- A **system error message** in the viewport (prefixed with `⚠`).

Rate limit errors include retry-after information when available.

---

## 4. Conversation Viewport

The main scrollable area showing the interaction history.

### 4.1 Message Rendering

| Element | Style |
|---------|-------|
| User messages | Bold, prefixed with `❯` glyph |
| Agent responses | Normal weight, streamed token-by-token |
| Response stats | Dim, appended after every agent response: `Expense: 0.0031 ( 1.2K in / 847 out )\nTime: 2.4s` |
| Thinking blocks | Dimmed (50% opacity), collapsible with `▸`/`▾` chevron, italic |
| Tool calls | Indented, prefixed with tool icon (`💻` shell, `📄` file, `🔍` search, `🌐` web) |
| Tool results | Indented further, in a bordered box (single-line border) |
| Diff reviews | Full diff panel with colored +/- lines (see §4.3) |
| System messages | Centered, dim, no prefix |
| Error messages | Prefixed with `⚠`, red-tinted |

### 4.2 Streaming Long Outputs (DeepSeek V4 Thinking)

DeepSeek V4 models produce very long reasoning/thinking blocks (often 2000+ tokens before the final answer). The TUI handles this with:

1. **Live streaming into a collapsible panel**: Thinking tokens stream into a dedicated `<think>` panel that auto-scrolls. The panel header shows a live token counter: `▾ Thinking (1,247 tokens…)`.

2. **Auto-collapse on completion**: When the `</think>` delimiter arrives (or the model switches to final output), the thinking panel auto-collapses to a single summary line: `▸ Thinking (2,103 tokens) — 4.2s`. User can expand with `Ctrl+o` or click.

3. **Viewport pinning**: While thinking streams, the viewport pins to the bottom (auto-scroll). If the user scrolls up manually, auto-scroll disengages (re-engages on new user message).

4. **Partial render optimization**: Only the last N visible lines of the thinking block are rendered to the terminal. Earlier lines are buffered in memory but not painted — this prevents the terminal from choking on rapid token delivery (>100 tokens/sec).

5. **Interleaved thinking + output**: If the model interleaves thinking and output (multi-step reasoning), each thinking segment gets its own collapsible block, numbered: `▸ Think #1 (800 tokens)`, `▸ Think #2 (1,200 tokens)`.

```
┌─ Thinking (streaming…) ──────────────────────────────────────┐
│ Let me analyze the parser structure. The current              │
│ implementation uses a recursive descent approach, but         │
│ the user wants to switch to a Pratt parser for better         │
│ operator precedence handling. I need to consider...           │
│ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │
│                                          1,247 tokens • 3.1s │
└──────────────────────────────────────────────────────────────┘
```

After completion:
```
▸ Thinking (2,103 tokens) — 4.2s                    [`Ctrl+o` to expand]
```

### 4.3 Diff Review Panel

When the agent proposes a file write, the diff review renders inline:

```
┌─ Agent proposes: src/parser.rs ───────────────────────────────┐
│  @@ -12,3 +12,5 @@                                           │
│   fn parse(input: &str) -> Result<Ast> {                      │
│  -    todo!()                                                 │
│  +    let tokens = tokenize(input)?;                          │
│  +    build_ast(&tokens)                                      │
│   }                                                           │
├───────────────────────────────────────────────────────────────┤
│  [A]ccept  [S]ession-accept  [E]dit  [X]Abort  [O]Expand     │
└───────────────────────────────────────────────────────────────┘
```

Colors: `-` lines in red, `+` lines in green, context in default. The `[O]` key expands to full file diff (same as `Ctrl+O` from HITL design).

### 4.4 Scrolling & Navigation

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Scroll viewport line-by-line |
| `PgUp`/`PgDn` | Scroll viewport by page |
| `Home`/`End` or `g`/`G` | Jump to top/bottom of history |
| `Ctrl+U`/`Ctrl+D` | Half-page scroll (vim-style) |

Scrolling disengages auto-scroll. Pressing `End` or `G` or typing a new message re-engages it.

---

## 5. Input Composer

A multi-line text input area at the bottom of the screen.

### 5.1 Layout

```
 ❯ |                                                    [Enter to send]
```

- Prompt glyph: `❯` (bold, colored to match session theme).
- Grows vertically as the user types multi-line input (up to 10 lines before internal scroll).
- Shows `[Enter to send]` hint on first use, then hides.

### 5.2 Keybindings (Composer)

| Key | Action |
|-----|--------|
| `Enter` | Send message (if non-empty) |
| `Alt+Enter` / `Ctrl+J` | Insert newline |
| `Ctrl+C` | Interrupt agent (if running) / clear input (if idle) |
| `Ctrl+D` | End session (graceful) |
| `Ctrl+Shift+P` | Freeze & backup (all agents) |
| `Ctrl+Shift+M` | Modify last prompt (re-edit) |
| `?` | Open query sub-agent overlay (when not in text input mode) |
| `Tab` | Autocomplete `/` commands |
| `↑` (at empty input) | Recall previous user message |

### 5.3 Slash Commands

Typing `/` activates a floating autocomplete panel above the composer:

```
┌─────────────────────────────────┐
│ /model     Change active model  │
│ /skills    List active skills   │
│ /agents    Show sub-agent tree  │
│ /tasks     Show task progress   │
│ /freeze    Freeze all agents    │
│ /resume    Resume session       │
│ /memory    Memory commands      │
│ /help      Show all commands    │
└─────────────────────────────────┘
```

Arrow keys navigate, `Tab` or `Enter` selects, `Esc` dismisses.

---

## 6. Query Sub-Agent Overlay

A core UX feature: a lightweight sub-agent for fast questions that shares the main agent's context window exactly (zero additional token cost).

### 6.1 Design Rationale

Users often need to ask quick questions mid-task ("what's the signature of X?", "explain this error") without derailing the main agent's current work. The query sub-agent:
- **Shares context**: Reads the same conversation history and memory as the main agent. No context duplication.
- **Non-destructive**: Its responses do NOT append to the main agent's conversation history. The main agent never sees the query exchange.
- **Always alive**: Spawned at session start, never killed (only hidden/shown).
- **Single layer**: Cannot be nested. Pressing `?` while the overlay is open does nothing.

### 6.2 Activation & Dismissal

| Key | State | Action |
|-----|-------|--------|
| `?` | Main viewport active, composer empty | Open query overlay |
| `?` | Main viewport active, composer has text | Insert literal `?` character |
| `q` | Query overlay active, composer empty | Dismiss overlay, return to main |
| `q` | Query overlay active, composer has text | Insert literal `q` character |
| `Esc` | Query overlay active | Dismiss overlay (always, regardless of composer state) |

### 6.3 Overlay Layout

The query overlay renders as a floating panel covering ~80% of the viewport height, with the main viewport dimmed behind it:

```
┌─ Query Agent ─────────────────────────────────────────────────┐
│                                                               │
│  (conversation history within this overlay session)           │
│                                                               │
│  ❯ what's the return type of parse()?                         │
│                                                               │
│  The `parse` function returns `Result<Ast, ParseError>`       │
│  where `Ast` is defined in src/ast.rs...                      │
│                                                               │
├───────────────────────────────────────────────────────────────┤
│  ❯ |                                          [q to go back]  │
└───────────────────────────────────────────────────────────────┘
```

**Behavior:**
- The overlay has its own mini conversation history (cleared on session end, not persisted).
- Responses stream the same way as the main viewport (token-by-token, thinking blocks, etc.).
- The main agent continues running in the background while the overlay is open. If the main agent produces output, a subtle notification appears at the overlay border: `[main agent active ↓]`.

### 6.4 Context Sharing Implementation

The query sub-agent does NOT maintain a separate LLM conversation. Instead:
1. On each query, it constructs a prompt from: `SOUL.md` + `USER.md` + main agent's current conversation history (read-only snapshot) + the user's query.
2. The response is streamed to the overlay viewport.
3. The response is NOT appended to the main agent's history.
4. This means the query agent is "stateless" relative to the main agent — each query is independent (but sees the full main context).

**Token cost**: Only the query's input tokens (context snapshot + question) + output tokens. No duplication of stored context — it's the same context the main agent already has loaded.

---

## 7. Panels & Collapsible Sections

Inspired by Hermes TUI's collapsible banner sections. These appear in the status area (between banner and viewport) and can be toggled:

### 7.1 Skills Panel

```
▾ Skills (3 active)
  ├─ rust-formatter    [action]  post-write on **/*.rs
  ├─ test-runner       [hybrid]  semantic: "modifying rust code"
  └─ code-review       [context] always-on
```

### 7.2 Sub-Agents Panel

```
▾ Sub-Agents (2 running, 1 frozen)
  ├─ parser-impl       [running]  turn 12/50  src/parser/**
  ├─ test-writer       [running]  turn 3/50   tests/**
  └─ docs-updater      [frozen]   turn 8/50   docs/**
```

### 7.3 Tasks Panel

```
▾ Tasks (2/7 complete)
  ├─ ✓ Define AST types
  ├─ ✓ Implement tokenizer
  ├─ ◉ Implement parser (in progress — parser-impl)
  ├─ ○ Write parser tests
  ├─ ○ Implement code generator
  ├─ ○ Integration tests
  └─ ○ Documentation
```

**Toggle**: Click section header or press `1`/`2`/`3` (when not in composer). Default state: Skills collapsed, Sub-Agents collapsed, Tasks expanded (if tasks exist).

---

## 8. Theming & Colors

### 8.1 Color Scheme

Default theme: "Arcane" — dark background with purple/blue accent palette.

| Element | Color |
|---------|-------|
| Background | Terminal default (transparent) |
| Banner text | Gradient purple→blue (`#7B2FBE` → `#00D4FF`) |
| User messages | Bold white |
| Agent responses | Default foreground |
| Thinking blocks | Dim (50% brightness), italic |
| Tool calls | Cyan |
| Errors | Red |
| Diff `+` lines | Green |
| Diff `-` lines | Red |
| Status bar bg | Dark gray (`#1a1a2e`) |
| Prompt glyph | Purple (`#7B2FBE`) |
| Query overlay border | Electric blue (`#00D4FF`) |

### 8.2 Terminal Compatibility

| Terminal | Support Level |
|---------|--------------|
| Truecolor (24-bit) | Full gradient, all colors |
| 256-color | Approximated palette, no gradient |
| 16-color | Bold/dim only, no custom colors |
| No-color (`NO_COLOR=1`) | Monochrome, structural indicators only |

Detection: Query `COLORTERM`, `TERM`, and probe via OSC 4 on startup.

---

## 9. Notification System

### 9.1 In-TUI Notifications

Non-blocking toast-style notifications in the top-right corner:

```
                                    ┌─────────────────────────┐
                                    │ ✓ parser-impl completed │
                                    │   3 files modified      │
                                    └─────────────────────────┘
```

Auto-dismiss after 5 seconds. Stack vertically if multiple arrive.

### 9.2 Terminal Notifications

When the TUI is in the background (user switched to another tmux pane or terminal tab):

- **OSC 9** (iTerm2/WezTerm/Ghostty): Desktop notification with title "Arcana" and event summary.
- **OSC 99** (Kitty): Native notification.
- **Bell** (`\x07`): Fallback for terminals without OSC support. Configurable: `notifications.bell = true/false`.

---

## 10. Performance Considerations

### 10.1 Streaming at High Token Rates

DeepSeek V4 can deliver 100+ tokens/second. The TUI must not drop frames:

- **Batch rendering**: Accumulate tokens for up to 16ms (one frame at 60fps) before triggering a repaint. This coalesces rapid token arrivals into single frame updates.
- **Viewport culling**: Only render lines visible in the viewport. Off-screen content is stored in a line buffer but not painted.
- **Incremental layout**: When a new token arrives, only re-layout the current paragraph (not the entire history).
- **Ring buffer for history**: Conversation history beyond 10,000 lines is evicted from the render buffer (still accessible via scroll, loaded on demand from the line store).

### 10.2 Memory Usage

- Conversation text: stored as a `Vec<Line>` where each `Line` is a styled rope segment.
- Thinking blocks: stored compressed (zstd) after collapse. Decompressed on expand.
- Target: < 50MB RSS for a 2-hour session with heavy streaming.

---

## 11. Accessibility

- **Screen reader mode** (`--accessible` flag or `NO_ANIMATIONS=1`): Disables animations, uses plain text indicators instead of Unicode glyphs, outputs to stdout line-by-line (no alternate screen).
- **High contrast**: Respects `TERM_PROGRAM` hints and system high-contrast settings.
- **Keyboard-only**: All features accessible without mouse. Mouse support is optional enhancement (click to expand sections, drag to select text).

---

## 12. Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | TUI framework (widgets, layout, rendering) |
| `crossterm` | Terminal backend (events, raw mode, alternate screen) |
| `tokio` | Async runtime (event loop, socket listeners, LLM streaming) |
| `syntect` | Syntax highlighting for code blocks in responses |
| `unicode-width` | Correct CJK/emoji width calculation |
| `textwrap` | Word-wrapping for long lines |
| `similar` | Diff computation for file review panels |
| `zstd` | Compression for collapsed thinking blocks |

---

## 13. Open Questions

- [ ] Should the query sub-agent overlay support syntax-highlighted code responses? (adds complexity to the overlay renderer) --- Yes I would.
- [ ] Mouse support: drag-to-select for copy? Or rely on terminal's native selection? --- Yes I would like to add copy paste support and mouse support
- [ ] Image rendering in terminal (for models that return images): sixel/kitty graphics protocol support? --- currently no
- [ ] Should thinking blocks be searchable (`/` search within collapsed blocks)? --- Great idea! I want it able to support thinking search
