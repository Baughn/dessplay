use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::state::StateView;

pub fn render(frame: &mut Frame, area: Rect, view: Option<&StateView>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Player ");

    let (content, style) = if let Some(view) = view {
        let filename = view
            .current_file
            .as_ref()
            .and_then(|id| view.playlist.iter().find(|p| p.id == *id))
            .map(|p| p.filename.as_str())
            .unwrap_or("No file loaded");

        let pos_secs = view.position as u64;
        let pos_m = pos_secs / 60;
        let pos_s = pos_secs % 60;

        let play_indicator = if view.is_playing { ">" } else { "||" };

        (
            format!("  {play_indicator} {pos_m:02}:{pos_s:02}  {filename}"),
            Style::default(),
        )
    } else {
        (
            "  No file loaded".to_string(),
            Style::default().fg(Color::DarkGray),
        )
    };

    let paragraph = Paragraph::new(content).block(block).style(style);
    frame.render_widget(paragraph, area);
}
