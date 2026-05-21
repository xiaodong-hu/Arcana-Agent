use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::{Message, MessageRole, ThinkingBlock};

/// Bleu de France color for inline code highlighting
const BLEU_DE_FRANCE: Color = Color::Rgb(49, 140, 231); // #318CE7

/// Parse a line of text, highlighting `backtick content` in Bleu de France.
fn styled_line<'a>(text: &str, base_style: Style) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut rest = text;

    while let Some(start) = rest.find('`') {
        // Text before the backtick
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), base_style));
        }
        let after_tick = &rest[start + 1..];
        if let Some(end) = after_tick.find('`') {
            // Found closing backtick
            let code = &after_tick[..end];
            spans.push(Span::styled(
                format!("`{}`", code),
                Style::default().fg(BLEU_DE_FRANCE),
            ));
            rest = &after_tick[end + 1..];
        } else {
            // No closing backtick, render rest as-is
            spans.push(Span::styled(rest[start..].to_string(), base_style));
            rest = "";
            break;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }
    Line::from(spans)
}

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
        if !self.streaming_text.is_empty() || self.streaming_think.is_some() {
            let thinking = self.streaming_think.take().map(|t| ThinkingBlock {
                content: t.content,
                token_count: t.token_count,
                duration_ms: t.start_time.elapsed().as_millis() as u64,
                collapsed: true, // collapsed by default
                index: 0,
            });
            let msg = Message {
                role: MessageRole::Agent,
                content: std::mem::take(&mut self.streaming_text),
                timestamp: chrono::Utc::now(),
                thinking,
                tool_calls: Vec::new(),
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
            });
        }
        self.is_streaming = false;
    }

    /// Toggle all thinking blocks expand/collapse (Ctrl+O).
    pub fn toggle_thinking(&mut self) {
        let any_expanded = self.messages.iter().any(|m| {
            m.thinking.as_ref().is_some_and(|t| !t.collapsed)
        });
        for msg in &mut self.messages {
            if let Some(ref mut t) = msg.thinking {
                t.collapsed = any_expanded;
            }
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
        self.messages.iter().enumerate()
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
        });
        self.auto_scroll = true;
        self.scroll_offset = 0;
    }

    /// Add a horizontal separator line.
    pub fn add_separator(&mut self) {
        self.messages.push(Message {
            role: MessageRole::System,
            content: "─".repeat(80),
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
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
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme, focused_dialogue: Option<usize>) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Pigment green for focus bar
        const FOCUS_GREEN: Color = Color::Rgb(0, 166, 79);

        // Build rendered lines from messages, tracking which msg_idx each line belongs to
        let mut lines: Vec<(usize, Line)> = Vec::new();
        let mut focused_start_line: Option<usize> = None;

        for (msg_idx, msg) in self.messages.iter().enumerate() {
            // Track where focused dialogue starts
            if focused_dialogue == Some(msg_idx) {
                focused_start_line = Some(lines.len());
            }

            let is_focused = focused_dialogue.is_some_and(|f| {
                msg_idx == f || (msg_idx > f && msg_idx <= f + 2
                    && self.messages.get(f).is_some_and(|m| m.role == MessageRole::User))
            });

            match msg.role {
                MessageRole::User => {
                    let mut spans = vec![
                        Span::styled("❯ ", theme.prompt_glyph),
                        Span::styled(&msg.content, theme.user_message),
                    ];
                    if is_focused {
                        spans.insert(0, Span::styled("┃ ", Style::default().fg(FOCUS_GREEN)));
                    }
                    lines.push((msg_idx, Line::from(spans)));
                    lines.push((msg_idx, Line::from("")));
                }
                MessageRole::Agent => {
                    // Render thinking blocks (collapsed by default, Ctrl+O to expand)
                    if let Some(ref think) = msg.thinking {
                        if think.collapsed {
                            lines.push((msg_idx, Line::from(vec![
                                Span::styled(
                                    format!("▸ Thinking ({} tokens, {:.1}s) ",
                                        think.token_count, think.duration_ms as f64 / 1000.0),
                                    theme.thinking_block,
                                ),
                                Span::styled("ctrl+o to expand", Style::default().fg(Color::Rgb(160, 160, 170))),
                            ])));
                            lines.push((msg_idx, Line::from("")));
                        } else {
                            lines.push((msg_idx, Line::from(vec![
                                Span::styled(
                                    format!("▾ Thinking ({} tokens, {:.1}s) ",
                                        think.token_count, think.duration_ms as f64 / 1000.0),
                                    theme.thinking_block,
                                ),
                                Span::styled("ctrl+o to collapse", Style::default().fg(Color::Rgb(160, 160, 170))),
                            ])));
                            for line in think.content.lines() {
                                lines.push((msg_idx, styled_line(
                                    &format!("  {}", line),
                                    theme.thinking_block,
                                )));
                            }
                            lines.push((msg_idx, Line::from("")));
                        }
                    }

                    // Render tool calls
                    for tc in &msg.tool_calls {
                        let icon = tc.tool_type.icon();
                        let desc = format!("  {} {} ({:.1}s)", icon, tc.description, tc.duration_ms as f64 / 1000.0);
                        lines.push((msg_idx, Line::from(Span::styled(desc, theme.tool_call))));
                    }

                    // Render response content
                    for line in msg.content.lines() {
                        lines.push((msg_idx, styled_line(line, theme.agent_response)));
                    }
                    lines.push((msg_idx, Line::from("")));
                }
                MessageRole::System => {
                    let style = if msg.content.starts_with("Cost:") || msg.content.starts_with('─') {
                        theme.thinking_block
                    } else {
                        Style::default().fg(Color::White)
                    };
                    if msg.content.starts_with("Cost:") {
                        lines.push((msg_idx, Line::from("")));
                    }
                    for line in msg.content.lines() {
                        lines.push((msg_idx, Line::from(Span::styled(line.to_string(), style))));
                    }
                    if !msg.content.starts_with('─') {
                        lines.push((msg_idx, Line::from("")));
                    }
                }
            }
        }

        // Render streaming content
        let stream_idx = self.messages.len();
        if self.is_streaming {
            if let Some(ref think) = self.streaming_think {
                let header = format!("▾ Thinking ({}  tokens…)", think.token_count);
                lines.push((stream_idx, Line::from(Span::styled(header, theme.thinking_block))));

                let think_lines: Vec<&str> = think.content.lines().collect();
                let visible_count = (inner.height as usize / 3).max(3);
                let start = think_lines.len().saturating_sub(visible_count);
                for line in &think_lines[start..] {
                    lines.push((stream_idx, Line::from(Span::styled(
                        format!("  {}", line),
                        theme.thinking_block,
                    ))));
                }
            }

            if !self.streaming_text.is_empty() {
                for line in self.streaming_text.lines() {
                    lines.push((stream_idx, styled_line(line, theme.agent_response)));
                }
            }
        }

        // Apply scroll offset — if focused, scroll to keep focused dialogue in upper third
        let total_lines = lines.len();
        let visible_height = inner.height as usize;

        let start_line = if let Some(focus_line) = focused_start_line {
            if !self.auto_scroll {
                // Place focused dialogue at ~1/3 from top
                let target = focus_line.saturating_sub(visible_height / 3);
                target.min(total_lines.saturating_sub(visible_height))
            } else {
                total_lines.saturating_sub(visible_height)
            }
        } else if self.auto_scroll || self.scroll_offset == 0 {
            total_lines.saturating_sub(visible_height)
        } else {
            total_lines.saturating_sub(visible_height + self.scroll_offset)
        };

        let visible_lines: Vec<Line> = lines
            .into_iter()
            .skip(start_line)
            .take(visible_height)
            .map(|(_, line)| line)
            .collect();

        let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }
}
