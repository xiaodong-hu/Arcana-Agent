use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::types::{StatusData, PanelState, TaskInfo, SkillInfo, SubAgentInfo, TaskStatus};

/// Render the status bar. Supports multiline expansion via panel_state toggles.
/// Default: expanded (multiline). Ctrl+T folds/expands tasks, Ctrl+S skills, Ctrl+A agents.
pub fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    status: &StatusData,
    panel_state: &PanelState,
    skills: &[SkillInfo],
    agents: &[SubAgentInfo],
    tasks: &[TaskInfo],
) {
    let mut lines: Vec<Line> = Vec::new();

    // Line 1: model + context progress (always shown)
    lines.push(build_main_line(status, theme));

    // Expanded sections based on panel_state
    if panel_state.tasks_expanded && !tasks.is_empty() {
        lines.push(build_tasks_line(tasks, theme));
    }
    if panel_state.skills_expanded && !skills.is_empty() {
        lines.push(build_skills_line(skills, theme));
    }
    if panel_state.agents_expanded && !agents.is_empty() {
        lines.push(build_agents_line(agents, theme));
    }

    // Render only what fits in the allocated area
    let text = ratatui::text::Text::from(lines);
    let paragraph = Paragraph::new(text).style(Style::default().bg(theme.status_bar_bg));
    frame.render_widget(paragraph, area);
}

/// Calculate how many lines the status bar needs.
pub fn status_bar_height(
    panel_state: &PanelState,
    skills: &[SkillInfo],
    agents: &[SubAgentInfo],
    tasks: &[TaskInfo],
) -> u16 {
    let mut h: u16 = 1; // main line always
    if panel_state.tasks_expanded && !tasks.is_empty() { h += 1; }
    if panel_state.skills_expanded && !skills.is_empty() { h += 1; }
    if panel_state.agents_expanded && !agents.is_empty() { h += 1; }
    h
}

fn build_main_line<'a>(status: &StatusData, _theme: &Theme) -> Line<'a> {
    let pct = if status.tokens_max > 0 {
        (status.tokens_used as f64 / status.tokens_max as f64 * 100.0) as usize
    } else { 0 };

    let bar_color = match pct {
        0..=49 => Color::Green,
        50..=79 => Color::Yellow,
        80..=94 => Color::Rgb(255, 165, 0),
        _ => Color::Red,
    };

    let filled = (pct as f32 / 10.0).round() as usize;
    let bar: String = format!("[{}{}]", "█".repeat(filled.min(10)), "░".repeat(10 - filled.min(10)));
    let tokens_str = format_tokens(status.tokens_used, status.tokens_max);

    let spans = vec![
        Span::styled(format!(" ⚗ {} ", status.model_name), Style::default().fg(Color::White)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {} {} ", bar, tokens_str), Style::default().fg(bar_color)),
    ];

    Line::from(spans)
}

fn build_tasks_line<'a>(tasks: &[TaskInfo], _theme: &Theme) -> Line<'a> {
    let done = tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
    let total = tasks.len();
    let items: Vec<String> = tasks.iter().map(|t| {
        let mark = match t.status {
            TaskStatus::Completed => "✓",
            TaskStatus::InProgress => "▶",
            TaskStatus::Pending => "○",
        };
        format!("{} {}", mark, t.name)
    }).collect();
    let spans = vec![
        Span::styled(format!(" Tasks {}/{}: ", done, total), Style::default().fg(Color::White)),
        Span::styled(items.join(" │ "), Style::default().fg(Color::Gray)),
    ];
    Line::from(spans)
}

fn build_skills_line<'a>(skills: &[SkillInfo], _theme: &Theme) -> Line<'a> {
    let names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    let spans = vec![
        Span::styled(format!(" Skills ({}): ", skills.len()), Style::default().fg(Color::Cyan)),
        Span::styled(names.join(", "), Style::default().fg(Color::Gray)),
    ];
    Line::from(spans)
}

fn build_agents_line<'a>(agents: &[SubAgentInfo], _theme: &Theme) -> Line<'a> {
    let running = agents.iter().filter(|a| a.status == "running").count();
    let frozen = agents.iter().filter(|a| a.status == "frozen").count();
    let names: Vec<String> = agents.iter().map(|a| format!("{}({})", a.name, a.status)).collect();
    let spans = vec![
        Span::styled(format!(" Agents {}/{}: ", running, frozen), Style::default().fg(Color::Magenta)),
        Span::styled(names.join(", "), Style::default().fg(Color::Gray)),
    ];
    Line::from(spans)
}

/// Format token counts (e.g., "8.2K/1M")
fn format_tokens(used: usize, max: usize) -> String {
    let used_str = if used >= 1_000_000 {
        format!("{:.1}M", used as f64 / 1_000_000.0)
    } else if used >= 1_000 {
        format!("{:.1}K", used as f64 / 1_000.0)
    } else {
        format!("{}", used)
    };
    let max_str = if max >= 1_000_000 {
        format!("{}M", max / 1_000_000)
    } else if max >= 1_000 {
        format!("{}K", max / 1_000)
    } else {
        format!("{}", max)
    };
    format!("{}/{}", used_str, max_str)
}
