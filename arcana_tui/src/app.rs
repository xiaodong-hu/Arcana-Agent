use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use crate::cli::ResumeArgs;
use crate::composer::Composer;
use crate::config::Config;
use crate::event::{self, AppEvent, KeyAction, classify_key};
use crate::overlay::QueryOverlay;
use crate::panels;
use crate::status_bar;
use crate::theme::Theme;
use crate::tui::Tui;
use crate::types::*;
use crate::viewport::Viewport;

/// Application state.
struct App {
    mode: ViewMode,
    theme: Theme,
    status: StatusData,
    viewport: Viewport,
    composer: Composer,
    overlay: QueryOverlay,
    panel_state: PanelState,
    skills: Vec<SkillInfo>,
    agents: Vec<SubAgentInfo>,
    tasks: Vec<TaskInfo>,
    toasts: Vec<Toast>,
    show_banner: bool,
    should_quit: bool,
    /// True when the user pressed Ctrl+B — ignore remaining tokens.
    generation_broken: bool,
    /// When the current LLM stream started (for break-generation timing).
    stream_started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Handle to the currently running LLM stream task (aborted on Ctrl+B).
    stream_handle: Option<tokio::task::JoinHandle<()>>,
    /// Exact shell commands approved by the human in this session.
    approved_commands: HashMap<String, ApprovedAuthorityRequest>,
    /// Current agent operation mode (Ask vs Agent).
    agent_mode: AgentMode,
    /// Whether mode selection UI is active (triggered by \mode).
    mode_selection_active: bool,
    /// Index into ALL_MODES for the currently highlighted mode.
    mode_selection_index: usize,
    /// Whether terminal-native text selection is active (toggled by Ctrl+Y).
    text_selection_active: bool,
    /// Pending authority confirmations (processed one per keypress).
    confirmation_queue: Vec<serde_json::Value>,
    /// Authority requests still to process in the current batch.
    pending_requests: Vec<serde_json::Value>,
    /// Responses collected so far in the current batch.
    pending_responses: Vec<serde_json::Value>,
}

impl App {
    fn new(config: &Config) -> Self {
        let theme = Theme::from_name(&config.display.theme);
        let status = StatusData {
            model_name: config.agents.main.model.clone(),
            ..StatusData::default()
        };

        Self {
            mode: ViewMode::Main,
            theme,
            status,
            viewport: Viewport::new(),
            composer: Composer::new(),
            overlay: QueryOverlay::new(),
            panel_state: PanelState::default(),
            skills: Vec::new(),
            agents: Vec::new(),
            tasks: Vec::new(),
            toasts: Vec::new(),
            show_banner: true,
            should_quit: false,
            generation_broken: false,
            stream_started_at: None,
            stream_handle: None,
            approved_commands: HashMap::new(),
            agent_mode: AgentMode::Agent,
            mode_selection_active: false,
            mode_selection_index: 0,
            text_selection_active: false,
            confirmation_queue: Vec::new(),
            pending_requests: Vec::new(),
            pending_responses: Vec::new(),
        }
    }

    fn handle_main_key(&mut self, action: KeyAction) {
        match action {
            KeyAction::ToggleTasks => {
                self.panel_state.tasks_expanded = !self.panel_state.tasks_expanded;
            }
            KeyAction::ToggleSkills => {
                self.panel_state.skills_expanded = !self.panel_state.skills_expanded;
            }
            KeyAction::ToggleAgents => {
                self.panel_state.agents_expanded = !self.panel_state.agents_expanded;
            }
            KeyAction::Expand => {
                // Ctrl+O: toggle ALL thinking chains
                self.viewport.toggle_thinking();
            }
            KeyAction::ToggleToolCalls => {
                self.viewport.toggle_tool_calls();
            }
            KeyAction::FocusDown => {
                // Ctrl+j: scroll viewport down
                self.viewport.scroll_down(3);
            }
            KeyAction::FocusUp => {
                // Ctrl+k: scroll viewport up
                self.viewport.scroll_up(3);
            }
            KeyAction::ToggleQuery => {
                if self.mode == ViewMode::QueryOverlay {
                    self.overlay.hide();
                    self.mode = ViewMode::Main;
                } else {
                    self.overlay.show();
                    self.mode = ViewMode::QueryOverlay;
                }
            }
            KeyAction::Char('j') if self.composer.is_empty() => {
                self.viewport.scroll_down(1);
            }
            KeyAction::Char('k') if self.composer.is_empty() => {
                self.viewport.scroll_up(1);
            }
            KeyAction::Char('g') if self.composer.is_empty() => {
                self.viewport.scroll_to_top(10000);
            }
            KeyAction::Char('G') if self.composer.is_empty() => {
                self.viewport.scroll_to_bottom();
            }
            KeyAction::Char(c) => {
                self.composer.insert_char(c);
                self.show_banner = false;
            }
            KeyAction::Enter => {
                // Enter is handled in the event loop for LLM dispatch;
                // this handles the case where composer is empty (no-op)
            }
            KeyAction::Newline => {
                self.composer.insert_newline();
            }
            KeyAction::Tab => {
                self.composer.autocomplete_or_tab();
            }
            KeyAction::Backspace => {
                self.composer.backspace();
            }
            KeyAction::Delete => {
                self.composer.delete();
            }
            KeyAction::Left => {
                self.composer.move_left();
            }
            KeyAction::Right => {
                self.composer.move_right();
            }
            KeyAction::WordLeft => {
                self.composer.move_word_left();
            }
            KeyAction::WordRight => {
                self.composer.move_word_right();
            }
            KeyAction::DeleteWordLeft => {
                self.composer.delete_word_left();
            }
            KeyAction::JumpTop => {
                self.composer.jump_top();
            }
            KeyAction::JumpBottom => {
                self.composer.jump_bottom();
            }
            KeyAction::Home => {
                self.composer.move_home();
            }
            KeyAction::End => {
                self.composer.move_end();
            }
            KeyAction::Up => {
                if self.composer.input.is_empty() || self.composer.history_index.is_some() {
                    self.composer.recall_previous();
                } else {
                    self.composer.move_up();
                }
            }
            KeyAction::Down => {
                if self.composer.history_index.is_some() {
                    self.composer.recall_next();
                } else if !self.composer.input.is_empty() {
                    self.composer.move_down();
                }
            }
            KeyAction::PageUp => {
                self.viewport.scroll_up(20);
            }
            KeyAction::PageDown => {
                self.viewport.scroll_down(20);
            }
            KeyAction::HalfPageUp => {
                self.viewport.scroll_up(10);
            }
            KeyAction::HalfPageDown => {
                self.viewport.scroll_down(10);
            }
            KeyAction::Interrupt => {
                if !self.composer.is_empty() {
                    self.composer.clear();
                }
            }
            KeyAction::BreakGeneration => {
                // Stop LLM generation immediately — abort the tokio task.
                if self.viewport.is_streaming {
                    self.viewport.is_streaming = false;
                    self.generation_broken = true;
                    if let Some(handle) = self.stream_handle.take() {
                        handle.abort();
                    }
                }
            }
            KeyAction::Freeze => {
                self.toasts.push(Toast {
                    message: "Session frozen".into(),
                    detail: None,
                    created_at: chrono::Utc::now(),
                });
            }
            _ => {}
        }
    }

    fn handle_overlay_key(&mut self, action: KeyAction) {
        match action {
            KeyAction::Escape | KeyAction::ToggleQuery => {
                self.overlay.hide();
                self.mode = ViewMode::Main;
            }
            KeyAction::Expand => {
                self.overlay.toggle_thinking();
            }
            KeyAction::Char(c) => {
                self.overlay.composer.insert_char(c);
            }
            KeyAction::Tab => {
                self.overlay.composer.autocomplete_or_tab();
            }
            KeyAction::Newline => {
                self.overlay.composer.insert_newline();
            }
            KeyAction::Backspace => {
                self.overlay.composer.backspace();
            }
            KeyAction::Delete => {
                self.overlay.composer.delete();
            }
            KeyAction::Left => {
                self.overlay.composer.move_left();
            }
            KeyAction::Right => {
                self.overlay.composer.move_right();
            }
            KeyAction::WordLeft => {
                self.overlay.composer.move_word_left();
            }
            KeyAction::WordRight => {
                self.overlay.composer.move_word_right();
            }
            KeyAction::DeleteWordLeft => {
                self.overlay.composer.delete_word_left();
            }
            KeyAction::Home => {
                self.overlay.composer.move_home();
            }
            KeyAction::End => {
                self.overlay.composer.move_end();
            }
            KeyAction::JumpTop => {
                self.overlay.composer.jump_top();
            }
            KeyAction::JumpBottom => {
                self.overlay.composer.jump_bottom();
            }
            KeyAction::Up => {
                if self.overlay.composer.is_empty() || self.overlay.composer.history_index.is_some()
                {
                    self.overlay.composer.recall_previous();
                } else {
                    self.overlay.composer.move_up();
                }
            }
            KeyAction::Down => {
                if self.overlay.composer.history_index.is_some() {
                    self.overlay.composer.recall_next();
                } else if !self.overlay.composer.input.is_empty() {
                    self.overlay.composer.move_down();
                }
            }
            KeyAction::Interrupt => {
                self.overlay.composer.clear();
            }
            KeyAction::BreakGeneration => {
                if self.overlay.is_streaming {
                    self.overlay.is_streaming = false;
                    self.generation_broken = true;
                    if let Some(handle) = self.stream_handle.take() {
                        handle.abort();
                    }
                }
            }
            KeyAction::FocusDown => {
                self.overlay.scroll_down(3);
            }
            KeyAction::FocusUp => {
                self.overlay.scroll_up(3);
            }
            KeyAction::Enter => {} // handled in event loop
            _ => {}
        }
    }

    fn handle_llm_error(&mut self, err: LlmError) {
        let msg = format!("{}", err);
        let detail = match &err {
            LlmError::RateLimit {
                retry_after_secs: Some(s),
                ..
            } => Some(format!(
                "Will retry in {}s. Consider reducing request frequency.",
                s
            )),
            LlmError::RateLimit { .. } => {
                Some("Rate limit reached. Wait before sending more requests.".into())
            }
            _ => None,
        };
        // Show as error toast
        self.toasts.push(Toast {
            message: msg,
            detail,
            created_at: chrono::Utc::now(),
        });
        // Also append to viewport as system error message
        self.viewport.add_error_message(format!("{}", err));
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let status_h = status_bar::status_bar_height(
            &self.panel_state,
            &self.skills,
            &self.agents,
            &self.tasks,
        );
        let task_panel_h = panels::task_panel_height(&self.panel_state, &self.tasks);
        let composer_h = self
            .composer
            .height_for_width(area.width)
            .min(area.height / 2);

        // Layout: viewport fills, composer + status at bottom
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Min(5),                     // viewport fills
                Constraint::Length(task_panel_h),        // task panel
                Constraint::Length(composer_h + 1),      // composer + gap
                Constraint::Length(status_h),            // status bar at bottom
            ])
            .split(area);

        self.viewport.render(frame, chunks[0], &self.theme);

        panels::render_task_panel(frame, chunks[1], &self.panel_state, &self.tasks);

        // Composer area: slight lift (gap above)
        let composer_area = Rect::new(
            chunks[2].x,
            chunks[2].y + 1,
            chunks[2].width,
            chunks[2].height.saturating_sub(1),
        );
        self.composer.render(frame, composer_area, &self.theme);

        status_bar::render_status_bar(
            frame,
            chunks[3],
            &self.theme,
            &self.status,
            &self.panel_state,
            &self.skills,
            &self.agents,
            &self.tasks,
            self.agent_mode,
        );

        if self.mode == ViewMode::QueryOverlay {
            self.overlay.render(frame, area, &self.theme);
        }

        // --- Mode selection floating panel ---
        if self.mode_selection_active {
            render_mode_selection(frame, area, self.mode_selection_index, &self.theme);
        }

        render_toasts(frame, area, &self.toasts);
    }
}

fn render_mode_selection(frame: &mut Frame, area: Rect, selected: usize, theme: &Theme) {
    let panel_w = 52u16;
    let panel_h = (ALL_MODES.len() as u16 + 4);
    let x = area.width.saturating_sub(panel_w) / 2;
    let y = area.height.saturating_sub(panel_h) / 2;
    let panel_area = Rect::new(x, y, panel_w, panel_h);

    // Clear background
    frame.render_widget(ratatui::widgets::Clear, panel_area);

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(180, 160, 60)))
        .title(" Select Agent Mode ")
        .title_style(
            Style::default()
                .fg(Color::Rgb(180, 160, 60))
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(block, panel_area);

    let inner = panel_area.inner(ratatui::layout::Margin::new(1, 1));
    let mut lines: Vec<Line> = Vec::new();

    for (i, mode) in ALL_MODES.iter().enumerate() {
        let is_selected = i == selected;
        let prefix = if is_selected { "❯ " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Rgb(180, 160, 60))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(160, 160, 170))
        };
        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(mode.label(), style),
            Span::raw("  "),
            Span::styled(
                mode.description(),
                Style::default().fg(Color::Rgb(120, 120, 130)),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter select  ·  Esc cancel",
        Style::default().fg(Color::Rgb(120, 120, 130)),
    )));

    let paragraph = ratatui::widgets::Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_toasts(frame: &mut Frame, area: Rect, toasts: &[Toast]) {
    let now = chrono::Utc::now();
    let visible: Vec<&Toast> = toasts
        .iter()
        .filter(|t| (now - t.created_at).num_seconds() < 5)
        .collect();

    for (i, toast) in visible.iter().enumerate() {
        let width = (toast.message.len() as u16 + 4).min(area.width.saturating_sub(4));
        let height: u16 = if toast.detail.is_some() { 3 } else { 2 };
        let x = area.width.saturating_sub(width + 2);
        let y = 1 + (i as u16 * (height + 1));
        if y + height > area.height {
            break;
        }

        let toast_area = Rect::new(x, y, width, height);

        // Use red border for error toasts (those containing "error" or "limit")
        let border_color = if toast.message.contains("error")
            || toast.message.contains("limit")
            || toast.message.contains("Rate")
        {
            Color::Red
        } else {
            Color::Green
        };

        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(border_color));
        let inner = block.inner(toast_area);
        frame.render_widget(ratatui::widgets::Clear, toast_area);
        frame.render_widget(block, toast_area);
        let text = ratatui::widgets::Paragraph::new(toast.message.as_str())
            .style(Style::default().fg(Color::White));
        frame.render_widget(text, inner);
    }
}

struct AuthorityDaemon {
    child: Option<Child>,
}

impl Drop for AuthorityDaemon {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn ensure_authority_daemon(
    inherit_terminal: bool,
) -> Result<AuthorityDaemon, Box<dyn std::error::Error>> {
    let socket_path = Path::new(".arcana/authority.sock");
    if authority_socket_ready(socket_path) {
        return Ok(AuthorityDaemon { child: None });
    }
    if socket_path.exists() {
        fs::remove_file(socket_path)?;
    }

    let binary = find_authority_binary()
        .ok_or("cannot find authority_and_recording binary; build it before launching Arcana")?;
    let mut command = std::process::Command::new(binary);
    command.arg(".");
    if inherit_terminal {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
    } else {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }

    let mut child = command.spawn()?;
    for _ in 0..50 {
        if authority_socket_ready(socket_path) {
            return Ok(AuthorityDaemon { child: Some(child) });
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();
    Err("authority daemon did not create .arcana/authority.sock in time".into())
}

fn authority_socket_ready(socket_path: &Path) -> bool {
    socket_path.exists() && UnixStream::connect(socket_path).is_ok()
}

pub(crate) fn find_authority_binary() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent()?;
    let candidates = [
        repo_root
            .join("authority_and_recording")
            .join("target")
            .join("release")
            .join("authority_and_recording"),
        repo_root
            .join("authority_and_recording")
            .join("target")
            .join("debug")
            .join("authority_and_recording"),
        repo_root
            .join("arcana_tui")
            .join("target")
            .join("release")
            .join("authority_and_recording"),
        repo_root
            .join("arcana_tui")
            .join("target")
            .join("debug")
            .join("authority_and_recording"),
    ];
    candidates.into_iter().find(|path| path.exists())
}

/// Run the interactive TUI session.
pub async fn interactive(
    model: Option<String>,
    provider: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;

    if let Some(m) = model {
        config.agents.main.model = m;
    }
    if let Some(p) = provider {
        config.agents.main.provider = p;
    }

    let _authority_daemon = ensure_authority_daemon(false)?;
    let mut tui = Tui::new()?;
    let mut app = App::new(&config);

    // Inject banner into viewport as scrollable content
    app.viewport.add_banner(&config.agents.main.model);

    let (mut event_tx, mut events, mut event_handle) = event::spawn_event_reader();

    // Conversation history for LLM context
    let mut conversation: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": build_system_prompt(app.agent_mode)})];

    loop {
        tui.draw(|frame| app.render(frame))?;

        if let Some(evt) = events.recv().await {
            match evt {
                AppEvent::Key(key) => {
                    let action = classify_key(&key);

                    // --- Mode selection overlay (triggered by \mode) ---
                    if app.mode_selection_active {
                        match action {
                            KeyAction::Up => {
                                if app.mode_selection_index == 0 {
                                    app.mode_selection_index = ALL_MODES.len() - 1;
                                } else {
                                    app.mode_selection_index -= 1;
                                }
                                continue;
                            }
                            KeyAction::Down => {
                                if app.mode_selection_index + 1 >= ALL_MODES.len() {
                                    app.mode_selection_index = 0;
                                } else {
                                    app.mode_selection_index += 1;
                                }
                                continue;
                            }
                            KeyAction::Enter => {
                                let new_mode = ALL_MODES[app.mode_selection_index];
                                app.agent_mode = new_mode;
                                app.mode_selection_active = false;
                                // Refresh system prompt in conversation
                                conversation[0] = serde_json::json!({
                                    "role": "system",
                                    "content": build_system_prompt(app.agent_mode)
                                });
                                app.viewport.add_error_message(format!(
                                    "Mode switched to: {} — {}",
                                    new_mode.label(),
                                    new_mode.description()
                                ));
                                continue;
                            }
                            KeyAction::Escape => {
                                app.mode_selection_active = false;
                                continue;
                            }
                            _ => {
                                // Any other key dismisses the selection
                                app.mode_selection_active = false;
                            }
                        }
                    }

                    match app.mode {
                        ViewMode::Main => {
                            // --- Command selection mode (↑↓ browse, Esc exit) ---
                            if app.composer.is_in_selection_mode() {
                                match action {
                                    KeyAction::Up => {
                                        app.composer.select_prev();
                                        continue;
                                    }
                                    KeyAction::Down => {
                                        app.composer.select_next();
                                        continue;
                                    }
                                    KeyAction::Escape => {
                                        app.composer.exit_selection_mode();
                                        continue;
                                    }
                                    KeyAction::Enter => {
                                        app.composer.fill_selected_command();
                                        continue;
                                    }
                                    KeyAction::Char(_) | KeyAction::Tab => {
                                        // Typing or tab exits selection mode,
                                        // then fall through to normal handling below.
                                        app.composer.exit_selection_mode();
                                    }
                                    _ => {} // other keys fall through normally
                                }
                            }
                            // --- Enter selection mode on Down when input is `\` ---
                            if action == KeyAction::Down && app.composer.input == "\\" {
                                app.composer.maybe_enter_selection_mode();
                                continue;
                            }
                            // Check if this is an Enter that will send a message
                            if action == KeyAction::Enter && !app.composer.is_empty() {
                                let input = app.composer.take_input();
                                // Handle slash commands
                                let trimmed = input.trim();
                                let is_command = trimmed.starts_with('\\');
                                match trimmed {
                                    "\\quit" | "\\q" => {
                                        app.should_quit = true;
                                    }
                                    "\\clear" => {
                                        app.viewport.messages.clear();
                                        conversation.truncate(1); // keep system msg
                                    }
                                    "\\help" => {
                                        app.viewport.add_error_message(
                                            "Commands:\n\
  \\quit          Exit session\n\
  \\clear         Clear viewport\n\
  \\mode          Switch agent mode (Ask / Agent)\n\
  \\status        Show model/token info\n\
  \\usage         Session token/cost stats\n\
  \\working_dir   Show current working directory\n\
  \\check         System health check\n\
  \\config list   Show config.toml\n\
  \\config edit   Open config.toml in $EDITOR\n\
  \\authorization list     Show authorized commands\n\
  \\authorization add      Add command to allow list\n\
  \\authorization remove   Remove from allow list\n\
  \\authorization edit     Open authority.toml in $EDITOR\n\
  \\instruction show       Show INSTRUCTION.md\n\
  \\instruction edit       Open INSTRUCTION.md in $EDITOR\n\
  \\behavioral show        Show behavioral line\n\
  \\behavioral edit        Edit behavioral line in $EDITOR\n\
\n\
Hotkeys:\n\
  Ctrl+e         Open $EDITOR for prompt\n\
  Ctrl+/         Toggle query agent\n\
  Ctrl+o         Expand/collapse thinking chains\n\
  Ctrl+x         Expand/collapse tool-call panels\n\
  Ctrl+j/k       Scroll viewport down/up\n\
  Ctrl+h/l       Move cursor word left/right\n\
  Ctrl+w         Delete word left\n\
  Ctrl+Up/Down   Jump to start/end of input\n\
  Ctrl+Left/Right  Move cursor by word\n\
  Ctrl+Enter     New line in prompt\n\
  Home/End       Start/end of current line\n\
  Ctrl+b         Stop LLM generation\n\
  Ctrl+c         Clear prompt"
                                                .into(),
                                        );
                                    }
                                    "\\mode" => {
                                        app.mode_selection_active = true;
                                        app.mode_selection_index = ALL_MODES
                                            .iter()
                                            .position(|m| *m == app.agent_mode)
                                            .unwrap_or(0);
                                    }
                                    "\\status" => {
                                        app.viewport.add_error_message(format!(
                                            "Model: {} │ Tokens: {}/{} │ Tasks: {}",
                                            app.status.model_name,
                                            app.status.tokens_used,
                                            app.status.tokens_max,
                                            app.tasks.len()
                                        ));
                                    }
                                    "\\usage" => {
                                        let in_str =
                                            format_token_count(app.status.session_input_tokens);
                                        let out_str =
                                            format_token_count(app.status.session_output_tokens);
                                        app.viewport.add_error_message(format!(
                                            "Session Usage:\n  Requests: {}\n  Tokens: {} in / {} out\n  Total cost: {:.4}",
                                            app.status.session_requests, in_str, out_str, app.status.session_cost
                                        ));
                                    }
                                    "\\working_dir" => {
                                        let cwd = std::env::current_dir()
                                            .map(|p| p.display().to_string())
                                            .unwrap_or_else(|_| "<unknown>".into());
                                        app.viewport.add_error_message(format!(
                                            "Working directory:\n  {cwd}\n\nWorkspace:\n  {cwd}/.arcana/"
                                        ));
                                    }
                                    "\\config" | "\\config list" => {
                                        let path = Config::path()?;
                                        if !path.exists() {
                                            Config::default().save()?;
                                        }
                                        let content = std::fs::read_to_string(&path)?;
                                        app.viewport.add_error_message(format!(
                                            "Config file: {}\n\n{}",
                                            path.display(),
                                            content
                                        ));
                                    }
                                    "\\config edit" => {
                                        let path = Config::path()?;
                                        if !path.exists() {
                                            Config::default().save()?;
                                        }
                                        let editor = config.editor.command.clone();
                                        event_handle.abort();
                                        tui.suspend()?;
                                        let _ =
                                            std::process::Command::new(&editor).arg(&path).status();
                                        tui.resume()?;
                                        let (tx, rx, handle) = event::spawn_event_reader();
                                        event_tx = tx;
                                        events = rx;
                                        event_handle = handle;
                                        match Config::load() {
                                            Ok(new_config) => {
                                                config = new_config;
                                                app.status.model_name =
                                                    config.agents.main.model.clone();
                                                app.viewport.add_error_message(format!(
                                                    "Config reloaded from {}",
                                                    path.display()
                                                ));
                                            }
                                            Err(e) => app.viewport.add_error_message(format!(
                                                "Config edit saved, but reload failed: {e}"
                                            )),
                                        }
                                    }
                                    "\\authorization" | "\\authorization list" => {
                                        let path = dirs::home_dir()
                                            .unwrap_or_default()
                                            .join(".arcana/authority.toml");
                                        if path.exists() {
                                            if let Ok(content) = std::fs::read_to_string(&path) {
                                                app.viewport.add_error_message(format!(
                                                    "Authority config: {}\n\n{}",
                                                    path.display(),
                                                    content
                                                ));
                                            }
                                        } else {
                                            app.viewport.add_error_message(
                                                "No authority.toml found. Run: arcana onboard"
                                                    .into(),
                                            );
                                        }
                                    }
                                    cmd if cmd.starts_with("\\authorization add ") => {
                                        let pattern = cmd
                                            .strip_prefix("\\authorization add ")
                                            .unwrap()
                                            .trim();
                                        let path = dirs::home_dir()
                                            .unwrap_or_default()
                                            .join(".arcana/authority.toml");
                                        if let Ok(content) = std::fs::read_to_string(&path) {
                                            // Simple append to [commands] allow list
                                            let new = content.replace(
                                                "]\n\n# Commands that always require confirmation",
                                                &format!("    \"{}\",\n]\n\n# Commands that always require confirmation", pattern)
                                            );
                                            let _ = std::fs::write(&path, &new);
                                            app.viewport.add_error_message(format!(
                                                "✓ Added to allow: {}",
                                                pattern
                                            ));
                                        }
                                    }
                                    cmd if cmd.starts_with("\\authorization remove ") => {
                                        let pattern = cmd
                                            .strip_prefix("\\authorization remove ")
                                            .unwrap()
                                            .trim();
                                        let path = dirs::home_dir()
                                            .unwrap_or_default()
                                            .join(".arcana/authority.toml");
                                        if let Ok(content) = std::fs::read_to_string(&path) {
                                            let needle = format!("    \"{}\",\n", pattern);
                                            let new = content.replace(&needle, "");
                                            let _ = std::fs::write(&path, &new);
                                            app.viewport.add_error_message(format!(
                                                "✓ Removed: {}",
                                                pattern
                                            ));
                                        }
                                    }
                                    "\\authorization edit" => {
                                        let path = dirs::home_dir()
                                            .unwrap_or_default()
                                            .join(".arcana/authority.toml");
                                        let editor = config.editor.command.clone();
                                        // Stop event reader completely before editor
                                        event_handle.abort();
                                        tui.suspend()?;
                                        // Run editor with full terminal control
                                        let _ =
                                            std::process::Command::new(&editor).arg(&path).status();
                                        // Resume TUI and respawn event reader
                                        tui.resume()?;
                                        let (tx, rx, handle) = event::spawn_event_reader();
                                        event_tx = tx;
                                        events = rx;
                                        event_handle = handle;
                                        app.composer.clear();
                                        app.viewport.add_error_message(format!(
                                            "Authority config reloaded from {}",
                                            path.display()
                                        ));
                                    }
                                    "\\instruction" | "\\instruction show" => {
                                        match crate::instruction::load_or_create() {
                                            Ok(content) => {
                                                let path = crate::instruction::path()?;
                                                app.viewport.add_error_message(format!(
                                                    "Instruction file: {}\n\n{}",
                                                    path.display(),
                                                    content
                                                ));
                                            }
                                            Err(e) => app.viewport.add_error_message(format!(
                                                "Cannot load instruction: {e}"
                                            )),
                                        }
                                    }
                                    "\\instruction edit" => {
                                        let _ = crate::instruction::load_or_create()?;
                                        let path = crate::instruction::path()?;
                                        let editor = config.editor.command.clone();
                                        event_handle.abort();
                                        tui.suspend()?;
                                        let _ =
                                            std::process::Command::new(&editor).arg(&path).status();
                                        tui.resume()?;
                                        let (tx, rx, handle) = event::spawn_event_reader();
                                        event_tx = tx;
                                        events = rx;
                                        event_handle = handle;

                                        refresh_authorized_prompt_file();
                                        conversation[0] = serde_json::json!({
                                            "role": "system",
                                            "content": build_system_prompt(app.agent_mode)
                                        });
                                        app.viewport.add_error_message(format!(
                                            "Instruction reloaded from {}",
                                            path.display()
                                        ));
                                    }
                                    "\\behavioral" | "\\behavioral show" => {
                                        match crate::behavioral::load_or_create() {
                                            Ok(content) => {
                                                let path = crate::behavioral::path()?;
                                                app.viewport.add_error_message(format!(
                                                    "Behavioral line file: {}\n\n{}",
                                                    path.display(),
                                                    content
                                                ));
                                            }
                                            Err(e) => app.viewport.add_error_message(format!(
                                                "Cannot load behavioral line: {e}"
                                            )),
                                        }
                                    }
                                    "\\behavioral edit" => {
                                        let _ = crate::behavioral::load_or_create()?;
                                        let path = crate::behavioral::path()?;
                                        let editor = config.editor.command.clone();
                                        event_handle.abort();
                                        tui.suspend()?;
                                        let _ =
                                            std::process::Command::new(&editor).arg(&path).status();
                                        tui.resume()?;
                                        let (tx, rx, handle) = event::spawn_event_reader();
                                        event_tx = tx;
                                        events = rx;
                                        event_handle = handle;

                                        conversation[0] = serde_json::json!({
                                            "role": "system",
                                            "content": build_system_prompt(app.agent_mode)
                                        });
                                        app.viewport.add_error_message(format!(
                                            "Behavioral line reloaded from {}",
                                            path.display()
                                        ));
                                    }
                                    "\\check" => {
                                        let home =
                                            dirs::home_dir().unwrap_or_default().join(".arcana");
                                        let mut lines = Vec::new();
                                        let cfg = home.join("config.toml");
                                        lines.push(if cfg.exists() {
                                            "✓ config.toml"
                                        } else {
                                            "✗ config.toml (missing)"
                                        });
                                        let auth = home.join("authority.toml");
                                        lines.push(if auth.exists() {
                                            "✓ authority.toml"
                                        } else {
                                            "✗ authority.toml (missing)"
                                        });
                                        let soul = home.join("SOUL.md");
                                        lines.push(if soul.exists() {
                                            "✓ SOUL.md"
                                        } else {
                                            "✗ SOUL.md (missing)"
                                        });
                                        let user = home.join("USER.md");
                                        lines.push(if user.exists() {
                                            "✓ USER.md"
                                        } else {
                                            "✗ USER.md (missing)"
                                        });
                                        let instruction = home.join("INSTRUCTION.md");
                                        lines.push(if instruction.exists() {
                                            "✓ INSTRUCTION.md"
                                        } else {
                                            "✗ INSTRUCTION.md (missing)"
                                        });
                                        let key_ok = std::env::var("DEEPSEEK_API_KEY").is_ok();
                                        lines.push(if key_ok {
                                            "✓ DEEPSEEK_API_KEY (env)"
                                        } else {
                                            "✗ DEEPSEEK_API_KEY (not set)"
                                        });
                                        app.viewport.add_error_message(format!(
                                            "Health Check:\n  {}",
                                            lines.join("\n  ")
                                        ));
                                    }
                                    _ if is_command => {
                                        app.viewport.add_error_message(format!(
                                            "Unknown command: {}",
                                            trimmed
                                        ));
                                    }
                                    _ => {
                                        // Send to LLM
                                        app.viewport.add_user_message(input.clone());
                                        app.viewport.is_streaming = true;
                                        app.generation_broken = false;
                                        app.stream_started_at = Some(chrono::Utc::now());
                                        app.show_banner = false;

                                        let authority_context =
                                            authority_context_for_query(&input, &config).await;
                                        let user_content = if authority_context.is_empty() {
                                            input.clone()
                                        } else {
                                            format!("{authority_context}\n\nUser query:\n{input}")
                                        };

                                        conversation.push(serde_json::json!({
                                            "role": "user", "content": user_content
                                        }));

                                        // Abort any prior stream (defensive) and store the new handle.
                                        if let Some(old) = app.stream_handle.take() {
                                            old.abort();
                                        }
                                        app.stream_handle = Some(crate::llm::spawn_stream(
                                            &config,
                                            conversation.clone(),
                                            event_tx.clone(),
                                        ));
                                    }
                                }
                                // Add separator after command output
                                if is_command
                                    && trimmed != "\\quit"
                                    && trimmed != "\\q"
                                    && trimmed != "\\clear"
                                {
                                    app.viewport.add_separator();
                                }
                                app.show_banner = false;
                            } else if action == KeyAction::ToggleSelectionMode {
                                // Ctrl+Y: toggle terminal-native text selection.
                                // Disables mouse capture so the terminal handles the mouse
                                // for native selection & copy (Ctrl+Shift+C), while the TUI
                                // stays fully visible. Press Ctrl+Y again to re-enable.
                                app.text_selection_active = !app.text_selection_active;
                                tui.set_mouse_capture(!app.text_selection_active)?;
                                let msg = if app.text_selection_active {
                                    "Text selection ON — use mouse to select, Ctrl+Shift+C to copy. Ctrl+Y to exit."
                                } else {
                                    "Text selection OFF"
                                };
                                app.toasts.push(Toast {
                                    message: msg.into(),
                                    detail: None,
                                    created_at: chrono::Utc::now(),
                                });
                            } else if action == KeyAction::OpenEditor {
                                // Ctrl+e: open $EDITOR for prompt editing
                                let editor = config.editor.command.clone();
                                let tmp = std::env::temp_dir().join("arcana_prompt.md");
                                let _ = std::fs::write(&tmp, &app.composer.input);
                                // Calculate line:col for cursor positioning
                                let before = &app.composer.input[..app.composer.cursor_pos];
                                let line = before.matches('\n').count() + 1;
                                let col = before
                                    .rfind('\n')
                                    .map(|i| app.composer.cursor_pos - i - 1)
                                    .unwrap_or(app.composer.cursor_pos)
                                    + 1;
                                event_handle.abort();
                                tui.suspend()?;
                                let _ = std::process::Command::new(&editor)
                                    .arg(format!("+{}", line))
                                    .arg(&tmp)
                                    .env("ARCANA_COL", col.to_string())
                                    .status();
                                tui.resume()?;
                                let (tx, rx, handle) = event::spawn_event_reader();
                                event_tx = tx;
                                events = rx;
                                event_handle = handle;
                                // Load edited content back
                                if let Ok(content) = std::fs::read_to_string(&tmp) {
                                    let content = content.trim_end_matches('\n').to_string();
                                    app.composer.input = content;
                                    app.composer.cursor_pos = app.composer.input.len();
                                    app.composer.show_hint = false;
                                    app.composer.history_index = None;
                                }
                                let _ = std::fs::remove_file(&tmp);
                            } else {
                                app.handle_main_key(action);
                            }
                        }
                        ViewMode::QueryOverlay => {
                            // Handle Enter for overlay LLM dispatch
                            if action == KeyAction::Enter && !app.overlay.composer.is_empty() {
                                let input = app.overlay.composer.take_input();
                                app.overlay.messages.push(Message {
                                    role: MessageRole::User,
                                    content: input.clone(),
                                    timestamp: chrono::Utc::now(),
                                    thinking: None,
                                    tool_calls: Vec::new(),
                                    separator: None,
                                });
                                app.overlay.is_streaming = true;
                                app.generation_broken = false;
                                app.stream_started_at = Some(chrono::Utc::now());

                                let msgs = app.overlay.build_messages();
                                if let Some(old) = app.stream_handle.take() {
                                    old.abort();
                                }
                                app.stream_handle = Some(crate::llm::spawn_overlay_stream(
                                    &config,
                                    msgs,
                                    event_tx.clone(),
                                ));
                            } else {
                                app.handle_overlay_key(action);
                            }
                        }
                        ViewMode::DiffReview => {}
                        ViewMode::ModeSelection => {}
                    }
                }
                AppEvent::Paste(text) => {
                    // Insert pasted text directly preserving all formatting
                    let composer = if app.mode == ViewMode::QueryOverlay {
                        &mut app.overlay.composer
                    } else {
                        &mut app.composer
                    };
                    // Normalize line endings and insert as block
                    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                    composer.input.insert_str(composer.cursor_pos, &normalized);
                    composer.cursor_pos += normalized.len();
                    composer.show_hint = false;
                    app.show_banner = false;
                }
                AppEvent::ScrollUp(n) => {
                    app.viewport.scroll_up(n as usize);
                }
                AppEvent::ScrollDown(n) => {
                    app.viewport.scroll_down(n as usize);
                }
                AppEvent::Resize(_, _) => {}
                AppEvent::Token(token) => {
                    if app.generation_broken {
                        continue;
                    }
                    // Thinking tokens are prefixed with \x00THINK:
                    if let Some(think_text) = token.strip_prefix("\x00THINK:") {
                        app.viewport.append_think_token(think_text);
                    } else {
                        app.viewport.append_token(&token);
                    }
                }
                AppEvent::ThinkStart => {
                    if app.generation_broken {
                        continue;
                    }
                    app.viewport.start_thinking();
                }
                AppEvent::ThinkEnd => {
                    if app.generation_broken {
                        continue;
                    }
                    app.viewport.end_thinking();
                }
                AppEvent::ResponseComplete(stats) => {
                    if app.generation_broken {
                        // User pressed Ctrl+B — show break message with timing.
                        let elapsed = app
                            .stream_started_at
                            .map(|t| {
                                let dur = chrono::Utc::now() - t;
                                format!("{:.1}s", dur.num_milliseconds() as f64 / 1000.0)
                            })
                            .unwrap_or_else(|| "?".into());
                        app.viewport.finalize_response_with_stats(None);
                        app.viewport.messages.push(Message {
                            role: MessageRole::System,
                            content: format!("[Break Generation]\nTime: {elapsed}"),
                            timestamp: chrono::Utc::now(),
                            thinking: None,
                            tool_calls: Vec::new(),
                            separator: None,
                        });
                        app.viewport.add_separator();
                        app.generation_broken = false;
                        app.stream_started_at = None;
                        app.stream_handle = None;
                        continue;
                    }

                    // Store the response in conversation history
                    let response_text = app.viewport.streaming_text.clone();
                    let thinking_text = app
                        .viewport
                        .streaming_think
                        .as_ref()
                        .map(|t| t.content.clone());

                    // Update token usage in status
                    if let Some(s) = &stats {
                        app.status.tokens_used += s.input_tokens + s.output_tokens;
                        app.status.session_input_tokens += s.input_tokens;
                        app.status.session_output_tokens += s.output_tokens;
                        app.status.session_cost += s.cost;
                        app.status.session_requests += 1;
                    }

                    let authority_requests = extract_authority_json_requests(&response_text);
                    if authority_requests.is_empty() {
                        app.viewport.finalize_response_with_stats(stats);
                        app.viewport.add_separator();
                    } else {
                        app.viewport.streaming_text =
                            display_text_without_authority_requests(&response_text);
                        app.viewport.finalize_response_for_tool_calls();
                    }

                    // Append assistant message to conversation (with reasoning for cache)
                    let mut msg = serde_json::json!({
                        "role": "assistant",
                        "content": response_text
                    });
                    if let Some(thinking) = thinking_text {
                        msg["reasoning_content"] = serde_json::json!(thinking);
                    }
                    conversation.push(msg);

                    if !authority_requests.is_empty() {
                        let socket_path = Path::new(".arcana/authority.sock");
                        let mut responses = Vec::new();
                        for request in authority_requests {
                            let approved = match authority_command_cache_key(&request)
                                .and_then(|key| app.approved_commands.get(&key).cloned())
                            {
                                Some(approved) => approved,
                                None => {
                                    // Auto-approve safe read-only operations without prompting
                                    if is_safe_authority_request(&request) {
                                        let details = authority_request_details(&request);
                                        let safe = ApprovedAuthorityRequest {
                                            request: confirmed_authority_request(request.clone()),
                                            tool_type: details.tool_type,
                                            description: details.target,
                                            action: details.action.map(str::to_string),
                                        };
                                        if let Some(key) =
                                            authority_command_cache_key(&safe.request)
                                        {
                                            app.approved_commands.insert(key, safe.clone());
                                        }
                                        safe
                                    } else {
                                        // TUI-based inline confirmation (no shell prompt)
                                        event_handle.abort();
                                        let (tx, mut rx, handle) = event::spawn_event_reader();
                                        let approval = tui_approve_authority_request(
                                            &mut app, &mut tui, &tx, &mut rx, request.clone(),
                                        )?;
                                        handle.abort();
                                        // Respawn the main event reader
                                        let (tx2, rx2, handle2) = event::spawn_event_reader();
                                        event_tx = tx2;
                                        events = rx2;
                                        event_handle = handle2;

                                        match approval {
                                            AuthorityApproval::Approved(approved) => {
                                                if let Some(key) =
                                                    authority_command_cache_key(&approved.request)
                                                {
                                                    app.approved_commands
                                                        .insert(key, approved.clone());
                                                }
                                                approved
                                            }
                                            AuthorityApproval::Aborted {
                                                response,
                                                tool_type,
                                                description,
                                                action,
                                            } => {
                                                app.viewport.add_tool_call(ToolCall {
                                                    tool_type,
                                                    description,
                                                    action,
                                                    result: Some(response.to_string()),
                                                    duration_ms: 0,
                                                    collapsed: false,
                                                });
                                                responses.push(response);
                                                continue;
                                            }
                                        }
                                    }
                                }
                            };

                            app.viewport.add_tool_call(ToolCall {
                                tool_type: approved.tool_type,
                                description: approved.description.clone(),
                                action: approved.action.clone(),
                                result: None,
                                duration_ms: 0,
                                collapsed: false,
                            });
                            tui.draw(|frame| app.render(frame))?;

                            let started_at = Instant::now();
                            let response = if socket_path.exists() {
                                match authority_request(socket_path, approved.request.clone()) {
                                    Ok(response) => response,
                                    Err(e) => serde_json::json!({
                                        "status": "denied",
                                        "reason": format!("AAS bridge failed: {e}")
                                    }),
                                }
                            } else {
                                serde_json::json!({
                                    "status": "denied",
                                    "reason": "AAS bridge failed: .arcana/authority.sock not found"
                                })
                            };
                            app.viewport.finish_latest_tool_call(
                                format_authority_tool_result(&response),
                                started_at.elapsed().as_millis() as u64,
                            );
                            responses.push(response);
                        }
                        app.viewport.add_sub_separator();

                        let response_text = responses
                            .into_iter()
                            .map(|response| response.to_string())
                            .collect::<Vec<_>>()
                            .join("\n");
                        conversation.push(serde_json::json!({
                            "role": "user",
                            "content": format!("AAS returned these JSON responses, one per line:\n{response_text}\n\nContinue the task using these results. If a response is denied or aborted, report it and stop that operation.")
                        }));

                        app.viewport.is_streaming = true;
                        app.generation_broken = false;
                        app.stream_started_at = Some(chrono::Utc::now());
                        app.stream_handle = Some(crate::llm::spawn_stream(
                            &config,
                            conversation.clone(),
                            event_tx.clone(),
                        ));
                        continue;
                    }

                    app.stream_handle = None;
                }
                AppEvent::LlmError(err) => {
                    app.stream_handle = None;
                    app.handle_llm_error(err);
                }
                AppEvent::Toast { message, detail } => {
                    app.toasts.push(Toast {
                        message,
                        detail,
                        created_at: chrono::Utc::now(),
                    });
                }
                AppEvent::Tick => {
                    let now = chrono::Utc::now();
                    app.toasts
                        .retain(|t| (now - t.created_at).num_seconds() < 5);
                }
                // Overlay (query agent) events
                AppEvent::OverlayToken(token) => {
                    if app.generation_broken {
                        continue;
                    }
                    if let Some(think_text) = token.strip_prefix("\x00THINK:") {
                        app.overlay.append_think_token(think_text);
                    } else {
                        app.overlay.append_token(&token);
                    }
                }
                AppEvent::OverlayThinkStart => {
                    if app.generation_broken {
                        continue;
                    }
                    app.overlay.start_thinking();
                }
                AppEvent::OverlayThinkEnd => {
                    if app.generation_broken {
                        continue;
                    }
                    app.overlay.end_thinking();
                }
                AppEvent::OverlayResponseComplete => {
                    if app.generation_broken {
                        let elapsed = app
                            .stream_started_at
                            .map(|t| {
                                let dur = chrono::Utc::now() - t;
                                format!("{:.1}s", dur.num_milliseconds() as f64 / 1000.0)
                            })
                            .unwrap_or_else(|| "?".into());
                        app.overlay.messages.push(Message {
                            role: MessageRole::System,
                            content: format!("[Break Generation]\nTime: {elapsed}"),
                            timestamp: chrono::Utc::now(),
                            thinking: None,
                            tool_calls: Vec::new(),
                            separator: None,
                        });
                        app.generation_broken = false;
                        app.stream_started_at = None;
                        app.stream_handle = None;
                        app.overlay.is_streaming = false;
                        continue;
                    }
                    app.overlay.finalize_response();
                    app.stream_handle = None;
                }
                AppEvent::OverlayError(msg) => {
                    app.stream_handle = None;
                    app.overlay.is_streaming = false;
                    app.overlay.messages.push(Message {
                        role: MessageRole::System,
                        content: format!("⚠ {}", msg),
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                        tool_calls: Vec::new(),
                        separator: None,
                    });
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    tui.restore()?;
    println!("Session ended.");
    Ok(())
}

/// Run a single-shot query (non-interactive).
pub async fn single_shot(
    query: &str,
    model: &Option<String>,
    provider: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let model_name = model.as_ref().unwrap_or(&config.agents.main.model);
    let provider_name = provider.as_ref().unwrap_or(&config.agents.main.provider);

    println!("[Arcana] Model: {} ({})", model_name, provider_name);
    println!("[Arcana] Query: {}", query);
    println!();

    let _authority_daemon = ensure_authority_daemon(true)?;

    // Resolve API key (env var takes priority over literal "$VAR" in config)
    let api_key = config.resolve_api_key(provider_name).ok_or_else(|| {
        format!(
            "No API key found for provider '{}'. Set the appropriate env var.",
            provider_name
        )
    })?;

    // Resolve base URL
    let base_url = match provider_name.as_str() {
        "deepseek" => {
            let url = &config.providers.deepseek.base_url;
            if url.is_empty() {
                "https://api.deepseek.com".to_string()
            } else {
                url.clone()
            }
        }
        "openai" => {
            let url = &config.providers.openai.base_url;
            if url.is_empty() {
                "https://api.openai.com/v1".to_string()
            } else {
                url.clone()
            }
        }
        "anthropic" => {
            let url = &config.providers.anthropic.base_url;
            if url.is_empty() {
                "https://api.anthropic.com".to_string()
            } else {
                url.clone()
            }
        }
        _ => return Err(format!("Unsupported provider: {}", provider_name).into()),
    };

    let authority_context = authority_context_for_query(query, &config).await;
    let user_content = if authority_context.is_empty() {
        query.to_string()
    } else {
        format!("{authority_context}\n\nUser query:\n{query}")
    };

    // Build request body
    let thinking_config = &config.agents.main.thinking;
    let client = reqwest::Client::new();
    let mut messages = vec![
        serde_json::json!({"role": "system", "content": build_system_prompt(AgentMode::Agent)}),
        serde_json::json!({"role": "user", "content": user_content}),
    ];
    let mut last_usage = None;

    for turn in 0..6 {
        let data = send_single_shot_chat(
            &client,
            &base_url,
            &api_key,
            model_name,
            &messages,
            thinking_config.enabled,
            &thinking_config.reasoning_effort,
        )
        .await?;

        if let Some(usage) = data.get("usage") {
            last_usage = Some(usage.clone());
        }

        if let Some(reasoning) = data["choices"][0]["message"]["reasoning_content"].as_str() {
            if !reasoning.is_empty() {
                println!("\x1b[2m<thinking>\n{}\n</thinking>\x1b[0m\n", reasoning);
            }
        }

        let content = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let requests = extract_authority_json_requests(&content);
        if requests.is_empty() {
            println!("{}", content);
            break;
        }

        messages.push(serde_json::json!({"role": "assistant", "content": content}));

        let socket_path = Path::new(".arcana/authority.sock");
        let mut responses = Vec::new();
        for request in requests {
            let response = if socket_path.exists() {
                match authority_request(socket_path, request.clone()) {
                    Ok(response) => response,
                    Err(e) => serde_json::json!({
                        "status": "denied",
                        "reason": format!("AAS bridge failed: {e}")
                    }),
                }
            } else {
                serde_json::json!({
                    "status": "denied",
                    "reason": "AAS bridge failed: .arcana/authority.sock not found"
                })
            };
            println!("[Arcana] AAS {} -> {}", request, response);
            responses.push(response);
        }

        let response_text = responses
            .into_iter()
            .map(|response| response.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        messages.push(serde_json::json!({
            "role": "user",
            "content": format!("AAS returned these JSON responses, one per line:\n{response_text}\n\nContinue the task using these results. If a response is denied or aborted, report it and stop that operation.")
        }));

        if turn == 5 {
            println!("AAS bridge stopped after too many authority-request turns.");
        }
    }

    if let Some(usage) = last_usage {
        let input = usage["prompt_tokens"].as_u64().unwrap_or(0);
        let output = usage["completion_tokens"].as_u64().unwrap_or(0);
        println!("\n\x1b[2m[tokens: {} in / {} out]\x1b[0m", input, output);
    }

    Ok(())
}

async fn send_single_shot_chat(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model_name: &str,
    messages: &[serde_json::Value],
    thinking_enabled: bool,
    reasoning_effort: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut body = serde_json::json!({
        "model": model_name,
        "messages": messages,
        "stream": false
    });
    if thinking_enabled {
        body["thinking"] = serde_json::json!({"type": "enabled"});
        body["reasoning_effort"] = serde_json::json!(reasoning_effort);
    }

    let resp = client
        .post(format!("{}/chat/completions", base_url))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API error ({}): {}", status, text).into());
    }

    Ok(resp.json().await?)
}

fn build_system_prompt(mode: AgentMode) -> String {
    match mode {
        AgentMode::Ask => "You are a professional research assistant. \
             Answer profoundly, pedagogically, and concisely."
            .to_string(),
        AgentMode::Agent => {
            // 1. Structured authority config (generated by AAS daemon)
            let authority_config =
                fs::read_to_string(".arcana/authorized_prompt.md").unwrap_or_default();

            // 2. AAS API reference (pure, no behavioral instructions)
            let instruction = crate::instruction::load_or_create().unwrap_or_default();

            // 3. Behavioral line — tells the LLM when to use tools (user-editable)
            let behavior = crate::behavioral::load_or_create().unwrap_or_default();

            let mut parts: Vec<String> = Vec::new();
            if !authority_config.trim().is_empty() {
                parts.push(authority_config.trim().to_string());
            }
            if !instruction.trim().is_empty() {
                parts.push(instruction.trim().to_string());
            }
            parts.push(behavior.trim().to_string());
            parts.join("\n\n")
        }
    }
}

fn refresh_authorized_prompt_file() {
    let socket_path = Path::new(".arcana/authority.sock");
    if !socket_path.exists() {
        return;
    }
    let Ok(response) = authority_request(socket_path, serde_json::json!({"op": "prompt"})) else {
        return;
    };
    let Some(content) = response.get("content").and_then(|content| content.as_str()) else {
        return;
    };
    if let Some(parent) = Path::new(".arcana/authorized_prompt.md").parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(".arcana/authorized_prompt.md", content);
}

async fn authority_context_for_query(query: &str, _config: &Config) -> String {
    let urls = extract_urls(query);
    if urls.is_empty() {
        return String::new();
    }

    let socket_path = Path::new(".arcana/authority.sock");
    let mut sections = Vec::new();
    for url in urls {
        if !socket_path.exists() {
            sections.push(format!(
                "Source: {url}\nFetch skipped: authority socket not found"
            ));
            continue;
        }

        match authority_url_context(socket_path, &url).await {
            Ok(Some(section)) => sections.push(section),
            Ok(None) => {}
            Err(auth_err) => sections.push(format!("Source: {url}\nFetch failed: {auth_err}")),
        }
    }

    if sections.is_empty() {
        String::new()
    } else {
        format!("Authority-fetched context:\n\n{}", sections.join("\n\n"))
    }
}

#[derive(Clone)]
struct ApprovedAuthorityRequest {
    request: serde_json::Value,
    tool_type: ToolType,
    description: String,
    action: Option<String>,
}

enum AuthorityApproval {
    Approved(ApprovedAuthorityRequest),
    Aborted {
        response: serde_json::Value,
        tool_type: ToolType,
        description: String,
        action: Option<String>,
    },
}

/// Fetch a URL via the authority daemon over its unix socket.
async fn authority_url_context(
    socket_path: &Path,
    url: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let resp = authority_request(
        socket_path,
        serde_json::json!({"op": "fetch", "url": url, "tag": null}),
    )?;

    if resp["status"] == "denied" {
        let reason = resp["reason"].as_str().unwrap_or("unknown reason");
        return Ok(Some(format!(
            "Source: {url}\nDenied by authority: {reason}"
        )));
    }

    if resp["status"] != "fetched" {
        return Ok(None);
    }

    let Some(path) = resp["path"].as_str() else {
        return Ok(None);
    };

    let bytes = fs::read(PathBuf::from(path))?;
    let text = if looks_like_pdf(&bytes) {
        format!("[PDF fetched: {} bytes]", bytes.len())
    } else {
        clean_html_for_llm(&String::from_utf8_lossy(&bytes))
    };

    Ok(Some(format!("Source: {url}\n\n{text}")))
}

/// Clean raw HTML before throwing it into the LLM conversation.
///
/// Strategy:
/// 1. Remove entire blocks that are never meaningful: script, style, noscript, svg,
///    nav, footer, header, aside, iframe, form, template, link, meta.
/// 2. Remove HTML comments.
/// 3. Extract \<title\> and meta-description as a header.
/// 4. Strip all remaining tags, collapse whitespace, decode entities.
/// 5. Remove very short lines that look like navigation boilerplate.
/// 6. Truncate to a generous cap to avoid blowing up the context window.
fn clean_html_for_llm(raw: &str) -> String {
    let mut html = raw.to_string();

    // --- Step 1: remove whole-tag blocks that carry zero meaning ---
    for (start, end) in &[
        ("<script", "</script>"),
        ("<style", "</style>"),
        ("<noscript", "</noscript>"),
        ("<svg", "</svg>"),
        ("<nav", "</nav>"),
        ("<footer", "</footer>"),
        ("<header", "</header>"),
        ("<aside", "</aside>"),
        ("<iframe", "</iframe>"),
        ("<form", "</form>"),
        ("<template", "</template>"),
    ] {
        html = remove_html_block(&html, start, end);
    }

    // Remove self-closing / single tags that are pure metadata or styling
    for tag in &[
        "<link ", "<meta ", "<base ", "<input ", "<br ", "<hr ", "<img ", "<source ",
    ] {
        html = remove_self_closing_tags(&html, tag);
    }

    // --- Step 2: remove HTML comments ---
    html = remove_html_comments(&html);

    // --- Step 3: extract useful metadata before stripping tags ---
    let metadata = extract_page_metadata(&html);

    // --- Step 4: strip all remaining HTML tags, collapse whitespace ---
    let body_text = strip_tags_collapse_whitespace(&html);

    // --- Step 5: remove likely-navigation lines ---
    let cleaned_body = filter_noise_lines(&body_text);

    // --- Step 6: decode entities ---
    let decoded = decode_html_entities(&cleaned_body);

    // --- Step 7: truncate to a generous cap ---
    let capped = truncate_chars(&decoded, 80_000);

    // --- Assemble: metadata header + body ---
    if metadata.is_empty() {
        capped
    } else {
        format!("{metadata}\n\n{capped}")
    }
}

// ---------------------------------------------------------------------------
// HTML cleaning helpers
// ---------------------------------------------------------------------------

/// Remove blocks delimited by start_pat … end_pat (case‑insensitive).
fn remove_html_block(input: &str, start_pat: &str, end_pat: &str) -> String {
    let mut out = input.to_string();
    loop {
        let lower = out.to_lowercase();
        let Some(start) = lower.find(&start_pat.to_lowercase()) else {
            break;
        };
        let Some(end_rel) = lower[start..].find(&end_pat.to_lowercase()) else {
            out.replace_range(start.., " ");
            break;
        };
        let end = start + end_rel + end_pat.len();
        out.replace_range(start..end, " ");
    }
    out
}

/// Remove self‑closing / void tags like `<link ...>`, `<meta ...>`, `<br>`, etc.
fn remove_self_closing_tags(input: &str, tag_start: &str) -> String {
    let mut out = input.to_string();
    let tag_lower = tag_start.to_lowercase();
    loop {
        let lower = out.to_lowercase();
        let Some(pos) = lower.find(&tag_lower) else {
            break;
        };
        // Find the closing >
        let rest = &lower[pos + tag_lower.len()..];
        let Some(end_off) = rest.find('>') else { break };
        out.replace_range(pos..pos + tag_lower.len() + end_off + 1, " ");
    }
    out
}

/// Remove `<!-- ... -->` comments.
fn remove_html_comments(input: &str) -> String {
    let mut out = input.to_string();
    loop {
        let Some(start) = out.find("<!--") else { break };
        let Some(end_rel) = out[start..].find("-->") else {
            break;
        };
        out.replace_range(start..start + end_rel + 3, " ");
    }
    out
}

/// Pull out \<title\> and common meta descriptions before tag‑stripping.
fn extract_page_metadata(html: &str) -> String {
    let mut parts: Vec<String> = Vec::new();

    // <title>
    if let Some(raw) = extract_between_case_insensitive(html, "<title", "</title>") {
        let text = raw.split_once('>').map(|(_, v)| v).unwrap_or(&raw);
        let text = decode_html_entities(text.trim());
        if !text.is_empty() {
            parts.push(format!("Title: {text}"));
        }
    }

    // <meta name="description" ...> and og:title/description
    let lower = html.to_lowercase();
    let mut search_from = 0;
    while let Some(idx) = lower[search_from..].find("<meta") {
        let start = search_from + idx;
        let Some(end_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + end_rel + 1;
        let tag = &html[start..end];
        let tag_lower = &lower[start..end];
        if (tag_lower.contains("name=\"description\"")
            || tag_lower.contains("name='description'")
            || tag_lower.contains("property=\"og:title\"")
            || tag_lower.contains("property='og:title'")
            || tag_lower.contains("property=\"og:description\"")
            || tag_lower.contains("property='og:description'"))
            && let Some(content) = extract_attr(tag, "content")
        {
            let content = decode_html_entities(content.trim());
            if !content.is_empty() {
                parts.push(content);
            }
        }
        search_from = end;
    }

    parts.join("\n")
}

/// Find text between two case‑insensitive patterns.
fn extract_between_case_insensitive(input: &str, start_pat: &str, end_pat: &str) -> Option<String> {
    let lower = input.to_lowercase();
    let start = lower.find(&start_pat.to_lowercase())?;
    let end_rel = lower[start..].find(&end_pat.to_lowercase())?;
    Some(input[start..start + end_rel].to_string())
}

/// Extract the value of an HTML attribute (e.g. `content="…"`).
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let needle = format!("{attr}=");
    let idx = lower.find(&needle)?;
    let rest = &tag[idx + needle.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_start = quote.len_utf8();
    let value_end = rest[value_start..].find(quote)?;
    Some(rest[value_start..value_start + value_end].to_string())
}

/// Strip all HTML tags: replace `<...>` with a space, then collapse runs of whitespace.
fn strip_tags_collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(64_000));
    let mut in_tag = false;
    let mut last_was_space = false;
    let mut last_was_newline = false;

    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                // Emit a space before a tag if we're in text (prevents word‑gluing)
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            '>' => {
                in_tag = false;
            }
            _ if in_tag => {}
            '\n' | '\r' => {
                if !last_was_newline {
                    out.push('\n');
                    last_was_newline = true;
                }
                last_was_space = true;
            }
            c if c.is_whitespace() => {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
                last_was_newline = false;
            }
            c => {
                out.push(c);
                last_was_space = false;
                last_was_newline = false;
            }
        }
    }
    out
}

/// Remove lines that look like navigation / footer / cookie‑banner boilerplate.
///
/// A line is considered noise when:
/// - it is ≤ 3 "words" (separated by whitespace) and ≤ 60 characters, AND
/// - it matches common boilerplate patterns.
fn filter_noise_lines(text: &str) -> String {
    let noise_patterns = &[
        "cookie",
        "privacy policy",
        "terms of service",
        "terms of use",
        "sign in",
        "log in",
        "login",
        "sign up",
        "register",
        "subscribe",
        "contact us",
        "about us",
        "help center",
        "faq",
        "accessibility",
        "all rights reserved",
        "copyright ©",
        "copyright 20",
        "powered by",
        "follow us",
        "share this",
        "print page",
        "back to top",
        "scroll to top",
        "skip to content",
        "skip to main",
        "toggle navigation",
        "open menu",
        "close menu",
        "main menu",
        "home page",
        "site map",
        "rss feed",
        "atom feed",
        "advertisement",
        "sponsored",
        "related articles",
        "you might also like",
        "recommended for you",
        "leave a comment",
        "add comment",
        "reply",
        "previous page",
        "next page",
        "page 1 of",
        "this site uses cookies",
        "we use cookies",
        "accept cookies",
        "cookie settings",
        "manage cookies",
    ];

    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        let word_count = trimmed.split_whitespace().count();
        let is_short = word_count <= 3 && trimmed.len() <= 60;
        if is_short {
            let lower = trimmed.to_lowercase();
            if noise_patterns.iter().any(|p| lower.contains(p)) {
                continue; // skip this line
            }
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

/// Decode common HTML entities.
fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&ldquo;", "\u{201c}")
        .replace("&rdquo;", "\u{201d}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&rsquo;", "\u{2019}")
        .replace("&hellip;", "…")
        .replace("&copy;", "©")
        .replace("&reg;", "®")
        .replace("&trade;", "™")
}

/// TUI-based inline confirmation: shows the request in the viewport with a
/// confirmation panel, keeps the UI fully visible, and waits for a keypress.
fn tui_approve_authority_request(
    app: &mut App,
    tui: &mut Tui,
    tx: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    mut request: serde_json::Value,
) -> Result<AuthorityApproval, Box<dyn std::error::Error>> {
    loop {
        let details = authority_request_details(&request);

        // Add tool call with confirmation prompt
        let confirm_idx = app.viewport.messages.iter().rev()
            .find(|m| m.role == MessageRole::Agent)
            .map(|m| m.tool_calls.len())
            .unwrap_or(0);

        app.viewport.add_tool_call(ToolCall {
            tool_type: details.tool_type,
            description: details.target.clone(),
            action: details.action.map(str::to_string),
            result: Some(format!("?  Yes [y/Enter]  |  Edit [e]  |  No [n]")),
            duration_ms: 0,
            collapsed: false,
        });

        // Render with confirmation visible
        tui.draw(|frame| app.render(frame))?;

        // Wait for a key (blocking — we're inside a synchronous function)
        let answer = loop {
            match rx.blocking_recv() {
                Some(AppEvent::Key(key)) => {
                    let action = classify_key(&key);
                    match action {
                        KeyAction::Char('y') | KeyAction::Enter => break "y",
                        KeyAction::Char('n') | KeyAction::Escape => break "n",
                        KeyAction::Char('e') => break "e",
                        _ => {
                            tui.draw(|frame| app.render(frame))?;
                        }
                    }
                }
                Some(_) => {} // ignore ticks, resizes, etc.
                None => break "n",
            }
        };

        // Update the tool call with result
        if let Some(msg) = app.viewport.messages.iter_mut().rev()
            .find(|m| m.role == MessageRole::Agent)
        {
            if let Some(tc) = msg.tool_calls.last_mut() {
                match answer {
                    "y" => {
                        tc.result = Some("OK.".to_string());
                        return Ok(AuthorityApproval::Approved(ApprovedAuthorityRequest {
                            request: confirmed_authority_request(request),
                            tool_type: details.tool_type,
                            description: details.target,
                            action: details.action.map(str::to_string),
                        }));
                    }
                    "e" => {
                        match edit_authority_target(&details.target) {
                            Ok(edited) if !edited.trim().is_empty() => {
                                request = edit_authority_request_target(request, edited.trim());
                                // Remove the temporary tool call and loop again
                                msg.tool_calls.pop();
                                continue;
                            }
                            Ok(_) => {
                                tc.result = Some("Aborted (empty edit).".to_string());
                                return Ok(aborted_authority_approval(
                                    details.tool_type, details.target, details.action,
                                    details.abort_error_type,
                                    format!("{} edit produced an empty request", details.kind),
                                ));
                            }
                            Err(e) => {
                                tc.result = Some(format!("Aborted (edit failed: {e})."));
                                return Ok(aborted_authority_approval(
                                    details.tool_type, details.target, details.action,
                                    details.abort_error_type,
                                    format!("{} edit failed: {e}", details.kind),
                                ));
                            }
                        }
                    }
                    _ => {
                        tc.result = Some("Aborted.".to_string());
                        return Ok(aborted_authority_approval(
                            details.tool_type, details.target.clone(), details.action,
                            details.abort_error_type,
                            format!("{} aborted by user: {}", details.kind, details.target),
                        ));
                    }
                }
            }
        }
        // Fallback
        return Ok(aborted_authority_approval(
            details.tool_type, details.target, details.action,
            details.abort_error_type, "confirmation lost".into(),
        ));
    }
}

fn approve_authority_request(
    mut request: serde_json::Value,
) -> Result<AuthorityApproval, Box<dyn std::error::Error>> {
    loop {
        let details = authority_request_details(&request);
        clear_authority_confirmation_screen();
        eprintln!();
        eprintln!(
            "[{}] LLM requires {} `{}`. Confirm Allowance?",
            details.kind, details.verb, details.target
        );
        if details.kind == "Tool Call" {
            eprintln!("    - Yes and Run [y/Enter]");
        } else {
            eprintln!("    - Yes [y/Enter]");
        }
        eprintln!("    - No and Edit [e]");
        eprint!("    - No and Abort [n/a]: ");
        std::io::stderr().flush().ok();

        let mut input = String::new();
        let answer = if std::io::stdin().read_line(&mut input).is_ok() {
            input.trim().to_ascii_lowercase()
        } else {
            "n".into()
        };

        if answer.is_empty() || answer == "y" || answer == "yes" {
            return Ok(AuthorityApproval::Approved(ApprovedAuthorityRequest {
                request: confirmed_authority_request(request),
                tool_type: details.tool_type,
                description: details.target,
                action: details.action.map(str::to_string),
            }));
        }

        if answer == "e" {
            match edit_authority_target(&details.target) {
                Ok(edited) if !edited.trim().is_empty() => {
                    request = edit_authority_request_target(request, edited.trim());
                    continue;
                }
                Ok(_) => {
                    return Ok(aborted_authority_approval(
                        details.tool_type,
                        details.target,
                        details.action,
                        details.abort_error_type,
                        format!("{} edit produced an empty request", details.kind),
                    ));
                }
                Err(e) => {
                    return Ok(aborted_authority_approval(
                        details.tool_type,
                        details.target,
                        details.action,
                        details.abort_error_type,
                        format!("{} edit failed: {e}", details.kind),
                    ));
                }
            }
        }

        if answer == "n" || answer == "no" || answer == "a" || answer == "abort" {
            return Ok(aborted_authority_approval(
                details.tool_type,
                details.target.clone(),
                details.action,
                details.abort_error_type,
                format!("{} aborted by user: {}", details.kind, details.target),
            ));
        }
    }
}

fn clear_authority_confirmation_screen() {
    eprint!("\x1b[2J\x1b[H");
    std::io::stderr().flush().ok();
}

struct AuthorityRequestDetails {
    kind: &'static str,
    verb: &'static str,
    target: String,
    action: Option<&'static str>,
    tool_type: ToolType,
    abort_error_type: &'static str,
}

fn authority_request_details(request: &serde_json::Value) -> AuthorityRequestDetails {
    match request.get("op").and_then(|op| op.as_str()).unwrap_or("") {
        "exec_shell" => AuthorityRequestDetails {
            kind: "Tool Call",
            verb: "run of",
            target: request["command"].as_str().unwrap_or("").to_string(),
            action: None,
            tool_type: ToolType::Shell,
            abort_error_type: "ToolCallAbortError",
        },
        "exec" => {
            let cmd = request["cmd"].as_str().unwrap_or("");
            let args = request["args"]
                .as_array()
                .map(|args| {
                    args.iter()
                        .filter_map(|arg| arg.as_str())
                        .map(shell_display_arg)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            AuthorityRequestDetails {
                kind: "Tool Call",
                verb: "run of",
                target: if args.is_empty() {
                    cmd.to_string()
                } else {
                    format!("{cmd} {args}")
                },
                action: None,
                tool_type: ToolType::Shell,
                abort_error_type: "ToolCallAbortError",
            }
        }
        "fetch" => AuthorityRequestDetails {
            kind: "Web Access",
            verb: "web fetch/browse of",
            target: request["url"].as_str().unwrap_or("").to_string(),
            action: Some("Fetch"),
            tool_type: ToolType::Web,
            abort_error_type: "WebAccessAbortError",
        },
        "read" | "read_text" | "write" | "write_text" | "delete" => AuthorityRequestDetails {
            kind: "File Access",
            verb: match request["op"].as_str().unwrap_or("") {
                "read" | "read_text" => "read access of",
                "write" | "write_text" => "write access of",
                _ => "delete access of",
            },
            target: request["path"].as_str().unwrap_or("").to_string(),
            action: match request["op"].as_str().unwrap_or("") {
                "read" | "read_text" => Some("Read"),
                "write" | "write_text" => Some("Write"),
                _ => Some("Delete"),
            },
            tool_type: ToolType::File,
            abort_error_type: "FileAccessAbortError",
        },
        "rename" => AuthorityRequestDetails {
            kind: "File Access",
            verb: "rename access of",
            target: format!(
                "{} -> {}",
                request["src"].as_str().unwrap_or(""),
                request["dst"].as_str().unwrap_or("")
            ),
            action: Some("Rename"),
            tool_type: ToolType::File,
            abort_error_type: "FileAccessAbortError",
        },
        "register_command" => AuthorityRequestDetails {
            kind: "Authority Registration",
            verb: "registration of",
            target: request["pattern"].as_str().unwrap_or("").to_string(),
            action: Some("Register"),
            tool_type: ToolType::Other,
            abort_error_type: "ToolRegistrationAbortError",
        },
        "register_web" => AuthorityRequestDetails {
            kind: "Authority Registration",
            verb: "registration of",
            target: request["domain"].as_str().unwrap_or("").to_string(),
            action: Some("Register"),
            tool_type: ToolType::Web,
            abort_error_type: "WebAccessRegistrationAbortError",
        },
        "register_filesystem" => AuthorityRequestDetails {
            kind: "Authority Registration",
            verb: "registration of",
            target: request["path"].as_str().unwrap_or("").to_string(),
            action: Some("Register"),
            tool_type: ToolType::File,
            abort_error_type: "FileAccessRegistrationAbortError",
        },
        "register_tool" => AuthorityRequestDetails {
            kind: "Authority Registration",
            verb: "registration of",
            target: request["path"].as_str().unwrap_or("").to_string(),
            action: Some("Register"),
            tool_type: ToolType::Other,
            abort_error_type: "ToolRegistrationAbortError",
        },
        _ => AuthorityRequestDetails {
            kind: "Authority Request",
            verb: "request of",
            target: request.to_string(),
            action: Some("Request"),
            tool_type: ToolType::Other,
            abort_error_type: "ToolCallAbortError",
        },
    }
}

fn shell_display_arg(arg: &str) -> String {
    if arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '=' | '+')
    }) {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\"'\"'"))
    }
}

fn aborted_authority_approval(
    tool_type: ToolType,
    description: String,
    action: Option<&'static str>,
    error_type: &'static str,
    message: String,
) -> AuthorityApproval {
    AuthorityApproval::Aborted {
        response: serde_json::json!({
            "status": "aborted",
            "error_type": error_type,
            "message": message
        }),
        tool_type,
        description,
        action: action.map(str::to_string),
    }
}

fn confirmed_authority_request(mut request: serde_json::Value) -> serde_json::Value {
    if let Some(op) = request.get("op").and_then(|op| op.as_str()) {
        let confirmed_op = match op {
            "exec" => Some("exec_confirmed"),
            "exec_shell" => Some("exec_shell_confirmed"),
            "fetch" => Some("fetch_confirmed"),
            "write" => Some("write_confirmed"),
            "write_text" => Some("write_text_confirmed"),
            "delete" => Some("delete_confirmed"),
            "rename" => Some("rename_confirmed"),
            "register_tool" => Some("register_tool_confirmed"),
            "register_command" => Some("register_command_confirmed"),
            "register_web" => Some("register_web_confirmed"),
            "register_filesystem" => Some("register_filesystem_confirmed"),
            _ => None,
        };
        if let Some(confirmed_op) = confirmed_op {
            request["op"] = serde_json::json!(confirmed_op);
        }
    }
    request
}

/// Safe read-only commands that never need human confirmation.
const SAFE_COMMANDS: &[&str] = &[
    "echo",
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "find",
    "locate",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "awk",
    "sed",
    "file",
    "stat",
    "du",
    "df",
    "which",
    "type",
    "whereis",
    "pwd",
    "env",
    "printenv",
    "whoami",
    "hostname",
    "uname",
    "date",
    "cal",
    "uptime",
    "ps",
    "top",
    "git diff",
    "git status",
    "git log",
    "git show",
    "git branch",
    "git tag",
    "git remote",
    "git stash list",
    "tree",
];

/// Check whether an authority request can be auto-approved without human confirmation.
fn is_safe_authority_request(request: &serde_json::Value) -> bool {
    let op = request.get("op").and_then(|op| op.as_str()).unwrap_or("");

    // Safe: read-only file operations within project workspace (not .arcana/, /etc, /proc)
    if matches!(op, "read" | "read_text" | "query") {
        if let Some(path) = request.get("path").and_then(|p| p.as_str()) {
            if !path.contains(".arcana") && !path.starts_with("/etc") && !path.starts_with("/proc")
            {
                return true;
            }
        }
    }

    // Safe: well-known read-only shell commands
    if op == "exec_shell" {
        if let Some(command) = request.get("command").and_then(|c| c.as_str()) {
            let first_line = command.lines().next().unwrap_or("").trim();
            return SAFE_COMMANDS.iter().any(|safe| {
                first_line == *safe
                    || first_line.starts_with(&format!("{safe} "))
                    || (safe.starts_with("git ") && first_line == *safe)
            });
        }
    }

    if op == "exec" {
        let cmd = request.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
        return SAFE_COMMANDS.iter().any(|safe| {
            cmd == *safe
                || (safe.starts_with("git ") && cmd == "git" && {
                    let args = request.get("args").and_then(|a| a.as_array());
                    if let Some(args) = args {
                        if let Some(sub) = args.first().and_then(|a| a.as_str()) {
                            return matches!(
                                sub,
                                "diff"
                                    | "status"
                                    | "log"
                                    | "show"
                                    | "branch"
                                    | "tag"
                                    | "remote"
                                    | "stash"
                            );
                        }
                    }
                    false
                })
        });
    }

    false
}

fn authority_command_cache_key(request: &serde_json::Value) -> Option<String> {
    let op = request.get("op").and_then(|op| op.as_str())?;
    let op = op.strip_suffix("_confirmed").unwrap_or(op);
    match op {
        "exec_shell" => {
            let command = request
                .get("command")
                .and_then(|command| command.as_str())?;
            Some(format!("exec_shell\0{command}"))
        }
        "exec" => {
            let cmd = request.get("cmd").and_then(|cmd| cmd.as_str())?;
            let args = request
                .get("args")
                .and_then(|args| serde_json::to_string(args).ok())
                .unwrap_or_else(|| "[]".to_string());
            Some(format!("exec\0{cmd}\0{args}"))
        }
        _ => None,
    }
}

fn edit_authority_request_target(
    mut request: serde_json::Value,
    edited: &str,
) -> serde_json::Value {
    match request.get("op").and_then(|op| op.as_str()).unwrap_or("") {
        "exec_shell" => request["command"] = serde_json::json!(edited),
        "exec" => {
            request = serde_json::json!({"op": "exec_shell", "command": edited});
        }
        "fetch" => request["url"] = serde_json::json!(edited),
        "read" | "read_text" | "write" | "write_text" | "delete" => {
            request["path"] = serde_json::json!(edited)
        }
        "register_command" => request["pattern"] = serde_json::json!(edited),
        "register_web" => request["domain"] = serde_json::json!(edited),
        "register_filesystem" | "register_tool" => request["path"] = serde_json::json!(edited),
        "rename" => {
            if let Some((src, dst)) = edited.split_once("->") {
                request["src"] = serde_json::json!(src.trim());
                request["dst"] = serde_json::json!(dst.trim());
            }
        }
        _ => {}
    }
    request
}

fn edit_authority_target(current: &str) -> Result<String, Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("arcana_authority_request.txt");
    fs::write(&path, current)?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".into());
    let status = std::process::Command::new(editor).arg(&path).status()?;
    if !status.success() {
        return Err("editor exited with non-zero status".into());
    }
    let mut edited = String::new();
    fs::File::open(path)?.read_to_string(&mut edited)?;
    Ok(edited)
}

fn format_authority_tool_result(response: &serde_json::Value) -> String {
    match response.get("status").and_then(|status| status.as_str()) {
        Some("exec_result") => {
            let stdout = response["stdout"].as_str().unwrap_or("");
            let stderr = response["stderr"].as_str().unwrap_or("");
            let diff = response["diff"].as_str().unwrap_or("");
            let mut out = String::new();
            if !stdout.is_empty() {
                out.push_str(stdout.trim_end_matches('\n'));
            }
            if !stderr.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(stderr.trim_end_matches('\n'));
            }
            if !diff.trim().is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(diff.trim_end_matches('\n'));
            }
            out
        }
        Some("mutation") => {
            let mut out = mutation_summary(response);
            let diff = response["diff"].as_str().unwrap_or("");
            if !diff.trim().is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(diff.trim_end_matches('\n'));
            }
            out
        }
        Some("fetched") => format!(
            "fetched: {} ({} bytes)",
            response["path"].as_str().unwrap_or("<unknown>"),
            response["bytes"].as_u64().unwrap_or(0)
        ),
        Some("content") => {
            let bytes = response["data"]
                .as_str()
                .map(|data| data.len())
                .unwrap_or(0);
            format!("content returned: {bytes} base64 characters")
        }
        Some("text") => {
            let chars = response["text"]
                .as_str()
                .map(|text| text.chars().count())
                .unwrap_or(0);
            format!("text returned: {chars} characters")
        }
        Some("ok") => "ok".to_string(),
        Some("denied") => format!(
            "denied: {}",
            response["reason"].as_str().unwrap_or("unknown reason")
        ),
        Some("aborted") => format!(
            "aborted: {}: {}",
            response["error_type"].as_str().unwrap_or("AbortError"),
            response["message"].as_str().unwrap_or("")
        ),
        _ => response.to_string(),
    }
}

fn mutation_summary(response: &serde_json::Value) -> String {
    let Some(records) = response["records"].as_array() else {
        return String::new();
    };
    records
        .iter()
        .filter_map(|record| {
            let seq = record["seq"].as_u64()?;
            let op = record["op"].as_str().unwrap_or("mutation");
            let path = record["path"].as_str().unwrap_or("<unknown>");
            Some(format!("recorded #{seq}: {op} {path}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn authority_request(
    socket_path: &Path,
    req: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path)?;
    writeln!(stream, "{}", req)?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(line.trim())?)
}

struct AuthorityJsonRequestMatch {
    request: serde_json::Value,
    range: Range<usize>,
}

fn extract_authority_json_requests(content: &str) -> Vec<serde_json::Value> {
    extract_authority_json_request_matches(content)
        .into_iter()
        .map(|matched| matched.request)
        .collect()
}

fn display_text_without_authority_requests(content: &str) -> String {
    let matches = extract_authority_json_request_matches(content);
    if matches.is_empty() {
        return content.trim_end_matches('\n').to_string();
    }

    let mut visible = String::new();
    let mut last = 0usize;
    for matched in matches {
        visible.push_str(&content[last..matched.range.start]);
        last = matched.range.end;
    }
    visible.push_str(&content[last..]);

    visible
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && trimmed != "```" && trimmed != "```json"
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end_matches('\n')
        .to_string()
}

fn extract_authority_json_request_matches(content: &str) -> Vec<AuthorityJsonRequestMatch> {
    let mut requests = Vec::new();
    let mut candidate = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0usize;

    for (idx, ch) in content.char_indices() {
        if depth == 0 {
            if ch != '{' {
                continue;
            }
            candidate.clear();
            start = idx;
            in_string = false;
            escaped = false;
        }

        if in_string && ch == '\n' {
            candidate.push_str("\\n");
        } else {
            candidate.push(ch);
        }

        if escaped {
            escaped = false;
            continue;
        }
        if in_string && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }

        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&candidate) {
                        if let Some(request) = normalize_authority_request(value) {
                            requests.push(AuthorityJsonRequestMatch {
                                request,
                                range: start..idx + ch.len_utf8(),
                            });
                        }
                    }
                    candidate.clear();
                    in_string = false;
                    escaped = false;
                }
            }
            _ => {}
        }
    }

    requests
}

fn normalize_authority_request(value: serde_json::Value) -> Option<serde_json::Value> {
    if let Some(op) = value.get("op").and_then(|op| op.as_str()) {
        if op.ends_with("_confirmed") {
            return None;
        }
        return Some(value);
    }

    if value.get("action").and_then(|action| action.as_str()) == Some("execute") {
        let language = value
            .get("language")
            .and_then(|language| language.as_str())?;
        let code = value.get("code").and_then(|code| code.as_str())?;
        return language_execution_command(language, code)
            .map(|command| serde_json::json!({"op": "exec_shell", "command": command}));
    }

    let command = value.get("command").and_then(|command| command.as_str())?;
    let params = value.get("params")?;
    match command {
        "run_terminal_cmd" => {
            let cmd = params.get("cmd").and_then(|cmd| cmd.as_str())?;
            Some(serde_json::json!({"op": "exec_shell", "command": cmd}))
        }
        "read_file" => {
            let path = params.get("path").and_then(|path| path.as_str())?;
            Some(serde_json::json!({"op": "read_text", "path": path}))
        }
        "write_file" => {
            let path = params.get("path").and_then(|path| path.as_str())?;
            let content = params.get("content").and_then(|content| content.as_str())?;
            Some(serde_json::json!({
                "op": "write_text",
                "path": path,
                "content": content
            }))
        }
        "browser" | "fetch" => {
            let url = params
                .get("url")
                .or_else(|| params.get("website"))
                .and_then(|url| url.as_str())?;
            Some(serde_json::json!({"op": "fetch", "url": url, "tag": null}))
        }
        _ => None,
    }
}

fn language_execution_command(language: &str, code: &str) -> Option<String> {
    let executable = match language.to_ascii_lowercase().as_str() {
        "python" | "python3" => "python3",
        "julia" => "julia",
        "bash" | "sh" | "shell" => "sh",
        _ => return None,
    };
    let delimiter = "ARCANA_TOOL_EOF";
    Some(format!(
        "{executable} << '{delimiter}'\n{code}\n{delimiter}"
    ))
}

fn extract_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for token in text.split_whitespace() {
        let url = token.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '<' | '>' | ')' | '(' | ',' | '.' | '?' | '!' | ';' | ':'
            )
        });
        if (url.starts_with("https://") || url.starts_with("http://"))
            && !urls.iter().any(|u| u == url)
        {
            urls.push(url.to_string());
        }
    }
    urls
}

fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_authority_json_from_visible_response() {
        let content =
            "I'll ask AAS.\n{\"op\":\"exec_shell\",\"command\":\"python3 -c \\\"print(1)\\\"\"}\n";

        let requests = extract_authority_json_requests(content);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["op"], "exec_shell");
        assert_eq!(
            display_text_without_authority_requests(content),
            "I'll ask AAS."
        );
    }

    #[test]
    fn extracts_multiline_authority_json_string() {
        let content = "{\"op\":\"exec_shell\",\"command\":\"cat << 'EOF'\nhello\nEOF\"}";

        let requests = extract_authority_json_requests(content);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]["command"].as_str().unwrap(),
            "cat << 'EOF'\nhello\nEOF"
        );
        assert_eq!(display_text_without_authority_requests(content), "");
    }

    #[test]
    fn command_cache_key_matches_confirmed_and_unconfirmed_shell_calls() {
        let request = serde_json::json!({"op":"exec_shell","command":"python3 script.py"});
        let confirmed =
            serde_json::json!({"op":"exec_shell_confirmed","command":"python3 script.py"});

        assert_eq!(
            authority_command_cache_key(&request),
            authority_command_cache_key(&confirmed)
        );
    }

    #[test]
    fn wrapper_write_file_normalizes_to_write_text() {
        let value = serde_json::json!({
            "command": "write_file",
            "params": {
                "path": "notes.md",
                "content": "plain text"
            }
        });

        let request = normalize_authority_request(value).unwrap();
        assert_eq!(request["op"], "write_text");
        assert_eq!(request["content"], "plain text");
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("\n[truncated]");
    out
}

/// Resume a previous session.
pub async fn resume(args: ResumeArgs) -> Result<(), Box<dyn std::error::Error>> {
    if args.last {
        println!("[Arcana] Resuming last session...");
    } else if let Some(ref id) = args.session {
        println!("[Arcana] Resuming session: {}", id);
    } else {
        println!("[Arcana] No session specified. Use --last or provide a session ID.");
    }
    println!("(Session resume — implementation pending)");
    Ok(())
}
