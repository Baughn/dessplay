use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct PaneRects {
    pub chat: Rect,
    pub chat_input: Rect,
    pub recent_series: Rect,
    pub users: Rect,
    pub playlist: Rect,
    pub player_status: Rect,
    pub keybindings: Rect,
}

pub fn compute_layout(area: Rect) -> PaneRects {
    // Split: main area | player status (3 lines) | keybindings (1 line)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    let main_area = vertical[0];
    let player_status = vertical[1];
    let keybindings = vertical[2];

    // Split main: left 50% | right 50%
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    let left = horizontal[0];
    let right = horizontal[1];

    // Left: chat area with input line at bottom
    let chat_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(left);

    let chat = chat_split[0];
    let chat_input = chat_split[1];

    // Right: recent series | users | playlist (roughly equal)
    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(right);

    PaneRects {
        chat,
        chat_input,
        recent_series: right_split[0],
        users: right_split[1],
        playlist: right_split[2],
        player_status,
        keybindings,
    }
}
