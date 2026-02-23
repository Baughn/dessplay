use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::tui::ui_state::{MetadataAssignState, MetadataAssignStep};

pub fn keybindings(step: &MetadataAssignStep) -> Vec<(&'static str, &'static str)> {
    match step {
        MetadataAssignStep::SelectSeries => vec![
            ("Enter", "Select"),
            ("Esc", "Cancel"),
            ("Ctrl-C", "Quit"),
        ],
        MetadataAssignStep::EnterEpisode => vec![
            ("Enter", "Confirm"),
            ("Esc", "Cancel"),
            ("Ctrl-C", "Quit"),
        ],
    }
}

pub fn render_metadata_assign(area: Rect, buf: &mut Buffer, state: &MetadataAssignState) {
    Clear.render(area, buf);

    let title = match state.step {
        MetadataAssignStep::SelectSeries => " Select Series ",
        MetadataAssignStep::EnterEpisode => " Enter Episode Number ",
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    match state.step {
        MetadataAssignStep::SelectSeries => {
            render_series_list(inner, buf, state);
        }
        MetadataAssignStep::EnterEpisode => {
            render_episode_input(inner, buf, state);
        }
    }
}

fn render_series_list(area: Rect, buf: &mut Buffer, state: &MetadataAssignState) {
    if state.series_list.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "No known series. Esc to cancel.",
            Style::default().fg(Color::DarkGray),
        )));
        msg.render(area, buf);
        return;
    }

    let visible_height = area.height as usize;
    let scroll = if state.selected >= visible_height {
        state.selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line<'_>> = state
        .series_list
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, series)| {
            let is_selected = i == state.selected;
            let mut style = Style::default().fg(Color::White);
            if is_selected {
                style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
            }
            Line::from(Span::styled(&series.name, style))
        })
        .collect();

    Paragraph::new(lines).render(area, buf);
}

fn render_episode_input(area: Rect, buf: &mut Buffer, state: &MetadataAssignState) {
    let selected_series = state
        .series_list
        .get(state.selected)
        .map(|s| s.name.as_str())
        .unwrap_or("(unknown)");

    let lines = vec![
        Line::from(Span::styled(
            format!("Series: {selected_series}"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Episode (e.g. 1, S1, C1, T1):",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            format!("> {}", state.episode_input.text),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    Paragraph::new(lines).render(area, buf);
}
