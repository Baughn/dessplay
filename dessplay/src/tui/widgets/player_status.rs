use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// Format seconds as MM:SS or HH:MM:SS.
fn format_time(secs: f64) -> String {
    let total = secs as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Render the player status bar with progress, current file, and blocking users.
pub fn render_player_status(
    area: Rect,
    buf: &mut Buffer,
    current_file_name: Option<&str>,
    position_secs: f64,
    duration_secs: Option<f64>,
    is_playing: bool,
    blocking_users: &[String],
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines = Vec::new();

    // Line 1: Play/pause indicator + progress bar + time
    let play_indicator = if is_playing { ">" } else { "||" };
    let pos_str = format_time(position_secs);
    let dur_str = duration_secs.map_or_else(|| "??:??".to_string(), format_time);

    // Calculate progress bar width
    let time_text = format!(" {pos_str} / {dur_str} ");
    let indicator_width = play_indicator.len() + 2; // [>] or [||]
    let bar_available =
        (inner.width as usize).saturating_sub(indicator_width + 1 + time_text.len());

    let progress_fraction = duration_secs
        .filter(|&d| d > 0.0)
        .map(|d| (position_secs / d).clamp(0.0, 1.0))
        .unwrap_or(0.0);

    let filled = (bar_available as f64 * progress_fraction) as usize;
    let empty = bar_available.saturating_sub(filled);

    let bar = format!(
        "[{play_indicator}] {}{}{time_text}",
        "=".repeat(filled),
        " ".repeat(empty),
    );

    lines.push(Line::from(Span::styled(
        format!("  {bar}"),
        Style::default().fg(Color::White),
    )));

    // Line 2: Now playing
    if inner.height > 1 {
        let file_name = current_file_name.unwrap_or("Idle");
        lines.push(Line::from(Span::styled(
            format!("  Now Playing: {file_name}"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }

    // Line 3: Blocking users (if any)
    if inner.height > 2 && !blocking_users.is_empty() {
        let names = blocking_users.join(", ");
        lines.push(Line::from(Span::styled(
            format!("  Waiting for: {names}"),
            Style::default().fg(Color::Yellow),
        )));
    }

    let paragraph = Paragraph::new(lines);
    paragraph.render(inner, buf);
}
