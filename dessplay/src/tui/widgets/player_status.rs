use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// Stub widget for player status — progress bar and current file.
/// Full implementation in Phase 7 (player bridge).
pub fn render_player_status(
    area: Rect,
    buf: &mut Buffer,
    current_file_name: Option<&str>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let file_name = current_file_name.unwrap_or("Idle");

    let lines = vec![
        Line::from(Span::styled(
            format!("  Now Playing: {file_name}"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    paragraph.render(inner, buf);
}
