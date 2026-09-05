//! A view of a durably saved adventure. Actions cross the normal UI bridge;
//! no dungeon time passes here, including while help or notices are open.

use super::*;
use crate::roguelike::{Action, BODY_PARTS, HEIGHT, Outcome, Point, Run, WIDTH};
use crate::roguelike_store::Command;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::widgets::{Paragraph, Wrap};

/// A saved expedition displayed above the client's recent chat.
pub struct RoguelikeModal {
    run: Option<Run>,
    waiting: bool,
    error: Option<String>,
    notices: Vec<String>,
    help: bool,
    help_scroll: u16,
}

impl Default for RoguelikeModal {
    fn default() -> Self {
        Self::new()
    }
}

impl RoguelikeModal {
    /// Display loading until the local save has been read.
    pub fn new() -> Self {
        Self {
            run: None,
            waiting: true,
            error: None,
            notices: Vec::new(),
            help: false,
            help_scroll: 0,
        }
    }

    /// Display a committed save and accept the next turn.
    pub fn set_run(&mut self, run: Run) {
        self.run = Some(run);
        self.waiting = false;
        self.error = None;
    }

    /// Keep the last saved view and explain why a command failed.
    pub fn set_error(&mut self, error: String) {
        self.waiting = false;
        self.error = Some(error);
    }

    /// Arrivals remain visible through moves, saves, help, and game over.
    pub fn set_notice(&mut self, notice: String) {
        self.notices.push(notice);
    }

    fn can_act(&self) -> bool {
        !self.waiting && self.run.as_ref().is_some_and(|run| !run.is_finished())
    }

    fn act(&mut self, action: Action) -> Option<Msg> {
        if !self.can_act() {
            return Some(Msg::None);
        }
        self.waiting = true;
        Some(Msg::Roguelike(Command::Act(action)))
    }

    fn new_run(&mut self) -> Option<Msg> {
        if !self.waiting && self.run.as_ref().is_some_and(Run::is_finished) {
            self.waiting = true;
            Some(Msg::Roguelike(Command::NewRun))
        } else {
            Some(Msg::None)
        }
    }

    fn toggle_help(&mut self) -> Option<Msg> {
        self.help = !self.help;
        self.help_scroll = 0;
        Some(Msg::None)
    }

    fn acknowledge(&mut self) -> Option<Msg> {
        self.notices.clear();
        Some(Msg::None)
    }

    fn close(&mut self) -> Option<Msg> {
        if self.help {
            self.help = false;
            Some(Msg::None)
        } else {
            Some(Msg::CloseModal)
        }
    }

    /// Active controls, derived from the same tables as input dispatch.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        let mut bar = Vec::new();
        if !self.notices.is_empty() {
            bar.push(("Enter", "Acknowledge arrival"));
        }
        if self.help {
            bar.extend(HELP.bar());
        } else if self.can_act() {
            bar.extend(PLAY.bar());
        } else if !self.waiting && self.run.as_ref().is_some_and(Run::is_finished) {
            bar.extend(FINISHED.bar());
        }
        bar.extend(COMMON.bar());
        bar
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal = LogModal::area(area);
        frame.render_widget(Clear, modal);
        let title = if self.waiting && self.run.is_some() {
            " THE WAITING BELOW · saving... "
        } else {
            " THE WAITING BELOW "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                title,
                Style::default().add_modifier(Modifier::BOLD),
            ))
            .title_bottom(" ?: guide   F4: return to chat ");
        let mut inner = block.inner(modal);
        frame.render_widget(block, modal);
        if inner.is_empty() {
            return;
        }

        if !self.notices.is_empty() {
            // Put the acknowledgement first so even a narrow frame explains
            // how to dismiss the banner. Latest arrival is always visible.
            let notices = self
                .notices
                .iter()
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            let text = format!("[Enter: acknowledge] {notices}");
            let height = wrapped_height(&text, inner.width).min(3).min(inner.height);
            frame.render_widget(
                Paragraph::new(text)
                    .style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                    .wrap(Wrap { trim: false }),
                take_rows(&mut inner, height),
            );
        }
        if let Some(error) = &self.error {
            let text = format!("Could not save/load: {error}. F4 closes; reopen to retry.");
            let height = wrapped_height(&text, inner.width).min(3).min(inner.height);
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: false }),
                take_rows(&mut inner, height),
            );
        }
        if inner.is_empty() {
            return;
        }
        if self.help {
            self.render_help(frame, inner);
            return;
        }
        let Some(run) = &self.run else {
            let text = if self.error.is_some() {
                "A lantern flickers beneath the waiting room.\n\nYour expedition could not be opened."
            } else {
                "A lantern flickers beneath the waiting room.\n\nLoading your saved expedition..."
            };
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
            return;
        };

        if run.is_finished() {
            render_epitaph(frame, inner, run);
            return;
        }

        let goal = if run.relic {
            "The ember is yours. Return to the surface < on depth 1."
        } else {
            "Find the buried ember on floor 5; bring it back to the surface."
        };
        frame.render_widget(
            Paragraph::new(goal).style(Style::default().fg(Color::Cyan)),
            take_rows(&mut inner, 1),
        );
        let status = format!(
            "Depth {}  Turn {}  Blood {}/1000  Breath {}/100  Pain {}  Bleed {}",
            run.depth + 1,
            run.turns,
            run.body.blood,
            run.body.stamina,
            run.body.pain(),
            run.body.bleeding(),
        );
        frame.render_widget(
            Paragraph::new(status).style(if run.body.bleeding() > 0 || run.body.blood < 400 {
                Style::default().fg(Color::LightRed)
            } else {
                Style::default()
            }),
            take_rows(&mut inner, 1),
        );

        let wide = inner.width >= 80;
        if !wide {
            frame.render_widget(
                Paragraph::new(supplies(run)).style(theme::dim()),
                take_rows(&mut inner, 1),
            );
            let wounds = ["Head", "Torso", "L.arm", "R.arm", "L.leg", "R.leg"]
                .iter()
                .zip(&run.body.wounds)
                .map(|(part, wound)| {
                    Span::styled(
                        format!(
                            "{part} {}{}  ",
                            wound.severity,
                            if wound.bleeding > 0 { "!" } else { "" }
                        ),
                        if wound.bleeding > 0 {
                            Style::default().fg(Color::LightRed)
                        } else if wound.severity > 0 {
                            Style::default().fg(Color::Yellow)
                        } else {
                            theme::dim()
                        },
                    )
                })
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(Line::from(wounds)), take_rows(&mut inner, 1));
        }
        let message_height = inner.height.saturating_sub(4).min(3);
        let map_height = inner.height.saturating_sub(message_height);
        let mut map = take_rows(&mut inner, map_height);
        if wide {
            let sidebar = Rect {
                x: map.right().saturating_sub(27),
                width: 27,
                ..map
            };
            map.width = map.width.saturating_sub(28);
            render_body(frame, sidebar, run);
        }
        render_map(frame, map, run);
        if !inner.is_empty() {
            let lines: Vec<_> = run
                .log
                .iter()
                .rev()
                .take(usize::from(inner.height))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|text| Line::from(text.as_str()))
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }

    fn render_help(&mut self, frame: &mut Frame, area: Rect) {
        let text = concat!(
            "A FIVE-MINUTE EXPEDITION\n",
            "Recover the buried ember from floor five, then climb out.\n",
            "Every action is saved. Closing the modal or client pauses everything.\n\n",
            "MOVE / FIGHT   Arrows, numpad 1-9, or vi keys:\n",
            "                 y k u       7 8 9\n",
            "                 h @ l       4 @ 6\n",
            "                 b j n       1 2 3\n",
            "Walk into an enemy to attack; walk over supplies to collect them.\n",
            ". / 5: wait   a: bandage   e: eat   r: rest   < / >: stairs\n\n",
            "STAY ALIVE\n",
            "Wounds affect separate body parts. Bleeding drains blood each turn.\n",
            "Bandages stop bleeding; rest when safe to recover. Watch your breath\n",
            "and nutrition (100 is well fed). Retreat can be wiser than a fight.\n",
            "Rest is one turn, never an unattended fast-forward.\n\n",
            "MAP   @ you   # wall   . floor   < up   > down\n",
            "r ash rat   h hollow pilgrim   W iron warden\n",
            "! bandages   % food   $ gold   ) weapon   [ armor   * ember\n",
            "Only nearby spaces are visible; explored terrain remains dim.\n",
            "Enemies and supplies are shown only while in sight.\n\n",
            "Friends arriving appear above this guide. Enter acknowledges them.\n",
            "Your final expedition summary is shared in chat when you die.\n",
            "n: new expedition after death or escape   F4: return to chat\n",
            "Up/Down: scroll guide   ? / Esc: close guide"
        );
        let rows: usize = text
            .lines()
            .map(|line| usize::from(wrapped_height(line, area.width)))
            .sum();
        let max = rows.saturating_sub(usize::from(area.height));
        self.help_scroll = self.help_scroll.min(max.min(usize::from(u16::MAX)) as u16);
        frame.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .scroll((self.help_scroll, 0)),
            area,
        );
    }
}

fn take_rows(area: &mut Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    let taken = Rect { height, ..*area };
    area.y += height;
    area.height -= height;
    taken
}

fn wrapped_height(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 0;
    }
    super::super::components::wrap_body(text, usize::from(width), usize::from(width))
        .len()
        .min(usize::from(u16::MAX)) as u16
}

fn supplies(run: &Run) -> String {
    format!(
        "Bandages {}  Food {}  Nutrition {}  Gold {}  Weapon +{}  Armor +{}",
        run.bandages, run.food, run.body.hunger, run.gold, run.weapon, run.armor
    )
}

fn render_body(frame: &mut Frame, area: Rect, run: &Run) {
    let mut lines = vec![Line::from(Span::styled(
        "CONDITION",
        Style::default().fg(Color::Cyan),
    ))];
    for (part, wound) in BODY_PARTS.iter().zip(&run.body.wounds) {
        let label = if wound.severity == 0 {
            "sound".to_string()
        } else {
            format!(
                "wound {}{}",
                wound.severity,
                if wound.bleeding > 0 { " bleeding" } else { "" }
            )
        };
        lines.push(Line::from(Span::styled(
            format!("{part:<10} {label}"),
            if wound.bleeding > 0 {
                Style::default().fg(Color::LightRed)
            } else if wound.severity > 0 {
                Style::default().fg(Color::Yellow)
            } else {
                theme::dim()
            },
        )));
    }
    lines.extend([
        Line::from(format!("Bandages {}   Food {}", run.bandages, run.food)),
        Line::from(format!("Nutrition {}   Gold {}", run.body.hunger, run.gold)),
        Line::from(format!("Weapon +{}   Armor +{}", run.weapon, run.armor)),
        Line::from(format!("Kills {}   Deepest {}", run.kills, run.deepest)),
    ]);
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_map(frame: &mut Frame, area: Rect, run: &Run) {
    if area.is_empty() {
        return;
    }
    let width = usize::from(area.width).min(WIDTH as usize);
    let height = usize::from(area.height).min(HEIGHT as usize);
    let left = (run.position.x.max(0) as usize)
        .saturating_sub(width / 2)
        .min((WIDTH as usize).saturating_sub(width));
    let top = (run.position.y.max(0) as usize)
        .saturating_sub(height / 2)
        .min((HEIGHT as usize).saturating_sub(height));
    let map_area = Rect {
        x: area.x + (area.width - width as u16) / 2,
        width: width as u16,
        ..area
    };
    let rows: Vec<_> = (top..top + height)
        .map(|y| {
            Line::from(
                (left..left + width)
                    .map(|x| {
                        let point = Point {
                            x: x as i32,
                            y: y as i32,
                        };
                        let glyph = run.glyph(point);
                        let visible = run
                            .floor()
                            .visible
                            .get(y * WIDTH as usize + x)
                            .copied()
                            .unwrap_or(false);
                        let style = if glyph == '@' {
                            Style::default()
                                .fg(Color::LightCyan)
                                .add_modifier(Modifier::BOLD)
                        } else if !visible {
                            theme::dim()
                        } else {
                            Style::default().fg(match glyph {
                                '#' => Color::Gray,
                                '.' => Color::DarkGray,
                                '<' | '>' => Color::LightCyan,
                                '$' | '*' => Color::Yellow,
                                '!' | '%' => Color::LightGreen,
                                ')' | '[' => Color::LightBlue,
                                _ => Color::LightRed,
                            })
                        };
                        Span::styled(glyph.to_string(), style)
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), map_area);
}

fn render_epitaph(frame: &mut Frame, area: Rect, run: &Run) {
    let (heading, color) = match &run.outcome {
        Outcome::Dead(_) => ("HERE ENDS YOUR EXPEDITION", Color::LightRed),
        Outcome::Escaped => ("THE EMBER COMES HOME", Color::LightGreen),
        Outcome::Alive => return,
    };
    let mut lines = vec![
        Line::from(Span::styled(
            heading,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(run.summary()),
        Line::from(format!(
            "{} turns · {} kills · {} gold · deepest floor {}",
            run.turns, run.kills, run.gold, run.deepest
        )),
        Line::from(""),
        Line::from("n: begin a new expedition   ?: guide   F4 / Esc: return to chat"),
        Line::from(""),
    ];
    lines.extend(
        run.log
            .iter()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|text| Line::from(text.as_str())),
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

static COMMON: Keymap<RoguelikeModal, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Char('?'),
        bar: Some(("?", "Guide")),
        action: RoguelikeModal::toggle_help,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Enter),
        bar: None,
        action: RoguelikeModal::acknowledge,
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Esc),
        bar: Some(("Esc", "Close")),
        action: RoguelikeModal::close,
    },
]);

static FINISHED: Keymap<RoguelikeModal, Msg> = Keymap(&[Binding {
    pattern: KeyPattern::Char('n'),
    bar: Some(("n", "New expedition")),
    action: RoguelikeModal::new_run,
}]);

static HELP: Keymap<RoguelikeModal, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Down),
        bar: Some(("↑/↓", "Scroll guide")),
        action: |modal| {
            modal.help_scroll = modal.help_scroll.saturating_add(1);
            Some(Msg::None)
        },
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Up),
        bar: None,
        action: |modal| {
            modal.help_scroll = modal.help_scroll.saturating_sub(1);
            Some(Msg::None)
        },
    },
    Binding {
        pattern: KeyPattern::Plain(Key::PageDown),
        bar: None,
        action: |modal| {
            modal.help_scroll = modal.help_scroll.saturating_add(8);
            Some(Msg::None)
        },
    },
    Binding {
        pattern: KeyPattern::Plain(Key::PageUp),
        bar: None,
        action: |modal| {
            modal.help_scroll = modal.help_scroll.saturating_sub(8);
            Some(Msg::None)
        },
    },
]);

static PLAY: Keymap<RoguelikeModal, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Plain(Key::Up),
        bar: Some(("arrows/vi/1-9", "Move/fight")),
        action: |m| m.act(Action::Move(0, -1)),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Down),
        bar: None,
        action: |m| m.act(Action::Move(0, 1)),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Left),
        bar: None,
        action: |m| m.act(Action::Move(-1, 0)),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Right),
        bar: None,
        action: |m| m.act(Action::Move(1, 0)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['k', '8']),
        bar: None,
        action: |m| m.act(Action::Move(0, -1)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['j', '2']),
        bar: None,
        action: |m| m.act(Action::Move(0, 1)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['h', '4']),
        bar: None,
        action: |m| m.act(Action::Move(-1, 0)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['l', '6']),
        bar: None,
        action: |m| m.act(Action::Move(1, 0)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['y', '7']),
        bar: None,
        action: |m| m.act(Action::Move(-1, -1)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['u', '9']),
        bar: None,
        action: |m| m.act(Action::Move(1, -1)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['b', '1']),
        bar: None,
        action: |m| m.act(Action::Move(-1, 1)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['n', '3']),
        bar: None,
        action: |m| m.act(Action::Move(1, 1)),
    },
    Binding {
        pattern: KeyPattern::Chars(&['.', '5']),
        bar: Some(("./5", "Wait")),
        action: |m| m.act(Action::Wait),
    },
    Binding {
        pattern: KeyPattern::Char('a'),
        bar: Some(("a", "Bandage")),
        action: |m| m.act(Action::Bandage),
    },
    Binding {
        pattern: KeyPattern::Char('e'),
        bar: Some(("e", "Eat")),
        action: |m| m.act(Action::Eat),
    },
    Binding {
        pattern: KeyPattern::Char('r'),
        bar: Some(("r", "Rest")),
        action: |m| m.act(Action::Rest),
    },
    Binding {
        pattern: KeyPattern::Chars(&['<', '>']),
        bar: Some(("</>", "Stairs")),
        action: |m| m.act(Action::Stairs),
    },
]);

passive_modal!(RoguelikeModal);

impl AppComponent<Msg, NoUserEvent> for RoguelikeModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if let Some(msg) = COMMON.dispatch(self, ev) {
            return Some(msg);
        }
        let map = if self.help {
            &HELP
        } else if self.run.as_ref().is_some_and(Run::is_finished) {
            &FINISHED
        } else {
            &PLAY
        };
        map.dispatch(self, ev).or(Some(Msg::None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};
    use tuirealm::ratatui::{Terminal, backend::TestBackend};
    use tuirealm::testing::buffer_to_string;

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn render(modal: &mut RoguelikeModal, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        buffer_to_string(
            terminal
                .draw(|frame| modal.render(frame, frame.area()))
                .unwrap()
                .buffer,
        )
    }

    #[test]
    fn actions_require_a_saved_response_and_do_not_mutate_the_view() {
        let mut modal = RoguelikeModal::new();
        assert_eq!(modal.on(&key(Key::Char('.'))), Some(Msg::None));
        let run = Run::new(19);
        modal.set_run(run.clone());
        assert_eq!(
            modal.on(&key(Key::Char('.'))),
            Some(Msg::Roguelike(Command::Act(Action::Wait)))
        );
        assert_eq!(modal.run, Some(run.clone()));
        assert_eq!(modal.on(&key(Key::Char('.'))), Some(Msg::None));
        modal.set_error("disk full".into());
        assert_eq!(modal.run, Some(run));
        assert_eq!(
            modal.on(&key(Key::Char('.'))),
            Some(Msg::Roguelike(Command::Act(Action::Wait)))
        );
    }

    #[test]
    fn vi_and_numpad_diagonals_do_not_collide_with_supplies_or_new_runs() {
        for (chars, action) in [
            (vec!['y', '7'], Action::Move(-1, -1)),
            (vec!['u', '9'], Action::Move(1, -1)),
            (vec!['b', '1'], Action::Move(-1, 1)),
            (vec!['n', '3'], Action::Move(1, 1)),
            (vec!['a'], Action::Bandage),
        ] {
            for ch in chars {
                let mut modal = RoguelikeModal::new();
                modal.set_run(Run::new(19));
                assert_eq!(
                    modal.on(&key(Key::Char(ch))),
                    Some(Msg::Roguelike(Command::Act(action)))
                );
            }
        }
        let mut modal = RoguelikeModal::new();
        let mut run = Run::new(19);
        run.outcome = Outcome::Dead("a test of courage".into());
        modal.set_run(run);
        assert_eq!(modal.on(&key(Key::Char('b'))), Some(Msg::None));
        assert_eq!(
            modal.on(&key(Key::Char('n'))),
            Some(Msg::Roguelike(Command::NewRun))
        );
    }

    #[test]
    fn shifted_stair_and_help_characters_work() {
        for ch in ['<', '>'] {
            let mut modal = RoguelikeModal::new();
            modal.set_run(Run::new(19));
            let event = Event::Keyboard(KeyEvent {
                code: Key::Char(ch),
                modifiers: KeyModifiers::SHIFT,
            });
            assert_eq!(
                modal.on(&event),
                Some(Msg::Roguelike(Command::Act(Action::Stairs)))
            );
        }
    }

    #[test]
    fn arrivals_survive_loading_help_and_finished_runs_until_acknowledged() {
        let mut modal = RoguelikeModal::new();
        modal.set_notice("Nero joined".into());
        assert!(render(&mut modal, 100, 40).contains("Nero joined"));
        modal.set_run(Run::new(19));
        modal.on(&key(Key::Char('?')));
        assert!(render(&mut modal, 100, 40).contains("Nero joined"));
        assert_eq!(modal.on(&key(Key::Char('.'))), Some(Msg::None));
        modal.on(&key(Key::Esc));
        let mut run = Run::new(19);
        run.outcome = Outcome::Escaped;
        modal.set_run(run);
        assert!(render(&mut modal, 100, 40).contains("Nero joined"));
        assert_eq!(modal.on(&key(Key::Enter)), Some(Msg::None));
        assert!(!render(&mut modal, 100, 40).contains("Nero joined"));
    }

    #[test]
    fn viewport_keeps_the_player_visible_and_never_draws_over_chat() {
        let mut modal = RoguelikeModal::new();
        modal.set_run(Run::new(19));
        for (width, height) in [(40, 24), (80, 24), (120, 40)] {
            let screen = render(&mut modal, width, height);
            assert!(screen.contains('@'), "{width}x{height}: {screen}");
            let last_game_row = u32::from(height) * 2 / 3;
            for line in screen.lines().skip(last_game_row as usize) {
                assert!(line.trim().is_empty(), "game rendered below modal: {line}");
            }
        }
        for (width, height) in [(0, 0), (1, 1), (2, 2), (10, 6)] {
            render(&mut modal, width, height);
        }
    }

    #[test]
    fn help_scroll_reaches_the_end_on_a_short_terminal() {
        let mut modal = RoguelikeModal::new();
        modal.set_run(Run::new(19));
        modal.on(&key(Key::Char('?')));
        for _ in 0..50 {
            modal.on(&key(Key::PageDown));
        }
        let screen = render(&mut modal, 80, 24);
        assert!(screen.contains("close guide"), "{screen}");
    }
}
