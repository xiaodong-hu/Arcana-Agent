use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::highlight;

const LIGHT_GRAY: Color = Color::Rgb(160, 160, 170);
const BG_ADDED: Color = Color::Rgb(0, 55, 30);    // dark green bg
const BG_REMOVED: Color = Color::Rgb(70, 10, 10);  // dark red bg
const BG_HEADER: Color = Color::Rgb(40, 40, 60);

/// A single line in a diff.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub content: String,
    /// Syntax-highlighted spans (populated after show_diff)
    pub spans: Vec<highlight::StyledSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Header,
    Context,
    Added,
    Removed,
}

/// User action on the diff review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffAction {
    Accept,
    EditExternal,
    Reject,
}

/// Git diff review panel state.
#[derive(Debug, Clone)]
pub struct DiffPanel {
    pub visible: bool,
    pub file_path: String,
    pub lines: Vec<DiffLine>,
    pub scroll: usize,
    pub action: Option<DiffAction>,
    pub selected: usize,
}

impl Default for DiffPanel {
    fn default() -> Self {
        Self {
            visible: false,
            file_path: String::new(),
            lines: Vec::new(),
            scroll: 0,
            action: None,
            selected: 0,
        }
    }
}

impl DiffPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the diff panel with a unified diff string.
    pub fn show_diff(&mut self, file_path: String, diff_text: &str) {
        self.visible = true;
        self.scroll = 0;
        self.selected = 0;
        self.action = None;

        // Parse diff lines
        let raw_lines: Vec<(DiffKind, &str)> = diff_text
            .lines()
            .map(|l| {
                let kind = if l.starts_with("+++") || l.starts_with("---") || l.starts_with("@@") {
                    DiffKind::Header
                } else if l.starts_with('+') {
                    DiffKind::Added
                } else if l.starts_with('-') {
                    DiffKind::Removed
                } else {
                    DiffKind::Context
                };
                (kind, l)
            })
            .collect();

        // Reconstruct source code (strip diff prefix) for highlighting
        let lang = highlight::detect_language(&file_path).unwrap_or("");
        let source: String = raw_lines
            .iter()
            .map(|(kind, line)| {
                let stripped = match kind {
                    DiffKind::Header => *line,
                    DiffKind::Added | DiffKind::Removed => line.get(1..).unwrap_or(""),
                    DiffKind::Context => line.get(1..).unwrap_or(line),
                };
                format!("{}\n", stripped)
            })
            .collect();

        let highlighted = highlight::highlight_lines(&source, lang);

        self.lines = raw_lines
            .iter()
            .enumerate()
            .map(|(i, (kind, line))| {
                let spans = highlighted.get(i).cloned().unwrap_or_default();
                DiffLine {
                    kind: *kind,
                    content: line.to_string(),
                    spans,
                }
            })
            .collect();

        self.file_path = file_path;
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max = self.lines.len().saturating_sub(5);
        self.scroll = (self.scroll + n).min(max);
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1).min(2);
    }

    pub fn confirm(&mut self) {
        self.action = Some(match self.selected {
            0 => DiffAction::Accept,
            1 => DiffAction::EditExternal,
            _ => DiffAction::Reject,
        });
        self.visible = false;
    }

    pub fn reject(&mut self) {
        self.action = Some(DiffAction::Reject);
        self.visible = false;
    }

    /// Render the diff review panel (full screen overlay).
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(format!(" Diff Review: {} ", self.file_path))
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let diff_height = inner.height.saturating_sub(3) as usize;
        let footer_y = inner.y + inner.height.saturating_sub(2);

        // Render syntax-highlighted diff lines
        let visible_lines: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll)
            .take(diff_height)
            .map(|dl| {
                let bg = match dl.kind {
                    DiffKind::Header => BG_HEADER,
                    DiffKind::Added => BG_ADDED,
                    DiffKind::Removed => BG_REMOVED,
                    DiffKind::Context => Color::Reset,
                };

                let prefix = match dl.kind {
                    DiffKind::Added => {
                        Span::styled("+", Style::default().fg(Color::Green).bg(bg).bold())
                    }
                    DiffKind::Removed => {
                        Span::styled("-", Style::default().fg(Color::Red).bg(bg).bold())
                    }
                    DiffKind::Header => Span::styled("", Style::default()),
                    DiffKind::Context => Span::styled(" ", Style::default().bg(bg)),
                };

                if dl.kind == DiffKind::Header {
                    // Headers: just cyan bold, no syntax highlight
                    return Line::from(Span::styled(
                        &dl.content,
                        Style::default().fg(Color::Cyan).bg(bg).bold(),
                    ));
                }

                let mut spans: Vec<Span> = vec![prefix];
                if dl.spans.is_empty() {
                    // Fallback: plain text
                    let text = dl.content.get(1..).unwrap_or(&dl.content);
                    spans.push(Span::styled(
                        text.to_string(),
                        Style::default().fg(Color::White).bg(bg),
                    ));
                } else {
                    for s in &dl.spans {
                        spans.push(Span::styled(
                            s.text.clone(),
                            Style::default().fg(s.fg).bg(bg),
                        ));
                    }
                }
                Line::from(spans)
            })
            .collect();

        let diff_area = Rect::new(inner.x, inner.y, inner.width, diff_height as u16);
        let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, diff_area);

        // Footer
        let total = self.lines.len();
        let pct = if total > 0 {
            (self.scroll * 100) / total.max(1)
        } else {
            0
        };
        let scroll_info = format!(" {}/{} ({}%) ", self.scroll + 1, total, pct);

        let options = ["Accept", "Edit in $EDITOR", "Reject"];
        let mut footer_spans: Vec<Span> = Vec::new();
        for (i, opt) in options.iter().enumerate() {
            let (prefix, style) = if i == self.selected {
                ("❯ ", Style::default().fg(Color::Green).bold())
            } else {
                ("  ", Style::default().fg(LIGHT_GRAY))
            };
            footer_spans.push(Span::styled(prefix, style));
            footer_spans.push(Span::styled(*opt, style));
            if i < 2 {
                footer_spans.push(Span::styled("  │  ", Style::default().fg(LIGHT_GRAY)));
            }
        }
        footer_spans.push(Span::styled(scroll_info, Style::default().fg(LIGHT_GRAY)));

        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        frame.render_widget(Paragraph::new(Line::from(footer_spans)), footer_area);

        let hint_area = Rect::new(inner.x, footer_y + 1, inner.width, 1);
        let hint = Line::from(Span::styled(
            "↑↓/j/k scroll │ ←→ select │ Enter confirm │ Esc reject │ Tab edit",
            Style::default().fg(LIGHT_GRAY),
        ));
        frame.render_widget(Paragraph::new(hint), hint_area);
    }
}
