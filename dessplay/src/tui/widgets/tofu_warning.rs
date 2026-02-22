use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use crate::tui::ui_state::TofuWarningState;

pub fn keybindings() -> &'static [(&'static str, &'static str)] {
    &[
        ("Ctrl-F", "Accept new certificate"),
        ("Esc", "Reject"),
        ("Ctrl-C", "Quit"),
    ]
}

/// Format a fingerprint as colon-separated hex bytes.
fn format_fingerprint(fp: &[u8]) -> String {
    fp.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn render_tofu_warning(area: Rect, buf: &mut Buffer, state: &TofuWarningState) {
    Clear.render(area, buf);

    let block = Block::default()
        .title(" CERTIFICATE CHANGED ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 8 || inner.width < 30 {
        return;
    }

    // Center content
    let content_width = inner.width.min(72);
    let h_pad = (inner.width - content_width) / 2;
    let content_area = Rect {
        x: inner.x + h_pad,
        y: inner.y + 1,
        width: content_width,
        height: inner.height.saturating_sub(2),
    };

    let warn_style = Style::default()
        .fg(Color::Red)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::Yellow);
    let fp_style = Style::default().fg(Color::White);
    let dim_style = Style::default().fg(Color::Gray);

    let stored_fp = format_fingerprint(&state.stored_fingerprint);
    let received_fp = format_fingerprint(&state.received_fingerprint);

    let lines = vec![
        Line::from(Span::styled(
            "The server's TLS certificate has changed!",
            warn_style,
        )),
        Line::from(""),
        Line::from(Span::styled("This could mean:", dim_style)),
        Line::from(Span::styled(
            "  - The server was reinstalled or its keys were rotated",
            dim_style,
        )),
        Line::from(Span::styled(
            "  - Someone is intercepting your connection (MITM attack)",
            dim_style,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Server:   ", label_style),
            Span::styled(&state.server, fp_style),
        ]),
        Line::from(""),
        Line::from(Span::styled("Stored:   ", label_style)),
        Line::from(Span::styled(stored_fp, fp_style)),
        Line::from(""),
        Line::from(Span::styled("Received: ", label_style)),
        Line::from(Span::styled(received_fp, fp_style)),
        Line::from(""),
        Line::from(Span::styled(
            "Do NOT accept unless you are SURE this change is intentional",
            warn_style,
        )),
        Line::from(Span::styled(
            "(e.g. you know the server was reset).",
            warn_style,
        )),
    ];

    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(content_area, buf);
}
