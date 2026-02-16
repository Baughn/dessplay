use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Player ");
    let paragraph = Paragraph::new("  No file loaded")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}
