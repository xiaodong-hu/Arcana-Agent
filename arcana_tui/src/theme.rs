use ratatui::style::{Color, Modifier, Style};

/// Color theme for the TUI.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub banner_gradient: (Color, Color),
    pub user_message: Style,
    pub agent_response: Style,
    pub thinking_block: Style,
    pub tool_call: Style,
    pub error: Style,
    pub diff_added: Style,
    pub diff_removed: Style,
    pub diff_context: Style,
    pub status_bar_bg: Color,
    pub prompt_glyph: Style,
    pub composer_bg: Color,
    pub composer_text: Style,
    pub overlay_border: Color,
    pub dim: Style,
    pub system_message: Style,
}

impl Theme {
    /// The default "Arcane" theme — dark with purple/blue accents.
    pub fn arcane() -> Self {
        Self {
            name: "arcane".into(),
            banner_gradient: (
                Color::Rgb(123, 47, 190), // #7B2FBE deep purple
                Color::Rgb(0, 212, 255),  // #00D4FF electric blue
            ),
            user_message: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            agent_response: Style::default().fg(Color::White),
            thinking_block: Style::default()
                .fg(Color::Rgb(140, 140, 160)) // lighter gray, visible on transparent terminals
                .add_modifier(Modifier::ITALIC),
            tool_call: Style::default().fg(Color::Cyan),
            error: Style::default().fg(Color::Red),
            diff_added: Style::default().fg(Color::Green),
            diff_removed: Style::default().fg(Color::Red),
            diff_context: Style::default().fg(Color::White),
            status_bar_bg: Color::Rgb(26, 26, 46), // #1a1a2e
            prompt_glyph: Style::default()
                .fg(Color::Rgb(0, 166, 79)) // pigment green
                .add_modifier(Modifier::BOLD),
            composer_bg: Color::Rgb(16, 12, 8),  // smoky black #100C08
            composer_text: Style::default().fg(Color::White),
            overlay_border: Color::Rgb(0, 212, 255),
            dim: Style::default().fg(Color::Rgb(140, 140, 160)),
            system_message: Style::default()
                .fg(Color::Rgb(140, 140, 160))
                .add_modifier(Modifier::ITALIC),
        }
    }

    /// Light theme for light terminals.
    pub fn light() -> Self {
        Self {
            name: "light".into(),
            banner_gradient: (
                Color::Rgb(90, 30, 150),
                Color::Rgb(0, 120, 200),
            ),
            user_message: Style::default()
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            agent_response: Style::default().fg(Color::Black),
            thinking_block: Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
            tool_call: Style::default().fg(Color::Blue),
            error: Style::default().fg(Color::Red),
            diff_added: Style::default().fg(Color::Green),
            diff_removed: Style::default().fg(Color::Red),
            diff_context: Style::default().fg(Color::Black),
            status_bar_bg: Color::Rgb(230, 230, 240),
            prompt_glyph: Style::default()
                .fg(Color::Rgb(90, 30, 150))
                .add_modifier(Modifier::BOLD),
            composer_bg: Color::Rgb(240, 240, 245), // light smoky
            composer_text: Style::default().fg(Color::Black),
            overlay_border: Color::Rgb(0, 120, 200),
            dim: Style::default().fg(Color::Gray),
            system_message: Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        }
    }

    /// Load theme by name.
    pub fn from_name(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            _ => Self::arcane(),
        }
    }
}
