use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// Stub widget for Recent Series — full implementation in Phase 8 (media scanning).
pub fn render_recent_series(
    area: Rect,
    buf: &mut Buffer,
    _selected: usize,
    focused: bool,
) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Recent Series ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let placeholder = Paragraph::new(Line::from(Span::styled(
        "No media scanned yet",
        Style::default().fg(Color::DarkGray),
    )));
    placeholder.render(inner, buf);
}
