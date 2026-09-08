//! Only committed observations are rendered. Recovery is a cancellable UI
//! controller: it issues one normal saved action and waits for its acknowledgement.
use super::*;
use crate::config::RoguelikeEffects;
use crate::roguelike::{
    Action, Body, EventKind, HEIGHT, LootKind, Outcome, PartKind, Point, RunView, Supplies, WIDTH,
};
use crate::roguelike_store::Command;
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::Paragraph;

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

struct ConditionRow {
    part: PartKind,
    #[cfg(test)]
    text: String,
    source: Option<(String, usize)>,
}

#[derive(Default)]
struct ConditionViewport {
    rows: Vec<ConditionRow>,
    cursor: ListCursor,
    measured: crate::ui::layout::MeasuredLines,
}
impl ConditionViewport {
    fn refresh(
        &mut self,
        body: &Body,
        width: u16,
        renderer: &mut crate::ui::layout::Renderer,
        style: Style,
    ) {
        let selected = self
            .rows
            .get(self.cursor.index())
            .map(|row| (row.part, row.source.clone()));
        let entries = condition_entries(body, false);
        let rows: Vec<_> = entries.iter().map(|(_, row)| row.clone()).collect();
        let Ok(measured) = renderer.measure_lines("rogue-condition", &rows, width, style) else {
            return;
        };
        self.rows = measured
            .rows
            .iter()
            .filter_map(|row| {
                entries
                    .iter()
                    .find(|(_, entry)| entry.key == row.key)
                    .map(|(part, _)| ConditionRow {
                        part: *part,
                        #[cfg(test)]
                        text: row.text.clone(),
                        source: row.source.clone(),
                    })
            })
            .collect();
        if let Some((part, source)) = selected {
            let first = self.rows.iter().position(|row| row.part == part);
            let anchored = source.and_then(|(binding, source)| {
                self.rows
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, row)| {
                        row.part == part
                            && row
                                .source
                                .as_ref()
                                .is_some_and(|(field, at)| field == &binding && *at <= source)
                    })
                    .map(|(index, _)| index)
            });
            if let Some(index) = anchored.or(first) {
                self.cursor.set(index);
            }
        }
        self.cursor.clamp(self.rows.len());
        self.measured = measured;
    }
}

fn condition_entries(
    body: &Body,
    injured_only: bool,
) -> Vec<(PartKind, crate::ui::layout::PresentedRow)> {
    body.conditions()
        .into_iter()
        .filter(|condition| !injured_only || condition.injured)
        .map(|condition| {
            (
                condition.part,
                crate::ui::layout::PresentedRow {
                    key: format!("{:?}", condition.part),
                    data: crate::ui::layout::Presentation::default()
                        .text("region", condition.name)
                        .text("details", condition.details.join(", "))
                        .boolean("injured", condition.injured),
                    gap_after: false,
                },
            )
        })
        .collect()
}

/// A committed expedition observation above the live party chat strip.
pub struct RoguelikeModal {
    run: Option<RunView>,
    waiting: bool,
    error: Option<String>,
    notices: Vec<String>,
    page: Page,
    cursor: ListCursor,
    guide_scroll: crate::ui::layout::DocumentScroll,
    journal_scroll: crate::ui::layout::DocumentScroll,
    condition: ConditionViewport,
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
            guide_scroll: Default::default(),
            journal_scroll: Default::default(),
            condition: ConditionViewport::default(),
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
        self.condition.cursor.reset();
        self.guide_scroll = Default::default();
        if self.page == Page::Journal {
            // Scrollback opens on the latest entries, clamped after wrapping.
            self.journal_scroll.to_end();
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
        if let Ok(bundle) = crate::ui::layout::LayoutBundle::builtin() {
            let mut renderer = crate::ui::layout::Renderer::new(bundle);
            if let Ok(shell) = renderer.arrange(
                "page-shell",
                area,
                &crate::ui::layout::Presentation::default(),
            ) {
                self.render_layout(frame, shell.slot("page"), &mut renderer);
            }
        }
    }
    pub(crate) fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut crate::ui::layout::Renderer,
    ) {
        use crate::ui::layout::Presentation;

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
        let notices = if self.notices.is_empty() {
            String::new()
        } else {
            format!(
                "[Enter: acknowledge] {}",
                self.notices
                    .iter()
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        };
        let data = Presentation::default()
            .text("title", title)
            .text("footer", " ?: guide  p: journal  F4: chat ")
            .text("notices", notices)
            .boolean("has-notices", !self.notices.is_empty())
            .text(
                "error",
                self.error.as_ref().map_or_else(String::new, |error| {
                    format!("Could not save/load: {error}. F4 closes; reopen to retry.")
                }),
            )
            .boolean("has-error", self.error.is_some())
            .color_variable("--dungeon-border", border);
        let Ok(scene) = renderer.arrange("rogue", area, &data) else {
            return;
        };
        scene.paint_with_slots(frame, |name, frame, area, _| {
            if name == "body" {
                self.render_page(frame, area, renderer);
            }
        });
    }
    fn render_page(
        &mut self,
        frame: &mut Frame,
        mut inner: Rect,
        renderer: &mut crate::ui::layout::Renderer,
    ) {
        if inner.is_empty() {
            return;
        }
        let content = inner;
        if self.page == Page::Guide {
            let rows = [crate::ui::layout::PresentedRow {
                key: "guide".into(),
                data: crate::ui::layout::Presentation::default().text("body", GUIDE),
                gap_after: false,
            }];
            let _ = renderer.paint_document(
                frame,
                inner,
                "rogue-document",
                &rows,
                &mut self.guide_scroll,
                Style::default(),
            );
            return;
        }
        let Some(run) = &self.run else {
            let data = crate::ui::layout::Presentation::default()
                .text("notice", "A lantern flickers beneath the waiting room.")
                .text("detail", "Loading your saved expedition...");
            if let Ok(scene) = renderer.arrange("rogue-loading", inner, &data) {
                scene.paint_with_slots(frame, |_, _, _, _| {});
            }
            return;
        };
        match self.page {
            Page::Journal => {
                let rows = journal_presentations(run, false);
                let _ = renderer.paint_document(
                    frame,
                    inner,
                    "rogue-journal-row",
                    &rows,
                    &mut self.journal_scroll,
                    Style::default(),
                );
                return;
            }
            Page::Condition => {
                let data = crate::ui::layout::Presentation::default().text(
                    "heading",
                    "CONDITION · ↑/↓ scroll · a: treat selected region",
                );
                let Ok(scene) = renderer.arrange("rogue-inspection", inner, &data) else {
                    return;
                };
                scene.paint(frame);
                inner = scene.slot("body");
                self.condition
                    .refresh(&run.body, inner.width, renderer, scene.style("body"));
                self.condition
                    .measured
                    .paint(frame, inner, self.condition.cursor.index());
                scene.paint_overlays(frame);
                return;
            }
            Page::Equipment => {
                let data = crate::ui::layout::Presentation::default()
                    .text(
                        "heading",
                        "EQUIPMENT · x: swap · Enter: equip selected ground item",
                    )
                    .text("note", run.movement_summary())
                    .boolean("has-note", true);
                let Ok(scene) = renderer.arrange("rogue-inspection", inner, &data) else {
                    return;
                };
                scene.paint(frame);
                let (lines, _) = equipment_rows(run);
                self.cursor.clamp(lines.len());
                let rows = lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| crate::ui::layout::PresentedRow {
                        key: format!("equipment:{index}"),
                        data: crate::ui::layout::Presentation::default().text("description", text),
                        gap_after: false,
                    })
                    .collect::<Vec<_>>();
                let _ = renderer.paint_rows(
                    frame,
                    scene.slot("body"),
                    "rogue-equipment-row",
                    &rows,
                    Some(self.cursor.index()),
                    scene.style("body"),
                );
                scene.paint_overlays(frame);
                return;
            }
            _ => {}
        }
        if run.is_finished() {
            render_epitaph(frame, inner, run, renderer);
            return;
        }
        let objective = match self.direction {
            Some(Direction::Attack) => "Attack: choose a direction (Esc cancels)".into(),
            Some(Direction::Close) => "Close door: choose a direction (Esc cancels)".into(),
            None => run.objective(),
        };
        let reach = run.body.effective_weapon(&run.gear).reach();
        let mut data = crate::ui::layout::Presentation::default()
            .text("objective", objective)
            .text("weapon", run.gear.active.name())
            .text(
                "reach",
                format!("Move toward enemies within {reach} tiles to thrust without moving."),
            )
            .boolean("has-reach", reach > 1)
            .boolean("wide", inner.width >= 80);
        for (key, label, value) in [
            ("depth", "Depth", run.depth as u64 + 1),
            ("blood", "Blood", u64::from(run.body.blood)),
            ("breath", "Breath", u64::from(run.body.stamina)),
            ("pain", "Pain", u64::from(run.body.pain())),
            ("bleed", "Bleed", u64::from(run.body.bleeding())),
            ("linen", "Linen", u64::from(run.supplies.bandages)),
            ("splints", "Splints", u64::from(run.supplies.splints)),
            ("food", "Food", u64::from(run.supplies.food)),
            ("nutrition", "Nutrition", u64::from(run.body.hunger)),
            ("gold", "Gold", u64::from(run.gold)),
        ] {
            data = data
                .text(&format!("{key}-label"), label)
                .text(key, value.to_string());
        }
        let Ok(scene) = renderer.arrange("rogue-game", inner, &data) else {
            return;
        };
        let wounds = wound_presentations(run);
        let threats = threat_presentations(run);
        let journal = journal_presentations(run, true);
        let mut tail = crate::ui::layout::DocumentScroll::default();
        tail.to_end();
        scene.paint_with_slots(frame, |name, frame, area, style| match name {
            "sidebar" => {
                let _ = renderer.paint_rogue_summary(frame, area, &wounds, &threats, style);
            }
            "map" => render_map(frame, area, run, renderer, style),
            "journal" => {
                let _ = renderer.paint_document(
                    frame,
                    area,
                    "rogue-recent-event",
                    &journal,
                    &mut tail,
                    style,
                );
            }
            _ => {}
        });
        if let Some(recovery) = &self.recovery {
            let mut data = crate::ui::layout::Presentation::default()
                .text("title", " RECOVERING ")
                .text("footer", " Any key stops recovery ")
                .text(
                    "phase",
                    run.journal
                        .iter()
                        .rev()
                        .find(|event| event.kind == EventKind::Recovery)
                        .map_or("Preparing care", |event| event.text.as_str()),
                );
            for (key, label, value) in [
                ("blood", "Blood", run.body.blood),
                ("breath", "Breath", run.body.stamina),
                ("nutrition", "Nutrition", run.body.hunger),
                ("bleed", "Bleeding", run.body.bleeding()),
                ("pain", "Pain", run.body.pain()),
                ("linen", "Linen", run.supplies.bandages),
                ("splints", "Splints", run.supplies.splints),
                ("food", "Food", run.supplies.food),
            ] {
                data = data
                    .text(&format!("{key}-label"), label)
                    .text(key, value.to_string());
            }
            for (key, start, now) in [
                ("linen", recovery.started.bandages, run.supplies.bandages),
                ("splints", recovery.started.splints, run.supplies.splints),
                ("food", recovery.started.food, run.supplies.food),
            ] {
                data = data.text(
                    &format!("{key}-used"),
                    format!("(-{})", start.saturating_sub(now)),
                );
            }
            if let Ok(scene) = renderer.arrange("rogue-recovery", content, &data) {
                scene.paint_with_slots(frame, |name, frame, area, style| {
                    if name == "wounds" {
                        let _ = renderer.paint_bounded(
                            frame,
                            area,
                            "rogue-condition",
                            &wounds,
                            "More wounds:",
                            "v",
                            style,
                        );
                    }
                });
            }
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

fn threat_presentations(run: &RunView) -> Vec<crate::ui::layout::PresentedRow> {
    run.enemies
        .iter()
        .map(|enemy| crate::ui::layout::PresentedRow {
            key: format!("threat:{}", enemy.id),
            data: crate::ui::layout::Presentation::default()
                .text("name", &enemy.name)
                .text("separator", ":")
                .text("intent", &enemy.intent),
            gap_after: false,
        })
        .collect()
}

fn journal_presentations(run: &RunView, recent: bool) -> Vec<crate::ui::layout::PresentedRow> {
    run.journal
        .iter()
        .map(|event| crate::ui::layout::PresentedRow {
            key: event.id.to_string(),
            data: crate::ui::layout::Presentation::default()
                .text("open", "[")
                .text("close", "]")
                .text("time", event.time.to_string())
                .text("body", &event.text)
                .style(
                    "body",
                    if recent {
                        event_style(event.kind)
                    } else {
                        Style::default()
                    },
                ),
            gap_after: false,
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

// Fit whole entries and an explicit omission marker into the measured viewport.

fn wound_presentations(run: &RunView) -> Vec<crate::ui::layout::PresentedRow> {
    let mut rows: Vec<_> = condition_entries(&run.body, true)
        .into_iter()
        .map(|(_, row)| row)
        .collect();
    if rows.is_empty() {
        rows.push(crate::ui::layout::PresentedRow {
            key: "uninjured".into(),
            data: crate::ui::layout::Presentation::default().text("details", "No wounds"),
            gap_after: false,
        });
    }
    rows
}

fn render_map(
    frame: &mut Frame,
    area: Rect,
    run: &RunView,
    renderer: &crate::ui::layout::Renderer,
    inherited: Style,
) {
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
                            renderer.paint_style(theme::with_authored_style(
                                if cell.threatened {
                                    style.bg(Color::DarkGray).add_modifier(Modifier::UNDERLINED)
                                } else {
                                    style
                                },
                                inherited,
                            )),
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
fn render_epitaph(
    frame: &mut Frame,
    area: Rect,
    run: &RunView,
    renderer: &mut crate::ui::layout::Renderer,
) {
    let heading = match &run.outcome {
        Outcome::Dead(_) => "HERE ENDS YOUR EXPEDITION",
        Outcome::Escaped if run.relic => "THE EMBER COMES HOME",
        Outcome::Escaped => "YOU ESCAPED WITH YOUR LIFE",
        Outcome::Alive => return,
    };
    let data = crate::ui::layout::Presentation::default()
        .text("heading", heading)
        .text("summary", run.summary())
        .text("actions", "n: new expedition   p: journal   F4 / Esc: chat");
    if let Ok(scene) = renderer.arrange("rogue-epitaph", area, &data) {
        scene.paint(frame);
        scene.paint_overlays(frame);
    }
}

const GUIDE: &str = concat!(
    "THE WAITING BELOW\nEscape alive whenever you choose. Bring the ember home for an exceptional victory.\nEvery action is saved; closing the dungeon pauses it.\n\n",
    "MOVE / FIGHT  Arrows, numpad 1-9, or vi keys:\n  y k u     7 8 9\n  h @ l     4 @ 6\n  b j n     1 2 3\n",
    "Uppercase vi keys sprint: faster, noisy, and costly in breath. Walking never restores breath.\nMoving toward a visible enemy within weapon reach attacks without moving. A spear reaches 2 tiles (one empty tile between you and the enemy). f then direction also attacks; sprinting stays movement-only.\n",
    "./5 wait · a bandage · e eat · r automatic care · </> stairs\ng interact: take the ember or use a fountain · c then direction closes a door\nx swap weapons · i equipment and ground items · v inspect injuries\n\n",
    "WOUNDS LAST\nBleeding drains blood. Armor protects body regions. Splints support fractures; linen controls bleeding. Rest automatically performs useful care using your supplies. Ordinary care cannot regrow destroyed anatomy.\n",
    "Wounds use words: scratched, wounded, badly wounded, or mangled. Bone, nerve, organ damage and bleeding remain separate. The sidebar wraps injured regions above threats; v shows every region. In condition, arrows/page keys scroll lines and a treats the highlighted line's region.\n",
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
        if matches!(self.page, Page::Journal | Page::Guide)
            && let Some(key) = plain(ev)
        {
            let delta = match key {
                Key::Up => Some(-1),
                Key::Down => Some(1),
                Key::PageUp => Some(-(crate::ui::widgets::list::PAGE_STEP as i64)),
                Key::PageDown => Some(crate::ui::widgets::list::PAGE_STEP as i64),
                _ => None,
            };
            if let Some(delta) = delta {
                if self.page == Page::Journal {
                    self.journal_scroll.advance(delta);
                } else {
                    self.guide_scroll.advance(delta);
                }
                return Some(Msg::None);
            }
        }
        if self.page != Page::Game {
            if self.page == Page::Condition {
                if let Some(key) = plain(ev) {
                    if self.condition.cursor.nav(key, self.condition.rows.len()) {
                        return Some(Msg::None);
                    }
                    match key {
                        Key::Char('a') => {
                            let part = self
                                .condition
                                .rows
                                .get(self.condition.cursor.index())
                                .map(|row| row.part);
                            return part.map_or(Some(Msg::None), |part| {
                                self.act(Action::Treat(part.index()))
                            });
                        }
                        Key::Char('v') => return self.page(Page::Game),
                        _ => {}
                    }
                }
                return Some(Msg::None);
            }
            let len = match (&self.run, self.page) {
                (Some(run), Page::Equipment) => equipment_rows(run).0.len(),
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
                    (Page::Equipment, Key::Char('i')) => {
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
    use proptest::prelude::*;

    fn injured_view() -> RunView {
        let mut view = Run::new(19).view();
        for part in &mut view.body.parts {
            part.flesh = 10;
            part.bone = 10;
            part.nerve = 10;
            part.bleeding = 18;
        }
        view.body.eyes = [0, 10];
        view.body.brain = 50;
        view.body.heart = 50;
        view.body.lungs = [0, 50];
        view.body.stamina = view.body.breath_capacity();
        view.body.validate().unwrap();
        view
    }

    #[test]
    fn treatment_follows_the_displayed_region_through_wrapping_resize_and_replies() {
        for part in PartKind::ALL {
            let mut modal = RoguelikeModal::new();
            let mut view = injured_view();
            modal.set_run(view.clone());
            modal.on(&key(Key::Char('v')));
            render(&mut modal, 24, 18);
            let name = part.name(crate::roguelike::BodyKind::Human);
            for _ in 0..300 {
                let row = &modal.condition.rows[modal.condition.cursor.index()];
                if row.text.starts_with(name) {
                    break;
                }
                modal.on(&key(Key::Down));
            }
            assert!(
                modal.condition.rows[modal.condition.cursor.index()]
                    .text
                    .starts_with(name)
            );
            modal.on(&key(Key::Down)); // A continuation of this heavily injured region.
            for (width, height) in [(120, 40), (40, 24), (24, 18)] {
                let before = view.clone();
                render(&mut modal, width, height);
                assert_eq!(
                    modal.run,
                    Some(before),
                    "browsing spends no simulation time"
                );
                assert_eq!(
                    modal.on(&key(Key::Char('a'))),
                    Some(Msg::Roguelike(Command::Act(Action::Treat(part.index()))))
                );
                // Earlier entries shrink when a committed reply adopts healing.
                for previous in view.body.parts.iter_mut().take(part.index()) {
                    previous.flesh = 80;
                    previous.bone = 100;
                    previous.nerve = 100;
                    previous.bleeding = 0;
                }
                modal.set_run(view.clone());
            }
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn file_only_condition_reordering_retains_the_treatment_identity() {
        use crate::ui::layout::{LayoutBundle, Presentation, Renderer};
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("templates")).unwrap();
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let mut modal = RoguelikeModal::new();
        modal.set_run(injured_view());
        modal.on(&key(Key::Char('v')));
        let mut terminal =
            tuirealm::ratatui::Terminal::new(tuirealm::ratatui::backend::TestBackend::new(35, 24))
                .unwrap();
        let mut paint = |modal: &mut RoguelikeModal, renderer: &mut Renderer| {
            terminal
                .draw(|frame| {
                    let shell = renderer
                        .arrange("page-shell", frame.area(), &Presentation::default())
                        .unwrap();
                    modal.render_layout(frame, shell.slot("page"), renderer);
                })
                .unwrap();
        };
        paint(&mut modal, &mut renderer);
        let part = PartKind::ALL[1];
        for _ in 0..200 {
            if modal.condition.rows[modal.condition.cursor.index()].part == part {
                break;
            }
            modal.on(&key(Key::Down));
        }
        assert_eq!(
            modal.condition.rows[modal.condition.cursor.index()].part,
            part
        );
        std::fs::write(directory.path().join("templates/condition.xml"), r#"<templates version="1"><template name="rogue-condition"><column><text bind="details" style="white-space: normal"/><text bind="region"/></column></template></templates>"#).unwrap();
        renderer.install(LayoutBundle::load(directory.path()).unwrap());
        paint(&mut modal, &mut renderer);
        assert_eq!(
            modal.condition.rows[modal.condition.cursor.index()].part,
            part
        );
        assert!(
            matches!(modal.on(&key(Key::Char('a'))), Some(Msg::Roguelike(Command::Act(Action::Treat(index)))) if index == part.index())
        );
    }

    #[allow(clippy::unwrap_used)]
    fn rendered_summary(
        view: &RunView,
        area: Rect,
        wounds_only: bool,
    ) -> tuirealm::ratatui::buffer::Buffer {
        use crate::ui::layout::{LayoutBundle, Renderer};
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(
            area.right().saturating_add(2),
            area.bottom().saturating_add(2),
        ))
        .unwrap();
        terminal
            .draw(|frame| {
                for y in 0..frame.area().height {
                    for x in 0..frame.area().width {
                        if !area.contains(tuirealm::ratatui::layout::Position::new(x, y)) {
                            frame.buffer_mut()[(x, y)].set_symbol("#");
                        }
                    }
                }
                let wounds = wound_presentations(view);
                if wounds_only {
                    renderer
                        .paint_bounded(
                            frame,
                            area,
                            "rogue-condition",
                            &wounds,
                            "More wounds:",
                            "v",
                            Style::default(),
                        )
                        .unwrap();
                } else {
                    renderer
                        .paint_rogue_summary(
                            frame,
                            area,
                            &wounds,
                            &threat_presentations(view),
                            Style::default(),
                        )
                        .unwrap();
                }
            })
            .unwrap()
            .buffer
            .clone()
    }

    #[test]
    fn overloaded_sidebar_reports_omissions_without_hiding_the_separator() {
        let mut view = injured_view();
        view.enemies = (0..6)
            .map(|id| crate::roguelike::VisibleEnemy {
                id,
                position: Point { x: 1, y: 1 },
                name: "iron warden".into(),
                intent: "raises its weapon: strike nearby soon".into(),
                condition: String::new(),
            })
            .collect();
        let buffer = rendered_summary(&view, Rect::new(2, 1, 28, 11), false);
        let mut rows = (1..12)
            .map(|y| {
                (2..30)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        let separator = rows.iter().position(String::is_empty).unwrap();
        assert!(rows[..separator].iter().any(|row| row == "More wounds: v"));
        assert!(
            rows[separator + 1..]
                .iter()
                .any(|row| row.contains("iron warden:"))
        );
        assert!(rows.last().unwrap().ends_with("more threats"));
        assert!(rows.len() <= 11);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]
        #[test]
        fn wound_and_threat_rows_fit_arbitrary_viewports(width in 0_u16..100, height in 0_u16..50, x in 0_u16..20, y in 0_u16..10, enemies in 0_u64..25) {
            let mut view = injured_view();
            view.enemies = (0..enemies).map(|id| crate::roguelike::VisibleEnemy {
                id, position: Point { x: 1, y: 1 }, name: "cavern brute".into(),
                intent: "rears back to strike nearby soon".into(), condition: String::new(),
            }).collect();
            let area = Rect::new(x, y, width, height);
            for wounds_only in [false, true] {
                let buffer = rendered_summary(&view, area, wounds_only);
                for row in 0..buffer.area.height { for column in 0..buffer.area.width {
                    if !area.contains(tuirealm::ratatui::layout::Position::new(column, row)) {
                        prop_assert_eq!(buffer[(column, row)].symbol(), "#");
                    }
                } }
            }
        }
    }

    #[test]
    fn recent_journal_wraps_and_keeps_the_latest_consequences() {
        let mut modal = RoguelikeModal::new();
        let mut view = Run::new(19).view();
        view.journal.clear();
        let mut text = "The ash rat jumps up to bite your torso. ".repeat(10);
        text.push_str("Your left lung is damaged. Breathing grows harder.");
        view.journal.push(crate::roguelike::JournalEntry {
            id: 1,
            time: 0,
            text,
            kind: EventKind::Injury,
        });
        modal.set_run(view);
        let screen = render(&mut modal, 40, 24);
        let words = screen
            .lines()
            .map(|line| line.trim_matches(['│', ' ']))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(words.contains("Your left lung is damaged."), "{screen}");
        assert!(words.contains("Breathing grows harder."), "{screen}");
    }

    #[test]
    fn file_only_sidebar_relocation_preserves_recovery_and_game_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        let xml = include_str!("../layout/assets/templates/rogue.xml").replace(
            "        <slot id=\"rogue-map\" name=\"map\" />\n        <slot id=\"rogue-sidebar\" name=\"sidebar\" if=\"wide\" />",
            "        <slot id=\"rogue-sidebar\" name=\"sidebar\" if=\"wide\" />\n        <slot id=\"rogue-map\" name=\"map\" />",
        );
        std::fs::write(dir.path().join("templates/rogue.xml"), xml).unwrap();
        std::fs::write(
            dir.path().join("style.css"),
            "#rogue-sidebar { margin: 0 1ch 0 0; }",
        )
        .unwrap();
        let mut renderer = crate::ui::layout::Renderer::new(
            crate::ui::layout::LayoutBundle::load(dir.path()).unwrap(),
        );
        let mut modal = RoguelikeModal::new();
        modal.set_run(restable());
        modal.on(&key(Key::Char('r')));
        assert!(modal.recovery.is_some());
        let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();
        let frame = terminal
            .draw(|frame| modal.render_layout(frame, frame.area(), &mut renderer))
            .unwrap();
        let text = buffer_to_string(frame.buffer);
        assert!(
            text.lines().any(|line| line.starts_with("│No wounds")),
            "{text}"
        );
        assert!(
            modal.recovery.is_some(),
            "rendering custom layout does not cancel committed recovery"
        );
        assert!(modal.waiting);
        assert_eq!(modal.run.as_ref().unwrap().body.stamina, 20);
    }

    #[test]
    fn wound_sidebar_wraps_and_separates_threats() {
        let mut modal = RoguelikeModal::new();
        let mut view = Run::new(19).view();
        view.body.parts[8].flesh = 70;
        view.body.parts[8].bone = 90;
        view.enemies = vec![crate::roguelike::VisibleEnemy {
            id: 0,
            position: Point { x: 1, y: 1 },
            name: "ash rat".into(),
            intent: "watching".into(),
            condition: String::new(),
        }];
        modal.set_run(view);
        let screen = render(&mut modal, 120, 50);
        let sidebar: Vec<String> = screen
            .lines()
            .map(|line| line.chars().skip(91).take(28).collect())
            .collect();
        let text = sidebar
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("left foot scratched") && text.contains("bone damaged"),
            "{screen}"
        );
        assert!(!text.contains("head sound"), "{screen}");
        let threat = sidebar
            .iter()
            .position(|line| line.contains("ash rat:"))
            .unwrap();
        assert!(sidebar[threat - 1].trim().is_empty(), "{screen}");
    }

    #[test]
    fn condition_scroll_reaches_inside_an_entry_taller_than_the_viewport() {
        let mut modal = RoguelikeModal::new();
        let mut view = Run::new(19).view();
        view.body.parts[0].flesh = 10;
        view.body.parts[0].bone = 10;
        view.body.parts[0].nerve = 20;
        view.body.parts[0].bleeding = 20;
        view.body.eyes = [0, 10];
        view.body.brain = 10;
        modal.set_run(view);
        modal.on(&key(Key::Char('v')));
        let mut seen = String::new();
        for _ in 0..100 {
            seen.push_str(
                &render(&mut modal, 24, 18)
                    .lines()
                    .map(|line| line.trim_matches(['│', ' ']))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            modal.on(&key(Key::Down));
        }
        assert!(seen.contains("brain damaged"));
        assert!(seen.contains("right foot sound"));
    }

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
    fn document_scroll_anchor_does_not_advance_over_blank_lines() {
        let mut renderer =
            crate::ui::layout::Renderer::new(crate::ui::layout::LayoutBundle::builtin().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();
        let mut scroll = crate::ui::layout::DocumentScroll::default();
        scroll.advance(1);
        let rows = [crate::ui::layout::PresentedRow {
            key: "guide".into(),
            data: crate::ui::layout::Presentation::default()
                .text("body", "first\n\nthird\nfourth\nfifth\nsixth"),
            gap_after: false,
        }];
        for _ in 0..3 {
            let frame = terminal
                .draw(|frame| {
                    renderer
                        .paint_document(
                            frame,
                            frame.area(),
                            "rogue-document",
                            &rows,
                            &mut scroll,
                            Style::default(),
                        )
                        .unwrap()
                })
                .unwrap();
            let text = buffer_to_string(frame.buffer);
            let mut lines = text.lines();
            assert!(
                lines.next().unwrap().trim().is_empty(),
                "repaint retains the blank top row: {text}"
            );
            assert_eq!(lines.next().unwrap().trim(), "third");
        }
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
