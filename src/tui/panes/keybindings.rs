use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::FocusedPane;

pub fn render(frame: &mut Frame, area: Rect, focused: FocusedPane) {
    let spans = match focused {
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
        FocusedPane::RecentSeries | FocusedPane::Playlist => vec![
            key("Tab"),
            desc(" Next pane "),
            sep(),
            key("Ctrl-C"),
            desc(" Quit"),
        ],
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
