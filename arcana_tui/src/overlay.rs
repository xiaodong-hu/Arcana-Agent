use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::composer::Composer;
use crate::theme::Theme;
use crate::types::Message;
use crate::viewport::Viewport;

/// The query sub-agent overlay panel.
#[derive(Debug)]
pub struct QueryOverlay {
    /// Whether the overlay is currently visible
    pub visible: bool,
    /// Overlay-local conversation (not persisted)
    pub messages: Vec<Message>,
    /// Overlay composer
    pub composer: Composer,
    /// Scroll offset within the overlay
    pub scroll_offset: usize,
    /// Whether the main agent has produced output while overlay is open
    pub main_agent_active: bool,
}

impl Default for QueryOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            messages: Vec::new(),
            composer: Composer::new(),
            scroll_offset: 0,
            main_agent_active: false,
        }
    }
}

impl QueryOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the overlay.
    pub fn show(&mut self) {
        self.visible = true;
        self.main_agent_active = false;
    }

    /// Hide the overlay (does not clear history).
    pub fn hide(&mut self) {
        self.visible = false;
        self.composer.clear();
    }

    /// Signal that the main agent produced output.
    pub fn notify_main_active(&mut self) {
        if self.visible {
            self.main_agent_active = true;
        }
    }

    /// Render the overlay as a floating panel.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.visible {
            return;
        }

        // Calculate overlay dimensions (~80% of viewport height)
        let overlay_height = (area.height as f32 * 0.8) as u16;
        let overlay_width = area.width.saturating_sub(4);
        let x = (area.width - overlay_width) / 2;
        let y = (area.height - overlay_height) / 2;

        let overlay_area = Rect::new(
            area.x + x,
            area.y + y,
            overlay_width,
            overlay_height,
        );

        // Clear the area behind the overlay
        frame.render_widget(Clear, overlay_area);

        // Draw the overlay border
        let mut title = " Query Agent ".to_string();
        if self.main_agent_active {
            title.push_str("[main agent active ↓] ");
        }

        let block = Block::default()
            .title(title)
            .title_alignment(Alignment::Left)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.overlay_border));

        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);

        // Split inner into conversation area and composer
        let composer_height = self.composer.height().min(3);
        let conv_height = inner.height.saturating_sub(composer_height + 1); // +1 for separator

        let conv_area = Rect::new(inner.x, inner.y, inner.width, conv_height);
        let sep_area = Rect::new(inner.x, inner.y + conv_height, inner.width, 1);
        let comp_area = Rect::new(
            inner.x,
            inner.y + conv_height + 1,
            inner.width,
            composer_height,
        );

        // Render conversation
        let mut lines: Vec<Line> = Vec::new();
        for msg in &self.messages {
            match msg.role {
                crate::types::MessageRole::User => {
                    lines.push(Line::from(vec![
                        Span::styled("❯ ", theme.prompt_glyph),
                        Span::styled(&msg.content, theme.user_message),
                    ]));
                    lines.push(Line::from(""));
                }
                crate::types::MessageRole::Agent => {
                    for line in msg.content.lines() {
                        lines.push(Line::from(Span::styled(
                            line.to_string(),
                            theme.agent_response,
                        )));
                    }
                    lines.push(Line::from(""));
                }
                _ => {}
            }
        }

        let visible_height = conv_area.height as usize;
        let start = lines.len().saturating_sub(visible_height);
        let visible_lines: Vec<Line> = lines.into_iter().skip(start).collect();

        let conv_paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
        frame.render_widget(conv_paragraph, conv_area);

        // Render separator with hint
        let hint = "[q to go back]";
        let sep_line = Line::from(vec![
            Span::styled(
                "─".repeat((inner.width as usize).saturating_sub(hint.len() + 1)),
                Style::default().fg(theme.overlay_border),
            ),
            Span::styled(hint, theme.dim),
        ]);
        frame.render_widget(Paragraph::new(sep_line), sep_area);

        // Render composer
        self.composer.render(frame, comp_area, theme);
    }
}
