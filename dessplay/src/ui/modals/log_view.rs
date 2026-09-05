use super::*;
use crate::logging::{LiveLogging, LogLevel, LogScope};
use tuirealm::event::{KeyEvent, KeyModifiers};
use tuirealm::ratatui::widgets::Paragraph;

/// Live diagnostic log, positioned above the last few chat lines.
pub struct LogModal {
    logging: Option<LiveLogging>,
    revision: u64,
    // None follows the tail; stable line/fragment identity survives eviction.
    anchor: Option<(u64, usize)>,
    row_keys: Vec<(u64, usize)>,
    top: usize,
    page: usize,
    focus: usize,
    dropdown: Option<usize>,
    error: Option<String>,
}

impl LogModal {
    /// Shared boundary for the overlay and the recent-chat strip beneath it.
    pub(crate) fn area(area: Rect) -> Rect {
        Rect {
            height: (u32::from(area.height) * 2 / 3) as u16,
            ..area
        }
    }

    /// Open on the newest retained lines.
    pub fn new(logging: Option<LiveLogging>) -> Self {
        Self {
            logging,
            revision: 0,
            anchor: None,
            row_keys: Vec::new(),
            top: 0,
            page: 1,
            focus: 0,
            dropdown: None,
            error: None,
        }
    }

    /// The shell's idle tick repaints only when new lines arrive.
    pub fn refresh_needed(&self) -> bool {
        self.logging
            .as_ref()
            .is_some_and(|logs| logs.revision() != self.revision)
    }

    /// Bindings are visible even when the terminal is too short for the footer.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Tab", "Control"),
            ("Enter", "Choose"),
            ("↑/↓", "Scroll/select"),
            ("End", "Live"),
            ("Esc", "Close"),
        ]
    }

    fn scroll_to(&mut self, top: usize) {
        let max = self.row_keys.len().saturating_sub(self.page);
        self.top = top.min(max);
        self.anchor = if top >= max {
            None
        } else {
            self.row_keys.get(self.top).copied()
        };
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Full width, upper two-thirds. Never grow downward on a tiny terminal.
        let modal = Self::area(area);
        frame.render_widget(Clear, modal);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_style(true))
            .title(if self.anchor.is_none() {
                "Logs · LIVE"
            } else {
                "Logs · scrollback"
            });
        let inner = block.inner(modal);
        frame.render_widget(block, modal);
        if inner.is_empty() {
            return;
        }
        let Some(logging) = &self.logging else {
            frame.render_widget(
                Paragraph::new("Live logging is unavailable in this session."),
                inner,
            );
            return;
        };
        let levels = logging.levels();
        let startup = logging.startup_filter().to_string();
        // Snapshot first, then derive its revision: a concurrent append must
        // still trigger the next refresh.
        let lines = logging.lines();
        self.revision = lines.last().map_or(0, |line| line.id + 1);
        let mut controls = vec![Line::from(format!("Session only · Startup: {startup}"))];
        for (idx, label) in ["DessPlay", "Rust (other crates)"].iter().enumerate() {
            let style = if self.focus == idx + 1 {
                theme::highlight_style()
            } else {
                Style::default()
            };
            controls.push(Line::from(Span::styled(
                format!("{label}: [ {} ▾ ]", levels[idx].label()),
                style,
            )));
        }
        let header_height = inner.height.min(3);
        frame.render_widget(
            Paragraph::new(controls),
            Rect {
                height: header_height,
                ..inner
            },
        );
        let body = Rect {
            y: inner.y + header_height,
            height: inner.height.saturating_sub(header_height + 1),
            ..inner
        };
        if body.width > 0 && body.height > 0 {
            let mut rows = Vec::new();
            self.row_keys.clear();
            for line in lines {
                for (part, (text, _)) in super::super::components::wrap_body(
                    &line.text,
                    body.width as usize,
                    body.width as usize,
                )
                .into_iter()
                .enumerate()
                {
                    self.row_keys.push((line.id, part));
                    rows.push(Line::from(text));
                }
            }
            self.page = body.height as usize;
            let max = rows.len().saturating_sub(self.page);
            self.top = self.anchor.map_or(max, |key| {
                self.row_keys
                    .iter()
                    .position(|row| *row >= key)
                    .unwrap_or(max)
                    .min(max)
            });
            if self.anchor.is_some() {
                self.anchor = self.row_keys.get(self.top).copied();
            }
            frame.render_widget(
                Paragraph::new(
                    rows.into_iter()
                        .skip(self.top)
                        .take(self.page)
                        .collect::<Vec<_>>(),
                ),
                body,
            );
        }
        let footer = self.error.as_deref().unwrap_or(
            "Tab: control · Enter: dropdown · ↑/↓/PgUp/PgDn: scroll · End: live · F11/Esc: close",
        );
        frame.render_widget(
            Paragraph::new(footer).style(theme::dim()),
            Rect {
                y: inner.bottom().saturating_sub(1),
                height: 1,
                ..inner
            },
        );
        if let Some(selected) = self.dropdown {
            let popup = Rect {
                x: inner.x,
                y: (inner.y + self.focus as u16 + 1).min(inner.bottom()),
                width: inner.width.min(24),
                height: inner
                    .bottom()
                    .saturating_sub(inner.y + self.focus as u16 + 1)
                    .min(9),
            };
            frame.render_widget(Clear, popup);
            let items: Vec<_> = LogLevel::ALL
                .iter()
                .map(|level| ListItem::new(level.label()))
                .collect();
            let mut state =
                tuirealm::ratatui::widgets::ListState::default().with_selected(Some(selected));
            frame.render_stateful_widget(
                tuirealm::ratatui::widgets::List::new(items)
                    .block(Block::default().borders(Borders::ALL))
                    .highlight_style(theme::highlight_style()),
                popup,
                &mut state,
            );
        }
    }
}

passive_modal!(LogModal);

impl AppComponent<Msg, NoUserEvent> for LogModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        let backwards = matches!(ev, Event::Keyboard(KeyEvent { code: Key::BackTab, modifiers }) if *modifiers == KeyModifiers::NONE || *modifiers == KeyModifiers::SHIFT);
        if backwards && self.dropdown.is_none() {
            self.focus = (self.focus + 2) % 3;
            return Some(Msg::None);
        }
        let Some(key) = plain(ev) else {
            return Some(Msg::None);
        };
        if let Some(selected) = self.dropdown.as_mut() {
            match key {
                Key::Up => *selected = selected.saturating_sub(1),
                Key::Down => *selected = (*selected + 1).min(LogLevel::ALL.len() - 1),
                Key::Home => *selected = 0,
                Key::End => *selected = LogLevel::ALL.len() - 1,
                Key::Esc => self.dropdown = None,
                Key::Enter => {
                    let level = LogLevel::ALL[*selected];
                    let scope = if self.focus == 1 {
                        LogScope::DessPlay
                    } else {
                        LogScope::Rust
                    };
                    self.error = self
                        .logging
                        .as_ref()
                        .and_then(|logs| logs.set_level(scope, level).err());
                    self.dropdown = None;
                }
                _ => {}
            }
            return Some(Msg::None);
        }
        match key {
            Key::Esc => return Some(Msg::CloseModal),
            Key::Tab => self.focus = (self.focus + 1) % 3,
            Key::Enter if self.focus > 0 => {
                if let Some(logging) = &self.logging {
                    self.dropdown = LogLevel::ALL
                        .iter()
                        .position(|level| *level == logging.levels()[self.focus - 1]);
                }
            }
            Key::Up => self.scroll_to(self.top.saturating_sub(1)),
            Key::Down => self.scroll_to(self.top.saturating_add(1)),
            Key::PageUp => self.scroll_to(self.top.saturating_sub(self.page)),
            Key::PageDown => self.scroll_to(self.top.saturating_add(self.page)),
            Key::Home => {
                self.top = 0;
                self.anchor = self.row_keys.first().copied();
            }
            Key::End => self.anchor = None,
            _ => {}
        }
        Some(Msg::None)
    }
}
