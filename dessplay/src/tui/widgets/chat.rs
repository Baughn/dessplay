use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::tui::ui_state::InputState;
use dessplay_core::types::UserId;

/// Render the chat messages pane.
pub fn render_chat_messages(
    area: Rect,
    buf: &mut Buffer,
    messages: &[(&UserId, &str)],
    scroll: usize,
    focused: bool,
) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Chat ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let lines: Vec<Line<'_>> = messages
        .iter()
        .map(|(uid, text)| {
            Line::from(vec![
                Span::styled(
                    format!("<{}>", uid.0),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::raw(*text),
            ])
        })
        .collect();

    // Scroll from bottom: show the last N lines that fit
    let total_lines = lines.len();
    let visible = inner.height as usize;
    let max_scroll = total_lines.saturating_sub(visible);
    let effective_scroll = scroll.min(max_scroll);
    let skip = total_lines.saturating_sub(visible + effective_scroll);

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((skip as u16, 0));

    paragraph.render(inner, buf);
}

/// Render the chat input line.
pub fn render_chat_input(area: Rect, buf: &mut Buffer, input: &InputState, focused: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let style = if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let prompt = "> ";
    let text = format!("{prompt}{}", input.text);

    let line = Line::from(Span::styled(text, style));
    let paragraph = Paragraph::new(line);
    paragraph.render(area, buf);

    // Show cursor position when focused
    if focused {
        let cursor_x = area.x + prompt.len() as u16 + input.cursor as u16;
        if cursor_x < area.x + area.width {
            buf[(cursor_x, area.y)].set_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White),
            );
        }
    }
}
