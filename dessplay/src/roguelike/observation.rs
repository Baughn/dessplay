//! One honest player observation for both terminal clients and automated play.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// One map cell after perception and terrain memory have been applied.
pub struct ViewCell {
    /// Player-visible map character.
    pub glyph: char,
    /// Currently perceived rather than remembered.
    pub visible: bool,
    /// Whether this terrain has ever been perceived.
    pub remembered: bool,
    /// A currently perceived attack or collapse threatens this cell.
    pub threatened: bool,
    /// Last perceived terrain. Never the actual terrain of an unseen changed cell.
    pub terrain: Option<Tile>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Only the information available about a perceived creature.
pub struct VisibleEnemy {
    /// Stable identity on the current floor, independent of movement.
    pub id: u64,
    /// Current map position.
    pub position: Point,
    /// Perceived creature name.
    pub name: String,
    /// Readable description of the actual next committed action.
    pub intent: String,
    /// Visible consequences of the creature injuries.
    pub condition: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Complete player observation shared by the terminal UI and plain harness.
pub struct RunView {
    /// Original seed for reproducible expeditions.
    pub seed: u64,
    /// Current floor, zero based.
    pub depth: usize,
    /// Deepest floor visited, one based.
    pub deepest: usize,
    /// Current map position.
    pub position: Point,
    /// Known anatomy and physiological reserves.
    pub body: Body,
    /// Equipped armor and the two-weapon kit.
    pub gear: Equipment,
    /// Carried ordinary treatment and food supplies.
    pub supplies: Supplies,
    /// Coins currently carried.
    pub gold: u32,
    /// Creatures slain during this expedition.
    pub kills: u32,
    /// Number of completed player actions.
    pub turns: u64,
    /// Elapsed simulation time; ordinary walking costs 100.
    pub time: u64,
    /// Whether the ember has been taken.
    pub relic: bool,
    /// Living, dead, or safely escaped.
    pub outcome: Outcome,
    /// Current or frozen final score.
    pub score: u32,
    /// Frozen account of the character after this expedition.
    pub epilogue: String,
    /// Cumulative serious injuries, used to present only newly committed trauma.
    pub serious_wounds: u32,
    /// Row-major observed map; undiscovered cells contain no hidden terrain.
    pub cells: Vec<ViewCell>,
    /// Only creatures currently perceived by the player.
    pub enemies: Vec<VisibleEnemy>,
    /// Items on the player tile, in equipment-choice order.
    pub ground: Vec<LootKind>,
    /// Recent player-observable events, oldest first.
    pub journal: Vec<JournalEntry>,
    /// Whether current sights or sounds warn against automatic rest.
    pub danger: bool,
    /// Whether another automatic recovery step is currently useful.
    pub can_rest: bool,
    /// Result of the last committed action.
    pub last_step: StepResult,
    /// Stable report text, identical to the saved expedition summary.
    pub summary_text: String,
}
impl RunView {
    /// Current movement tradeoffs, before water or rubble adds terrain delay.
    pub fn movement_summary(&self) -> String {
        format!(
            "Level ground: walk {} time; sprint {} time / {} breath",
            self.body.movement_cost(false, &self.gear),
            self.body.movement_cost(true, &self.gear),
            self.body.sprint_cost(&self.gear)
        )
    }
    /// Whether the expedition has reached an immutable ending.
    pub fn is_finished(&self) -> bool {
        self.outcome != Outcome::Alive
    }
    /// Read a character through the player observation boundary.
    pub fn glyph(&self, p: Point) -> char {
        p.index()
            .and_then(|i| self.cells.get(i))
            .map_or(' ', |c| c.glyph)
    }
    /// Read terrain, treating unknown or out-of-bounds cells as walls.
    pub fn tile(&self, p: Point) -> Tile {
        p.index()
            .and_then(|i| self.cells.get(i))
            .and_then(|c| c.terrain)
            .unwrap_or(Tile::Wall)
    }
    /// Return the stable player-facing expedition report.
    pub fn summary(&self) -> String {
        self.summary_text.clone()
    }
    /// Explain the current expedition objective.
    pub fn objective(&self) -> String {
        if self.is_finished() {
            return self.summary();
        }
        if self.relic {
            "The dungeon is awake. Bring the ember to the surface < on floor 1.".into()
        } else {
            "Return alive whenever you choose. Taking the ember on floor 5 awakens the dungeon."
                .into()
        }
    }
}
impl From<Run> for RunView {
    fn from(run: Run) -> Self {
        run.view()
    }
}
impl From<&Run> for RunView {
    fn from(run: &Run) -> Self {
        run.view()
    }
}

impl Run {
    pub(super) fn reveal(&mut self) {
        let p = self.position;
        let radius = self.body.vision_radius();
        let floor = &mut self.floors[self.depth];
        floor.visible.fill(false);
        for y in (p.y - radius).max(0)..=(p.y + radius).min(HEIGHT - 1) {
            for x in (p.x - radius).max(0)..=(p.x + radius).min(WIDTH - 1) {
                let point = Point { x, y };
                // Chebyshev adjacency remains perceptible even with both eyes lost.
                if ((x - p.x).pow(2) + (y - p.y).pow(2) <= radius * radius
                    || p.distance(point) <= 1)
                    && floor.sight(p, point)
                {
                    let i = (y * WIDTH + x) as usize;
                    floor.visible[i] = true;
                    floor.remembered[i] = Some(floor.tiles[i]);
                }
            }
        }
    }
    /// Evaluate danger from current perception and recent audible warnings.
    pub fn danger(&self) -> bool {
        self.alert_until > self.time
            || self
                .floor()
                .enemies
                .iter()
                .any(|e| e.position.index().is_some_and(|i| self.floor().visible[i]))
    }
    /// Compute exploration and banked-loot score, or return the frozen ending score.
    pub fn score(&self) -> u32 {
        self.final_score.unwrap_or_else(|| {
            let explored = self
                .floors
                .iter()
                .map(|f| f.remembered.iter().filter(|t| t.is_some()).count() as u32)
                .sum::<u32>();
            let escaped = self.outcome == Outcome::Escaped;
            explored
                + 100 * (self.deepest.saturating_sub(1) as u32)
                + if escaped {
                    100 + 10 * self.gold + if self.has_ember() { 10_000 } else { 0 }
                } else {
                    0
                }
        })
    }
    /// Return the stable player-facing expedition report.
    pub fn summary(&self) -> String {
        let result = match &self.outcome {
            Outcome::Alive => "is exploring".to_owned(),
            Outcome::Dead(cause) => format!("died {cause}"),
            Outcome::Escaped if self.has_ember() => "escaped with the ember".into(),
            Outcome::Escaped => "escaped alive without the ember".into(),
        };
        format!(
            "The Waiting Below: {result}; floor {}/{FLOOR_COUNT}, {} kills, {} gold, {} turns, {} points.{}",
            self.deepest,
            self.kills,
            self.gold,
            self.turns,
            self.score(),
            if self.epilogue.is_empty() {
                String::new()
            } else {
                format!(" {}", self.epilogue)
            }
        )
    }
    /// Read a character through the player observation boundary.
    pub fn glyph(&self, p: Point) -> char {
        self.view().glyph(p)
    }
    /// Build an observation containing no unseen creatures or unremembered terrain.
    pub fn view(&self) -> RunView {
        let floor = self.floor();
        // Older saves may already contain a long run of routine-care messages.
        // Collapse their presentation without rewriting history during a read.
        let mut journal = self.journal.clone();
        journal.dedup_by(|later, earlier| {
            later.kind == EventKind::Recovery
                && earlier.kind == EventKind::Recovery
                && later.text == earlier.text
        });
        let cells = (0..CELLS)
            .map(|i| {
                let p = Point {
                    x: (i as i32) % WIDTH,
                    y: (i as i32) / WIDTH,
                };
                let visible = floor.visible[i];
                let terrain = floor.remembered[i];
                let mut glyph = terrain.map_or(' ', Tile::glyph);
                if visible {
                    if let Some(loot) = floor.loot.iter().find(|l| l.position == p) {
                        glyph = loot.kind.glyph();
                    }
                    if let Some(enemy) = floor.enemies.iter().find(|e| e.position == p) {
                        glyph = enemy.kind.glyph();
                    }
                }
                if p == self.position {
                    glyph = '@';
                }
                let threatened = visible
                    && (floor.threatened(p)
                        || floor.enemies.iter().any(|e| {
                            e.position.index().is_some_and(|j| floor.visible[j])
                                && matches!(e.intent,EnemyIntent::Strike{target,..} if target==p)
                        }));
                ViewCell {
                    glyph,
                    visible,
                    remembered: terrain.is_some(),
                    threatened,
                    terrain,
                }
            })
            .collect();
        let enemies = floor
            .enemies
            .iter()
            .filter(|e| e.position.index().is_some_and(|i| floor.visible[i]))
            .map(|e| {
                let intent = match e.intent {
                    EnemyIntent::Idle => "watching".into(),
                    EnemyIntent::Strike { target, at } => format!(
                        "raising weapon: strike ({},{}) in {} time",
                        target.x,
                        target.y,
                        at.saturating_sub(floor.time)
                    ),
                    EnemyIntent::Calling { at } => {
                        format!("calling in {} time", at.saturating_sub(floor.time))
                    }
                    EnemyIntent::Recovering { until } => {
                        format!("recovering for {} time", until.saturating_sub(floor.time))
                    }
                };
                VisibleEnemy {
                    id: e.id,
                    position: e.position,
                    name: e.kind.name().into(),
                    intent,
                    condition: format!(
                        "bleed {}, breath {}, pain {}",
                        e.body.bleeding(),
                        e.body.stamina,
                        e.body.pain()
                    ),
                }
            })
            .collect();
        RunView {
            seed: self.seed,
            depth: self.depth,
            deepest: self.deepest,
            position: self.position,
            body: self.body.clone(),
            gear: self.gear.clone(),
            supplies: self.supplies.clone(),
            gold: self.gold,
            kills: self.kills,
            turns: self.turns,
            time: self.time,
            relic: self.has_ember(),
            outcome: self.outcome.clone(),
            score: self.score(),
            epilogue: self.epilogue.clone(),
            serious_wounds: self.serious_wounds,
            cells,
            enemies,
            ground: floor
                .loot
                .iter()
                .filter(|l| l.position == self.position)
                .map(|l| l.kind)
                .collect(),
            journal,
            danger: self.danger(),
            can_rest: !self.is_finished()
                && !self.danger()
                && self.body.can_recover(&self.supplies),
            last_step: self.last_step.clone(),
            summary_text: self.summary(),
        }
    }
}
