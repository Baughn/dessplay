use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// All the layout rectangles for the main screen.
pub struct AppLayout {
    pub chat_messages: Rect,
    pub chat_input: Rect,
    pub recent_series: Rect,
    pub users: Rect,
    pub playlist: Rect,
    pub player_status: Rect,
    pub keybinding_bar: Rect,
}

/// Compute the layout for the main screen.
///
/// ```text
/// +----------------------------------+------------------+
/// |          Chat Messages           | Recent Series    |
/// |                                  +------------------+
/// |                                  | Users            |
/// +----------------------------------+------------------+
/// |   [chat input line]              | Playlist         |
/// +----------------------------------+------------------+
/// |  Player Status (3 lines)                            |
/// +-----------------------------------------------------+
/// |  Keybinding bar (1 line)                            |
/// +-----------------------------------------------------+
/// ```
pub fn compute_layout(area: Rect) -> AppLayout {
    // Vertical split: [main content | player status (3) | keybinding bar (1)]
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),       // main content
            Constraint::Length(3),     // player status
            Constraint::Length(1),     // keybinding bar
        ])
        .split(area);

    let main_area = vertical[0];
    let player_status = vertical[1];
    let keybinding_bar = vertical[2];

    // Main horizontal: [left 50% | right 50%]
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    let left = horizontal[0];
    let right = horizontal[1];

    // Left column: [chat messages | chat input (1 line)]
    let left_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(left);

    let chat_messages = left_split[0];
    let chat_input = left_split[1];

    // Right column: [recent series (30%) | users (30%) | playlist (40%)]
    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(40),
        ])
        .split(right);

    let recent_series = right_split[0];
    let users = right_split[1];
    let playlist = right_split[2];

    AppLayout {
        chat_messages,
        chat_input,
        recent_series,
        users,
        playlist,
        player_status,
        keybinding_bar,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn layout_120x40() {
        let area = Rect::new(0, 0, 120, 40);
        let layout = compute_layout(area);

        // Player status is 3 lines from bottom
        assert_eq!(layout.player_status.height, 3);
        // Keybinding bar is 1 line at very bottom
        assert_eq!(layout.keybinding_bar.height, 1);
        assert_eq!(layout.keybinding_bar.y + layout.keybinding_bar.height, 40);
        // Chat input is 1 line
        assert_eq!(layout.chat_input.height, 1);
        // All rects have non-zero dimensions
        assert!(layout.chat_messages.height > 0);
        assert!(layout.recent_series.height > 0);
        assert!(layout.users.height > 0);
        assert!(layout.playlist.height > 0);
    }

    #[test]
    fn layout_80x24() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = compute_layout(area);

        assert_eq!(layout.player_status.height, 3);
        assert_eq!(layout.keybinding_bar.height, 1);
        assert!(layout.chat_messages.height > 0);
    }

    #[test]
    fn layout_horizontal_split() {
        let area = Rect::new(0, 0, 100, 30);
        let layout = compute_layout(area);

        // Chat should be roughly left half
        assert!(layout.chat_messages.width >= 45);
        assert!(layout.chat_messages.width <= 55);
        // Playlist should be roughly right half
        assert!(layout.playlist.width >= 45);
        assert!(layout.playlist.width <= 55);
    }
}
