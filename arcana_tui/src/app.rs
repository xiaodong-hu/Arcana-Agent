use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::banner;
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
                if self.composer.input.is_empty() {
                    self.composer.recall_previous();
                } else {
                    self.composer.move_up();
                }
            }
            KeyAction::Down => {
                if self.composer.input.is_empty() {
                    self.composer.recall_next();
                } else {
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
                if self.overlay.composer.input.is_empty() {
                    self.overlay.composer.recall_next();
                } else {
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

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(status_h),
                Constraint::Min(5),
                Constraint::Length(task_panel_h),
                Constraint::Length(composer_h),
            ])
            .split(area);

        status_bar::render_status_bar(
            frame,
            chunks[0],
            &self.theme,
            &self.status,
            &self.panel_state,
            &self.skills,
            &self.agents,
            &self.tasks,
        );

        self.viewport.render(frame, chunks[1], &self.theme);

        panels::render_task_panel(frame, chunks[2], &self.panel_state, &self.tasks);

        self.composer.render(frame, chunks[3], &self.theme);

        if self.mode == ViewMode::QueryOverlay {
            self.overlay.render(frame, area, &self.theme);
        }

        render_toasts(frame, area, &self.toasts);
    }
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

    let mut tui = Tui::new()?;
    let mut app = App::new(&config);

    // Inject banner into viewport as scrollable content
    app.viewport.add_banner(&config.agents.main.model);

    let (mut event_tx, mut events, mut event_handle) = event::spawn_event_reader();

    // Conversation history for LLM context
    let mut conversation: Vec<serde_json::Value> =
        vec![serde_json::json!({"role": "system", "content": system_prompt_with_authority()})];

    loop {
        tui.draw(|frame| app.render(frame))?;

        if let Some(evt) = events.recv().await {
            match evt {
                AppEvent::Key(key) => {
                    let action = classify_key(&key);
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
  \\status        Show model/token info\n\
  \\usage         Session token/cost stats\n\
  \\working_dir   Show current working directory\n\
  \\check         System health check\n\
  \\auth list     Show authorized commands\n\
  \\auth add      Add command to allow list\n\
  \\auth remove   Remove from allow list\n\
  \\auth edit     Open authority.toml in $EDITOR\n\
\n\
Hotkeys:\n\
  Ctrl+e         Open $EDITOR for prompt\n\
  Ctrl+/         Toggle query agent\n\
  Ctrl+o         Expand/collapse thinking chains\n\
  Ctrl+j/k       Scroll viewport down/up\n\
  Ctrl+h/l       Move cursor word left/right\n\
  Ctrl+w         Delete word left\n\
  Ctrl+Up/Down   Jump to start/end of input\n\
  Ctrl+Left/Right  Move cursor by word\n\
  Ctrl+Enter     New line in prompt\n\
  Home/End       Start/end of current line\n\
  Ctrl+b         Stop LLM generation\n\
  Ctrl+c         Clear prompt\n\
  \\quit          Exit session"
                                                .into(),
                                        );
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
                                    "\\auth" | "\\auth list" => {
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
                                    cmd if cmd.starts_with("\\auth add ") => {
                                        let pattern =
                                            cmd.strip_prefix("\\auth add ").unwrap().trim();
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
                                    cmd if cmd.starts_with("\\auth remove ") => {
                                        let pattern =
                                            cmd.strip_prefix("\\auth remove ").unwrap().trim();
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
                                    "\\auth edit" => {
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

                    app.viewport.finalize_response_with_stats(stats);
                    app.viewport.add_separator();

                    // Append assistant message to conversation (with reasoning for cache)
                    let mut msg = serde_json::json!({
                        "role": "assistant",
                        "content": response_text
                    });
                    if let Some(thinking) = thinking_text {
                        msg["reasoning_content"] = serde_json::json!(thinking);
                    }
                    conversation.push(msg);
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

    println!("[arcana] Model: {} ({})", model_name, provider_name);
    println!("[arcana] Query: {}", query);
    println!();

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
        serde_json::json!({"role": "system", "content": system_prompt_with_authority()}),
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
            println!("[arcana] AAS {} -> {}", request, response);
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

fn system_prompt_with_authority() -> String {
    let base = "You are a helpful assistant.\n\nArcana-Agent AAS bridge: you cannot open the authority socket yourself. To call the Arcana Authority System, output one JSON object per line using the documented AAS API, with no markdown wrapper. Arcana-Agent will relay those JSON lines to AAS, return the JSON responses to you, and then you must continue from the returned results. For natural-language requests that require running code, computing with a program, inspecting or changing local files, fetching URLs, or using external commands, call AAS yourself before answering. If the user asks you to write a script for a concrete input, verify it by running it through AAS unless the user explicitly says not to run it. If AAS returns an aborted or denied response, report it and stop that operation.";
    match fs::read_to_string(".arcana/authorized_prompt.md") {
        Ok(prompt) => format!("{}\n\n{}", prompt.trim_end(), base),
        Err(_) => base.to_string(),
    }
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

fn extract_authority_json_requests(content: &str) -> Vec<serde_json::Value> {
    let mut requests = Vec::new();

    let mut candidate = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in content.chars() {
        if depth == 0 {
            if ch != '{' {
                continue;
            }
            candidate.clear();
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
                        if value.get("op").and_then(|op| op.as_str()).is_some() {
                            requests.push(value);
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
        println!("[arcana] Resuming last session...");
    } else if let Some(ref id) = args.session {
        println!("[arcana] Resuming session: {}", id);
    } else {
        println!("[arcana] No session specified. Use --last or provide a session ID.");
    }
    println!("(Session resume — implementation pending)");
    Ok(())
}
