use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::Theme;
use crate::types::{Message, MessageRole, ThinkingBlock};

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
        banner.push_str(&format!("    Model:      {:<20} Session:    new\n", model_name));
        banner.push_str(&format!("    Provider:   {:<20} Sub-agents: query + spawn\n", "deepseek"));
        banner.push_str(&format!("                                                     "));
        banner.push_str(&format!("                                                     "));
        self.messages.push(Message {
            role: MessageRole::System,
            content: banner,
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
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
        if !self.streaming_text.is_empty() || self.streaming_think.is_some() {
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
    pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Build rendered lines from messages
        let mut lines: Vec<(usize, Line)> = Vec::new();

        for (msg_idx, msg) in self.messages.iter().enumerate() {

            match msg.role {
                MessageRole::User => {
                    let content_lines: Vec<&str> = msg.content.split('\n').collect();
                    for (i, line_text) in content_lines.iter().enumerate() {
                        if i == 0 {
                            lines.push((msg_idx, Line::from(vec![
                                Span::styled("❯ ", theme.prompt_glyph),
                                Span::styled(line_text.to_string(), theme.user_message),
                            ])));
                        } else if line_text.is_empty() {
                            lines.push((msg_idx, Line::from("")));
                        } else {
                            lines.push((msg_idx, Line::from(vec![
                                Span::raw("  "),
                                Span::styled(line_text.to_string(), theme.user_message),
                            ])));
                        }
                    }
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
                            for md_line in crate::render_md::render_markdown(&think.content, theme.thinking_block) {
                                // Indent thinking content
                                let mut spans = vec![Span::raw("  ".to_string())];
                                spans.extend(md_line.spans);
                                lines.push((msg_idx, Line::from(spans)));
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

                    // Render response content with markdown formatting
                    for md_line in crate::render_md::render_markdown(&msg.content, theme.agent_response) {
                        lines.push((msg_idx, md_line));
                    }
                    lines.push((msg_idx, Line::from("")));
                }
                MessageRole::System => {
                    let is_cost = msg.content.starts_with("Cost:");
                    let is_sep = msg.content.starts_with('─');
                    let is_banner = msg.content.contains("█████") || msg.content.contains("╔══");
                    if is_cost {
                        lines.push((msg_idx, Line::from("")));
                    }
                    if is_sep {
                        let sep_str = "─".repeat(inner.width as usize);
                        lines.push((msg_idx, Line::from(Span::styled(
                            sep_str, Style::default().fg(Color::White)
                        ))));
                    } else if is_banner {
                        // Render banner with gradient colors
                        let content_lines: Vec<&str> = msg.content.lines().collect();
                        let art_lines = content_lines.iter().take_while(|l| {
                            let t = l.trim_start();
                            t.is_empty() || t.starts_with('░') || t.starts_with('█') || t.starts_with('╚')
                        }).count();
                        for (i, line) in content_lines.iter().enumerate() {
                            if line.is_empty() {
                                lines.push((msg_idx, Line::from("")));
                            } else if i < art_lines {
                                let t = i as f32 / art_lines.max(1) as f32;
                                let color = interpolate_color(theme.banner_gradient.0, theme.banner_gradient.1, t);
                                lines.push((msg_idx, Line::from(Span::styled(
                                    line.to_string(),
                                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                                ))));
                            } else {
                                lines.push((msg_idx, Line::from(Span::styled(
                                    line.to_string(), theme.dim,
                                ))));
                            }
                        }
                    } else {
                        let style = if is_cost {
                            theme.thinking_block
                        } else {
                            Style::default().fg(Color::White)
                        };
                        for line in msg.content.lines() {
                            lines.push((msg_idx, Line::from(Span::styled(line.to_string(), style))));
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
                    let header = format!("▸ Thinking ({} tokens, {:.1}s) ",
                        think.token_count, elapsed);
                    lines.push((stream_idx, Line::from(vec![
                        Span::styled(header, theme.thinking_block),
                        Span::styled("ctrl+o to expand", Style::default().fg(Color::Rgb(160, 160, 170))),
                    ])));
                } else {
                    let header = format!("▾ Thinking ({} tokens, {:.1}s) ",
                        think.token_count, elapsed);
                    lines.push((stream_idx, Line::from(vec![
                        Span::styled(header, theme.thinking_block),
                        Span::styled("ctrl+o to collapse", Style::default().fg(Color::Rgb(160, 160, 170))),
                    ])));

                    for md_line in crate::render_md::render_markdown(&think.content, theme.thinking_block) {
                        let mut spans = vec![Span::raw("  ".to_string())];
                        spans.extend(md_line.spans);
                        lines.push((stream_idx, Line::from(spans)));
                    }
                }
            }

            if !self.streaming_text.is_empty() {
                for md_line in crate::render_md::render_markdown(&self.streaming_text, theme.agent_response) {
                    lines.push((stream_idx, md_line));
                }
            }
        }

        // Wrap lines to viewport width so each entry = one visual row
        let panel_width = inner.width as usize;
        let mut wrapped: Vec<(usize, Line)> = Vec::new();
        for (idx, line) in lines {
            let line_w: usize = line.spans.iter()
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
                        if w + cw > panel_width { break; }
                        w += cw;
                        byte_end = i + ch.len_utf8();
                    }
                    if byte_end == 0 && !remaining.is_empty() {
                        byte_end = remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    }
                    wrapped.push((idx, Line::from(Span::styled(remaining[..byte_end].to_string(), style))));
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

/// Linearly interpolate between two RGB colors.
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
