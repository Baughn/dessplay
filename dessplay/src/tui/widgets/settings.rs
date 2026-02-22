use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::tui::ui_state::SettingsState;

pub fn render_settings(area: Rect, buf: &mut Buffer, state: &SettingsState) {
    Clear.render(area, buf);

    let block = Block::default()
        .title(" Settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 10 || inner.width < 30 {
        return;
    }

    // Center the form in the available space
    let form_width = inner.width.min(60);
    let h_pad = (inner.width - form_width) / 2;
    let form_area = Rect {
        x: inner.x + h_pad,
        y: inner.y + 1,
        width: form_width,
        height: inner.height.saturating_sub(2),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // username
            Constraint::Length(2), // server
            Constraint::Length(2), // player
            Constraint::Length(2), // password
            Constraint::Min(3),   // media roots
            Constraint::Length(2), // validation/save hint
        ])
        .split(form_area);

    render_field(buf, rows[0], "Username", &state.username, state.focused_field == 0);
    render_field(buf, rows[1], "Server", &state.server, state.focused_field == 1);

    // Player: show as toggle rather than free text
    render_toggle_field(buf, rows[2], "Player", &state.player, state.focused_field == 2);

    // Password: show masked
    let masked = "*".repeat(state.password.len());
    render_field(buf, rows[3], "Password", &masked, state.focused_field == 3);

    // Media roots
    render_media_roots(buf, rows[4], &state.media_roots, state.focused_field == 4);

    // Validation / save hint
    let hint = if state.is_valid() {
        Line::from(Span::styled(
            "  Ctrl-S to save",
            Style::default().fg(Color::Green),
        ))
    } else {
        let mut issues = Vec::new();
        if state.username.trim().is_empty() {
            issues.push("username required");
        }
        if state.server.trim().is_empty() {
            issues.push("server required");
        }
        if state.media_roots.is_empty() {
            issues.push("add at least one media root");
        }
        Line::from(Span::styled(
            format!("  {}", issues.join(", ")),
            Style::default().fg(Color::Red),
        ))
    };
    Paragraph::new(hint).render(rows[5], buf);
}

fn render_field(buf: &mut Buffer, area: Rect, label: &str, value: &str, focused: bool) {
    if area.height == 0 {
        return;
    }

    let label_style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let value_style = if focused {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Gray)
    };

    let lines = vec![
        Line::from(Span::styled(format!("  {label}:"), label_style)),
        Line::from(Span::styled(format!("  {value}"), value_style)),
    ];
    Paragraph::new(lines).render(area, buf);
}

fn render_toggle_field(buf: &mut Buffer, area: Rect, label: &str, value: &str, focused: bool) {
    if area.height == 0 {
        return;
    }

    let label_style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let hint = if focused { " (Enter to toggle)" } else { "" };

    let lines = vec![
        Line::from(Span::styled(format!("  {label}:"), label_style)),
        Line::from(vec![
            Span::styled(format!("  {value}"), Style::default().fg(Color::Green)),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]),
    ];
    Paragraph::new(lines).render(area, buf);
}

fn render_media_roots(buf: &mut Buffer, area: Rect, roots: &[std::path::PathBuf], focused: bool) {
    if area.height == 0 {
        return;
    }

    let label_style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let mut lines = vec![Line::from(Span::styled("  Media Roots:", label_style))];

    if roots.is_empty() {
        let hint = if focused {
            "(Enter to add)"
        } else {
            "(none)"
        };
        lines.push(Line::from(Span::styled(
            format!("    {hint}"),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, root) in roots.iter().enumerate() {
            let marker = if i == 0 { " [download]" } else { "" };
            let style = if i == 0 {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("    {}", root.display()), style),
                Span::styled(marker, Style::default().fg(Color::Blue)),
            ]));
        }
        if focused {
            lines.push(Line::from(Span::styled(
                "    Enter=add  d=remove  Ctrl-j/k=reorder",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    Paragraph::new(lines).render(area, buf);
}
