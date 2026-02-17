use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::state::StateView;

pub fn render(frame: &mut Frame, area: Rect, view: Option<&StateView>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Playlist ");

    if let Some(view) = view
        && !view.playlist.is_empty()
    {
        let items: Vec<ListItem> = view
            .playlist
            .iter()
            .map(|item| {
                let is_current = view.current_file.as_ref() == Some(&item.id);
                let style = if is_current {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let prefix = if is_current { "> " } else { "  " };
                ListItem::new(Line::from(Span::styled(
                    format!("{prefix}{}", item.filename),
                    style,
                )))
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    } else {
        let paragraph = Paragraph::new("  (empty)")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }
}
