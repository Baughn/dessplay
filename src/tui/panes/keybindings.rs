use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::FocusedPane;

pub fn render(frame: &mut Frame, area: Rect, focused: FocusedPane, modal_open: bool) {
    let spans = if modal_open {
        vec![
            key("Up/Down"),
            desc(" Select "),
            sep(),
            key("Enter"),
            desc(" Open/Add "),
            sep(),
            key("Backspace"),
            desc(" Parent dir "),
            sep(),
            key("Esc"),
            desc(" Close "),
            sep(),
            key("Ctrl-C"),
            desc(" Quit"),
        ]
    } else {
        match focused {
            FocusedPane::Chat => vec![
                key("Tab"),
                desc(" Next pane "),
                sep(),
                key("Enter"),
                desc(" Send "),
                sep(),
                key("Esc"),
                desc(" Clear "),
                sep(),
                key("Ctrl-C"),
                desc(" Quit"),
            ],
            FocusedPane::RecentSeries => vec![
                key("Tab"),
                desc(" Next pane "),
                sep(),
                key("Up/Down"),
                desc(" Select "),
                sep(),
                key("Enter"),
                desc(" Browse "),
                sep(),
                key("Ctrl-C"),
                desc(" Quit"),
            ],
            FocusedPane::Playlist => vec![
                key("Tab"),
                desc(" Next pane "),
                sep(),
                key("Ctrl-C"),
                desc(" Quit"),
            ],
        }
    };

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

fn key(s: &str) -> Span<'_> {
    Span::styled(
        s,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
}

fn desc(s: &str) -> Span<'_> {
    Span::styled(s, Style::default().fg(Color::DarkGray))
}

fn sep() -> Span<'static> {
    Span::styled("| ", Style::default().fg(Color::DarkGray))
}
