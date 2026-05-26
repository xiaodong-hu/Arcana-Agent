use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::Theme;
use crate::types::{Message, MessageRole, ThinkingBlock, ToolCall, ToolType};

const TOOL_HINT: Color = Color::Rgb(160, 160, 170);
const TOOL_OUTPUT: Color = Color::Rgb(185, 185, 195);
const PIGMENT_GREEN: Color = Color::Rgb(0, 165, 80);
const AMBER_SAE_ECE: Color = Color::Rgb(255, 126, 0);
const AWESOME_RED: Color = Color::Rgb(255, 33, 82);
const BU_RED: Color = Color::Rgb(204, 0, 0); // Boston University Red
const DIFF_ADDED_BG: Color = Color::Rgb(0, 55, 30); // dark green bg for added lines
const DIFF_REMOVED_BG: Color = Color::Rgb(70, 10, 10); // dark red bg for removed lines

/// Viewport state: manages scroll position and message rendering.
#[derive(Debug)]
pub struct Viewport {
    /// All messages in the conversation
    pub messages: Vec<Message>,
    /// Scroll offset from the bottom (0 = pinned to bottom)
    pub scroll_offset: usize,
    /// Whether auto-scroll is engaged
    pub auto_scroll: bool,
    /// Currently streaming thinking block (if any)
    pub streaming_think: Option<StreamingThink>,
    /// Currently streaming response text
    pub streaming_text: String,
    /// Whether we're currently receiving a response
    pub is_streaming: bool,
    /// Whether thinking panels (including streaming) are collapsed
    pub think_collapsed: bool,
    /// Whether tool-call panels are collapsed
    pub tool_calls_collapsed: bool,
    /// Last rendered visual line count, used to keep manual-scroll views stable as content grows.
    last_total_lines: usize,
}

#[derive(Debug)]
pub struct StreamingThink {
    pub content: String,
    pub token_count: usize,
    pub start_time: std::time::Instant,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            streaming_think: None,
            streaming_text: String::new(),
            is_streaming: false,
            think_collapsed: true,
            tool_calls_collapsed: false,
            last_total_lines: 0,
        }
    }
}

impl Viewport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a token to the current streaming response.
    pub fn append_token(&mut self, token: &str) {
        self.streaming_text.push_str(token);
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    /// Add the welcome banner as scrollable content.
    pub fn add_banner(&mut self, model_name: &str) {
        let art = [
            "                                                     ",
            "    ░█████╗░██████╗░░█████╗░░█████╗░███╗░░██╗░█████╗░",
            "    ██╔══██╗██╔══██╗██╔══██╗██╔══██╗████╗░██║██╔══██╗",
            "    ███████║██████╔╝██║░░╚═╝███████║██╔██╗██║███████║",
            "    ██╔══██║██╔══██╗██║░░██╗██╔══██║██║╚████║██╔══██║",
            "    ██║░░██║██║░░██║╚█████╔╝██║░░██║██║░╚███║██║░░██║",
            "    ╚═╝░░╚═╝╚═╝░░╚═╝░╚════╝░╚═╝░░╚═╝╚═╝░░╚══╝╚═╝░░╚═╝",
        ];
        let mut banner = String::new();
        for line in &art {
            banner.push_str(line);
            banner.push('\n');
        }
        banner.push('\n');
        banner.push_str("    The Arcane Agent — Memory · Skills · Authority\n");
        banner.push('\n');
        banner.push_str(&format!(
            "    Model:      {:<20} Session:    new\n",
            model_name
        ));
        banner.push_str(&format!(
            "    Provider:   {:<20} Sub-agents: query + spawn\n",
            "deepseek"
        ));
        banner.push_str(&format!(
            "                                                     "
        ));
        banner.push_str(&format!(
            "                                                     "
        ));
        self.messages.push(Message {
            role: MessageRole::System,
            content: banner,
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
            separator: None,
        });
    }

    /// Start a thinking block.
    pub fn start_thinking(&mut self) {
        self.streaming_think = Some(StreamingThink {
            content: String::new(),
            token_count: 0,
            start_time: std::time::Instant::now(),
        });
    }

    /// Append a token to the current thinking block.
    pub fn append_think_token(&mut self, token: &str) {
        if let Some(ref mut think) = self.streaming_think {
            think.content.push_str(token);
            think.token_count += 1;
        }
    }

    /// End the current thinking block (collapse it).
    pub fn end_thinking(&mut self) {
        // Thinking content stays in streaming_think until response finalizes
    }

    /// Finalize the current streaming response into a message.
    pub fn finalize_response(&mut self) {
        self.finalize_response_with_stats(None);
    }

    /// Finalize response and append usage stats line.
    pub fn finalize_response_with_stats(&mut self, stats: Option<crate::types::ResponseStats>) {
        self.finalize_response_with_options(stats, false);
    }

    /// Finalize a response without stats and create an agent message even when
    /// visible content was stripped, so tool-call panels have a stable anchor.
    pub fn finalize_response_for_tool_calls(&mut self) {
        self.finalize_response_with_options(None, true);
    }

    fn finalize_response_with_options(
        &mut self,
        stats: Option<crate::types::ResponseStats>,
        force_agent_message: bool,
    ) {
        if force_agent_message || !self.streaming_text.is_empty() || self.streaming_think.is_some()
        {
            let thinking = self.streaming_think.take().map(|t| ThinkingBlock {
                content: t.content.trim_end_matches('\n').to_string(),
                token_count: t.token_count,
                duration_ms: t.start_time.elapsed().as_millis() as u64,
                collapsed: true,
                index: 0,
            });
            let content = self.streaming_text.trim_end_matches('\n').to_string();
            self.streaming_text.clear();
            let msg = Message {
                role: MessageRole::Agent,
                content,
                timestamp: chrono::Utc::now(),
                thinking,
                tool_calls: Vec::new(),
                separator: None,
            };
            self.messages.push(msg);
        }
        if let Some(s) = stats {
            self.messages.push(Message {
                role: MessageRole::System,
                content: s.format_line(),
                timestamp: chrono::Utc::now(),
                thinking: None,
                tool_calls: Vec::new(),
                separator: None,
            });
        }
        self.is_streaming = false;
        // Don't force auto_scroll — user may be reading above
        // auto_scroll re-engages when user scrolls back to bottom
    }

    /// Toggle all thinking blocks expand/collapse (Ctrl+O).
    pub fn toggle_thinking(&mut self) {
        self.think_collapsed = !self.think_collapsed;
        for msg in &mut self.messages {
            if let Some(ref mut t) = msg.thinking {
                t.collapsed = self.think_collapsed;
            }
        }
        self.scroll_offset = 0;
    }

    /// Toggle all Shell tool-call panels expand/collapse (Ctrl+X).
    /// Non-Shell (authority request) panels are always compact and unaffected.
    pub fn toggle_tool_calls(&mut self) {
        self.tool_calls_collapsed = !self.tool_calls_collapsed;
        for msg in &mut self.messages {
            for tc in &mut msg.tool_calls {
                if tc.tool_type == ToolType::Shell {
                    tc.collapsed = self.tool_calls_collapsed;
                }
            }
        }
        self.scroll_offset = 0;
    }

    /// Attach a tool-call panel to the most recent agent response.
    pub fn add_tool_call(&mut self, mut tool_call: ToolCall) {
        // Only Shell tool calls respect the global collapse toggle.
        if tool_call.tool_type == ToolType::Shell {
            tool_call.collapsed = self.tool_calls_collapsed;
        }
        if let Some(msg) = self
            .messages
            .iter_mut()
            .rev()
            .find(|msg| msg.role == MessageRole::Agent)
        {
            msg.tool_calls.push(tool_call);
        }
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    /// Fill the latest pending tool-call panel with its result.
    pub fn finish_latest_tool_call(&mut self, result: String, duration_ms: u64) {
        if let Some(tc) = self
            .messages
            .iter_mut()
            .rev()
            .filter(|msg| msg.role == MessageRole::Agent)
            .flat_map(|msg| msg.tool_calls.iter_mut().rev())
            .find(|tc| tc.result.is_none())
        {
            tc.result = Some(result);
            tc.duration_ms = duration_ms;
        }
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }

    /// Toggle thinking for a specific dialogue (by user message index).
    pub fn toggle_thinking_at(&mut self, user_msg_idx: usize) {
        // Find the agent response following this user message
        if user_msg_idx + 1 < self.messages.len() {
            if let Some(ref mut t) = self.messages[user_msg_idx + 1].thinking {
                t.collapsed = !t.collapsed;
            }
        }
    }

    /// Get indices of all user messages (each represents a dialogue).
    pub fn dialogue_indices(&self) -> Vec<usize> {
        self.messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == MessageRole::User)
            .map(|(i, _)| i)
            .collect()
    }

    /// Scroll so that the dialogue at the given index is in the upper-middle area.
    pub fn scroll_to_dialogue(&mut self, _msg_idx: usize) {
        // We'll handle this in the render by computing line offsets
        // For now, disable auto_scroll so the focused view takes over
        self.auto_scroll = false;
    }

    /// Add a user message.
    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::User,
            content,
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
            separator: None,
        });
        self.auto_scroll = true;
        self.scroll_offset = 0;
    }

    /// Add an error message (displayed as system message).
    pub fn add_error_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::System,
            content,
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
            separator: None,
        });
        self.auto_scroll = true;
        self.scroll_offset = 0;
    }

    /// Add a horizontal separator line (full dialogue boundary).
    pub fn add_separator(&mut self) {
        self.messages.push(Message {
            role: MessageRole::System,
            content: "─".repeat(80),
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
            separator: Some(crate::types::SeparatorKind::Full),
        });
    }

    /// Add a sub-separator for within-dialogue breaks (dark gray).
    pub fn add_sub_separator(&mut self) {
        self.messages.push(Message {
            role: MessageRole::System,
            content: "─".repeat(80),
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
            separator: Some(crate::types::SeparatorKind::Partial),
        });
    }

    /// Scroll up by N lines.
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        self.auto_scroll = false;
    }

    /// Scroll down by N lines.
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        if self.scroll_offset == 0 {
            self.auto_scroll = true;
        }
    }

    /// Jump to bottom.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.auto_scroll = true;
    }

    /// Jump to top.
    pub fn scroll_to_top(&mut self, total_lines: usize) {
        self.scroll_offset = total_lines;
        self.auto_scroll = false;
    }

    /// Render the viewport into the given area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Build rendered lines from messages
        let mut lines: Vec<(usize, Line)> = Vec::new();

        for (msg_idx, msg) in self.messages.iter().enumerate() {
            match msg.role {
                MessageRole::User => {
                    let bg = Style::default().bg(theme.composer_bg);
                    let text_style = theme.composer_text;
                    let fill_w = inner.width as usize;
                    let content_lines: Vec<&str> = msg.content.split('\n').collect();
                    for (i, line_text) in content_lines.iter().enumerate() {
                        let prefix = if i == 0 { "❯ " } else { "  " };
                        let prefix_style = if i == 0 {
                            theme.prompt_glyph.bg(theme.composer_bg)
                        } else {
                            bg
                        };
                        let text = if line_text.is_empty() && i > 0 {
                            // empty continuation line → just background fill
                            let pad = fill_w.saturating_sub(2); // "  "
                            lines.push((
                                msg_idx,
                                Line::from(vec![
                                    Span::styled("  ", bg),
                                    Span::styled(" ".repeat(pad), bg),
                                ]),
                            ));
                            continue;
                        } else {
                            line_text.to_string()
                        };
                        let prefix_w = unicode_width::UnicodeWidthStr::width(prefix);
                        let text_w = unicode_width::UnicodeWidthStr::width(text.as_str());
                        let used_w = prefix_w + text_w;
                        let pad = fill_w.saturating_sub(used_w);
                        let mut spans = vec![
                            Span::styled(prefix, prefix_style),
                            Span::styled(text, text_style.bg(theme.composer_bg)),
                        ];
                        if pad > 0 {
                            spans.push(Span::styled(" ".repeat(pad), bg));
                        }
                        lines.push((msg_idx, Line::from(spans)));
                    }
                    // Full-width background separator line after user message
                    lines.push((
                        msg_idx,
                        Line::from(vec![Span::styled(" ".repeat(fill_w), bg)]),
                    ));
                }
                MessageRole::Agent => {
                    // Render thinking blocks (collapsed by default, Ctrl+O to expand)
                    if let Some(ref think) = msg.thinking {
                        if think.collapsed {
                            lines.push((
                                msg_idx,
                                Line::from(vec![
                                    Span::styled(
                                        format!(
                                            "▸ Thinking ({} tokens, {:.1}s) ",
                                            think.token_count,
                                            think.duration_ms as f64 / 1000.0
                                        ),
                                        theme.thinking_block,
                                    ),
                                    Span::styled(
                                        "ctrl+o to expand",
                                        Style::default().fg(Color::Rgb(160, 160, 170)),
                                    ),
                                ]),
                            ));
                            lines.push((msg_idx, Line::from("")));
                        } else {
                            lines.push((
                                msg_idx,
                                Line::from(vec![
                                    Span::styled(
                                        format!(
                                            "▾ Thinking ({} tokens, {:.1}s) ",
                                            think.token_count,
                                            think.duration_ms as f64 / 1000.0
                                        ),
                                        theme.thinking_block,
                                    ),
                                    Span::styled(
                                        "ctrl+o to collapse",
                                        Style::default().fg(Color::Rgb(160, 160, 170)),
                                    ),
                                ]),
                            ));
                            for md_line in crate::render_md::render_markdown(
                                &think.content,
                                theme.thinking_block,
                            ) {
                                // Indent thinking content
                                let mut spans = vec![Span::raw("  ".to_string())];
                                spans.extend(md_line.spans);
                                lines.push((msg_idx, Line::from(spans)));
                            }
                            lines.push((msg_idx, Line::from("")));
                        }
                    }

                    // Render tool-call panels
                    for tc in &msg.tool_calls {
                        let (heading, heading_color) = if tc.tool_type == ToolType::Shell {
                            ("[Arcana Run]: ", PIGMENT_GREEN)
                        } else {
                            ("[Arcana Request]: ", PIGMENT_GREEN)
                        };

                        // ── Shell: full panel with inline command, timing, result ──
                        if tc.tool_type == ToolType::Shell {
                            let mut first_spans: Vec<Span> = vec![
                                Span::styled(
                                    heading,
                                    Style::default()
                                        .fg(heading_color)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    shell_tool_label(tc),
                                    Style::default().fg(heading_color),
                                ),
                                Span::styled("`", Style::default().fg(heading_color)),
                            ];

                            let cmd_all_lines: Vec<&str> = tc.description.lines().collect();
                            let cmd_first_line = cmd_all_lines.first().copied().unwrap_or("");
                            push_shell_highlighted(&mut first_spans, cmd_first_line);
                            if cmd_all_lines.len() > 1 {
                                first_spans.push(Span::styled(
                                    " ...` ",
                                    Style::default().fg(heading_color),
                                ));
                            } else {
                                first_spans
                                    .push(Span::styled("` ", Style::default().fg(heading_color)));
                            }

                            let hint = if tc.collapsed {
                                "ctrl+x to expand"
                            } else {
                                "ctrl+x to fold"
                            };
                            first_spans.push(Span::styled(hint, Style::default().fg(TOOL_HINT)));

                            lines.push((msg_idx, Line::from(first_spans)));

                            // Collapsed: just the first line, nothing below
                            if tc.collapsed {
                                continue;
                            }

                            if cmd_all_lines.len() > 1 {
                                for cmd_line in &cmd_all_lines[1..] {
                                    let hl = crate::highlight::highlight_lines(cmd_line, "bash");
                                    for spans in &hl {
                                        let mut line_spans = vec![Span::raw("  ")];
                                        for span in spans {
                                            line_spans.push(Span::styled(
                                                span.text.clone(),
                                                Style::default().fg(span.fg),
                                            ));
                                        }
                                        lines.push((msg_idx, Line::from(line_spans)));
                                    }
                                }
                            }

                            if let Some(result) = &tc.result {
                                if !result.is_empty() {
                                    lines.push((msg_idx, Line::from("")));
                                }
                                // Split result into pre-diff content and diff section
                                if let Some(diff_start) = result.find("diff --git") {
                                    let pre = result[..diff_start].trim();
                                    let diff = result[diff_start..].trim();
                                    // Render pre-diff content (stdout/stderr)
                                    for line in pre.lines() {
                                        lines.push((
                                            msg_idx,
                                            Line::from(vec![
                                                Span::raw("  "),
                                                Span::styled(
                                                    line.to_string(),
                                                    Style::default().fg(TOOL_OUTPUT),
                                                ),
                                            ]),
                                        ));
                                    }
                                    if !pre.is_empty() && !diff.is_empty() {
                                        lines.push((msg_idx, Line::from("")));
                                    }
                                    // Render styled diff
                                    let file_path = &tc.description;
                                    for styled_line in render_styled_diff(diff, file_path, inner.width.saturating_sub(2)) {
                                        let mut spans = vec![Span::raw("  ")];
                                        spans.extend(styled_line.spans);
                                        lines.push((msg_idx, Line::from(spans)));
                                    }
                                } else {
                                    for line in result.lines() {
                                        lines.push((
                                            msg_idx,
                                            Line::from(vec![
                                                Span::raw("  "),
                                                Span::styled(
                                                    line.to_string(),
                                                    Style::default().fg(TOOL_OUTPUT),
                                                ),
                                            ]),
                                        ));
                                    }
                                }
                            }
                            lines.push((msg_idx, Line::from("")));
                            continue;
                        }

                        // ── Non-Shell: compact single line with status appended ──
                        let action = tc.action.as_deref().unwrap_or(match tc.tool_type {
                            ToolType::File => "File",
                            ToolType::Web => "Web",
                            ToolType::Search => "Search",
                            _ => "Request",
                        });
                        let object = match tc.tool_type {
                            ToolType::File => "File",
                            ToolType::Web => "Web",
                            ToolType::Search => "Search",
                            _ => "Authority",
                        };
                        let suffix = match tc.tool_type {
                            ToolType::Search => " for ",
                            _ => " Access to ",
                        };
                        let status = tc.result.as_deref().and_then(compact_request_status);
                        let mut request_spans = vec![
                            Span::styled(
                                heading,
                                Style::default()
                                    .fg(heading_color)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(format!("{object} "), Style::default().fg(heading_color)),
                            Span::styled(
                                action.to_string(),
                                Style::default()
                                    .fg(AMBER_SAE_ECE)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(suffix, Style::default().fg(heading_color)),
                            Span::styled("`", Style::default().fg(heading_color)),
                            Span::styled(
                                tc.description.clone(),
                                Style::default().fg(heading_color),
                            ),
                            Span::styled("`.", Style::default().fg(heading_color)),
                        ];
                        if let Some(status) = status {
                            request_spans.push(Span::raw(" "));
                            request_spans.push(Span::styled(
                                status,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                        lines.push((msg_idx, Line::from(request_spans)));
                        if let Some(result) = &tc.result {
                            if let Some(extra) = expanded_request_result(result) {
                                if let Some(diff_start) = extra.find("diff --git") {
                                    let pre = extra[..diff_start].trim();
                                    let diff = extra[diff_start..].trim();
                                    for line in pre.lines() {
                                        lines.push((
                                            msg_idx,
                                            Line::from(vec![
                                                Span::raw("  "),
                                                Span::styled(
                                                    line.to_string(),
                                                    Style::default().fg(TOOL_OUTPUT),
                                                ),
                                            ]),
                                        ));
                                    }
                                    if !pre.is_empty() && !diff.is_empty() {
                                        lines.push((msg_idx, Line::from("")));
                                    }
                                    for styled_line in render_styled_diff(diff, &tc.description, inner.width.saturating_sub(2)) {
                                        let mut spans = vec![Span::raw("  ")];
                                        spans.extend(styled_line.spans);
                                        lines.push((msg_idx, Line::from(spans)));
                                    }
                                } else {
                                    for line in extra.lines() {
                                        lines.push((
                                            msg_idx,
                                            Line::from(vec![
                                                Span::raw("  "),
                                                Span::styled(
                                                    line.to_string(),
                                                    Style::default().fg(TOOL_OUTPUT),
                                                ),
                                            ]),
                                        ));
                                    }
                                }
                                lines.push((msg_idx, Line::from("")));
                            }
                        }
                    }

                    // Render response content with markdown formatting
                    for md_line in
                        crate::render_md::render_markdown(&msg.content, theme.agent_response)
                    {
                        lines.push((msg_idx, md_line));
                    }
                    lines.push((msg_idx, Line::from("")));
                }
                MessageRole::System => {
                    // Check for separator first (new typed field)
                    if let Some(sep_kind) = msg.separator {
                        let sep_str = "─".repeat(inner.width as usize);
                        let color = match sep_kind {
                            crate::types::SeparatorKind::Full => Color::White,
                            crate::types::SeparatorKind::Partial => Color::Rgb(80, 80, 90),
                        };
                        lines.push((
                            msg_idx,
                            Line::from(Span::styled(sep_str, Style::default().fg(color))),
                        ));
                        continue;
                    }

                    let is_cost = msg.content.starts_with("Cost:");
                    let is_banner = msg.content.contains("█████") || msg.content.contains("╔══");
                    if is_cost {
                        lines.push((msg_idx, Line::from("")));
                    }
                    if is_banner {
                        // Render banner with gradient colors
                        let content_lines: Vec<&str> = msg.content.lines().collect();
                        let art_lines = content_lines
                            .iter()
                            .take_while(|l| {
                                let t = l.trim_start();
                                t.is_empty()
                                    || t.starts_with('░')
                                    || t.starts_with('█')
                                    || t.starts_with('╚')
                            })
                            .count();
                        for (i, line) in content_lines.iter().enumerate() {
                            if line.is_empty() {
                                lines.push((msg_idx, Line::from("")));
                            } else if i < art_lines {
                                let t = i as f32 / art_lines.max(1) as f32;
                                let color = interpolate_color(
                                    theme.banner_gradient.0,
                                    theme.banner_gradient.1,
                                    t,
                                );
                                lines.push((
                                    msg_idx,
                                    Line::from(Span::styled(
                                        line.to_string(),
                                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                                    )),
                                ));
                            } else {
                                lines.push((
                                    msg_idx,
                                    Line::from(Span::styled(line.to_string(), theme.dim)),
                                ));
                            }
                        }
                    } else {
                        let style = if is_cost {
                            theme.thinking_block
                        } else {
                            Style::default().fg(Color::White)
                        };
                        for line in msg.content.lines() {
                            lines
                                .push((msg_idx, Line::from(Span::styled(line.to_string(), style))));
                        }
                    }
                }
            }
        }

        // Render streaming content
        let stream_idx = self.messages.len();
        if self.is_streaming {
            if let Some(ref think) = self.streaming_think {
                let elapsed = think.start_time.elapsed().as_secs_f64();
                if self.think_collapsed {
                    let header = format!(
                        "▸ Thinking ({} tokens, {:.1}s) ",
                        think.token_count, elapsed
                    );
                    lines.push((
                        stream_idx,
                        Line::from(vec![
                            Span::styled(header, theme.thinking_block),
                            Span::styled(
                                "ctrl+o to expand",
                                Style::default().fg(Color::Rgb(160, 160, 170)),
                            ),
                        ]),
                    ));
                } else {
                    let header = format!(
                        "▾ Thinking ({} tokens, {:.1}s) ",
                        think.token_count, elapsed
                    );
                    lines.push((
                        stream_idx,
                        Line::from(vec![
                            Span::styled(header, theme.thinking_block),
                            Span::styled(
                                "ctrl+o to collapse",
                                Style::default().fg(Color::Rgb(160, 160, 170)),
                            ),
                        ]),
                    ));

                    for md_line in
                        crate::render_md::render_markdown(&think.content, theme.thinking_block)
                    {
                        let mut spans = vec![Span::raw("  ".to_string())];
                        spans.extend(md_line.spans);
                        lines.push((stream_idx, Line::from(spans)));
                    }
                }
            }

            if !self.streaming_text.is_empty() {
                for md_line in
                    crate::render_md::render_markdown(&self.streaming_text, theme.agent_response)
                {
                    lines.push((stream_idx, md_line));
                }
            }
        }

        // Wrap lines to viewport width so each entry = one visual row
        let panel_width = inner.width as usize;
        let mut wrapped: Vec<(usize, Line)> = Vec::new();
        for (idx, line) in lines {
            let line_w: usize = line
                .spans
                .iter()
                .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if line_w <= panel_width || panel_width == 0 {
                wrapped.push((idx, line));
            } else {
                let full_text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
                let style = line.spans.first().map(|s| s.style).unwrap_or_default();
                let mut remaining = full_text.as_str();
                while !remaining.is_empty() {
                    let mut byte_end = 0;
                    let mut w = 0;
                    for (i, ch) in remaining.char_indices() {
                        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                        if w + cw > panel_width {
                            break;
                        }
                        w += cw;
                        byte_end = i + ch.len_utf8();
                    }
                    if byte_end == 0 && !remaining.is_empty() {
                        byte_end = remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    }
                    wrapped.push((
                        idx,
                        Line::from(Span::styled(remaining[..byte_end].to_string(), style)),
                    ));
                    remaining = &remaining[byte_end..];
                }
            }
        }
        let lines = wrapped;

        // --- Auto-scroll algorithm ---
        // 1. Determine cursor position (line index in `lines`)
        let cursor_line = if self.is_streaming {
            if !self.think_collapsed {
                // Thinking expanded
                if self.streaming_text.is_empty() {
                    // Still thinking, cursor = end of thinking content
                    lines.len().saturating_sub(1)
                } else {
                    // Outputting or finished — cursor stays at end of thinking panel
                    // Find where thinking ends (before streaming text starts)
                    lines.len().saturating_sub(1) // simplified: last line is latest content
                }
            } else {
                // Thinking collapsed
                if self.streaming_text.is_empty() && self.streaming_think.is_some() {
                    // Thinking with no output yet — cursor at head of output area
                    // (the collapsed thinking header line)
                    lines.len().saturating_sub(1)
                } else {
                    // Outputting — cursor at end of streaming text
                    lines.len().saturating_sub(1)
                }
            }
        } else {
            // Finished — cursor at very end (including Cost/Time)
            lines.len().saturating_sub(1)
        };

        // 2. Compute threshold and start_line
        let total_lines = lines.len();
        let visible_height = inner.height as usize;
        let threshold = (visible_height * 20 / 100).max(5); // lines from bottom
        let max_visible_cursor_pos = visible_height.saturating_sub(threshold); // max row for cursor

        if !self.auto_scroll && self.last_total_lines > 0 && total_lines > self.last_total_lines {
            self.scroll_offset = self
                .scroll_offset
                .saturating_add(total_lines - self.last_total_lines);
        }
        self.last_total_lines = total_lines;

        let max_scroll = total_lines.saturating_sub(visible_height);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }

        let start_line = if self.auto_scroll {
            // If cursor would be below threshold, scroll up until it's at threshold
            if cursor_line >= max_visible_cursor_pos {
                cursor_line - max_visible_cursor_pos
            } else {
                0
            }
        } else {
            max_scroll.saturating_sub(self.scroll_offset)
        };

        let visible_lines: Vec<Line> = lines
            .into_iter()
            .skip(start_line)
            .take(visible_height)
            .map(|(_, line)| line)
            .collect();

        let paragraph = Paragraph::new(visible_lines);
        frame.render_widget(paragraph, inner);
    }
}

fn shell_tool_label(tc: &ToolCall) -> String {
    if tc.result.is_some() {
        format!(
            "Shell (finished in {:.1}s): ",
            tc.duration_ms as f64 / 1000.0
        )
    } else {
        "Shell: ".to_string()
    }
}

fn push_shell_highlighted(spans: &mut Vec<Span>, command: &str) {
    let highlighted = crate::highlight::highlight_lines(command, "bash");
    if let Some(line) = highlighted.first() {
        for span in line {
            spans.push(Span::styled(
                span.text.clone(),
                Style::default().fg(span.fg),
            ));
        }
    } else {
        spans.push(Span::styled(command.to_string(), Style::default()));
    }
}

fn compact_request_status(result: &str) -> Option<String> {
    let trimmed = result.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return match json.get("status").and_then(|status| status.as_str()) {
            Some("ok") | Some("mutation") | Some("fetched") | Some("content") | Some("text") => {
                Some("OK".to_string())
            }
            Some("denied") => Some(format!(
                "Denied: {}",
                json.get("reason")
                    .and_then(|reason| reason.as_str())
                    .unwrap_or("unknown reason")
            )),
            Some("aborted") => Some(format!(
                "Aborted: {}",
                json.get("message")
                    .and_then(|message| message.as_str())
                    .unwrap_or("")
            )),
            _ => None,
        };
    }

    if trimmed == "ok"
        || trimmed.starts_with("recorded #")
        || trimmed.starts_with("fetched:")
        || trimmed.starts_with("content returned:")
        || trimmed.starts_with("text returned:")
    {
        return Some("OK".to_string());
    }
    if let Some(reason) = trimmed.strip_prefix("denied:") {
        return Some(format!("Denied:{}", reason));
    }
    if let Some(reason) = trimmed.strip_prefix("aborted:") {
        return Some(format!("Aborted:{}", reason));
    }
    trimmed.lines().next().map(str::to_string)
}

fn expanded_request_result(result: &str) -> Option<&str> {
    let trimmed = result.trim();
    if trimmed.is_empty()
        || trimmed == "ok"
        || trimmed.starts_with("denied:")
        || trimmed.starts_with("aborted:")
        || trimmed.starts_with("fetched:")
        || trimmed.starts_with("content returned:")
        || trimmed.starts_with("text returned:")
        || serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return None;
    }
    if trimmed.contains('\n') {
        Some(trimmed)
    } else {
        None
    }
}

/// Render a unified git diff as styled lines with line numbers, tree-sitter
/// highlighting, and background colors. Strips git metadata headers.
pub fn render_styled_diff<'a>(diff_text: &str, file_path: &str, panel_width: u16) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut old_line: u32 = 0;
    let mut new_line: u32 = 0;

    // Detect language from file path for tree-sitter highlighting
    let lang = crate::highlight::detect_language(file_path).unwrap_or("");

    // Collect non-header lines and their kind for highlighting
    let mut code_lines: Vec<(DiffLineKind, String)> = Vec::new();

    for line in diff_text.lines() {
        let trimmed = line.trim();

        // Skip git metadata headers
        if trimmed.is_empty()
            || trimmed.starts_with("diff --git")
            || trimmed.starts_with("index ")
            || trimmed.starts_with("--- ")
            || trimmed.starts_with("+++ ")
        {
            continue;
        }

        // Parse @@ hunk header for line numbers
        if trimmed.starts_with("@@") {
            if let Some((old, new)) = parse_hunk_header(trimmed) {
                old_line = old;
                new_line = new;
            }
            continue;
        }

        let (kind, prefix, ln) = if line.starts_with('+') {
            (DiffLineKind::Added, "+", Some(new_line))
        } else if line.starts_with('-') {
            (DiffLineKind::Removed, "-", Some(old_line))
        } else {
            (DiffLineKind::Context, " ", Some(new_line))
        };

        let content = if line.len() > 1 {
            line[1..].to_string()
        } else {
            String::new()
        };

        // Track line numbers
        match kind {
            DiffLineKind::Added => new_line = new_line.saturating_add(1),
            DiffLineKind::Removed => old_line = old_line.saturating_add(1),
            DiffLineKind::Context => {
                old_line = old_line.saturating_add(1);
                new_line = new_line.saturating_add(1);
            }
            _ => {}
        }

        code_lines.push((kind, content.clone()));

        let bg = match kind {
            DiffLineKind::Added => DIFF_ADDED_BG,
            DiffLineKind::Removed => DIFF_REMOVED_BG,
            _ => Color::Reset,
        };
        let fg = match kind {
            DiffLineKind::Added => Color::Rgb(0, 200, 100),
            DiffLineKind::Removed => Color::Rgb(255, 80, 80),
            _ => Color::Rgb(180, 180, 190),
        };

        let ln_str = ln
            .map(|n| format!("{:>4} ", n))
            .unwrap_or_else(|| "     ".to_string());

        lines.push(Line::from(vec![
            Span::styled(
                ln_str,
                Style::default().fg(Color::Rgb(100, 100, 110)).bg(bg),
            ),
            Span::styled(
                format!("{}", prefix),
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                content,
                Style::default().fg(Color::Rgb(210, 210, 220)).bg(bg),
            ),
        ]));
    }

    // Apply tree-sitter highlighting if we have a known language and content
    if !lang.is_empty() && !code_lines.is_empty() {
        let source: String = code_lines
            .iter()
            .map(|(_, content)| format!("{}\n", content))
            .collect();
        let highlighted = crate::highlight::highlight_lines(&source, lang);

        for (i, (kind, _)) in code_lines.iter().enumerate() {
            if i >= lines.len() {
                break;
            }
            let bg = match kind {
                DiffLineKind::Added => DIFF_ADDED_BG,
                DiffLineKind::Removed => DIFF_REMOVED_BG,
                _ => Color::Reset,
            };
            if let Some(hl_spans) = highlighted.get(i) {
                if !hl_spans.is_empty() {
                    // Keep prefix (line number + +/-), replace content spans
                    let prefix_spans: Vec<Span> = lines[i]
                        .spans
                        .iter()
                        .take(2) // line number + prefix
                        .cloned()
                        .collect();
                    let mut new_spans = prefix_spans;
                    for s in hl_spans {
                        new_spans.push(Span::styled(
                            s.text.clone(),
                            Style::default().fg(s.fg).bg(bg),
                        ));
                    }
                    lines[i] = Line::from(new_spans);
                }
            }
        }
    }

    // Pad each line to fill panel_width with the appropriate background
    let fill_w = panel_width as usize;
    for line in &mut lines {
        let used_w: usize = line.spans.iter()
            .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        let pad = fill_w.saturating_sub(used_w);
        if pad > 0 {
            let last_bg = line.spans.last()
                .map(|s| s.style.bg)
                .flatten()
                .unwrap_or(Color::Reset);
            line.spans.push(Span::styled(" ".repeat(pad), Style::default().bg(last_bg)));
        }
    }

    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffLineKind {
    Added,
    Removed,
    Context,
}

fn parse_hunk_header(header: &str) -> Option<(u32, u32)> {
    // "@@ -old_start,old_count +new_start,new_count @@"
    let inner = header.strip_prefix("@@")?.strip_suffix("@@")?.trim();
    let mut parts = inner.split_whitespace();
    let old_part = parts.next()?; // -old_start,old_count
    let new_part = parts.next()?; // +new_start,new_count
    let old_start = old_part
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse::<u32>()
        .ok()?;
    let new_start = new_part
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse::<u32>()
        .ok()?;
    Some((old_start, new_start))
}
fn interpolate_color(from: Color, to: Color, t: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let r = (r1 as f32 + (r2 as f32 - r1 as f32) * t) as u8;
            let g = (g1 as f32 + (g2 as f32 - g1 as f32) * t) as u8;
            let b = (b1 as f32 + (b2 as f32 - b1 as f32) * t) as u8;
            Color::Rgb(r, g, b)
        }
        _ => from,
    }
}
