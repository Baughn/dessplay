use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::tui::ui_state::FocusedPane;

/// A frame that reserves 1 line at the bottom for a keybinding bar.
pub struct WindowFrame {
    pub content: Rect,
    bar: Rect,
}

impl WindowFrame {
    pub fn new(area: Rect) -> Self {
        if area.height <= 1 {
            Self {
                content: area,
                bar: Rect::default(),
            }
        } else {
            Self {
                content: Rect {
                    height: area.height - 1,
                    ..area
                },
                bar: Rect {
                    x: area.x,
                    y: area.y + area.height - 1,
                    width: area.width,
                    height: 1,
                },
            }
        }
    }

    pub fn render_bar(&self, buf: &mut Buffer, bindings: &[(&str, &str)]) {
        render_bar(self.bar, buf, bindings);
    }
}

/// Render a keybinding bar for the main screen, routed by focused pane.
pub fn render_keybinding_bar(area: Rect, buf: &mut Buffer, focus: &FocusedPane) {
    let bindings: &[(&str, &str)] = match focus {
        FocusedPane::Chat => &[
            ("Tab", "Next pane"),
            ("Enter", "Send"),
            ("Esc", "Clear"),
            ("Ctrl-C", "Quit"),
        ],
        FocusedPane::Playlist => &[
            ("Tab", "Next pane"),
            ("a", "Add"),
            ("d", "Remove"),
            ("C-j/k", "Move"),
            ("Ctrl-C", "Quit"),
        ],
        FocusedPane::RecentSeries => &[
            ("Tab", "Next pane"),
            ("Enter", "Browse"),
            ("Ctrl-C", "Quit"),
        ],
    };
    render_bar(area, buf, bindings);
}

fn render_bar(area: Rect, buf: &mut Buffer, bindings: &[(&str, &str)]) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let key_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::DarkGray);
    let sep_style = Style::default().fg(Color::DarkGray);

    let mut spans: Vec<Span<'_>> = Vec::new();
    for (i, (key, desc)) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", sep_style));
        }
        spans.push(Span::styled(format!(" {key} "), key_style));
        spans.push(Span::styled(format!(" {desc}"), desc_style));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    paragraph.render(area, buf);
}
