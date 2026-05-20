use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application events (input + internal).
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Terminal key event
    Key(KeyEvent),
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
/// Returns (sender, receiver) so other subsystems can also send AppEvents.
pub fn spawn_event_reader() -> (mpsc::UnboundedSender<AppEvent>, mpsc::UnboundedReceiver<AppEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let tx2 = tx.clone();

    tokio::spawn(async move {
        loop {
            // Poll for crossterm events with a 250ms timeout (for tick generation)
            if event::poll(Duration::from_millis(250)).unwrap_or(false) {
                match event::read() {
                    Ok(CrosstermEvent::Key(key)) => {
                        if tx2.send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Resize(w, h)) => {
                        if tx2.send(AppEvent::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {} // Mouse events, etc.
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

    (tx, rx)
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
        (m, KeyCode::Char('j')) if m.contains(KeyModifiers::CONTROL) => KeyAction::Newline,
        (m, KeyCode::Char('o')) if m.contains(KeyModifiers::CONTROL) => KeyAction::Expand,
        (m, KeyCode::Char('g')) if m.contains(KeyModifiers::CONTROL) => KeyAction::OpenEditor,
        (m, KeyCode::Char('t')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleTasks,
        (m, KeyCode::Char('s')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleSkills,
        (m, KeyCode::Char('a')) if m.contains(KeyModifiers::CONTROL) => KeyAction::ToggleAgents,
        (m, KeyCode::Char('p')) if m.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) => {
            KeyAction::Freeze
        }

        // Alt+Enter or Ctrl+Enter for newline
        (m, KeyCode::Enter) if m.contains(KeyModifiers::ALT) => KeyAction::Newline,
        (m, KeyCode::Enter) if m.contains(KeyModifiers::CONTROL) => KeyAction::Newline,

        // Basic keys
        (_, KeyCode::Enter) => KeyAction::Enter,
        (_, KeyCode::Backspace) => KeyAction::Backspace,
        (_, KeyCode::Delete) => KeyAction::Delete,
        (_, KeyCode::Esc) => KeyAction::Escape,
        (_, KeyCode::Tab) => KeyAction::Tab,
        (_, KeyCode::Up) => KeyAction::Up,
        (_, KeyCode::Down) => KeyAction::Down,
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
