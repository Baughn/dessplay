use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::tui::ui_state::{FocusedPane, Screen};

pub fn render_keybinding_bar(area: Rect, buf: &mut Buffer, screen: &Screen, focus: &FocusedPane) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let bindings = match screen {
        Screen::Settings => vec![
            ("Tab", "Next field"),
            ("Shift-Tab", "Prev field"),
            ("Ctrl-S", "Save"),
            ("Ctrl-C", "Quit"),
        ],
        Screen::FileBrowser => vec![
            ("Enter", "Select"),
            ("Esc", "Back"),
            ("Ctrl-C", "Quit"),
        ],
        Screen::Main => match focus {
            FocusedPane::Chat => vec![
                ("Tab", "Next pane"),
                ("Enter", "Send"),
                ("Esc", "Clear"),
                ("Ctrl-C", "Quit"),
            ],
            FocusedPane::Playlist => vec![
                ("Tab", "Next pane"),
                ("a", "Add"),
                ("d", "Remove"),
                ("C-j/k", "Move"),
                ("Ctrl-C", "Quit"),
            ],
            FocusedPane::RecentSeries => vec![
                ("Tab", "Next pane"),
                ("Enter", "Browse"),
                ("Ctrl-C", "Quit"),
            ],
        },
    };

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
