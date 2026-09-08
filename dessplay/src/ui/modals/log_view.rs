use super::*;
use crate::logging::{LiveLogging, LogLevel, LogScope};
use tuirealm::event::{KeyEvent, KeyModifiers};
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::widgets::{Clear, Paragraph};

/// Live diagnostic log, positioned above the last few chat lines.
pub struct LogModal {
    logging: Option<LiveLogging>,
    revision: u64,
    // None follows the tail; stable line/source offsets survive wrapping changes.
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
        if let Ok(bundle) = crate::ui::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut crate::ui::layout::Renderer::new(bundle));
        }
    }

    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut crate::ui::layout::Renderer,
    ) {
        // Full width, upper two-thirds. Never grow downward on a tiny terminal.
        let modal = Self::area(area);
        frame.render_widget(Clear, modal);
        let title = if self.anchor.is_none() {
            "Logs · LIVE"
        } else {
            "Logs · scrollback"
        };
        let levels = self
            .logging
            .as_ref()
            .map_or([LogLevel::Startup; 2], |logging| logging.levels());
        let startup = self
            .logging
            .as_ref()
            .map_or_else(String::new, |logging| logging.startup_filter().to_string());
        let footer = self.error.as_deref().unwrap_or(
            "Tab: control · Enter: dropdown · ↑/↓/PgUp/PgDn: scroll · End: live · F11/Esc: close",
        );
        let mut data = crate::ui::layout::Presentation::default()
            .text("title", title)
            .text("session-label", "Session only · Startup: ")
            .text("startup", startup)
            .text("app-label", "DessPlay")
            .text("app-level", levels[0].label())
            .text("other-label", "Rust (other crates)")
            .text("other-level", levels[1].label())
            .text("picker-end", "▾ ]")
            .text("footer", footer)
            .text(
                "unavailable",
                "Live logging is unavailable in this session.",
            )
            .boolean("has-unavailable", self.logging.is_none())
            .boolean("available", self.logging.is_some())
            .boolean("choose-app", self.dropdown.is_some() && self.focus == 1)
            .boolean("choose-other", self.dropdown.is_some() && self.focus == 2);
        if let Some(id) = match self.focus {
            1 => Some("log-app-control"),
            2 => Some("log-other-control"),
            _ => None,
        } {
            data = data
                .state(id, "focus")
                .component_style(id, theme::highlight_style());
        }
        let scene = match renderer.arrange("log", modal, &data) {
            Ok(scene) => scene,
            Err(error) => {
                tracing::error!(%error, "log layout failed");
                return;
            }
        };
        scene.paint(frame);
        let body = scene.slot("body");
        let Some(logging) = &self.logging else {
            scene.paint_overlays(frame);
            return;
        };
        // Snapshot before deriving its revision so a concurrent append repaints.
        let lines = logging.lines();
        self.revision = lines.last().map_or(0, |line| line.id + 1);
        if body.width > 0 && body.height > 0 {
            let mut rows = Vec::new();
            self.row_keys.clear();
            for line in lines {
                for fragment in renderer.measured_text(&line.text, body.width).iter() {
                    self.row_keys.push((line.id, fragment.source.start));
                    rows.push(Line::from(fragment.text.clone()));
                }
            }
            self.page = body.height as usize;
            let max = rows.len().saturating_sub(self.page);
            self.top = self
                .anchor
                .map_or(max, |key| source_anchor(&self.row_keys, key).min(max));
            if self.anchor.is_some() {
                self.anchor = self.row_keys.get(self.top).copied();
            }
            frame.render_widget(
                Paragraph::new(
                    rows.into_iter()
                        .skip(self.top)
                        .take(self.page)
                        .collect::<Vec<_>>(),
                )
                .style(scene.style("body")),
                body,
            );
        }
        scene.paint_overlays(frame);
        if let Some(selected) = self.dropdown {
            let popup = scene.slot(if self.focus == 1 {
                "app-options"
            } else {
                "other-options"
            });
            let rows = LogLevel::ALL
                .iter()
                .map(|level| crate::ui::layout::PresentedRow {
                    key: level.label().into(),
                    data: crate::ui::layout::Presentation::default().text("label", level.label()),
                    gap_after: false,
                })
                .collect::<Vec<_>>();
            let _ = renderer.paint_rows(
                frame,
                popup,
                "log-option",
                &rows,
                Some(selected),
                Style::default(),
            );
        }
    }
}

fn source_anchor(rows: &[(u64, usize)], key: (u64, usize)) -> usize {
    let after = rows.partition_point(|row| *row <= key);
    if after > 0 && rows[after - 1].0 == key.0 {
        after - 1
    } else {
        after
    }
}

#[cfg(test)]
mod layout_tests {
    use super::source_anchor;
    #[test]
    fn resizing_retains_the_fragment_containing_the_original_source_offset() {
        assert_eq!(
            source_anchor(&[(1, 0), (1, 8), (1, 16), (2, 0)], (1, 12)),
            1
        );
        assert_eq!(source_anchor(&[(1, 0), (1, 20), (2, 0)], (1, 12)), 0);
        assert_eq!(source_anchor(&[(2, 0), (2, 8)], (1, 12)), 0);
        assert_eq!(source_anchor(&[], (1, 12)), 0);
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
