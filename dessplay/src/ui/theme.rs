//! Semantic tones -> concrete styles, in one place.

use tuirealm::ratatui::style::{Color, Modifier, Style};

use super::props::Tone;

/// Style for a tone.
pub fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Good => Style::default().fg(Color::Green),
        Tone::Blocked => Style::default().fg(Color::Red),
        Tone::Transfer => Style::default().fg(Color::Blue),
        Tone::Idle => Style::default().fg(Color::DarkGray),
        Tone::Muted => Style::default().add_modifier(Modifier::DIM),
        Tone::Normal => Style::default(),
    }
}

/// Border style for a pane, focused vs not.
pub fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// Style for selection highlight within a focused pane.
pub fn highlight_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Dim style for decoration lines (departed/seeders, group headings).
pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}
