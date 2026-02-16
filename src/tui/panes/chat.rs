use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::ChatMessage;

pub fn render(frame: &mut Frame, area: Rect, messages: &[ChatMessage], verbosity: u8) {
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

pub fn render_input(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Message ")
        .style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new("  (chat not yet implemented)")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}
