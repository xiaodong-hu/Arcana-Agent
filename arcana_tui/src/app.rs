use ratatui::prelude::*;
use ratatui::layout::{Constraint, Direction, Layout};

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
            KeyAction::Newline => { self.composer.insert_newline(); }
            KeyAction::Tab => { self.composer.autocomplete_or_tab(); }
            KeyAction::Backspace => { self.composer.backspace(); }
            KeyAction::Delete => { self.composer.delete(); }
            KeyAction::Left => { self.composer.move_left(); }
            KeyAction::Right => { self.composer.move_right(); }
            KeyAction::WordLeft => { self.composer.move_word_left(); }
            KeyAction::WordRight => { self.composer.move_word_right(); }
            KeyAction::Home => { self.composer.move_home(); }
            KeyAction::End => { self.composer.move_end(); }
            KeyAction::Up => {
                if self.composer.is_empty() {
                    self.composer.recall_previous();
                } else if self.composer.history_index.is_some() {
                    self.composer.recall_previous();
                } else if !self.composer.move_up() {
                    // Already on first line, do nothing
                }
            }
            KeyAction::Down => {
                if self.composer.history_index.is_some() {
                    self.composer.recall_next();
                } else if !self.composer.is_empty() {
                    self.composer.move_down();
                }
            }
            KeyAction::PageUp => { self.viewport.scroll_up(20); }
            KeyAction::PageDown => { self.viewport.scroll_down(20); }
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
            KeyAction::Escape | KeyAction::ToggleQuery => {
                self.overlay.hide();
                self.mode = ViewMode::Main;
            }
            KeyAction::Expand => {
                // Ctrl+O: toggle thinking in overlay
                self.overlay.toggle_thinking();
            }
            KeyAction::Char(c) => { self.overlay.composer.insert_char(c); }
            KeyAction::Tab => { self.overlay.composer.insert_tab(); }
            KeyAction::Enter => {
                // Enter sends the query — handled in event loop
            }
            KeyAction::Newline => { self.overlay.composer.insert_newline(); }
            KeyAction::Backspace => { self.overlay.composer.backspace(); }
            KeyAction::Delete => { self.overlay.composer.delete(); }
            KeyAction::Left => { self.overlay.composer.move_left(); }
            KeyAction::Right => { self.overlay.composer.move_right(); }
            KeyAction::Home => { self.overlay.composer.move_home(); }
            KeyAction::End => { self.overlay.composer.move_end(); }
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

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let banner_h = if self.show_banner { banner::banner_height(area.width) } else { 0 };
        let status_h = status_bar::status_bar_height(
            &self.panel_state, &self.skills, &self.agents, &self.tasks,
        );
        let task_panel_h = panels::task_panel_height(&self.panel_state, &self.tasks);
        let composer_h = self.composer.height_for_width(area.width).min(area.height / 2);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(banner_h),
                Constraint::Length(status_h),
                Constraint::Min(5),
                Constraint::Length(task_panel_h),
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

        panels::render_task_panel(frame, chunks[3], &self.panel_state, &self.tasks);

        self.composer.render(frame, chunks[4], &self.theme);

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
    let (mut event_tx, mut events, mut event_handle) = event::spawn_event_reader();

    // Conversation history for LLM context
    let mut conversation: Vec<serde_json::Value> = vec![
        serde_json::json!({"role": "system", "content": "You are a helpful assistant."})
    ];

    loop {
        tui.draw(|frame| app.render(frame))?;

        if let Some(evt) = events.recv().await {
            match evt {
                AppEvent::Key(key) => {
                    let action = classify_key(&key);
                    match app.mode {
                        ViewMode::Main => {
                            // Check if this is an Enter that will send a message
                            if action == KeyAction::Enter && !app.composer.is_empty() {
                                let input = app.composer.take_input();
                                // Handle slash commands
                                let is_command = input.starts_with('\\');
                                match input.trim() {
                                    "\\quit" | "\\q" => { app.should_quit = true; }
                                    "\\clear" => {
                                        app.viewport.messages.clear();
                                        conversation.truncate(1); // keep system msg
                                    }
                                    "\\help" => {
                                        app.viewport.add_error_message(
                                            "\\quit · \\clear · \\status · \\usage · \\check\n\
                                             \\auth list|add|remove|edit\n\
                                             Ctrl+/ query · Ctrl+t tasks · Ctrl+o thinking · Ctrl+d exit".into()
                                        );
                                    }
                                    "\\status" => {
                                        app.viewport.add_error_message(format!(
                                            "Model: {} │ Tokens: {}/{} │ Tasks: {}",
                                            app.status.model_name, app.status.tokens_used,
                                            app.status.tokens_max, app.tasks.len()
                                        ));
                                    }
                                    "\\usage" => {
                                        let in_str = format_token_count(app.status.session_input_tokens);
                                        let out_str = format_token_count(app.status.session_output_tokens);
                                        app.viewport.add_error_message(format!(
                                            "Session Usage:\n  Requests: {}\n  Tokens: {} in / {} out\n  Total cost: {:.4}",
                                            app.status.session_requests, in_str, out_str, app.status.session_cost
                                        ));
                                    }
                                    "\\auth" | "\\auth list" => {
                                        let path = dirs::home_dir().unwrap_or_default().join(".arcana/authority.toml");
                                        if path.exists() {
                                            if let Ok(content) = std::fs::read_to_string(&path) {
                                                app.viewport.add_error_message(format!(
                                                    "Authority config: {}\n\n{}", path.display(), content
                                                ));
                                            }
                                        } else {
                                            app.viewport.add_error_message(
                                                "No authority.toml found. Run: arcana onboard".into()
                                            );
                                        }
                                    }
                                    cmd if cmd.starts_with("\\auth add ") => {
                                        let pattern = cmd.strip_prefix("\\auth add ").unwrap().trim();
                                        let path = dirs::home_dir().unwrap_or_default().join(".arcana/authority.toml");
                                        if let Ok(content) = std::fs::read_to_string(&path) {
                                            // Simple append to [commands] allow list
                                            let new = content.replace(
                                                "]\n\n# Commands that always require confirmation",
                                                &format!("    \"{}\",\n]\n\n# Commands that always require confirmation", pattern)
                                            );
                                            let _ = std::fs::write(&path, &new);
                                            app.viewport.add_error_message(format!("✓ Added to allow: {}", pattern));
                                        }
                                    }
                                    cmd if cmd.starts_with("\\auth remove ") => {
                                        let pattern = cmd.strip_prefix("\\auth remove ").unwrap().trim();
                                        let path = dirs::home_dir().unwrap_or_default().join(".arcana/authority.toml");
                                        if let Ok(content) = std::fs::read_to_string(&path) {
                                            let needle = format!("    \"{}\",\n", pattern);
                                            let new = content.replace(&needle, "");
                                            let _ = std::fs::write(&path, &new);
                                            app.viewport.add_error_message(format!("✓ Removed: {}", pattern));
                                        }
                                    }
                                    "\\auth edit" => {
                                        let path = dirs::home_dir().unwrap_or_default().join(".arcana/authority.toml");
                                        let editor = config.editor.command.clone();
                                        // Stop event reader completely before editor
                                        event_handle.abort();
                                        tui.restore()?;
                                        // Run editor with full terminal control
                                        let _ = std::process::Command::new(&editor)
                                            .arg(&path)
                                            .status();
                                        // Respawn TUI and event reader
                                        tui = crate::tui::Tui::new()?;
                                        let (tx, rx, handle) = event::spawn_event_reader();
                                        event_tx = tx;
                                        events = rx;
                                        event_handle = handle;
                                        app.composer.clear();
                                        app.viewport.add_error_message(
                                            format!("Authority config reloaded from {}", path.display())
                                        );
                                    }
                                    "\\check" => {
                                        let home = dirs::home_dir().unwrap_or_default().join(".arcana");
                                        let mut lines = Vec::new();
                                        let cfg = home.join("config.toml");
                                        lines.push(if cfg.exists() { "✓ config.toml" } else { "✗ config.toml (missing)" });
                                        let auth = home.join("authority.toml");
                                        lines.push(if auth.exists() { "✓ authority.toml" } else { "✗ authority.toml (missing)" });
                                        let soul = home.join("SOUL.md");
                                        lines.push(if soul.exists() { "✓ SOUL.md" } else { "✗ SOUL.md (missing)" });
                                        let key_ok = std::env::var("DEEPSEEK_API_KEY").is_ok();
                                        lines.push(if key_ok { "✓ DEEPSEEK_API_KEY (env)" } else { "✗ DEEPSEEK_API_KEY (not set)" });
                                        app.viewport.add_error_message(
                                            format!("Health Check:\n  {}", lines.join("\n  "))
                                        );
                                    }
                                    _ if input.starts_with('\\') => {
                                        app.viewport.add_error_message(
                                            format!("Unknown command: {}", input.trim())
                                        );
                                    }
                                    _ => {
                                        // Send to LLM
                                        app.viewport.add_user_message(input.clone());
                                        app.viewport.is_streaming = true;
                                        app.show_banner = false;

                                        conversation.push(serde_json::json!({
                                            "role": "user", "content": input
                                        }));

                                        crate::llm::spawn_stream(
                                            &config, conversation.clone(), event_tx.clone()
                                        );
                                    }
                                }
                                // Add separator after command output
                                if is_command && input.trim() != "\\quit" && input.trim() != "\\q" && input.trim() != "\\clear" {
                                    app.viewport.add_separator();
                                }
                                app.show_banner = false;
                            } else if action == KeyAction::OpenEditor {
                                // Ctrl+e: open $EDITOR for prompt editing
                                let editor = config.editor.command.clone();
                                let tmp = std::env::temp_dir().join("arcana_prompt.md");
                                let _ = std::fs::write(&tmp, &app.composer.input);
                                event_handle.abort();
                                tui.restore()?;
                                let _ = std::process::Command::new(&editor).arg(&tmp).status();
                                tui = crate::tui::Tui::new()?;
                                let (tx, rx, handle) = event::spawn_event_reader();
                                event_tx = tx;
                                events = rx;
                                event_handle = handle;
                                // Load edited content back
                                if let Ok(content) = std::fs::read_to_string(&tmp) {
                                    app.composer.input = content;
                                    app.composer.cursor_pos = app.composer.input.len();
                                    app.composer.show_hint = false;
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

                                let msgs = app.overlay.build_messages();
                                crate::llm::spawn_overlay_stream(
                                    &config, msgs, event_tx.clone()
                                );
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
                AppEvent::ScrollUp(n) => { app.viewport.scroll_up(n as usize); }
                AppEvent::ScrollDown(n) => { app.viewport.scroll_down(n as usize); }
                AppEvent::Resize(_, _) => {}
                AppEvent::Token(token) => {
                    // Thinking tokens are prefixed with \x00THINK:
                    if let Some(think_text) = token.strip_prefix("\x00THINK:") {
                        app.viewport.append_think_token(think_text);
                    } else {
                        app.viewport.append_token(&token);
                    }
                }
                AppEvent::ThinkStart => { app.viewport.start_thinking(); }
                AppEvent::ThinkEnd => { app.viewport.end_thinking(); }
                AppEvent::ResponseComplete(stats) => {
                    // Store the response in conversation history
                    let response_text = app.viewport.streaming_text.clone();
                    let thinking_text = app.viewport.streaming_think
                        .as_ref().map(|t| t.content.clone());

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
                }
                AppEvent::LlmError(err) => { app.handle_llm_error(err); }
                AppEvent::Toast { message, detail } => {
                    app.toasts.push(Toast { message, detail, created_at: chrono::Utc::now() });
                }
                AppEvent::Tick => {
                    let now = chrono::Utc::now();
                    app.toasts.retain(|t| (now - t.created_at).num_seconds() < 5);
                }
                // Overlay (query agent) events
                AppEvent::OverlayToken(token) => {
                    if let Some(think_text) = token.strip_prefix("\x00THINK:") {
                        app.overlay.append_think_token(think_text);
                    } else {
                        app.overlay.append_token(&token);
                    }
                }
                AppEvent::OverlayThinkStart => { app.overlay.start_thinking(); }
                AppEvent::OverlayThinkEnd => { app.overlay.end_thinking(); }
                AppEvent::OverlayResponseComplete => { app.overlay.finalize_response(); }
                AppEvent::OverlayError(msg) => {
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

    // Resolve API key (env var takes priority over literal "$VAR" in config)
    let api_key = config.resolve_api_key(provider_name)
        .ok_or_else(|| format!("No API key found for provider '{}'. Set the appropriate env var.", provider_name))?;

    // Resolve base URL
    let base_url = match provider_name.as_str() {
        "deepseek" => {
            let url = &config.providers.deepseek.base_url;
            if url.is_empty() { "https://api.deepseek.com".to_string() } else { url.clone() }
        }
        "openai" => {
            let url = &config.providers.openai.base_url;
            if url.is_empty() { "https://api.openai.com/v1".to_string() } else { url.clone() }
        }
        "anthropic" => {
            let url = &config.providers.anthropic.base_url;
            if url.is_empty() { "https://api.anthropic.com".to_string() } else { url.clone() }
        }
        _ => return Err(format!("Unsupported provider: {}", provider_name).into()),
    };

    // Build request body
    let thinking_config = &config.agents.main.thinking;
    let mut body = serde_json::json!({
        "model": model_name,
        "messages": [
            {"role": "system", "content": "You are a helpful assistant."},
            {"role": "user", "content": query}
        ],
        "stream": false
    });
    if thinking_config.enabled {
        body["thinking"] = serde_json::json!({"type": "enabled"});
        body["reasoning_effort"] = serde_json::json!(thinking_config.reasoning_effort);
    }

    let client = reqwest::Client::new();
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

    let data: serde_json::Value = resp.json().await?;

    // Print reasoning if present
    if let Some(reasoning) = data["choices"][0]["message"]["reasoning_content"].as_str() {
        if !reasoning.is_empty() {
            println!("\x1b[2m<thinking>\n{}\n</thinking>\x1b[0m\n", reasoning);
        }
    }

    // Print the final answer
    if let Some(content) = data["choices"][0]["message"]["content"].as_str() {
        println!("{}", content);
    }

    // Print usage
    if let Some(usage) = data.get("usage") {
        let input = usage["prompt_tokens"].as_u64().unwrap_or(0);
        let output = usage["completion_tokens"].as_u64().unwrap_or(0);
        println!("\n\x1b[2m[tokens: {} in / {} out]\x1b[0m", input, output);
    }

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
