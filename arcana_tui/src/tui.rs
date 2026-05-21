use crossterm::{
    execute,
    event::{EnableBracketedPaste, DisableBracketedPaste, EnableMouseCapture, DisableMouseCapture},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, stdout};

/// Terminal wrapper that manages raw mode and alternate screen.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    /// Initialize the terminal (enter raw mode, alternate screen).
    pub fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    /// Draw a frame.
    pub fn draw<F>(&mut self, f: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(f)?;
        Ok(())
    }

    /// Suspend the TUI for running an external program (editor).
    /// Leaves alternate screen and disables raw mode so the program gets normal terminal.
    pub fn suspend(&mut self) -> io::Result<()> {
        execute!(self.terminal.backend_mut(), DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    /// Resume the TUI after an external program exits.
    /// Re-enters raw mode and alternate screen.
    pub fn resume(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture)?;
        self.terminal.clear()?;
        Ok(())
    }

    /// Restore the terminal to its original state (for exit).
    pub fn restore(&mut self) -> io::Result<()> {
        terminal::disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen)?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
