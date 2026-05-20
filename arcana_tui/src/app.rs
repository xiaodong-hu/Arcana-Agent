use ratatui::prelude::*;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::banner;
use crate::cli::ResumeArgs;
use crate::composer::Composer;
use crate::config::Config;
use crate::event::{self, AppEvent, KeyAction, classify_key};
use crate::overlay::QueryOverlay;
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
            KeyAction::Char('?') if self.composer.is_empty() => {
                self.overlay.show();
                self.mode = ViewMode::QueryOverlay;
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
                if !self.composer.is_empty() {
                    let input = self.composer.take_input();
                    self.viewport.add_user_message(input);
                    self.show_banner = false;
                }
            }
            KeyAction::Newline => { self.composer.insert_newline(); }
            KeyAction::Backspace => { self.composer.backspace(); }
            KeyAction::Delete => { self.composer.delete(); }
            KeyAction::Left => { self.composer.move_left(); }
            KeyAction::Right => { self.composer.move_right(); }
            KeyAction::Up => {
                if self.composer.is_empty() { self.composer.recall_previous(); }
                else { self.viewport.scroll_up(1); }
            }
            KeyAction::Down => { self.viewport.scroll_down(1); }
            KeyAction::PageUp => { self.viewport.scroll_up(20); }
            KeyAction::PageDown => { self.viewport.scroll_down(20); }
            KeyAction::Home => { self.viewport.scroll_to_top(10000); }
            KeyAction::End => { self.viewport.scroll_to_bottom(); }
            KeyAction::HalfPageUp => { self.viewport.scroll_up(10); }
            KeyAction::HalfPageDown => { self.viewport.scroll_down(10); }
            KeyAction::Interrupt => {
                if !self.composer.is_empty() { self.composer.clear(); }
            }
            KeyAction::EndSession => { self.should_quit = true; }
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
            KeyAction::Char('q') if self.overlay.composer.is_empty() => {
                self.overlay.hide();
                self.mode = ViewMode::Main;
            }
            KeyAction::Escape => {
                self.overlay.hide();
                self.mode = ViewMode::Main;
            }
            KeyAction::Char(c) => { self.overlay.composer.insert_char(c); }
            KeyAction::Enter => {
                if !self.overlay.composer.is_empty() {
                    let input = self.overlay.composer.take_input();
                    self.overlay.messages.push(Message {
                        role: MessageRole::User,
                        content: input,
                        timestamp: chrono::Utc::now(),
                        thinking: None,
                        tool_calls: Vec::new(),
                    });
                }
            }
            KeyAction::Newline => { self.overlay.composer.insert_newline(); }
            KeyAction::Backspace => { self.overlay.composer.backspace(); }
            KeyAction::Interrupt => { self.overlay.composer.clear(); }
            _ => {}
        }
    }

    fn handle_llm_error(&mut self, err: LlmError) {
        let msg = format!("{}", err);
        let detail = match &err {
            LlmError::RateLimit { retry_after_secs: Some(s), .. } => {
                Some(format!("Will retry in {}s. Consider reducing request frequency.", s))
            }
            LlmError::RateLimit { .. } => {
                Some("Rate limit reached. Wait before sending more requests.".into())
            }
            _ => None,
        };
        // Show as error toast
        self.toasts.push(Toast { message: msg, detail, created_at: chrono::Utc::now() });
        // Also append to viewport as system error message
        self.viewport.add_error_message(format!("{}", err));
    }

    fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        let banner_h = if self.show_banner { banner::banner_height(area.width) } else { 0 };
        let status_h = status_bar::status_bar_height(
            &self.panel_state, &self.skills, &self.agents, &self.tasks,
        );
        let composer_h = self.composer.height();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(banner_h),
                Constraint::Length(status_h),
                Constraint::Min(5),
                Constraint::Length(composer_h),
            ])
            .split(area);

        if self.show_banner && banner_h > 0 {
            banner::render_banner(frame, chunks[0], &self.theme, &self.status);
        }

        status_bar::render_status_bar(
            frame, chunks[1], &self.theme, &self.status,
            &self.panel_state, &self.skills, &self.agents, &self.tasks,
        );

        self.viewport.render(frame, chunks[2], &self.theme);
        self.composer.render(frame, chunks[3], &self.theme);

        if self.mode == ViewMode::QueryOverlay {
            self.overlay.render(frame, area, &self.theme);
        }

        render_toasts(frame, area, &self.toasts);
    }
}

fn render_toasts(frame: &mut Frame, area: Rect, toasts: &[Toast]) {
    let now = chrono::Utc::now();
    let visible: Vec<&Toast> = toasts.iter()
        .filter(|t| (now - t.created_at).num_seconds() < 5)
        .collect();

    for (i, toast) in visible.iter().enumerate() {
        let width = (toast.message.len() as u16 + 4).min(area.width.saturating_sub(4));
        let height: u16 = if toast.detail.is_some() { 3 } else { 2 };
        let x = area.width.saturating_sub(width + 2);
        let y = 1 + (i as u16 * (height + 1));
        if y + height > area.height { break; }

        let toast_area = Rect::new(x, y, width, height);

        // Use red border for error toasts (those containing "error" or "limit")
        let border_color = if toast.message.contains("error") || toast.message.contains("limit") || toast.message.contains("Rate") {
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
    let mut events = event::spawn_event_reader();

    loop {
        tui.draw(|frame| app.render(frame))?;

        if let Some(evt) = events.recv().await {
            match evt {
                AppEvent::Key(key) => {
                    let action = classify_key(&key);
                    match app.mode {
                        ViewMode::Main => app.handle_main_key(action),
                        ViewMode::QueryOverlay => app.handle_overlay_key(action),
                        ViewMode::DiffReview => {}
                    }
                }
                AppEvent::Resize(_, _) => {}
                AppEvent::Token(token) => { app.viewport.append_token(&token); }
                AppEvent::ThinkStart => { app.viewport.start_thinking(); }
                AppEvent::ThinkEnd => { app.viewport.end_thinking(); }
                AppEvent::ResponseComplete(stats) => { app.viewport.finalize_response_with_stats(stats); }
                AppEvent::LlmError(err) => { app.handle_llm_error(err); }
                AppEvent::Toast { message, detail } => {
                    app.toasts.push(Toast { message, detail, created_at: chrono::Utc::now() });
                }
                AppEvent::Tick => {
                    let now = chrono::Utc::now();
                    app.toasts.retain(|t| (now - t.created_at).num_seconds() < 5);
                }
                _ => {}
            }
        }

        if app.should_quit { break; }
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
    println!("(Single-shot mode — LLM integration pending)");
    Ok(())
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
