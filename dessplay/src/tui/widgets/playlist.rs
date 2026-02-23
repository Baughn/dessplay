use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use dessplay_core::types::FileId;

/// A playlist entry for display.
pub struct PlaylistEntry {
    pub file_id: FileId,
    /// Display name (filename from mapping, or hex hash fallback).
    pub display_name: String,
    /// Whether the file is missing locally.
    pub is_missing: bool,
    /// Whether this is the currently playing file (first in playlist).
    pub is_current: bool,
}

pub fn render_playlist(
    area: Rect,
    buf: &mut Buffer,
    entries: &[PlaylistEntry],
    selected: usize,
    focused: bool,
) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Playlist ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if entries.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "Empty playlist",
            Style::default().fg(Color::DarkGray),
        )));
        empty.render(inner, buf);
        return;
    }

    let lines: Vec<Line<'_>> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let mut style = Style::default();

            if entry.is_missing {
                style = style.fg(Color::Red);
            } else if entry.is_current {
                style = style.add_modifier(Modifier::BOLD);
            }

            if focused && i == selected {
                style = style
                    .fg(if entry.is_missing { Color::Red } else { Color::Black })
                    .bg(Color::White);
            }

            let prefix = if entry.is_current { "▶ " } else { "  " };
            Line::from(Span::styled(
                format!("{prefix}{}", entry.display_name),
                style,
            ))
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    paragraph.render(inner, buf);
}

/// Get a display name for a file: use local path filename, then CRDT filename, then hex hash.
pub fn file_display_name(file_id: &FileId, local_path: Option<&Path>, crdt_filename: Option<&str>) -> String {
    if let Some(path) = local_path {
        return path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{file_id}"));
    }
    if let Some(name) = crdt_filename {
        return name.to_string();
    }
    format!("{file_id}")
}
