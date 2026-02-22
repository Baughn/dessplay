use std::sync::atomic::Ordering;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::tui::ui_state::HashingState;

/// Render a centered modal overlay showing file hashing progress.
pub fn render_hashing_progress(area: Rect, buf: &mut Buffer, state: &HashingState) {
    // Size the modal: 50 wide, 7 tall (or smaller if terminal is tiny)
    let width = area.width.min(50);
    let height = area.height.min(7);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal = Rect::new(x, y, width, height);

    Clear.render(modal, buf);

    let block = Block::default()
        .title(" Hashing file ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(modal);
    block.render(modal, buf);

    if inner.height == 0 || inner.width < 10 {
        return;
    }

    let bytes_done = state.bytes_hashed.load(Ordering::Relaxed);
    let total = state.total_bytes;
    let fraction = if total > 0 {
        (bytes_done as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let percent = (fraction * 100.0) as u64;

    let mut lines = Vec::new();

    // Line 1: filename (truncated to fit)
    let max_name_len = inner.width as usize;
    let display_name = if state.filename.len() > max_name_len {
        format!("...{}", &state.filename[state.filename.len() - (max_name_len - 3)..])
    } else {
        state.filename.clone()
    };
    lines.push(Line::from(Span::styled(
        display_name,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    // Line 2: progress bar  [========>        ] 42%
    let pct_label = format!(" {percent}%");
    let bar_width = (inner.width as usize).saturating_sub(2 + pct_label.len()); // 2 for [ ]
    let filled = (bar_width as f64 * fraction) as usize;
    let empty = bar_width.saturating_sub(filled);
    let bar = format!("[{}{}]{pct_label}", "=".repeat(filled), " ".repeat(empty));
    lines.push(Line::from(Span::styled(bar, Style::default().fg(Color::Cyan))));

    // Line 3: bytes done / total
    if inner.height > 2 {
        let done_str = format_bytes(bytes_done);
        let total_str = format_bytes(total);
        lines.push(Line::from(Span::styled(
            format!("{done_str} / {total_str}"),
            Style::default().fg(Color::Gray),
        )));
    }

    Paragraph::new(lines).render(inner, buf);
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}
