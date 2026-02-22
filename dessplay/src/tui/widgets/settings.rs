use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::tui::ui_state::SettingsState;

pub fn keybindings() -> &'static [(&'static str, &'static str)] {
    &[
        ("Tab", "Next field"),
        ("Shift-Tab", "Prev field"),
        ("Ctrl-S", "Save"),
        ("Esc", "Cancel"),
        ("Ctrl-C", "Quit"),
    ]
}

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

    let has_alert = state.alert.is_some();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if has_alert { 2 } else { 0 }), // alert banner
            Constraint::Length(2), // username
            Constraint::Length(2), // server
            Constraint::Length(2), // player
            Constraint::Length(2), // password
            Constraint::Min(3),   // media roots
            Constraint::Length(2), // validation/save hint
        ])
        .split(form_area);

    // Alert banner
    if let Some(ref alert) = state.alert {
        let alert_line = Line::from(Span::styled(
            format!("  {alert}"),
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
        Paragraph::new(alert_line).render(rows[0], buf);
    }

    let username_ok = state.is_username_valid();
    let server_ok = state.is_server_valid();
    let roots_ok = state.has_media_roots();

    render_field_with_indicator(buf, rows[1], "Username", &state.username, state.focused_field == 0, username_ok);
    render_field_with_indicator(buf, rows[2], "Server", &state.server, state.focused_field == 1, server_ok);

    // Player: show as toggle rather than free text (always valid)
    render_toggle_field(buf, rows[3], "Player", &state.player, state.focused_field == 2);

    // Password: show masked (no validation)
    let masked = "*".repeat(state.password.len());
    render_field(buf, rows[4], "Password", &masked, state.focused_field == 3);

    // Media roots
    render_media_roots_with_indicator(buf, rows[5], &state.media_roots, state.focused_field == 4, roots_ok);

    // Validation / save hint
    let hint = if state.is_valid() {
        Line::from(Span::styled(
            "  Ctrl-S to save",
            Style::default().fg(Color::Green),
        ))
    } else {
        let mut issues = Vec::new();
        if !username_ok {
            issues.push("username required");
        }
        if let Some(err) = state.server_error() {
            issues.push(err);
        }
        if !roots_ok {
            issues.push("add at least one media root");
        }
        Line::from(Span::styled(
            format!("  {}", issues.join(", ")),
            Style::default().fg(Color::Red),
        ))
    };
    Paragraph::new(hint).render(rows[6], buf);
}

fn render_field_with_indicator(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    valid: bool,
) {
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

    let indicator = if valid {
        Span::styled(" [ok]", Style::default().fg(Color::Green))
    } else {
        Span::styled(" [X]", Style::default().fg(Color::Red))
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(format!("  {label}:"), label_style),
            indicator,
        ]),
        Line::from(Span::styled(format!("  {value}"), value_style)),
    ];
    Paragraph::new(lines).render(area, buf);
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

fn render_media_roots_with_indicator(
    buf: &mut Buffer,
    area: Rect,
    roots: &[std::path::PathBuf],
    focused: bool,
    valid: bool,
) {
    if area.height == 0 {
        return;
    }

    let label_style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let indicator = if valid {
        Span::styled(" [ok]", Style::default().fg(Color::Green))
    } else {
        Span::styled(" [X]", Style::default().fg(Color::Red))
    };

    let mut lines = vec![Line::from(vec![
        Span::styled("  Media Roots:", label_style),
        indicator,
    ])];

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

#[allow(dead_code)]
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
