use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::types::{AgentMode, StatusData, PanelState, TaskInfo, SkillInfo, SubAgentInfo, TaskStatus};

/// Render the status bar. Supports multiline expansion via panel_state toggles.
/// Default: expanded (multiline). Ctrl+S skills, Ctrl+A agents.
pub fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    status: &StatusData,
    panel_state: &PanelState,
    skills: &[SkillInfo],
    agents: &[SubAgentInfo],
    tasks: &[TaskInfo],
    agent_mode: AgentMode,
) {
    let mut lines: Vec<Line> = Vec::new();

    // Line 1: model + context progress + short counts (always shown)
    lines.push(build_main_line(status, tasks, skills, agents, agent_mode));

    // Expanded sections (tasks are now in the dedicated panel below viewport)
    if panel_state.skills_expanded && !skills.is_empty() {
        lines.push(build_skills_line(skills));
    }
    if panel_state.agents_expanded && !agents.is_empty() {
        lines.push(build_agents_line(agents));
    }

    let text = ratatui::text::Text::from(lines);
    let paragraph = Paragraph::new(text).style(Style::default().bg(theme.status_bar_bg));
    frame.render_widget(paragraph, area);
}

/// Calculate how many lines the status bar needs.
pub fn status_bar_height(
    panel_state: &PanelState,
    skills: &[SkillInfo],
    agents: &[SubAgentInfo],
    _tasks: &[TaskInfo],
) -> u16 {
    let mut h: u16 = 1; // main line always
    if panel_state.skills_expanded && !skills.is_empty() { h += 1; }
    if panel_state.agents_expanded && !agents.is_empty() { h += 1; }
    h
}

fn build_main_line<'a>(status: &StatusData, tasks: &[TaskInfo], skills: &[SkillInfo], agents: &[SubAgentInfo], agent_mode: AgentMode) -> Line<'a> {
    let filled = if status.tokens_max > 0 {
        ((status.tokens_used as f64 / status.tokens_max as f64) * 10.0).round() as usize
    } else { 0 };
    let bar: String = format!("[{}{}]", "█".repeat(filled.min(10)), "░".repeat(10 - filled.min(10)));
    let tokens_str = format_tokens(status.tokens_used, status.tokens_max);

    let tasks_done = tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
    let tasks_total = tasks.len();
    let sys_skills = skills.iter().filter(|s| s.system).count();
    let user_skills = skills.iter().filter(|s| !s.system).count();
    let agents_count = agents.len();

    // Light gray for all secondary info (visible on transparent terminals)
    let light_gray = Color::Rgb(160, 160, 170);
    let separator = Style::default().fg(light_gray);

    let spans = vec![
        Span::styled(format!(" {} ", status.model_name), Style::default().fg(Color::White)),
        Span::styled("│", separator),
        Span::styled(
            format!(" {} ", agent_mode.label()),
            Style::default().fg(if matches!(agent_mode, AgentMode::Agent) {
                Color::Rgb(0, 200, 100)
            } else {
                Color::Rgb(160, 160, 200)
            }),
        ),
        Span::styled("│", separator),
        Span::styled(format!(" {} {} ", bar, tokens_str), Style::default().fg(light_gray)),
        Span::styled("│", separator),
        Span::styled(format!(" Tasks: {}/{} ", tasks_done, tasks_total), Style::default().fg(light_gray)),
        Span::styled("│", separator),
        Span::styled(format!(" Sub-Agents: {} ", agents_count), Style::default().fg(light_gray)),
        Span::styled("│", separator),
        Span::styled(format!(" Skills (System/User): {}/{} ", sys_skills, user_skills), Style::default().fg(light_gray)),
    ];

    Line::from(spans)
}

fn build_skills_line<'a>(skills: &[SkillInfo]) -> Line<'a> {
    let names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    Line::from(vec![
        Span::styled(format!(" Skills ({}): ", skills.len()), Style::default().fg(Color::Cyan)),
        Span::styled(names.join(", "), Style::default().fg(Color::Gray)),
    ])
}

fn build_agents_line<'a>(agents: &[SubAgentInfo]) -> Line<'a> {
    let running = agents.iter().filter(|a| a.status == "running").count();
    let frozen = agents.iter().filter(|a| a.status == "frozen").count();
    let names: Vec<String> = agents.iter().map(|a| format!("{}({})", a.name, a.status)).collect();
    Line::from(vec![
        Span::styled(format!(" Agents {}/{}: ", running, frozen), Style::default().fg(Color::Magenta)),
        Span::styled(names.join(", "), Style::default().fg(Color::Gray)),
    ])
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
