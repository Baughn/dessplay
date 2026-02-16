use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, local_username: &str, peers: &[String]) {
    let block = Block::default().borders(Borders::ALL).title(" Users ");

    let mut lines = vec![Line::from(Span::styled(
        format!("  {local_username} (you)"),
        Style::default().fg(Color::White),
    ))];

    let mut sorted_peers: Vec<&String> = peers.iter().collect();
    sorted_peers.sort();

    for peer in sorted_peers {
        lines.push(Line::from(Span::styled(
            format!("  {peer}"),
            Style::default().fg(Color::White),
        )));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
