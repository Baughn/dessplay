use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::series_browser::SeriesEntry;

pub fn render_recent_series(
    area: Rect,
    buf: &mut Buffer,
    entries: &[SeriesEntry],
    selected: usize,
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

    if entries.is_empty() {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "No media scanned yet",
            Style::default().fg(Color::DarkGray),
        )));
        placeholder.render(inner, buf);
        return;
    }

    let visible_height = inner.height as usize;
    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line<'_>> = entries
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, entry)| {
            let is_selected = i == selected;

            let mut name_style = if entry.has_unwatched {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            if is_selected {
                name_style = name_style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
                if entry.has_unwatched {
                    name_style = name_style.fg(Color::Yellow);
                }
            }

            let indicator = if entry.has_unwatched { "● " } else { "  " };
            let indicator_style = if entry.has_unwatched {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            Line::from(vec![
                Span::styled(indicator, indicator_style),
                Span::styled(&entry.name, name_style),
            ])
        })
        .collect();

    Paragraph::new(lines).render(inner, buf);
}
