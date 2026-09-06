//! Only committed observations are rendered. Recovery is a cancellable UI
//! controller: it issues one normal saved action and waits for its acknowledgement.
use super::*;
use crate::config::RoguelikeEffects;
use crate::roguelike::{
    Action, EventKind, HEIGHT, LootKind, Outcome, Point, RunView, Supplies, WIDTH,
};
use crate::roguelike_store::Command;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::widgets::{Paragraph, Wrap};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Game,
    Guide,
    Journal,
    Condition,
    Equipment,
}
#[derive(Clone, Copy)]
enum Direction {
    Attack,
    Close,
}
struct Recovery {
    started: Supplies,
    due: u64,
}

/// A committed expedition observation above the live party chat strip.
pub struct RoguelikeModal {
    run: Option<RunView>,
    waiting: bool,
    error: Option<String>,
    notices: Vec<String>,
    page: Page,
    cursor: ListCursor,
    direction: Option<Direction>,
    recovery: Option<Recovery>,
    now: u64,
    effects: RoguelikeEffects,
    flash_until: u64,
}
impl Default for RoguelikeModal {
    fn default() -> Self {
        Self::new()
    }
}
impl RoguelikeModal {
    /// Open a loading view; no expedition action is accepted until its save arrives.
    pub fn new() -> Self {
        Self {
            run: None,
            waiting: true,
            error: None,
            notices: Vec::new(),
            page: Page::Game,
            cursor: ListCursor::default(),
            direction: None,
            recovery: None,
            now: 0,
            effects: RoguelikeEffects::Full,
            flash_until: 0,
        }
    }
    /// Adopt a committed observation and acknowledge the outstanding action.
    pub fn set_run(&mut self, run: RunView) {
        // Historical injuries must not flash on load, or replay on duplicate replies.
        if let Some(old) = &self.run {
            let previous = old.journal.last().map_or(0, |entry| entry.id);
            if old.seed == run.seed
                && run.serious_wounds > old.serious_wounds
                && run
                    .journal
                    .iter()
                    .any(|entry| entry.id > previous && entry.kind == EventKind::Injury)
            {
                self.flash_until = self.now.saturating_add(450);
            }
        }
        if run.is_finished()
            || run.danger
            || !run.can_rest
            || run.last_step.interrupted
            || !run.last_step.changed
        {
            self.cancel_recovery();
        } else if let Some(recovery) = &mut self.recovery {
            recovery.due = self.now.saturating_add(250);
        }
        self.run = Some(run);
        self.waiting = false;
        self.error = None;
    }
    /// Retain the previous observation and interrupt recovery after storage failure.
    pub fn set_error(&mut self, error: String) {
        self.cancel_recovery();
        self.waiting = false;
        self.error = Some(error);
    }
    /// Keep an arrival visible until acknowledged and immediately interrupt care.
    pub fn set_notice(&mut self, notice: String) {
        self.cancel_recovery();
        self.notices.push(notice);
    }
    /// Apply the local cosmetic preference without changing gameplay state.
    pub fn set_effects(&mut self, effects: RoguelikeEffects) {
        self.effects = effects;
    }
    /// Cancel future automatic steps, including after a late save acknowledgement.
    pub fn cancel_recovery(&mut self) {
        self.recovery = None;
    }
    /// Whether the user currently has automatic recovery enabled.
    pub fn recovering(&self) -> bool {
        self.recovery.is_some()
    }
    /// Advance the injected presentation clock and report whether effects need repainting.
    pub fn advance_clock(&mut self, now: u64) -> bool {
        let flashing = self.now < self.flash_until;
        self.now = self.now.max(now);
        self.effects == RoguelikeEffects::Full && (flashing || self.now < self.flash_until)
    }
    /// Whether recovery or a visible transient effect needs a fast presentation tick.
    pub fn ticking(&self) -> bool {
        self.recovery.is_some()
            || (self.effects == RoguelikeEffects::Full && self.now < self.flash_until)
    }
    /// Take one due recovery action, gated on the previous committed acknowledgement.
    pub fn due_action(&mut self) -> Option<Command> {
        if self.waiting || !self.recovery.as_ref().is_some_and(|r| self.now >= r.due) {
            return None;
        }
        if !self
            .run
            .as_ref()
            .is_some_and(|r| r.can_rest && !r.danger && !r.is_finished())
        {
            self.cancel_recovery();
            return None;
        }
        self.waiting = true;
        Some(Command::Act(Action::Rest))
    }
    fn can_act(&self) -> bool {
        !self.waiting && self.run.as_ref().is_some_and(|r| !r.is_finished())
    }
    fn act(&mut self, action: Action) -> Option<Msg> {
        if !self.can_act() {
            return Some(Msg::None);
        }
        self.waiting = true;
        Some(Msg::Roguelike(Command::Act(action)))
    }
    fn rest(&mut self) -> Option<Msg> {
        if self.can_act()
            && self.notices.is_empty()
            && let Some(run) = &self.run
            && run.can_rest
            && !run.danger
        {
            self.recovery = Some(Recovery {
                started: run.supplies.clone(),
                due: self.now,
            });
            return self.act(Action::Rest);
        }
        self.act(Action::Rest)
    }
    fn page(&mut self, page: Page) -> Option<Msg> {
        self.page = if self.page == page { Page::Game } else { page };
        self.direction = None;
        self.cursor.reset();
        if self.page == Page::Journal {
            // Scrollback opens on the latest entries, clamped after wrapping.
            self.cursor.set(usize::from(u16::MAX) - 1);
        } else if self.page == Page::Equipment
            && let Some(run) = &self.run
            && !run.ground.is_empty()
        {
            self.cursor.set(equipment_rows(run).1);
        }
        Some(Msg::None)
    }
    fn close(&mut self) -> Option<Msg> {
        if self.page != Page::Game || self.direction.is_some() {
            self.page = Page::Game;
            self.direction = None;
            Some(Msg::None)
        } else {
            Some(Msg::CloseModal)
        }
    }
    fn new_run(&mut self) -> Option<Msg> {
        if !self.waiting && self.run.as_ref().is_some_and(RunView::is_finished) {
            self.waiting = true;
            self.page = Page::Game;
            Some(Msg::Roguelike(Command::NewRun))
        } else {
            Some(Msg::None)
        }
    }
    /// Controls available in the current view, derived from the dispatch tables.
    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        let mut bar = if self.recovery.is_some() {
            vec![("any key", "Stop recovery")]
        } else if self.page != Page::Game {
            vec![("↑/↓/Pg", "Scroll"), ("Esc", "Dungeon")]
        } else if self.run.as_ref().is_some_and(RunView::is_finished) {
            FINISHED.bar()
        } else {
            PLAY.bar()
        };
        bar.extend(COMMON.bar());
        bar
    }
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let modal = LogModal::area(area);
        frame.render_widget(Clear, modal);
        let flash = self.effects == RoguelikeEffects::Full && self.now < self.flash_until;
        let injured = self.effects == RoguelikeEffects::Reduced
            && self
                .run
                .as_ref()
                .is_some_and(|r| r.body.pain() > 0 || r.body.brain < 100);
        let title = if self.waiting && self.run.is_some() {
            " THE WAITING BELOW · saving... "
        } else if flash
            && self
                .run
                .as_ref()
                .is_some_and(|r| r.body.pain() >= 20 || r.body.brain < 100)
        {
            " ░ THE WAITING BELOW ▒ "
        } else {
            " THE WAITING BELOW "
        };
        let border = if flash || injured {
            Color::LightRed
        } else {
            Color::Cyan
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title(title)
            .title_bottom(" ?: guide  p: journal  F4: chat ");
        let mut inner = block.inner(modal);
        frame.render_widget(block, modal);
        if !self.notices.is_empty() {
            let text = format!(
                "[Enter: acknowledge] {}",
                self.notices
                    .iter()
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ")
            );
            let height = wrapped_height(&text, inner.width).min(3).min(inner.height);
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Color::Yellow))
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
        let content = inner;
        if self.page == Page::Guide {
            render_scroll(frame, inner, GUIDE, &mut self.cursor);
            return;
        }
        let Some(run) = &self.run else {
            frame.render_widget(Paragraph::new("A lantern flickers beneath the waiting room.\nLoading your saved expedition..."), inner);
            return;
        };
        match self.page {
            Page::Journal => {
                let text = run
                    .journal
                    .iter()
                    .map(|e| format!("[{}] {}", e.time, e.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                render_scroll(frame, inner, &text, &mut self.cursor);
                return;
            }
            Page::Condition => {
                frame.render_widget(
                    Paragraph::new("CONDITION · ↑/↓ select · a: treat selected part"),
                    take_rows(&mut inner, 1),
                );
                let lines = run.body.condition_lines();
                self.cursor.clamp(lines.len());
                let items = wrapped_items(lines, inner.width);
                render_list_body(
                    frame,
                    inner,
                    items,
                    Some(self.cursor.index()),
                    Some(self.cursor.index()),
                );
                return;
            }
            Page::Equipment => {
                let heading = format!(
                    "EQUIPMENT · x: swap · Enter: equip selected ground item\n{}",
                    run.movement_summary()
                );
                let height = wrapped_height(&heading, inner.width);
                frame.render_widget(
                    Paragraph::new(heading).wrap(Wrap { trim: false }),
                    take_rows(&mut inner, height),
                );
                let (lines, _) = equipment_rows(run);
                self.cursor.clamp(lines.len());
                let items = wrapped_items(lines, inner.width);
                render_list_body(
                    frame,
                    inner,
                    items,
                    Some(self.cursor.index()),
                    Some(self.cursor.index()),
                );
                return;
            }
            _ => {}
        }
        if run.is_finished() {
            render_epitaph(frame, inner, run);
            return;
        }
        let objective = match self.direction {
            Some(Direction::Attack) => "Attack: choose a direction (Esc cancels)".into(),
            Some(Direction::Close) => "Close door: choose a direction (Esc cancels)".into(),
            None => run.objective(),
        };
        frame.render_widget(
            Paragraph::new(objective).style(Style::default().fg(Color::Cyan)),
            take_rows(&mut inner, 1),
        );
        frame.render_widget(
            Paragraph::new(format!(
                "Depth {}  Blood {}  Breath {}  Pain {}  Bleed {}",
                run.depth + 1,
                run.body.blood,
                run.body.stamina,
                run.body.pain(),
                run.body.bleeding()
            )),
            take_rows(&mut inner, 1),
        );
        frame.render_widget(
            Paragraph::new(supplies(run)).style(theme::dim()),
            take_rows(&mut inner, 1),
        );
        let reach = run.body.effective_weapon(&run.gear).reach();
        if reach > 1 {
            let hint =
                format!("Move toward enemies within {reach} tiles to thrust without moving.");
            let height = wrapped_height(&hint, inner.width);
            frame.render_widget(
                Paragraph::new(hint).wrap(Wrap { trim: false }),
                take_rows(&mut inner, height),
            );
        }
        let journal_height = inner.height.saturating_sub(4).min(7);
        let map_height = inner.height.saturating_sub(journal_height);
        let mut map = take_rows(&mut inner, map_height);
        if map.width >= 80 {
            let sidebar = Rect {
                x: map.right().saturating_sub(28),
                width: 28,
                ..map
            };
            map.width = map.width.saturating_sub(29);
            let mut lines = run.body.condition_lines();
            lines.extend(
                run.enemies
                    .iter()
                    .map(|e| format!("{}: {}", e.name, e.intent)),
            );
            frame.render_widget(Paragraph::new(lines.join("\n")), sidebar);
        }
        render_map(frame, map, run);
        let lines = run
            .journal
            .iter()
            .rev()
            .take(inner.height as usize)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|e| Line::from(Span::styled(e.text.as_str(), event_style(e.kind))))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), inner);
        if let Some(recovery) = &self.recovery {
            let width = content.width.saturating_sub(2).min(68);
            let height = content.height.saturating_sub(2).min(10);
            let panel = Rect {
                x: content.x + (content.width - width) / 2,
                y: content.y + (content.height - height) / 2,
                width,
                height,
            };
            frame.render_widget(Clear, panel);
            let text = format!(
                "{}\nBlood {}  Breath {}  Nutrition {}\nBleeding {}  Pain {}\nLinen {} (-{})  Splints {} (-{})  Food {} (-{})\n{}",
                run.journal
                    .iter()
                    .rev()
                    .find(|e| e.kind == EventKind::Recovery)
                    .map_or("Preparing care", |e| e.text.as_str()),
                run.body.blood,
                run.body.stamina,
                run.body.hunger,
                run.body.bleeding(),
                run.body.pain(),
                run.supplies.bandages,
                recovery
                    .started
                    .bandages
                    .saturating_sub(run.supplies.bandages),
                run.supplies.splints,
                recovery
                    .started
                    .splints
                    .saturating_sub(run.supplies.splints),
                run.supplies.food,
                recovery.started.food.saturating_sub(run.supplies.food),
                run.body.condition_lines().join(" · ")
            );
            frame.render_widget(
                Paragraph::new(text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" RECOVERING ")
                            .title_bottom(" Any key stops recovery "),
                    )
                    .wrap(Wrap { trim: false }),
                panel,
            );
        }
    }
}
// Rendering, initial focus, and Enter dispatch share one row-to-ground mapping.
fn equipment_rows(run: &RunView) -> (Vec<String>, usize) {
    let mut lines = run.gear.lines();
    lines.push(String::new());
    lines.push("GROUND · ↑/↓ inspect/select".into());
    let ground_start = lines.len();
    lines.extend(run.ground.iter().map(|item| match item {
        LootKind::Weapon(weapon) => weapon.description(),
        LootKind::Armor(armor) => armor.description(),
        _ => item.name(),
    }));
    (lines, ground_start)
}

fn wrapped_items(lines: Vec<String>, width: u16) -> Vec<ListItem<'static>> {
    lines
        .into_iter()
        .map(|text| {
            if text.is_empty() {
                return ListItem::new(Line::default());
            }
            let rows = super::super::components::wrap_body(
                &text,
                usize::from(width.max(1)),
                usize::from(width.max(1)),
            )
            .into_iter()
            .map(|(text, _)| Line::from(text))
            .collect::<Vec<_>>();
            ListItem::new(rows)
        })
        .collect()
}

fn event_style(kind: EventKind) -> Style {
    Style::default().fg(match kind {
        EventKind::Injury | EventKind::Danger => Color::LightRed,
        EventKind::Recovery => Color::LightGreen,
        EventKind::Discovery | EventKind::Ending => Color::Yellow,
        _ => Color::Reset,
    })
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
        0
    } else {
        text.split('\n')
            .map(|line| {
                super::super::components::wrap_body(line, width as usize, width as usize)
                    .len()
                    .max(1)
            })
            .sum::<usize>()
            .min(u16::MAX as usize) as u16
    }
}
fn render_scroll(frame: &mut Frame, area: Rect, text: &str, cursor: &mut ListCursor) {
    let rows: usize = text
        .lines()
        .map(|line| wrapped_height(line, area.width) as usize)
        .sum();
    cursor.clamp(rows.saturating_sub(area.height as usize) + 1);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((cursor.index().min(u16::MAX as usize) as u16, 0)),
        area,
    );
}
fn supplies(run: &RunView) -> String {
    format!(
        "Linen {}  Splints {}  Food {}  Nutrition {}  Gold {}  {}",
        run.supplies.bandages,
        run.supplies.splints,
        run.supplies.food,
        run.body.hunger,
        run.gold,
        run.gear.active.name()
    )
}
fn render_map(frame: &mut Frame, area: Rect, run: &RunView) {
    if area.is_empty() {
        return;
    }
    let width = (area.width as usize).min(WIDTH as usize);
    let height = (area.height as usize).min(HEIGHT as usize);
    let left = (run.position.x.max(0) as usize)
        .saturating_sub(width / 2)
        .min((WIDTH as usize).saturating_sub(width));
    let top = (run.position.y.max(0) as usize)
        .saturating_sub(height / 2)
        .min((HEIGHT as usize).saturating_sub(height));
    let rows = (top..top + height)
        .map(|y| {
            Line::from(
                (left..left + width)
                    .map(|x| {
                        let cell = &run.cells[y * WIDTH as usize + x];
                        let glyph = run.glyph(Point {
                            x: x as i32,
                            y: y as i32,
                        });
                        let style = if glyph == '@' {
                            Style::default()
                                .fg(Color::LightCyan)
                                .add_modifier(Modifier::BOLD)
                        } else if !cell.visible {
                            theme::dim()
                        } else {
                            Style::default().fg(match glyph {
                                '#' => Color::Gray,
                                '.' => Color::DarkGray,
                                '<' | '>' => Color::LightCyan,
                                '$' | '*' => Color::Yellow,
                                '!' | '%' | '&' => Color::LightGreen,
                                ')' | '[' => Color::LightBlue,
                                '+' | '/' => Color::Yellow,
                                _ => Color::LightRed,
                            })
                        };
                        Span::styled(
                            glyph.to_string(),
                            if cell.threatened {
                                style.bg(Color::DarkGray).add_modifier(Modifier::UNDERLINED)
                            } else {
                                style
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(rows),
        Rect {
            x: area.x + (area.width - width as u16) / 2,
            width: width as u16,
            ..area
        },
    );
}
fn render_epitaph(frame: &mut Frame, area: Rect, run: &RunView) {
    let heading = match &run.outcome {
        Outcome::Dead(_) => "HERE ENDS YOUR EXPEDITION",
        Outcome::Escaped if run.relic => "THE EMBER COMES HOME",
        Outcome::Escaped => "YOU ESCAPED WITH YOUR LIFE",
        Outcome::Alive => return,
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{heading}\n{}\n\nn: new expedition   p: journal   F4 / Esc: chat",
            run.summary()
        ))
        .wrap(Wrap { trim: false }),
        area,
    );
}
const GUIDE: &str = concat!(
    "THE WAITING BELOW\nEscape alive whenever you choose. Bring the ember home for an exceptional victory.\nEvery action is saved; closing the dungeon pauses it.\n\n",
    "MOVE / FIGHT  Arrows, numpad 1-9, or vi keys:\n  y k u     7 8 9\n  h @ l     4 @ 6\n  b j n     1 2 3\n",
    "Uppercase vi keys sprint: faster, noisy, and costly in breath. Walking never restores breath.\nMoving toward a visible enemy within weapon reach attacks without moving. A spear reaches 2 tiles (one empty tile between you and the enemy). f then direction also attacks; sprinting stays movement-only.\n",
    "./5 wait · a bandage · e eat · r automatic care · </> stairs\ng interact: take the ember or use a fountain · c then direction closes a door\nx swap weapons · i equipment and ground items · v inspect injuries\n\n",
    "WOUNDS LAST\nBleeding drains blood. Armor protects body regions. Splints support fractures; linen controls bleeding. Rest automatically performs useful care using your supplies. Ordinary care cannot regrow destroyed anatomy.\n",
    "Recovery takes four steps per second at most, waiting for each save. Any input stops it. Danger and party arrivals interrupt it.\n\n",
    "THE EMBER\nTaking it explicitly awakens the dungeon permanently. Expect warnings, breaches, swarms, collapses, and lulls. Prepare escape routes on the descent. You can leave without it.\n\n",
    "MAP  @ you · # wall · < up · > down · + closed door · / open door\nr rat · h pilgrim · W warden · B brute · * ember\nUnseen terrain is remembered; hidden creatures are never shown. Threatened tiles are underlined.\n",
    "p opens the full journal. Settings / Playback & display controls injury effects.\nFriends arriving appear above this guide. Enter acknowledges them.\n",
    "n: new expedition after death or escape · F4: return to chat\nUp/Down/PgUp/PgDown: scroll · ? / Esc: close guide"
);
static COMMON: Keymap<RoguelikeModal, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Char('?'),
        bar: Some(("?", "Guide")),
        action: |m| m.page(Page::Guide),
    },
    Binding {
        pattern: KeyPattern::Char('p'),
        bar: Some(("p", "Journal")),
        action: |m| m.page(Page::Journal),
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
static PLAY: Keymap<RoguelikeModal, Msg> = Keymap(&[
    Binding {
        pattern: KeyPattern::Chars(&['h', '4']),
        bar: Some(("arrows/vi/1-9", "Move/fight")),
        action: |m| m.movement(-1, 0, false),
    },
    Binding {
        pattern: KeyPattern::Char('H'),
        bar: Some(("Shift-vi", "Sprint")),
        action: |m| m.movement(-1, 0, true),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Left),
        bar: None,
        action: |m| m.movement(-1, 0, false),
    },
    Binding {
        pattern: KeyPattern::Chars(&['l', '6']),
        bar: None,
        action: |m| m.movement(1, 0, false),
    },
    Binding {
        pattern: KeyPattern::Char('L'),
        bar: None,
        action: |m| m.movement(1, 0, true),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Right),
        bar: None,
        action: |m| m.movement(1, 0, false),
    },
    Binding {
        pattern: KeyPattern::Chars(&['k', '8']),
        bar: None,
        action: |m| m.movement(0, -1, false),
    },
    Binding {
        pattern: KeyPattern::Char('K'),
        bar: None,
        action: |m| m.movement(0, -1, true),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Up),
        bar: None,
        action: |m| m.movement(0, -1, false),
    },
    Binding {
        pattern: KeyPattern::Chars(&['j', '2']),
        bar: None,
        action: |m| m.movement(0, 1, false),
    },
    Binding {
        pattern: KeyPattern::Char('J'),
        bar: None,
        action: |m| m.movement(0, 1, true),
    },
    Binding {
        pattern: KeyPattern::Plain(Key::Down),
        bar: None,
        action: |m| m.movement(0, 1, false),
    },
    Binding {
        pattern: KeyPattern::Chars(&['y', '7']),
        bar: None,
        action: |m| m.movement(-1, -1, false),
    },
    Binding {
        pattern: KeyPattern::Char('Y'),
        bar: None,
        action: |m| m.movement(-1, -1, true),
    },
    Binding {
        pattern: KeyPattern::Chars(&['u', '9']),
        bar: None,
        action: |m| m.movement(1, -1, false),
    },
    Binding {
        pattern: KeyPattern::Char('U'),
        bar: None,
        action: |m| m.movement(1, -1, true),
    },
    Binding {
        pattern: KeyPattern::Chars(&['b', '1']),
        bar: None,
        action: |m| m.movement(-1, 1, false),
    },
    Binding {
        pattern: KeyPattern::Char('B'),
        bar: None,
        action: |m| m.movement(-1, 1, true),
    },
    Binding {
        pattern: KeyPattern::Chars(&['n', '3']),
        bar: None,
        action: |m| m.movement(1, 1, false),
    },
    Binding {
        pattern: KeyPattern::Char('N'),
        bar: None,
        action: |m| m.movement(1, 1, true),
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
        bar: Some(("r", "Recover")),
        action: |m| m.rest(),
    },
    Binding {
        pattern: KeyPattern::Chars(&['<', '>']),
        bar: Some(("</>", "Stairs")),
        action: |m| m.act(Action::Stairs),
    },
    Binding {
        pattern: KeyPattern::Char('g'),
        bar: Some(("g", "Interact")),
        action: |m| m.act(Action::Interact),
    },
    Binding {
        pattern: KeyPattern::Char('x'),
        bar: Some(("x", "Swap weapon")),
        action: |m| m.act(Action::SwapWeapon),
    },
    Binding {
        pattern: KeyPattern::Char('i'),
        bar: Some(("i", "Equipment")),
        action: |m| m.page(Page::Equipment),
    },
    Binding {
        pattern: KeyPattern::Char('v'),
        bar: Some(("v", "Condition")),
        action: |m| m.page(Page::Condition),
    },
    Binding {
        pattern: KeyPattern::Char('f'),
        bar: Some(("f+dir", "Attack")),
        action: |m| {
            m.direction = Some(Direction::Attack);
            Some(Msg::None)
        },
    },
    Binding {
        pattern: KeyPattern::Char('c'),
        bar: Some(("c+dir", "Close door")),
        action: |m| {
            m.direction = Some(Direction::Close);
            Some(Msg::None)
        },
    },
]);
impl RoguelikeModal {
    fn movement(&mut self, dx: i32, dy: i32, sprint: bool) -> Option<Msg> {
        let action = match self.direction.take() {
            Some(Direction::Attack) => Action::Attack(dx, dy),
            Some(Direction::Close) => Action::CloseDoor(dx, dy),
            None if sprint => Action::Sprint(dx, dy),
            None => Action::Move(dx, dy),
        };
        self.act(action)
    }
}
passive_modal!(RoguelikeModal);
impl AppComponent<Msg, NoUserEvent> for RoguelikeModal {
    fn on(&mut self, ev: &Event<NoUserEvent>) -> Option<Msg> {
        if self.recovery.take().is_some() {
            return if plain(ev) == Some(Key::Esc) {
                self.close()
            } else {
                Some(Msg::None)
            };
        }
        if !self.notices.is_empty() && plain(ev) == Some(Key::Enter) {
            self.notices.clear();
            return Some(Msg::None);
        }
        if let Some(msg) = COMMON.dispatch(self, ev) {
            return Some(msg);
        }
        if self.page != Page::Game {
            let len = match (&self.run, self.page) {
                (Some(run), Page::Equipment) => equipment_rows(run).0.len(),
                (Some(run), Page::Condition) => run.body.condition_lines().len(),
                _ => u16::MAX as usize,
            };
            if let Some(key) = plain(ev) {
                if self.cursor.nav(key, len) {
                    return Some(Msg::None);
                }
                match (self.page, key) {
                    (Page::Equipment, Key::Enter) => {
                        let index = self.run.as_ref().and_then(|run| {
                            self.cursor
                                .index()
                                .checked_sub(equipment_rows(run).1)
                                .filter(|index| *index < run.ground.len())
                        });
                        return index
                            .map_or(Some(Msg::None), |index| self.act(Action::Equip(index)));
                    }
                    (Page::Equipment, Key::Char('x')) => return self.act(Action::SwapWeapon),
                    (Page::Condition, Key::Char('a')) => {
                        return self.act(Action::Treat(self.cursor.index()));
                    }
                    (Page::Equipment, Key::Char('i')) | (Page::Condition, Key::Char('v')) => {
                        return self.page(Page::Game);
                    }
                    _ => {}
                }
            }
            return Some(Msg::None);
        }
        if self.run.as_ref().is_some_and(RunView::is_finished) {
            FINISHED.dispatch(self, ev)
        } else {
            PLAY.dispatch(self, ev)
        }
        .or(Some(Msg::None))
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ground_section_has_a_blank_line_and_keeps_selection_working() {
        let mut modal = RoguelikeModal::new();
        let mut run = Run::new(19).view();
        run.ground = vec![
            LootKind::Weapon(crate::roguelike::WeaponKind::Spear),
            LootKind::Bandage,
        ];
        modal.set_run(run);
        modal.on(&key(Key::Char('i')));
        let screen = render(&mut modal, 120, 50);
        let lines: Vec<_> = screen.lines().collect();
        let ground = lines.iter().position(|l| l.contains("GROUND")).unwrap();
        assert!(
            lines[ground - 1].trim_matches(['│', ' ']).is_empty(),
            "{screen}"
        );
        assert_eq!(
            modal.on(&key(Key::Enter)),
            Some(Msg::Roguelike(Command::Act(Action::Equip(0))))
        );
    }

    #[test]
    fn holding_a_spear_explains_how_to_attack_at_reach() {
        let mut modal = RoguelikeModal::new();
        let mut run = Run::new(19).view();
        run.gear.active = crate::roguelike::WeaponKind::Spear;
        modal.set_run(run);
        let screen = render(&mut modal, 120, 40);
        assert!(
            screen.contains("2 tiles")
                && screen.contains("Move toward")
                && screen.contains("without moving"),
            "{screen}"
        );
    }

    #[test]
    fn equipment_shows_actual_movement_cost_with_heavy_armor() {
        use crate::roguelike::{ArmorMaterial, ArmorPiece, ArmorSlot, Run};
        let mut run = Run::new(42).view();
        run.gear.armor = ArmorSlot::ALL.map(|slot| {
            Some(ArmorPiece {
                slot,
                material: ArmorMaterial::Iron,
            })
        });
        let mut modal = RoguelikeModal::new();
        modal.set_run(run);
        modal.on(&key(Key::Char('i')));
        let screen = render(&mut modal, 120, 40);
        assert!(
            screen.contains("walk 150 time; sprint 100 time"),
            "{screen}"
        );
    }
    use crate::roguelike::Run;
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
    fn restable() -> RunView {
        let mut run = Run::new(19).view();
        run.can_rest = true;
        run.danger = false;
        run.body.stamina = 20;
        run.last_step.changed = true;
        run.last_step.interrupted = false;
        run
    }
    #[test]
    fn committed_responses_gate_actions_and_paced_recovery() {
        let mut modal = RoguelikeModal::new();
        let run = restable();
        modal.set_run(run.clone());
        assert_eq!(
            modal.on(&key(Key::Char('r'))),
            Some(Msg::Roguelike(Command::Act(Action::Rest)))
        );
        modal.advance_clock(1000);
        assert_eq!(
            modal.due_action(),
            None,
            "pending save blocks all further recovery"
        );
        modal.set_run(run.clone());
        modal.advance_clock(1249);
        assert_eq!(modal.due_action(), None);
        modal.advance_clock(1250);
        assert_eq!(modal.due_action(), Some(Command::Act(Action::Rest)));
        assert_eq!(modal.due_action(), None);
        assert_eq!(modal.run, Some(run));
    }
    #[test]
    fn cancellation_survives_late_acknowledgements() {
        for reason in 0..4 {
            let mut modal = RoguelikeModal::new();
            let run = restable();
            modal.set_run(run.clone());
            modal.on(&key(Key::Char('r')));
            match reason {
                0 => {
                    assert_eq!(modal.on(&key(Key::Char('h'))), Some(Msg::None));
                }
                1 => modal.set_notice("Nero joined".into()),
                2 => modal.set_error("disk full".into()),
                _ => modal.cancel_recovery(),
            }
            modal.set_run(run);
            modal.advance_clock(10_000);
            assert_eq!(modal.due_action(), None);
        }
    }
    #[test]
    fn observed_danger_or_exhausted_recovery_stops_automation() {
        for reason in 0..4 {
            let mut modal = RoguelikeModal::new();
            let mut run = restable();
            modal.set_run(run.clone());
            modal.on(&key(Key::Char('r')));
            match reason {
                0 => run.danger = true,
                1 => run.can_rest = false,
                2 => run.last_step.interrupted = true,
                _ => run.last_step.changed = false,
            }
            modal.set_run(run);
            modal.advance_clock(1000);
            assert_eq!(modal.due_action(), None);
        }
    }
    #[test]
    fn uppercase_and_direction_modes_use_normal_action_bridge() {
        for (keys, action) in [
            (vec!['H'], Action::Sprint(-1, 0)),
            (vec!['U'], Action::Sprint(1, -1)),
            (vec!['f', 'l'], Action::Attack(1, 0)),
            (vec!['c', 'b'], Action::CloseDoor(-1, 1)),
        ] {
            let mut modal = RoguelikeModal::new();
            modal.set_run(Run::new(19).view());
            for ch in &keys[..keys.len() - 1] {
                assert_eq!(modal.on(&key(Key::Char(*ch))), Some(Msg::None));
            }
            assert_eq!(
                modal.on(&key(Key::Char(*keys.last().unwrap()))),
                Some(Msg::Roguelike(Command::Act(action)))
            );
        }
    }
    #[test]
    fn small_terminals_preserve_chat_and_viewport() {
        let mut modal = RoguelikeModal::new();
        modal.set_run(Run::new(19).view());
        for (width, height) in [(40, 24), (80, 24), (120, 40)] {
            let screen = render(&mut modal, width, height);
            assert!(screen.contains('@'), "{screen}");
            for line in screen.lines().skip((height as usize) * 2 / 3) {
                assert!(line.trim().is_empty());
            }
        }
        for (width, height) in [(0, 0), (1, 1), (2, 2), (10, 6)] {
            render(&mut modal, width, height);
        }
    }
    #[test]
    fn arrivals_remain_visible_over_guide_and_outcome() {
        let mut modal = RoguelikeModal::new();
        modal.set_notice("Nero joined".into());
        modal.set_run(Run::new(19).view());
        modal.on(&key(Key::Char('?')));
        assert!(render(&mut modal, 100, 40).contains("Nero joined"));
        modal.on(&key(Key::Enter));
        assert!(!render(&mut modal, 100, 40).contains("Nero joined"));
    }
    #[test]
    fn guide_scroll_reaches_end() {
        let mut modal = RoguelikeModal::new();
        modal.on(&key(Key::Char('?')));
        for _ in 0..50 {
            modal.on(&key(Key::PageDown));
        }
        assert!(render(&mut modal, 80, 24).contains("close guide"));
    }
    #[test]
    fn injury_effects_are_live_cosmetic_and_do_not_replay() {
        use crate::roguelike::JournalEntry;
        let mut modal = RoguelikeModal::new();
        let mut run = Run::new(19).view();
        modal.set_run(run.clone());
        modal.advance_clock(1000);
        let id = run.journal.last().map_or(1, |entry| entry.id + 1);
        run.serious_wounds += 1;
        run.journal.push(JournalEntry {
            id,
            time: run.time,
            text: "Your arm breaks.".into(),
            kind: EventKind::Injury,
        });
        modal.set_run(run.clone());
        assert!(modal.ticking());
        modal.set_effects(RoguelikeEffects::Off);
        assert!(!modal.ticking());
        assert_eq!(
            modal.run,
            Some(run.clone()),
            "cosmetics never mutate observed state"
        );
        modal.advance_clock(2000);
        modal.set_effects(RoguelikeEffects::Full);
        modal.set_run(run.clone());
        assert!(!modal.ticking(), "same committed injury is never replayed");
        let mut opened = RoguelikeModal::new();
        opened.set_run(run);
        assert!(!opened.ticking(), "history does not flash on opening");
    }
    #[test]
    fn equipment_selection_keeps_ground_action_indices_stable() {
        use crate::roguelike::WeaponKind;
        let mut modal = RoguelikeModal::new();
        let mut run = Run::new(19).view();
        run.ground = vec![LootKind::Bandage, LootKind::Weapon(WeaponKind::Mace)];
        modal.set_run(run);
        modal.on(&key(Key::Char('i')));
        modal.on(&key(Key::Down));
        assert_eq!(
            modal.on(&key(Key::Enter)),
            Some(Msg::Roguelike(Command::Act(Action::Equip(1))))
        );
    }
    #[test]
    fn brain_damage_corrupts_only_decoration_and_respects_effect_modes() {
        use crate::roguelike::JournalEntry;
        let mut modal = RoguelikeModal::new();
        let mut view = Run::new(19).view();
        modal.set_run(view.clone());
        view.body.brain = 80;
        view.serious_wounds += 1;
        assert_eq!(view.body.pain(), 0);
        let id = view.journal.last().map_or(1, |entry| entry.id + 1);
        view.journal.push(JournalEntry {
            id,
            time: view.time,
            text: "A blow injures your brain.".into(),
            kind: EventKind::Injury,
        });
        modal.set_run(view.clone());
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let buffer = terminal
            .draw(|frame| modal.render(frame, frame.area()))
            .unwrap()
            .buffer;
        assert!(buffer_to_string(buffer).contains("░ THE WAITING BELOW ▒"));
        assert_eq!(buffer[(0, 0)].fg, Color::LightRed);
        modal.set_effects(RoguelikeEffects::Reduced);
        let buffer = terminal
            .draw(|frame| modal.render(frame, frame.area()))
            .unwrap()
            .buffer;
        assert!(!buffer_to_string(buffer).contains('░'));
        assert_eq!(buffer[(0, 0)].fg, Color::LightRed);
        modal.set_effects(RoguelikeEffects::Off);
        let buffer = terminal
            .draw(|frame| modal.render(frame, frame.area()))
            .unwrap()
            .buffer;
        assert!(!buffer_to_string(buffer).contains('░'));
        assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
        assert_eq!(modal.run, Some(view));
    }

    #[test]
    fn minor_injury_events_do_not_flash_the_frame() {
        use crate::roguelike::JournalEntry;
        let mut modal = RoguelikeModal::new();
        let mut view = Run::new(19).view();
        modal.set_run(view.clone());
        let id = view.journal.last().map_or(1, |entry| entry.id + 1);
        view.journal.push(JournalEntry {
            id,
            time: view.time,
            text: "Armor deflects the blow.".into(),
            kind: EventKind::Injury,
        });
        modal.set_run(view);
        assert!(!modal.ticking());
    }
    #[test]
    fn recovery_cancel_instruction_survives_wrapped_condition_details() {
        let mut modal = RoguelikeModal::new();
        let mut view = restable();
        view.body.brain = 80;
        view.body.parts[2].bone /= 3;
        modal.set_run(view);
        modal.on(&key(Key::Char('r')));
        let screen = render(&mut modal, 120, 40);
        assert!(screen.contains("Any key stops recovery"), "{screen}");
    }
}
