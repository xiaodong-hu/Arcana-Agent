use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application events (input + internal).
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Terminal key event
    Key(KeyEvent),
    /// Pasted text (bracketed paste)
    Paste(String),
    /// Mouse scroll
    ScrollUp(u16),
    ScrollDown(u16),
    /// Terminal resize
    Resize(u16, u16),
    /// A token arrived from the LLM stream
    Token(String),
    /// Thinking block delimiter
    ThinkStart,
    ThinkEnd,
    /// Agent response complete (with optional usage stats)
    ResponseComplete(Option<crate::types::ResponseStats>),
    /// Sub-agent status update
    SubAgentUpdate { id: String, status: String },
    /// Toast notification
    Toast { message: String, detail: Option<String> },
    /// LLM error (rate limit, API error, etc.)
    LlmError(crate::types::LlmError),
    /// Tick (for animations and elapsed time updates)
    Tick,
    // --- Overlay (query agent) events ---
    OverlayToken(String),
    OverlayThinkStart,
    OverlayThinkEnd,
    OverlayResponseComplete,
    OverlayError(String),
}

/// Spawn the terminal event reader task.
/// Returns (sender, receiver, task_handle) so the reader can be aborted for $EDITOR.
pub fn spawn_event_reader() -> (mpsc::UnboundedSender<AppEvent>, mpsc::UnboundedReceiver<AppEvent>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let tx2 = tx.clone();

    let handle = tokio::spawn(async move {
        loop {
            // Poll for crossterm events with a 250ms timeout (for tick generation)
            if event::poll(Duration::from_millis(250)).unwrap_or(false) {
                match event::read() {
                    Ok(CrosstermEvent::Key(key)) => {
                        if tx2.send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Paste(text)) => {
                        if tx2.send(AppEvent::Paste(text)).is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Resize(w, h)) => {
                        if tx2.send(AppEvent::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Mouse(MouseEvent { kind, .. })) => {
                        match kind {
                            MouseEventKind::ScrollUp => {
                                let _ = tx2.send(AppEvent::ScrollUp(3));
                            }
                            MouseEventKind::ScrollDown => {
                                let _ = tx2.send(AppEvent::ScrollDown(3));
                            }
                            _ => {}
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            } else {
                // Timeout — send tick for time-based updates
                if tx2.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    (tx, rx, handle)
}

/// Classify a key event into a high-level action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Character input
    Char(char),
    /// Enter (send message)
    Enter,
    /// Alt+Enter or Ctrl+J (newline in composer)
    Newline,
    /// Backspace
    Backspace,
    /// Delete
    Delete,
    /// Escape
    Escape,
    /// Tab (autocomplete)
    Tab,
    /// Arrow keys
    Up,
    Down,
    Left,
    Right,
    /// Ctrl+Left/Right (word movement)
    WordLeft,
    WordRight,
    /// Page navigation
    PageUp,
    PageDown,
    Home,
    End,
    /// Ctrl+U (half page up)
    HalfPageUp,
    /// Ctrl+D (half page down / end session)
    HalfPageDown,
    /// Ctrl+C (interrupt)
    Interrupt,
    /// Ctrl+D (end session)
    EndSession,
    /// Ctrl+Shift+P (freeze)
    Freeze,
    /// Ctrl+O (expand)
    Expand,
    /// Ctrl+G (open editor)
    OpenEditor,
    /// Ctrl+T (toggle tasks panel)
    ToggleTasks,
    /// Ctrl+S (toggle skills panel)
    ToggleSkills,
    /// Ctrl+A (toggle agents panel)
    ToggleAgents,
    /// Ctrl+/ (toggle query overlay)
    ToggleQuery,
    /// Ctrl+j (focus next dialogue)
    FocusDown,
    /// Ctrl+k (focus previous dialogue)
    FocusUp,
    /// Unknown / unhandled
    None,
}

/// Map a crossterm KeyEvent to our KeyAction.
pub fn classify_key(key: &KeyEvent) -> KeyAction {
    match (key.modifiers, key.code) {
        // Ctrl combinations
        (m, KeyCode::Char('c')) if m.contains(KeyModifiers::CONTROL) => KeyAction::Interrupt,
        (m, KeyCode::Char('d')) if m.contains(KeyModifiers::CONTROL) => KeyAction::EndSession,
        (m, KeyCode::Char('u')) if m.contains(KeyModifiers::CONTROL) => KeyAction::HalfPageUp,
        (m, KeyCode::Char('j')) if m.contains(KeyModifiers::CONTROL) => KeyAction::FocusDown,
        (m, KeyCode::Char('k')) if m.contains(KeyModifiers::CONTROL) => KeyAction::FocusUp,
        (m, KeyCode::Char('\n')) if m.contains(KeyModifiers::CONTROL) => KeyAction::Newline,
        (m, KeyCode::Char('\r')) if m.contains(KeyModifiers::CONTROL) => KeyAction::Newline,
        (m, KeyCode::Char('o')) if m.contains(KeyModifiers::CONTROL) => KeyAction::Expand,
        (m, KeyCode::Char('g')) if m.contains(KeyModifiers::CONTROL) => KeyAction::OpenEditor,
        (m, KeyCode::Char('t')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleTasks,
        (m, KeyCode::Char('s')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleSkills,
        (m, KeyCode::Char('a')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleAgents,
        (m, KeyCode::Char('/')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleQuery,
        (m, KeyCode::Char('_')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleQuery,
        (m, KeyCode::Char('p')) if m.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) => {
            KeyAction::Freeze
        }

        // Alt+Enter, Ctrl+Enter, or Shift+Enter for newline
        (m, KeyCode::Enter) if m.contains(KeyModifiers::ALT) => KeyAction::Newline,
        (m, KeyCode::Enter) if m.contains(KeyModifiers::CONTROL) => KeyAction::Newline,
        (m, KeyCode::Enter) if m.contains(KeyModifiers::SHIFT) => KeyAction::Newline,

        // Basic keys
        (_, KeyCode::Enter) => KeyAction::Enter,
        (_, KeyCode::Backspace) => KeyAction::Backspace,
        (_, KeyCode::Delete) => KeyAction::Delete,
        (_, KeyCode::Esc) => KeyAction::Escape,
        (_, KeyCode::Tab) => KeyAction::Tab,
        (_, KeyCode::Up) => KeyAction::Up,
        (_, KeyCode::Down) => KeyAction::Down,
        (m, KeyCode::Left) if m.contains(KeyModifiers::CONTROL) => KeyAction::WordLeft,
        (m, KeyCode::Right) if m.contains(KeyModifiers::CONTROL) => KeyAction::WordRight,
        (_, KeyCode::Left) => KeyAction::Left,
        (_, KeyCode::Right) => KeyAction::Right,
        (_, KeyCode::PageUp) => KeyAction::PageUp,
        (_, KeyCode::PageDown) => KeyAction::PageDown,
        (_, KeyCode::Home) => KeyAction::Home,
        (_, KeyCode::End) => KeyAction::End,

        // Character input
        (m, KeyCode::Char(c)) if !m.contains(KeyModifiers::CONTROL) => KeyAction::Char(c),

        _ => KeyAction::None,
    }
}
