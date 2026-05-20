use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::types::{PanelState, SkillInfo, SubAgentInfo, TaskInfo, TaskStatus};

/// Render collapsible panels (skills, sub-agents, tasks).
pub fn render_panels(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    state: &PanelState,
    skills: &[SkillInfo],
    agents: &[SubAgentInfo],
    tasks: &[TaskInfo],
) -> u16 {
    let mut y_offset: u16 = 0;

    // Skills panel
    if !skills.is_empty() {
        let header = if state.skills_expanded {
            format!("▾ Skills ({} active)", skills.len())
        } else {
            format!("▸ Skills ({} active)", skills.len())
        };

        let header_line = Line::from(Span::styled(header, Style::default().fg(Color::Cyan)));
        let header_area = Rect::new(area.x, area.y + y_offset, area.width, 1);
        frame.render_widget(Paragraph::new(header_line), header_area);
        y_offset += 1;

        if state.skills_expanded {
            for (i, skill) in skills.iter().enumerate() {
                let prefix = if i == skills.len() - 1 { "  └─" } else { "  ├─" };
                let line = format!(
                    "{} {:<18} [{}]  {}",
                    prefix, skill.name, skill.mode, skill.trigger_desc
                );
                let skill_line = Line::from(Span::styled(line, theme.dim));
                let skill_area = Rect::new(area.x, area.y + y_offset, area.width, 1);
                frame.render_widget(Paragraph::new(skill_line), skill_area);
                y_offset += 1;
            }
        }
    }

    // Sub-Agents panel
    if !agents.is_empty() {
        let running = agents.iter().filter(|a| a.status == "running").count();
        let frozen = agents.iter().filter(|a| a.status == "frozen").count();

        let header = if state.agents_expanded {
            format!("▾ Sub-Agents ({} running, {} frozen)", running, frozen)
        } else {
            format!("▸ Sub-Agents ({} running, {} frozen)", running, frozen)
        };

        let header_line = Line::from(Span::styled(header, Style::default().fg(Color::Magenta)));
        let header_area = Rect::new(area.x, area.y + y_offset, area.width, 1);
        frame.render_widget(Paragraph::new(header_line), header_area);
        y_offset += 1;

        if state.agents_expanded {
            for (i, agent) in agents.iter().enumerate() {
                let prefix = if i == agents.len() - 1 { "  └─" } else { "  ├─" };
                let line = format!(
                    "{} {:<18} [{}]  turn {}/{}  {}",
                    prefix, agent.name, agent.status, agent.turn_count, agent.max_turns, agent.scope
                );
                let agent_line = Line::from(Span::styled(line, theme.dim));
                let agent_area = Rect::new(area.x, area.y + y_offset, area.width, 1);
                frame.render_widget(Paragraph::new(agent_line), agent_area);
                y_offset += 1;
            }
        }
    }

    // Tasks panel
    if !tasks.is_empty() {
        let completed = tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();

        let header = if state.tasks_expanded {
            format!("▾ Tasks ({}/{} complete)", completed, tasks.len())
        } else {
            format!("▸ Tasks ({}/{} complete)", completed, tasks.len())
        };

        let header_line = Line::from(Span::styled(header, Style::default().fg(Color::White)));
        let header_area = Rect::new(area.x, area.y + y_offset, area.width, 1);
        frame.render_widget(Paragraph::new(header_line), header_area);
        y_offset += 1;

        if state.tasks_expanded {
            for (i, task) in tasks.iter().enumerate() {
                let prefix = if i == tasks.len() - 1 { "  └─" } else { "  ├─" };
                let (icon, suffix) = match task.status {
                    TaskStatus::Completed => ("✓", String::new()),
                    TaskStatus::InProgress => {
                        let agent_info = task
                            .assigned_agent
                            .as_ref()
                            .map(|a| format!(" (in progress — {})", a))
                            .unwrap_or_else(|| " (in progress)".into());
                        ("◉", agent_info)
                    }
                    TaskStatus::Pending => ("○", String::new()),
                };
                let line = format!("{} {} {}{}", prefix, icon, task.name, suffix);

                let style = match task.status {
                    TaskStatus::Completed => Style::default().fg(Color::Green),
                    TaskStatus::InProgress => Style::default().fg(Color::Yellow),
                    TaskStatus::Pending => theme.dim,
                };

                let task_line = Line::from(Span::styled(line, style));
                let task_area = Rect::new(area.x, area.y + y_offset, area.width, 1);
                frame.render_widget(Paragraph::new(task_line), task_area);
                y_offset += 1;
            }
        }
    }

    y_offset
}

/// Calculate the height needed for panels.
pub fn panels_height(
    state: &PanelState,
    skills: &[SkillInfo],
    agents: &[SubAgentInfo],
    tasks: &[TaskInfo],
) -> u16 {
    let mut height: u16 = 0;

    if !skills.is_empty() {
        height += 1; // header
        if state.skills_expanded {
            height += skills.len() as u16;
        }
    }

    if !agents.is_empty() {
        height += 1;
        if state.agents_expanded {
            height += agents.len() as u16;
        }
    }

    if !tasks.is_empty() {
        height += 1;
        if state.tasks_expanded {
            height += tasks.len() as u16;
        }
    }

    height
}
