use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::tui::ui_state::{FileBrowserOrigin, FileBrowserState};

pub fn keybindings(origin: &FileBrowserOrigin) -> Vec<(&'static str, &'static str)> {
    match origin {
        FileBrowserOrigin::SettingsMediaRoot => vec![
            ("Enter", "Open"),
            ("s", "Select dir"),
            ("Esc", "Back"),
            ("Ctrl-C", "Quit"),
        ],
        FileBrowserOrigin::Playlist => vec![
            ("Enter", "Select"),
            ("Esc", "Back"),
            ("Ctrl-C", "Quit"),
        ],
    }
}

pub fn render_file_browser(area: Rect, buf: &mut Buffer, state: &FileBrowserState) {
    // Clear the area first (overlay)
    Clear.render(area, buf);

    let title = format!(" {} ", state.current_dir.display());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let visible_height = inner.height as usize;

    // Compute scroll offset to keep selected item visible
    let scroll = if state.selected < state.scroll_offset {
        state.selected
    } else if state.selected >= state.scroll_offset + visible_height {
        state.selected - visible_height + 1
    } else {
        state.scroll_offset
    };

    let lines: Vec<Line<'_>> = state
        .entries
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, entry)| {
            let is_selected = i == state.selected;

            let icon = if entry.is_dir { "📁 " } else { "📄 " };
            let mut style = if entry.is_dir {
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            if is_selected {
                style = style.bg(Color::DarkGray);
            }

            Line::from(Span::styled(format!("{icon}{}", entry.name), style))
        })
        .collect();

    if lines.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "Empty directory",
            Style::default().fg(Color::DarkGray),
        )));
        empty.render(inner, buf);
    } else {
        let paragraph = Paragraph::new(lines);
        paragraph.render(inner, buf);
    }
}
