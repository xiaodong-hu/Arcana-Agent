use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// The input composer at the bottom of the screen.
#[derive(Debug)]
pub struct Composer {
    /// Current input text
    pub input: String,
    /// Cursor position (byte offset) within the input
    pub cursor_pos: usize,
    /// History of sent messages (for recall with ↑)
    pub history: Vec<String>,
    /// Current history index
    pub history_index: Option<usize>,
    /// Whether the first-use hint should be shown
    pub show_hint: bool,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            show_hint: true,
        }
    }
}

impl Composer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        self.show_hint = false;
    }

    /// Insert a tab (4 spaces) at the cursor position.
    pub fn insert_tab(&mut self) {
        self.input.insert_str(self.cursor_pos, "    ");
        self.cursor_pos += 4;
        self.show_hint = false;
    }

    /// Insert a newline at the cursor position (Ctrl+Enter).
    pub fn insert_newline(&mut self) {
        self.input.insert(self.cursor_pos, '\n');
        self.cursor_pos += 1;
        self.show_hint = false;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.input[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.drain(prev..self.cursor_pos);
            self.cursor_pos = prev;
        }
    }

    /// Delete the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor_pos < self.input.len() {
            let next = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i)
                .unwrap_or(self.input.len());
            self.input.drain(self.cursor_pos..next);
        }
    }

    /// Move cursor left by one character.
    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos = self.input[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    /// Move cursor right by one character.
    pub fn move_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i)
                .unwrap_or(self.input.len());
        }
    }

    /// Move cursor to start of current line.
    pub fn move_home(&mut self) {
        // Find the start of the current line
        let before = &self.input[..self.cursor_pos];
        if let Some(nl) = before.rfind('\n') {
            self.cursor_pos = nl + 1;
        } else {
            self.cursor_pos = 0;
        }
    }

    /// Move cursor to end of current line.
    pub fn move_end(&mut self) {
        let after = &self.input[self.cursor_pos..];
        if let Some(nl) = after.find('\n') {
            self.cursor_pos += nl;
        } else {
            self.cursor_pos = self.input.len();
        }
    }

    /// Take the current input (consume it) and add to history.
    pub fn take_input(&mut self) -> String {
        let input = std::mem::take(&mut self.input);
        self.cursor_pos = 0;
        self.history_index = None;
        if !input.trim().is_empty() {
            self.history.push(input.clone());
        }
        input
    }

    /// Clear the input without adding to history.
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor_pos = 0;
        self.history_index = None;
    }

    /// Recall previous message from history.
    pub fn recall_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor_pos = self.input.len();
    }

    /// Check if the input is empty (ignoring whitespace).
    pub fn is_empty(&self) -> bool {
        self.input.trim().is_empty()
    }

    /// Get the number of lines in the input.
    pub fn line_count(&self) -> usize {
        if self.input.is_empty() { 1 } else { self.input.lines().count().max(1) }
    }

    /// Calculate the height needed for the composer.
    pub fn height(&self) -> u16 {
        let lines = self.line_count().min(10) as u16;
        lines + 1 // +1 for top border
    }

    /// Get the current line and column (visual width) of the cursor.
    fn cursor_line_col(&self) -> (usize, u16) {
        let before_cursor = &self.input[..self.cursor_pos];
        let line = before_cursor.matches('\n').count();
        let line_start = before_cursor.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col_text = &self.input[line_start..self.cursor_pos];
        let col = UnicodeWidthStr::width(col_text) as u16;
        (line, col)
    }

    /// Render the composer.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let in_slash_mode = self.input.starts_with('/');
        let prompt = if in_slash_mode { "/ " } else { "❯ " };
        let prompt_width = UnicodeWidthStr::width(prompt) as u16;

        if self.input.is_empty() && self.show_hint {
            // Show hint
            let line = Line::from(vec![
                Span::styled(prompt, theme.prompt_glyph),
                Span::styled("[type / for commands, or enter message]", theme.dim),
            ]);
            frame.render_widget(Paragraph::new(line), inner);
            frame.set_cursor_position(Position::new(inner.x + prompt_width, inner.y));
            return;
        }

        // Build lines for multiline display
        let display_text = if in_slash_mode { &self.input[1..] } else { &self.input };
        let content_style = if in_slash_mode {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let prompt_style = if in_slash_mode {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            theme.prompt_glyph
        };

        let text_lines: Vec<&str> = if display_text.is_empty() {
            vec![""]
        } else {
            display_text.split('\n').collect()
        };

        let mut lines: Vec<Line> = Vec::new();
        for (i, line_text) in text_lines.iter().enumerate() {
            let mut spans = Vec::new();
            if i == 0 {
                spans.push(Span::styled(prompt, prompt_style));
            } else {
                spans.push(Span::styled("  ", Style::default())); // continuation indent
            }
            spans.push(Span::styled(line_text.to_string(), content_style));

            // Slash command hints on first line
            if i == 0 && in_slash_mode && self.input.len() <= 7 {
                let hint = slash_hint(&self.input);
                if !hint.is_empty() {
                    spans.push(Span::styled(
                        hint.to_string(),
                        Style::default().fg(Color::Rgb(255, 126, 0)), // Amber #FF7E00
                    ));
                }
            }
            lines.push(Line::from(spans));
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);

        // Calculate cursor position using unicode width
        let (cursor_line, cursor_col) = self.cursor_line_col();
        // Adjust for the hidden '/' in slash mode
        let adjusted_col = if in_slash_mode && cursor_line == 0 {
            // cursor_col includes the '/', subtract 1 char width
            cursor_col.saturating_sub(1)
        } else {
            cursor_col
        };

        let cursor_x = inner.x + prompt_width + adjusted_col;
        let cursor_y = inner.y + cursor_line as u16;
        frame.set_cursor_position(Position::new(
            cursor_x.min(inner.x + inner.width - 1),
            cursor_y.min(inner.y + inner.height - 1),
        ));
    }
}

/// Get slash command hint text.
fn slash_hint(input: &str) -> &'static str {
    match input {
        "/" => " quit · help · mode · clear · status",
        "/q" | "/qu" | "/qui" | "/quit" => " ← exit session",
        "/h" | "/he" | "/hel" | "/help" => " ← show commands",
        "/mo" | "/mod" | "/mode" => " ← switch mode",
        "/m" | "/model" => " ← change model",
        "/c" | "/cl" | "/cle" | "/clea" | "/clear" => " ← clear viewport",
        "/s" | "/st" | "/sta" | "/stat" | "/statu" | "/status" => " ← show status",
        _ => "",
    }
}
