use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use super::super::series_state::{SeriesItem, SeriesPaneState};

pub fn render(frame: &mut Frame, area: Rect, state: &SeriesPaneState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Recent Series ")
        .border_style(border_style);

    if state.items.is_empty() {
        let paragraph = Paragraph::new("  (no media roots)\n  Use /add-root <path>")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize; // borders
    let items: Vec<ListItem> = state
        .items
        .iter()
        .enumerate()
        .skip(state.scroll)
        .take(inner_height)
        .map(|(i, item)| {
            let is_selected = focused && i == state.selected;
            let prefix = if is_selected { "> " } else { "  " };
            match item {
                SeriesItem::MediaRoot { path } => {
                    let mut style = Style::default().fg(Color::Cyan);
                    if is_selected {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    ListItem::new(Line::from(Span::styled(format!("{prefix}{path}"), style)))
                }
                SeriesItem::RecentSeries { display_name, .. } => {
                    let mut style = Style::default().fg(Color::White);
                    if is_selected {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    ListItem::new(Line::from(Span::styled(
                        format!("{prefix}{display_name}"),
                        style,
                    )))
                }
            }
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}
