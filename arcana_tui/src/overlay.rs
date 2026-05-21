use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

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
    /// Whether currently streaming a response
    pub is_streaming: bool,
    /// Streaming text buffer
    pub streaming_text: String,
    /// Streaming thinking buffer
    pub streaming_think: Option<StreamingThink>,
    /// Whether thinking blocks are expanded
    pub thinking_expanded: bool,
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
            composer: Composer::new(),
            scroll_offset: 0,
            is_streaming: false,
            streaming_text: String::new(),
            streaming_think: None,
            thinking_expanded: false,
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
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
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
            .title(" Query Agent (Ctrl+/ toggle, \\hide to close) ")
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
                    lines.push(Line::from(vec![
                        Span::styled("❯ ", theme.prompt_glyph),
                        Span::styled(&msg.content, theme.user_message),
                    ]));
                    lines.push(Line::from(""));
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
                                Span::styled("ctrl+o", Style::default().fg(Color::Rgb(160, 160, 170))),
                            ]));
                        } else {
                            lines.push(Line::from(Span::styled(
                                format!("▾ Thinking ({} tokens, {:.1}s)",
                                    think.token_count, think.duration_ms as f64 / 1000.0),
                                theme.thinking_block,
                            )));
                            for line in think.content.lines() {
                                lines.push(styled_line_overlay(
                                    &format!("  {}", line), theme.thinking_block,
                                ));
                            }
                        }
                    }
                    for line in msg.content.lines() {
                        lines.push(styled_line_overlay(line, theme.agent_response));
                    }
                    lines.push(Line::from(""));
                }
                _ => {}
            }
        }

        // Streaming content
        if self.is_streaming {
            if let Some(ref think) = self.streaming_think {
                let header = format!("▾ Thinking ({}…)", think.token_count);
                lines.push(Line::from(Span::styled(header, theme.thinking_block)));
                let think_lines: Vec<&str> = think.content.lines().collect();
                let show = think_lines.len().min(5);
                for line in &think_lines[think_lines.len().saturating_sub(show)..] {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", line), theme.thinking_block,
                    )));
                }
            }
            if !self.streaming_text.is_empty() {
                for line in self.streaming_text.lines() {
                    lines.push(styled_line_overlay(line, theme.agent_response));
                }
            }
        }

        let visible_height = conv_area.height as usize;
        let start = lines.len().saturating_sub(visible_height);
        let visible_lines: Vec<Line> = lines.into_iter().skip(start).collect();
        frame.render_widget(Paragraph::new(visible_lines).wrap(Wrap { trim: false }), conv_area);

        // Separator
        let hint = " Ctrl+/ close │ \\hide │ ctrl+o thinking ";
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
