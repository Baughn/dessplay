use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::DisplayMessage;

pub fn render(frame: &mut Frame, area: Rect, messages: &[DisplayMessage], verbosity: u8) {
    let block = Block::default().borders(Borders::ALL).title(" Chat ");

    let lines: Vec<Line> = messages
        .iter()
        .filter(|m| m.min_verbosity <= verbosity)
        .map(|m| {
            Line::from(vec![
                Span::styled(
                    format!("[{}] ", m.timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(&m.text),
            ])
        })
        .collect();

    // Auto-scroll: skip lines that don't fit
    let inner_height = area.height.saturating_sub(2) as usize; // border top+bottom
    let skip = lines.len().saturating_sub(inner_height);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((skip as u16, 0));

    frame.render_widget(paragraph, area);
}

pub fn render_input(frame: &mut Frame, area: Rect, text: &str, cursor_pos: usize, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Message ")
        .style(border_style);

    let paragraph = Paragraph::new(Line::from(vec![Span::raw(" "), Span::raw(text)])).block(block);
    frame.render_widget(paragraph, area);

    if focused {
        // Position cursor inside the bordered block (+1 border, +1 padding)
        frame.set_cursor_position((area.x + 2 + cursor_pos as u16, area.y + 1));
    }
}
