use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use dessplay_core::types::{FileState, UserState};

/// A user entry for display.
pub struct UserEntry {
    pub name: String,
    pub user_state: UserState,
    pub file_state: FileState,
    pub is_self: bool,
}

/// Determine the display color for a user based on their ready state.
///
/// | State        | Color        | Condition                                  |
/// |--------------|--------------|-------------------------------------------|
/// | Ready        | Green        | UserState::Ready AND FileState::Ready      |
/// | Paused       | Red          | UserState::Paused                          |
/// | Not watching | Gray         | UserState::NotWatching                     |
/// | Downloading  | Green/Blue   | FileState::Downloading (green if >=20%)    |
fn user_color(user_state: &UserState, file_state: &FileState) -> Color {
    match user_state {
        UserState::Paused => Color::Red,
        UserState::NotWatching => Color::DarkGray,
        UserState::Ready => match file_state {
            FileState::Ready => Color::Green,
            FileState::Missing => Color::Red,
            FileState::Downloading { progress } => {
                if *progress >= 0.20 {
                    Color::Green
                } else {
                    Color::Blue
                }
            }
        },
    }
}

fn state_label(user_state: &UserState, file_state: &FileState) -> &'static str {
    match user_state {
        UserState::Paused => "Paused",
        UserState::NotWatching => "Not watching",
        UserState::Ready => match file_state {
            FileState::Ready => "Ready",
            FileState::Missing => "Missing",
            FileState::Downloading { .. } => "Downloading",
        },
    }
}

pub fn render_users(area: Rect, buf: &mut Buffer, users: &[UserEntry], focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Users ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let lines: Vec<Line<'_>> = users
        .iter()
        .map(|u| {
            let color = user_color(&u.user_state, &u.file_state);
            let label = state_label(&u.user_state, &u.file_state);
            let name_style = Style::default().fg(color);
            let suffix = if u.is_self { " (you)" } else { "" };

            Line::from(vec![
                Span::styled(
                    format!("{}{suffix}", u.name),
                    name_style.add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(format!("[{label}]"), Style::default().fg(color)),
            ])
        })
        .collect();

    if lines.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "No users connected",
            Style::default().fg(Color::DarkGray),
        )));
        empty.render(inner, buf);
    } else {
        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}
