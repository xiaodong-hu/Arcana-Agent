use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// The master list of all system slash commands.
pub const ALL_COMMANDS: &[&str] = &[
    "\\quit",
    "\\help",
    "\\clear",
    "\\status",
    "\\usage",
    "\\working_dir",
    "\\check",
    "\\auth show",
    "\\auth add ",
    "\\auth remove ",
    "\\auth edit",
    "\\config show",
    "\\config edit",
];

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
    /// Saved input before history recall (restored on Down past end)
    saved_input: String,
    /// Whether this composer is in overlay (query panel) mode
    pub overlay_mode: bool,
    /// Whether the first-use hint should be shown
    pub show_hint: bool,
    /// Whether command selection mode is active (browsing with ↑↓)
    pub selection_mode: bool,
    /// Index into ALL_COMMANDS for the currently highlighted command
    pub selection_index: usize,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            overlay_mode: false,
            show_hint: true,
            selection_mode: false,
            selection_index: 0,
        }
    }
}

impl Composer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.history_index = None;
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

    /// Try to autocomplete a slash command; if not applicable, insert tab.
    pub fn autocomplete_or_tab(&mut self) {
        if self.selection_mode {
            return; // no tab in selection mode
        }
        if self.input.starts_with('\\') && !self.input.contains('\n') {
            if let Some(completed) = autocomplete_slash(&self.input) {
                self.input = completed;
                self.cursor_pos = self.input.len();
                return;
            }
        }
        self.insert_tab();
    }

    /// Insert a newline at the cursor position (Ctrl+Enter).
    pub fn insert_newline(&mut self) {
        self.history_index = None;
        self.input.insert(self.cursor_pos, '\n');
        self.cursor_pos += 1;
        self.show_hint = false;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        self.history_index = None;
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
        self.history_index = None;
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
        self.history_index = None;
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
        self.history_index = None;
        if self.cursor_pos < self.input.len() {
            self.cursor_pos = self.input[self.cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_pos + i)
                .unwrap_or(self.input.len());
        }
    }

    /// Move cursor to start of current line (Home).
    pub fn move_home(&mut self) {
        self.history_index = None;
        let before = &self.input[..self.cursor_pos];
        self.cursor_pos = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    }

    /// Move cursor to end of current line (End).
    pub fn move_end(&mut self) {
        self.history_index = None;
        let after = &self.input[self.cursor_pos..];
        if let Some(nl) = after.find('\n') {
            self.cursor_pos += nl;
        } else {
            self.cursor_pos = self.input.len();
        }
    }

    /// Jump to start of entire input (Ctrl+Up).
    pub fn jump_top(&mut self) {
        self.cursor_pos = 0;
    }

    /// Jump to end of entire input (Ctrl+Down).
    pub fn jump_bottom(&mut self) {
        self.cursor_pos = self.input.len();
    }

    /// Delete word to the left of cursor (Ctrl+w).
    pub fn delete_word_left(&mut self) {
        self.history_index = None;
        if self.cursor_pos == 0 {
            return;
        }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor_pos;
        // Skip whitespace backwards
        while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars backwards
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.input.drain(pos..self.cursor_pos);
        self.cursor_pos = pos;
    }

    /// Move cursor left by one word.
    pub fn move_word_left(&mut self) {
        self.history_index = None;
        if self.cursor_pos == 0 {
            return;
        }
        // Skip whitespace backwards
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor_pos;
        while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars backwards
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.cursor_pos = pos;
    }

    /// Move cursor right by one word.
    pub fn move_word_right(&mut self) {
        self.history_index = None;
        let len = self.input.len();
        if self.cursor_pos >= len {
            return;
        }
        let bytes = self.input.as_bytes();
        let mut pos = self.cursor_pos;
        // Skip current word chars
        while pos < len && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip whitespace
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.cursor_pos = pos;
    }

    /// Move cursor up one line (for multiline input). Returns false if already on first line.
    pub fn move_up(&mut self) -> bool {
        let before = &self.input[..self.cursor_pos];
        let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        if cur_line_start == 0 {
            return false; // already on first line
        }
        let col = self.cursor_pos - cur_line_start;
        // Find previous line start
        let prev_line_start = self.input[..cur_line_start - 1]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prev_line_len = cur_line_start - 1 - prev_line_start;
        self.cursor_pos = prev_line_start + col.min(prev_line_len);
        true
    }

    /// Move cursor down one line (for multiline input). Returns false if already on last line.
    pub fn move_down(&mut self) -> bool {
        let after = &self.input[self.cursor_pos..];
        let before = &self.input[..self.cursor_pos];
        let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = self.cursor_pos - cur_line_start;
        // Find next newline
        if let Some(nl) = after.find('\n') {
            let next_line_start = self.cursor_pos + nl + 1;
            let next_line_end = self.input[next_line_start..]
                .find('\n')
                .map(|i| next_line_start + i)
                .unwrap_or(self.input.len());
            let next_line_len = next_line_end - next_line_start;
            self.cursor_pos = next_line_start + col.min(next_line_len);
            true
        } else {
            false // already on last line
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
        self.selection_mode = false;
    }

    // ------------------------------------------------------------------
    // Command selection mode (↑↓ browse, Esc exit, Enter fill)
    // ------------------------------------------------------------------

    /// Try to enter selection mode. Only valid when input is exactly `\`.
    pub fn maybe_enter_selection_mode(&mut self) {
        if !self.overlay_mode && self.input == "\\" {
            self.selection_mode = true;
            self.selection_index = 0;
        }
    }

    /// Exit selection mode, keeping `\` in the input for further typing.
    pub fn exit_selection_mode(&mut self) {
        self.selection_mode = false;
        self.input = "\\".to_string();
        self.cursor_pos = 1;
    }

    /// Move selection cursor up (wraps around).
    pub fn select_prev(&mut self) {
        if self.selection_index == 0 {
            self.selection_index = ALL_COMMANDS.len().saturating_sub(1);
        } else {
            self.selection_index -= 1;
        }
    }

    /// Move selection cursor down (wraps around).
    pub fn select_next(&mut self) {
        if self.selection_index + 1 >= ALL_COMMANDS.len() {
            self.selection_index = 0;
        } else {
            self.selection_index += 1;
        }
    }

    /// Fill the composer with the selected command and exit selection mode.
    pub fn fill_selected_command(&mut self) {
        if let Some(cmd) = ALL_COMMANDS.get(self.selection_index) {
            self.input = cmd.to_string();
            self.cursor_pos = self.input.len();
        }
        self.selection_mode = false;
    }

    /// Whether selection mode is active.
    pub fn is_in_selection_mode(&self) -> bool {
        self.selection_mode && !self.overlay_mode
    }

    /// Recall previous message from history (Up key).
    pub fn recall_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            // Save current input before first recall
            self.saved_input = self.input.clone();
        }
        let idx = match self.history_index {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor_pos = self.input.len();
    }

    /// Recall next (Down key) — returns to saved input when past end.
    pub fn recall_next(&mut self) {
        match self.history_index {
            None => {} // not in history mode, do nothing
            Some(idx) => {
                if idx + 1 >= self.history.len() {
                    // Past end — restore saved input
                    self.history_index = None;
                    self.input = std::mem::take(&mut self.saved_input);
                    self.cursor_pos = self.input.len();
                } else {
                    self.history_index = Some(idx + 1);
                    self.input = self.history[idx + 1].clone();
                    self.cursor_pos = self.input.len();
                }
            }
        }
    }

    /// Check if the input is empty (ignoring whitespace).
    pub fn is_empty(&self) -> bool {
        self.input.trim().is_empty()
    }

    /// Get the number of lines in the input.
    pub fn line_count(&self) -> usize {
        if self.input.is_empty() {
            1
        } else {
            self.input.split('\n').count()
        }
    }

    /// Calculate the height needed for the composer, accounting for word wrap.
    pub fn height_for_width(&self, width: u16) -> u16 {
        let max_lines = 10u16;
        let prompt_w = 2u16;
        let avail_w = width.saturating_sub(prompt_w).max(1) as usize;
        let display = if !self.overlay_mode && self.input.starts_with('\\') {
            &self.input[1..]
        } else {
            &self.input
        };
        let logical_lines: Vec<&str> = if display.is_empty() {
            vec![""]
        } else {
            display.split('\n').collect()
        };
        let mut visual_lines: u16 = 0;
        for line in &logical_lines {
            let w = UnicodeWidthStr::width(*line);
            visual_lines += ((w / avail_w) + 1) as u16;
        }
        // Command list shown when input is exactly "\" (with or without selection mode)
        let cmd_list_lines: u16 = if !self.overlay_mode && self.input == "\\" {
            ALL_COMMANDS.len() as u16 + 1 // +1 for the blank separator line
        } else {
            0
        };
        visual_lines.min(max_lines) + 1 + cmd_list_lines // +1 for top border
    }

    /// Fallback height (no width info).
    pub fn height(&self) -> u16 {
        self.height_for_width(80)
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

        let in_slash_mode = !self.overlay_mode && self.input.starts_with('\\');
        let prompt = if in_slash_mode { "\\ " } else { "❯ " };
        let prompt_width = UnicodeWidthStr::width(prompt) as u16;

        if self.input.is_empty() && self.show_hint {
            let hint_text = if self.overlay_mode {
                "[enter message]"
            } else {
                "[type \\ for commands, or enter message]"
            };
            let line = Line::from(vec![
                Span::styled(prompt, theme.prompt_glyph),
                Span::styled(hint_text, theme.dim),
            ]);
            frame.render_widget(Paragraph::new(line), inner);
            frame.set_cursor_position(Position::new(inner.x + prompt_width, inner.y));
            return;
        }

        let display_text = if in_slash_mode {
            &self.input[1..]
        } else {
            &self.input
        };
        let content_style = if in_slash_mode {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let prompt_style = if in_slash_mode {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            theme.prompt_glyph
        };

        let avail_w = inner.width.saturating_sub(prompt_width) as usize;
        let avail_w = avail_w.max(1);

        // Split into logical lines, then wrap each into visual lines
        let logical_lines: Vec<&str> = if display_text.is_empty() {
            vec![""]
        } else {
            display_text.split('\n').collect()
        };

        let mut visual_lines: Vec<Line> = Vec::new();
        let mut cursor_visual_y: u16 = 0;
        let mut cursor_visual_x: u16 = 0;
        let mut cursor_found = false;

        // Calculate cursor position in display_text
        let cursor_in_display = if in_slash_mode {
            self.cursor_pos.saturating_sub(1)
        } else {
            self.cursor_pos
        };

        let mut char_offset: usize = 0; // byte offset into display_text

        for (line_idx, logical_line) in logical_lines.iter().enumerate() {
            let line_prefix = if line_idx == 0 { prompt } else { "  " };
            let line_prefix_style = if line_idx == 0 {
                prompt_style
            } else {
                Style::default()
            };

            // Wrap this logical line into chunks of avail_w characters
            let mut remaining = *logical_line;
            let mut first_chunk = true;
            loop {
                let chunk_w = UnicodeWidthStr::width(remaining);
                let (chunk, rest) = if chunk_w <= avail_w {
                    (remaining, "")
                } else {
                    // Find the byte position where width exceeds avail_w
                    let mut byte_pos = 0;
                    let mut w = 0;
                    for (i, ch) in remaining.char_indices() {
                        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                        if w + cw > avail_w {
                            byte_pos = i;
                            break;
                        }
                        w += cw;
                        byte_pos = i + ch.len_utf8();
                    }
                    (&remaining[..byte_pos], &remaining[byte_pos..])
                };

                let mut spans = Vec::new();
                let this_prefix_w: u16;
                if first_chunk {
                    spans.push(Span::styled(line_prefix, line_prefix_style));
                    this_prefix_w = UnicodeWidthStr::width(line_prefix) as u16;
                    first_chunk = false;
                } else {
                    spans.push(Span::styled("  ", Style::default())); // wrap continuation
                    this_prefix_w = 2;
                }
                spans.push(Span::styled(chunk.to_string(), content_style));

                // Check if cursor is in this chunk
                if !cursor_found {
                    let chunk_start = char_offset;
                    let chunk_end = char_offset + chunk.len();
                    if cursor_in_display >= chunk_start && cursor_in_display <= chunk_end {
                        let cursor_text = &display_text[chunk_start..cursor_in_display];
                        cursor_visual_x =
                            this_prefix_w + UnicodeWidthStr::width(cursor_text) as u16;
                        cursor_visual_y = visual_lines.len() as u16;
                        cursor_found = true;
                    }
                }

                // Inline hint on first visual line
                if visual_lines.is_empty()
                    && in_slash_mode
                    && self.input.len() > 1
                    && self.input.len() <= 7
                {
                    let hint = slash_hint(&self.input);
                    if !hint.is_empty() {
                        spans.push(Span::styled(
                            hint.to_string(),
                            Style::default().fg(Color::Rgb(255, 165, 80)),
                        ));
                    }
                }

                visual_lines.push(Line::from(spans));
                char_offset += chunk.len();

                if rest.is_empty() {
                    break;
                }
                remaining = rest;
            }

            // Account for the '\n' between logical lines
            if line_idx < logical_lines.len() - 1 {
                char_offset += 1; // the '\n' byte
            }
        }

        // Vertical command list
        if in_slash_mode && self.input == "\\" {
            let normal_fg = Color::Rgb(255, 165, 80);
            let selected_fg = Color::Rgb(255, 255, 100);
            let cursor_glyph = "❯ ";

            // Blank separator line
            visual_lines.push(Line::from(Span::styled("", Style::default())));

            for (i, cmd) in ALL_COMMANDS.iter().enumerate() {
                let (prefix, fg) = if self.selection_mode && i == self.selection_index {
                    (cursor_glyph, selected_fg)
                } else {
                    ("  ", normal_fg)
                };
                let style = Style::default().fg(fg);
                if self.selection_mode && i == self.selection_index {
                    visual_lines.push(Line::from(vec![
                        Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
                        Span::styled(*cmd, style.add_modifier(Modifier::BOLD)),
                    ]));
                } else {
                    visual_lines.push(Line::from(vec![
                        Span::styled(prefix, style),
                        Span::styled(*cmd, style),
                    ]));
                }
            }
        }

        // Scroll visual_lines to keep cursor visible within inner.height
        let max_visible = inner.height as usize;
        let scroll_offset = if visual_lines.len() <= max_visible {
            0
        } else {
            // Ensure cursor_visual_y is within the visible window
            let cursor_y = cursor_visual_y as usize;
            if cursor_y >= max_visible {
                cursor_y - max_visible + 1
            } else {
                0
            }
        };

        let displayed: Vec<Line> = visual_lines
            .into_iter()
            .skip(scroll_offset)
            .take(max_visible)
            .collect();

        let paragraph = Paragraph::new(displayed);
        frame.render_widget(paragraph, inner);

        let adjusted_cursor_y = cursor_visual_y.saturating_sub(scroll_offset as u16);
        frame.set_cursor_position(Position::new(
            (inner.x + cursor_visual_x).min(inner.x + inner.width - 1),
            (inner.y + adjusted_cursor_y).min(inner.y + inner.height - 1),
        ));
    }
}

/// Get slash command hint text.
fn slash_hint(input: &str) -> &'static str {
    match input {
        "\\q" | "\\qu" | "\\qui" | "\\quit" => " ← exit session",
        "\\h" | "\\he" | "\\hel" | "\\help" => " ← show commands",
        "\\mo" | "\\mod" | "\\mode" => " ← switch mode",
        "\\m" | "\\model" => " ← change model",
        "\\c" | "\\cl" | "\\cle" | "\\clea" | "\\clear" => " ← clear viewport",
        "\\s" | "\\st" | "\\sta" | "\\stat" | "\\statu" | "\\status" => " ← show status",
        "\\u" | "\\us" | "\\usa" | "\\usag" | "\\usage" => " ← session token/cost stats",
        "\\au" | "\\aut" | "\\auth" => " show|add|remove|edit",
        "\\auth s" | "\\auth sh" | "\\auth sho" | "\\auth show" => " ← show authority config",
        "\\auth a" | "\\auth ad" | "\\auth add" => " <command> ← add to allow list",
        "\\auth r" | "\\auth re" | "\\auth rem" | "\\auth remo" | "\\auth remov"
        | "\\auth remove" => " <command> ← remove from allow list",
        "\\auth e" | "\\auth ed" | "\\auth edi" | "\\auth edit" => " ← open in $EDITOR",
        "\\co" | "\\con" | "\\conf" | "\\confi" | "\\config" => " show|edit",
        "\\config s" | "\\config sh" | "\\config sho" | "\\config show" => " ← show config.toml",
        "\\config e" | "\\config ed" | "\\config edi" | "\\config edit" => " ← open config.toml in $EDITOR",
        "\\w" | "\\wo" | "\\wor" | "\\work" | "\\worki" | "\\workin" | "\\working" => {
            " ← show working directory"
        }
        "\\working_" | "\\working_d" | "\\working_di" | "\\working_dir" => {
            " ← show working directory"
        }
        "\\ch" | "\\che" | "\\chec" | "\\check" => " ← system health check",
        _ => "",
    }
}

/// Autocomplete a partial command. Returns the full command if unambiguous.
fn autocomplete_slash(input: &str) -> Option<String> {
    if input == "\\" || input == "\\auth " || input == "\\config " {
        return None; // too ambiguous
    }
    let matches: Vec<&&str> = ALL_COMMANDS
        .iter()
        .filter(|c| c.starts_with(input))
        .collect();
    if matches.len() == 1 {
        Some(matches[0].to_string())
    } else {
        None
    }
}
