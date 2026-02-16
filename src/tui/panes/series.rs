use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Recent Series ");
    let paragraph = Paragraph::new("  (no media roots)")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}
