use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::composer::Composer;
use crate::theme::Theme;
use crate::types::{Message, MessageRole, ThinkingBlock};

/// Bleu de France for inline code
const BLEU_DE_FRANCE: Color = Color::Rgb(49, 140, 231);

/// The query sub-agent overlay panel.
#[derive(Debug)]
pub struct QueryOverlay {
    /// Whether the overlay is currently visible
    pub visible: bool,
    /// Overlay-local conversation
    pub messages: Vec<Message>,
    /// Overlay composer
    pub composer: Composer,
    /// Scroll offset within the overlay
    pub scroll_offset: usize,
    /// Whether auto-scroll is engaged
    pub auto_scroll: bool,
    /// Whether currently streaming a response
    pub is_streaming: bool,
    /// Streaming text buffer
    pub streaming_text: String,
    /// Streaming thinking buffer
    pub streaming_think: Option<StreamingThink>,
    /// Whether thinking blocks are expanded
    pub thinking_expanded: bool,
    /// Last rendered visual line count, used to keep manual-scroll views stable as content grows.
    last_total_lines: usize,
}

#[derive(Debug)]
pub struct StreamingThink {
    pub content: String,
    pub token_count: usize,
    pub start_time: std::time::Instant,
}

impl Default for QueryOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            messages: Vec::new(),
            composer: {
                let mut c = Composer::new();
                c.overlay_mode = true; // disables command hints
                c
            },
            scroll_offset: 0,
            auto_scroll: true,
            is_streaming: false,
            streaming_text: String::new(),
            streaming_think: None,
            thinking_expanded: false,
            last_total_lines: 0,
        }
    }
}

impl QueryOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(&mut self) {
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.composer.clear();
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(n);
        self.auto_scroll = false;
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        if self.scroll_offset == 0 {
            self.auto_scroll = true;
        }
    }

    pub fn toggle_thinking(&mut self) {
        self.thinking_expanded = !self.thinking_expanded;
        for msg in &mut self.messages {
            if let Some(ref mut t) = msg.thinking {
                t.collapsed = !self.thinking_expanded;
            }
        }
    }

    pub fn append_think_token(&mut self, token: &str) {
        if let Some(ref mut think) = self.streaming_think {
            think.content.push_str(token);
            think.token_count += 1;
        }
    }

    pub fn start_thinking(&mut self) {
        self.streaming_think = Some(StreamingThink {
            content: String::new(),
            token_count: 0,
            start_time: std::time::Instant::now(),
        });
    }

    pub fn end_thinking(&mut self) {
        // Keep streaming_think until finalize
    }

    pub fn append_token(&mut self, token: &str) {
        self.streaming_text.push_str(token);
    }

    pub fn finalize_response(&mut self) {
        if !self.streaming_text.is_empty() || self.streaming_think.is_some() {
            let thinking = self.streaming_think.take().map(|t| ThinkingBlock {
                content: t.content,
                token_count: t.token_count,
                duration_ms: t.start_time.elapsed().as_millis() as u64,
                collapsed: !self.thinking_expanded,
                index: 0,
            });
            self.messages.push(Message {
                role: MessageRole::Agent,
                content: std::mem::take(&mut self.streaming_text),
                timestamp: chrono::Utc::now(),
                thinking,
                tool_calls: Vec::new(),
                separator: None,
            });
        }
        self.is_streaming = false;
    }

    /// Build the conversation as JSON for the LLM API.
    pub fn build_messages(&self) -> Vec<serde_json::Value> {
        let mut msgs = vec![serde_json::json!({
            "role": "system",
            "content": "You are a helpful query assistant. Answer concisely."
        })];
        for msg in &self.messages {
            match msg.role {
                MessageRole::User => {
                    msgs.push(serde_json::json!({"role": "user", "content": msg.content}));
                }
                MessageRole::Agent => {
                    let mut m = serde_json::json!({"role": "assistant", "content": msg.content});
                    if let Some(ref t) = msg.thinking {
                        m["reasoning_content"] = serde_json::json!(t.content);
                    }
                    msgs.push(m);
                }
                _ => {}
            }
        }
        msgs
    }

    /// Render the overlay as a floating panel.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.visible {
            return;
        }

        let overlay_height = (area.height as f32 * 0.8) as u16;
        let overlay_width = area.width.saturating_sub(4);
        let x = (area.width - overlay_width) / 2;
        let y = (area.height - overlay_height) / 2;

        let overlay_area = Rect::new(area.x + x, area.y + y, overlay_width, overlay_height);
        frame.render_widget(Clear, overlay_area);

        let block = Block::default()
            .title(" Query Agent (Ctrl+/ or Esc to close) ")
            .title_alignment(Alignment::Left)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.overlay_border));

        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);

        let composer_height = self.composer.height().min(4);
        let conv_height = inner.height.saturating_sub(composer_height + 1);

        let conv_area = Rect::new(inner.x, inner.y, inner.width, conv_height);
        let sep_area = Rect::new(inner.x, inner.y + conv_height, inner.width, 1);
        let comp_area = Rect::new(inner.x, inner.y + conv_height + 1, inner.width, composer_height);

        // Render conversation
        let mut lines: Vec<Line> = Vec::new();
        for msg in &self.messages {
            match msg.role {
                MessageRole::User => {
                    let content_lines: Vec<&str> = msg.content.split('\n').collect();
                    for (i, line_text) in content_lines.iter().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled("❯ ", theme.prompt_glyph),
                                Span::styled(line_text.to_string(), theme.user_message),
                            ]));
                        } else if line_text.is_empty() {
                            lines.push(Line::from(""));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(line_text.to_string(), theme.user_message),
                            ]));
                        }
                    }
                }
                MessageRole::Agent => {
                    // Thinking block
                    if let Some(ref think) = msg.thinking {
                        if think.collapsed {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("▸ Thinking ({} tokens, {:.1}s) ",
                                        think.token_count, think.duration_ms as f64 / 1000.0),
                                    theme.thinking_block,
                                ),
                                Span::styled("ctrl+o to expand", Style::default().fg(Color::Rgb(160, 160, 170))),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("▾ Thinking ({} tokens, {:.1}s) ",
                                        think.token_count, think.duration_ms as f64 / 1000.0),
                                    theme.thinking_block,
                                ),
                                Span::styled("ctrl+o to collapse", Style::default().fg(Color::Rgb(160, 160, 170))),
                            ]));
                            for md_line in crate::render_md::render_markdown(&think.content, theme.thinking_block) {
                                let mut spans = vec![Span::raw("  ".to_string())];
                                spans.extend(md_line.spans);
                                lines.push(Line::from(spans));
                            }
                        }
                    }
                    for md_line in crate::render_md::render_markdown(&msg.content, theme.agent_response) {
                        lines.push(md_line);
                    }
                    lines.push(Line::from(""));
                }
                _ => {}
            }
        }

        // Streaming content
        if self.is_streaming {
            if let Some(ref think) = self.streaming_think {
                let elapsed = think.start_time.elapsed().as_secs_f64();
                if !self.thinking_expanded {
                    // Collapsed
                    let header = format!("▸ Thinking ({} tokens, {:.1}s) ",
                        think.token_count, elapsed);
                    lines.push(Line::from(vec![
                        Span::styled(header, theme.thinking_block),
                        Span::styled("ctrl+o to expand", Style::default().fg(Color::Rgb(160, 160, 170))),
                    ]));
                } else {
                    // Expanded
                    let header = format!("▾ Thinking ({} tokens, {:.1}s) ",
                        think.token_count, elapsed);
                    lines.push(Line::from(vec![
                        Span::styled(header, theme.thinking_block),
                        Span::styled("ctrl+o to collapse", Style::default().fg(Color::Rgb(160, 160, 170))),
                    ]));
                    for md_line in crate::render_md::render_markdown(&think.content, theme.thinking_block) {
                        let mut spans = vec![Span::raw("  ".to_string())];
                        spans.extend(md_line.spans);
                        lines.push(Line::from(spans));
                    }
                }
            }
            if !self.streaming_text.is_empty() {
                for md_line in crate::render_md::render_markdown(&self.streaming_text, theme.agent_response) {
                    lines.push(md_line);
                }
            }
        }

        // Wrap lines to panel width so scroll algorithm counts visual rows
        let panel_width = conv_area.width as usize;
        let mut wrapped_lines: Vec<Line> = Vec::new();
        for line in lines {
            let line_width: usize = line.spans.iter().map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref())).sum();
            if line_width <= panel_width || panel_width == 0 {
                wrapped_lines.push(line);
            } else {
                // Split into multiple visual lines
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
                        // At least one char
                        byte_end = remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                    }
                    wrapped_lines.push(Line::from(Span::styled(remaining[..byte_end].to_string(), style)));
                    remaining = &remaining[byte_end..];
                }
            }
        }
        let lines = wrapped_lines;

        // --- Auto-scroll algorithm (same as main viewport) ---
        let cursor_line = if self.is_streaming {
            lines.len().saturating_sub(1)
        } else {
            lines.len().saturating_sub(1)
        };

        let visible_height = conv_area.height as usize;
        let total = lines.len();
        let threshold = (visible_height * 20 / 100).max(5);
        let max_visible_cursor_pos = visible_height.saturating_sub(threshold);

        if !self.auto_scroll && self.last_total_lines > 0 && total > self.last_total_lines {
            self.scroll_offset = self
                .scroll_offset
                .saturating_add(total - self.last_total_lines);
        }
        self.last_total_lines = total;

        let max_scroll = total.saturating_sub(visible_height);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }

        let start = if self.auto_scroll {
            if cursor_line >= max_visible_cursor_pos {
                cursor_line - max_visible_cursor_pos
            } else {
                0
            }
        } else {
            max_scroll.saturating_sub(self.scroll_offset)
        };
        let visible_lines: Vec<Line> = lines.into_iter().skip(start).take(visible_height).collect();
        frame.render_widget(Paragraph::new(visible_lines), conv_area);

        // Separator
        let hint = " Ctrl+/ or Esc to close │ ctrl+o thinking ";
        let sep_line = Line::from(vec![
            Span::styled(
                "─".repeat((inner.width as usize).saturating_sub(hint.len())),
                Style::default().fg(theme.overlay_border),
            ),
            Span::styled(hint, Style::default().fg(Color::Rgb(160, 160, 170))),
        ]);
        frame.render_widget(Paragraph::new(sep_line), sep_area);

        // Composer
        self.composer.render(frame, comp_area, theme);
    }
}

/// Parse inline code in overlay text.
fn styled_line_overlay<'a>(text: &str, base_style: Style) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), base_style));
        }
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            spans.push(Span::styled(
                format!("`{}`", &after[..end]),
                Style::default().fg(BLEU_DE_FRANCE),
            ));
            rest = &after[end + 1..];
        } else {
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
