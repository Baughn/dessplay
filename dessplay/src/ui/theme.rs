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

/// Border color for a pane, focused vs not. Single source of truth so the
/// chat input border can match the rest of its pane.
pub fn border_color(focused: bool) -> Color {
    if focused {
        Color::Yellow
    } else {
        Color::DarkGray
    }
}

/// Border style for a pane, focused vs not.
pub fn border_style(focused: bool) -> Style {
    Style::default().fg(border_color(focused))
}

/// A deterministic, auto-generated color for a username, so names are
/// visually distinguishable in chat and the Users pane. Same name always
/// maps to the same color (FNV-1a over the bytes, into a fixed palette).
pub fn user_style(name: &str) -> Style {
    // Visually distinct hues; deliberately avoids plain Red/Green/Blue/Gray,
    // which carry ready-state meaning elsewhere in the UI.
    const PALETTE: [Color; 10] = [
        Color::Cyan,
        Color::Magenta,
        Color::LightCyan,
        Color::LightMagenta,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightRed,
        Color::Indexed(208), // orange
        Color::Indexed(141), // lavender
    ];
    // FNV-1a, hand-rolled for determinism across runs (RandomState is seeded).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Style::default().fg(PALETTE[(hash % PALETTE.len() as u64) as usize])
}

/// Style for a directory row in the file browser (files stay plain, so
/// directories read at a glance).
pub fn directory() -> Style {
    Style::default().fg(Color::Cyan)
}

/// Style for selection highlight within a focused pane.
pub fn highlight_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Dim style for decoration lines (departed/seeders, group headings).
pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_style_is_deterministic() {
        assert_eq!(user_style("Baughn").fg, user_style("Baughn").fg);
        assert_eq!(user_style("Nero").fg, user_style("Nero").fg);
    }

    #[test]
    fn user_style_spreads_across_palette() {
        // A handful of distinct names should land on more than one color
        // (a constant function would fail this).
        let names = ["Baughn", "Nero", "Quickshot", "Dagger", "Kim", "nas"];
        let colors: std::collections::HashSet<_> = names.iter().map(|n| user_style(n).fg).collect();
        assert!(colors.len() > 1, "all names mapped to the same color");
    }
}
