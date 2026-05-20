use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::Theme;
use crate::types::StatusData;

/// The ASCII art banner lines.
const BANNER_ART: &[&str] = &[
    "░█████╗░██████╗░░█████╗░░█████╗░███╗░░██╗░█████╗░",
    "██╔══██╗██╔══██╗██╔══██╗██╔══██╗████╗░██║██╔══██╗",
    "███████║██████╔╝██║░░╚═╝███████║██╔██╗██║███████║",
    "██╔══██║██╔══██╗██║░░██╗██╔══██║██║╚████║██╔══██║",
    "██║░░██║██║░░██║╚█████╔╝██║░░██║██║░╚███║██║░░██║",
    "╚═╝░░╚═╝╚═╝░░╚═╝░╚════╝░╚═╝░░╚═╝╚═╝░░╚══╝╚═╝░░╚═╝",
];

const TAGLINE: &str = "The Arcane Agent — Memory · Skills · Authority";

/// Render the welcome banner into the given area.
pub fn render_banner(frame: &mut Frame, area: Rect, theme: &Theme, status: &StatusData) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.banner_gradient.0))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 10 || inner.width < 54 {
        // Too small for full banner, show compact version
        let compact = Paragraph::new(format!("⚗ ARCANA — {}", TAGLINE))
            .style(theme.prompt_glyph)
            .alignment(Alignment::Center);
        frame.render_widget(compact, inner);
        return;
    }

    // Calculate vertical centering
    let total_lines = BANNER_ART.len() + 2 + 4; // art + gap + tagline + gap + metadata
    let start_y = if inner.height as usize > total_lines {
        (inner.height as usize - total_lines) / 2
    } else {
        0
    };

    // Render ASCII art with gradient coloring
    for (i, line) in BANNER_ART.iter().enumerate() {
        let y = start_y + i;
        if y >= inner.height as usize {
            break;
        }

        // Interpolate color across the banner lines
        let t = i as f32 / (BANNER_ART.len() - 1).max(1) as f32;
        let color = interpolate_color(theme.banner_gradient.0, theme.banner_gradient.1, t);

        let _x_offset = if inner.width as usize > line.chars().count() {
            (inner.width as usize - line.chars().count()) / 2
        } else {
            0
        };

        let span = Span::styled(*line, Style::default().fg(color).add_modifier(Modifier::BOLD));
        let paragraph = Paragraph::new(Line::from(span)).alignment(Alignment::Center);
        let line_area = Rect::new(inner.x, inner.y + y as u16, inner.width, 1);
        frame.render_widget(paragraph, line_area);
    }

    // Tagline
    let tagline_y = start_y + BANNER_ART.len() + 1;
    if tagline_y < inner.height as usize {
        let tagline = Paragraph::new(TAGLINE)
            .style(theme.dim)
            .alignment(Alignment::Center);
        let tagline_area = Rect::new(inner.x, inner.y + tagline_y as u16, inner.width, 1);
        frame.render_widget(tagline, tagline_area);
    }

    // Metadata lines
    let meta_y = tagline_y + 2;
    if meta_y < inner.height as usize {
        let meta1 = format!(
            "Model: {:<24} Session: new",
            status.model_name
        );
        let meta2 = format!(
            "Provider: {:<20} Sub-agents: query + spawn",
            "deepseek"
        );

        let m1 = Paragraph::new(meta1).style(theme.dim).alignment(Alignment::Center);
        let m2 = Paragraph::new(meta2).style(theme.dim).alignment(Alignment::Center);

        let m1_area = Rect::new(inner.x, inner.y + meta_y as u16, inner.width, 1);
        let m2_area = Rect::new(inner.x, inner.y + (meta_y + 1) as u16, inner.width, 1);
        frame.render_widget(m1, m1_area);
        frame.render_widget(m2, m2_area);
    }
}

/// Linearly interpolate between two RGB colors.
fn interpolate_color(from: Color, to: Color, t: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let r = (r1 as f32 + (r2 as f32 - r1 as f32) * t) as u8;
            let g = (g1 as f32 + (g2 as f32 - g1 as f32) * t) as u8;
            let b = (b1 as f32 + (b2 as f32 - b1 as f32) * t) as u8;
            Color::Rgb(r, g, b)
        }
        _ => from,
    }
}

/// Calculate the height needed for the banner.
pub fn banner_height(area_width: u16) -> u16 {
    if area_width < 54 {
        3 // Compact mode
    } else {
        (BANNER_ART.len() as u16) + 6 // Art + borders + tagline + metadata
    }
}
