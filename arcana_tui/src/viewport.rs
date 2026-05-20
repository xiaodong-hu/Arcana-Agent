use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::{Message, MessageRole, ThinkingBlock, ToolCall};

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
        if let Some(think) = self.streaming_think.take() {
            let duration = think.start_time.elapsed().as_millis() as u64;
            let block = ThinkingBlock {
                content: think.content,
                token_count: think.token_count,
                duration_ms: duration,
                collapsed: true,
                index: 0,
            };
            // Attach to the current message or store separately
            // For now, we'll track it in the streaming state
            let _ = block; // Will be attached when response completes
        }
    }

    /// Finalize the current streaming response into a message.
    pub fn finalize_response(&mut self) {
        self.finalize_response_with_stats(None);
    }

    /// Finalize response and append usage stats line.
    pub fn finalize_response_with_stats(&mut self, stats: Option<crate::types::ResponseStats>) {
        if !self.streaming_text.is_empty() {
            let msg = Message {
                role: MessageRole::Agent,
                content: std::mem::take(&mut self.streaming_text),
                timestamp: chrono::Utc::now(),
                thinking: None,
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
            content: format!("⚠ {}", content),
            timestamp: chrono::Utc::now(),
            thinking: None,
            tool_calls: Vec::new(),
        });
        self.auto_scroll = true;
        self.scroll_offset = 0;
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
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Build rendered lines from messages
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
                    // Render thinking blocks (collapsed)
                    if let Some(ref think) = msg.thinking {
                        if think.collapsed {
                            let summary = format!(
                                "▸ Thinking ({} tokens) — {:.1}s",
                                think.token_count,
                                think.duration_ms as f64 / 1000.0
                            );
                            lines.push(Line::from(Span::styled(summary, theme.thinking_block)));
                        } else {
                            let header = format!(
                                "▾ Thinking ({} tokens) — {:.1}s",
                                think.token_count,
                                think.duration_ms as f64 / 1000.0
                            );
                            lines.push(Line::from(Span::styled(header, theme.thinking_block)));
                            for line in think.content.lines() {
                                lines.push(Line::from(Span::styled(
                                    format!("  {}", line),
                                    theme.thinking_block,
                                )));
                            }
                        }
                    }

                    // Render tool calls
                    for tc in &msg.tool_calls {
                        let icon = tc.tool_type.icon();
                        let desc = format!("  {} {} ({:.1}s)", icon, tc.description, tc.duration_ms as f64 / 1000.0);
                        lines.push(Line::from(Span::styled(desc, theme.tool_call)));
                    }

                    // Render response content
                    for line in msg.content.lines() {
                        lines.push(Line::from(Span::styled(line.to_string(), theme.agent_response)));
                    }
                    lines.push(Line::from(""));
                }
                MessageRole::System => {
                    lines.push(Line::from(Span::styled(
                        &msg.content,
                        theme.system_message,
                    )));
                    lines.push(Line::from(""));
                }
            }
        }

        // Render streaming content
        if self.is_streaming {
            // Streaming thinking block
            if let Some(ref think) = self.streaming_think {
                let header = format!("▾ Thinking ({}  tokens…)", think.token_count);
                lines.push(Line::from(Span::styled(header, theme.thinking_block)));

                // Show last few lines of thinking
                let think_lines: Vec<&str> = think.content.lines().collect();
                let visible_count = (inner.height as usize / 3).max(3);
                let start = think_lines.len().saturating_sub(visible_count);
                for line in &think_lines[start..] {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", line),
                        theme.thinking_block,
                    )));
                }
            }

            // Streaming response text
            if !self.streaming_text.is_empty() {
                for line in self.streaming_text.lines() {
                    lines.push(Line::from(Span::styled(line.to_string(), theme.agent_response)));
                }
            }
        }

        // Apply scroll offset
        let total_lines = lines.len();
        let visible_height = inner.height as usize;
        let start_line = if self.auto_scroll || self.scroll_offset == 0 {
            total_lines.saturating_sub(visible_height)
        } else {
            total_lines.saturating_sub(visible_height + self.scroll_offset)
        };

        let visible_lines: Vec<Line> = lines
            .into_iter()
            .skip(start_line)
            .take(visible_height)
            .collect();

        let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }
}
