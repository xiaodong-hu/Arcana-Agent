use crossterm::event::{
    Event as CrosstermEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use futures::StreamExt;
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
    SubAgentUpdate {
        id: String,
        status: String,
    },
    /// Toast notification
    Toast {
        message: String,
        detail: Option<String>,
    },
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

/// Spawn the terminal event reader task using async EventStream.
/// The task is properly cancellable via abort() — no blocking poll.
pub fn spawn_event_reader() -> (
    mpsc::UnboundedSender<AppEvent>,
    mpsc::UnboundedReceiver<AppEvent>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let tx2 = tx.clone();

    let handle = tokio::spawn(async move {
        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_millis(250));

        loop {
            tokio::select! {
                _ = tick_interval.tick() => {
                    if tx2.send(AppEvent::Tick).is_err() { break; }
                }
                event = reader.next() => {
                    match event {
                        Some(Ok(CrosstermEvent::Key(key))) => {
                            // With kitty keyboard protocol, we get Press+Release events.
                            // Only process Press (and Repeat) to avoid double-firing.
                            if key.kind == KeyEventKind::Release {
                                continue;
                            }
                            if tx2.send(AppEvent::Key(key)).is_err() { break; }
                        }
                        Some(Ok(CrosstermEvent::Paste(text))) => {
                            if tx2.send(AppEvent::Paste(text)).is_err() { break; }
                        }
                        Some(Ok(CrosstermEvent::Resize(w, h))) => {
                            if tx2.send(AppEvent::Resize(w, h)).is_err() { break; }
                        }
                        Some(Ok(CrosstermEvent::Mouse(MouseEvent { kind, .. }))) => {
                            match kind {
                                MouseEventKind::ScrollUp => {
                                    let _ = tx2.send(AppEvent::ScrollUp(1));
                                }
                                MouseEventKind::ScrollDown => {
                                    let _ = tx2.send(AppEvent::ScrollDown(1));
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                        None => break,
                    }
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
    /// Ctrl+w (delete word left)
    DeleteWordLeft,
    /// Ctrl+Up (jump to start of input)
    JumpTop,
    /// Ctrl+Down (jump to end of input)
    JumpBottom,
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
    /// Ctrl+B (break/stop LLM generation)
    BreakGeneration,
    /// Ctrl+Shift+P (freeze)
    Freeze,
    /// Ctrl+Y (toggle terminal text selection mode)
    ToggleSelectionMode,
    /// Ctrl+O (expand)
    Expand,
    /// Ctrl+X (expand/collapse tool-call panels)
    ToggleToolCalls,
    /// Ctrl+P (expand/collapse diff panels beyond 20 lines)
    ToggleDiff,
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
    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let alt = mods.contains(KeyModifiers::ALT);

    match key.code {
        // --- Enter variants ---
        KeyCode::Enter if ctrl || shift || alt => KeyAction::Newline,
        KeyCode::Enter => KeyAction::Enter,

        // --- Ctrl+char combinations ---
        KeyCode::Char('c') if ctrl => KeyAction::Interrupt,
        KeyCode::Char('b') if ctrl => KeyAction::BreakGeneration,
        KeyCode::Char('u') if ctrl => KeyAction::HalfPageUp,
        KeyCode::Char('j') if ctrl => KeyAction::FocusDown,
        KeyCode::Char('k') if ctrl => KeyAction::FocusUp,
        KeyCode::Char('o') if ctrl => KeyAction::Expand,
        KeyCode::Char('x') if ctrl => KeyAction::ToggleToolCalls,
        KeyCode::Char('p') if ctrl => KeyAction::ToggleDiff,
        KeyCode::Char('y') if ctrl => KeyAction::ToggleSelectionMode,
        KeyCode::Char('w') if ctrl => KeyAction::DeleteWordLeft,
        KeyCode::Char('h') if ctrl => KeyAction::WordLeft,
        KeyCode::Char('l') if ctrl => KeyAction::WordRight,
        KeyCode::Char('g') | KeyCode::Char('e') if ctrl => KeyAction::OpenEditor,
        KeyCode::Char('t') if ctrl => KeyAction::ToggleTasks,
        KeyCode::Char('s') if ctrl => KeyAction::ToggleSkills,
        KeyCode::Char('a') if ctrl => KeyAction::ToggleAgents,
        // Ctrl+/ — kitty protocol sends Char('/'), legacy sends Char('7') for 0x1F
        KeyCode::Char('/') if ctrl => KeyAction::ToggleQuery,
        KeyCode::Char('_') if ctrl => KeyAction::ToggleQuery,
        KeyCode::Char('7') if ctrl => KeyAction::ToggleQuery,
        KeyCode::Char('p') if ctrl && shift => KeyAction::Freeze,
        // Ctrl+Enter as Char('\r') or Char('\n')
        KeyCode::Char('\r') | KeyCode::Char('\n') if ctrl => KeyAction::Newline,

        // --- Basic keys ---
        KeyCode::Backspace => KeyAction::Backspace,
        KeyCode::Delete => KeyAction::Delete,
        KeyCode::Esc => KeyAction::Escape,
        KeyCode::Tab => KeyAction::Tab,
        KeyCode::Up if ctrl => KeyAction::JumpTop,
        KeyCode::Down if ctrl => KeyAction::JumpBottom,
        KeyCode::Up => KeyAction::Up,
        KeyCode::Down => KeyAction::Down,
        KeyCode::Left if ctrl => KeyAction::WordLeft,
        KeyCode::Right if ctrl => KeyAction::WordRight,
        KeyCode::Left => KeyAction::Left,
        KeyCode::Right => KeyAction::Right,
        KeyCode::PageUp => KeyAction::PageUp,
        KeyCode::PageDown => KeyAction::PageDown,
        KeyCode::Home => KeyAction::Home,
        KeyCode::End => KeyAction::End,

        // --- Character input (no Ctrl) ---
        KeyCode::Char(c) if !ctrl => KeyAction::Char(c),

        _ => KeyAction::None,
    }
}
