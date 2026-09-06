//! The Waiting Below: a deterministic expedition. Only explicit actions advance time.
use serde::{Deserialize, Serialize};
pub mod anatomy;
mod observation;
mod simulation;
pub mod world;
pub use anatomy::*;
pub use observation::*;
pub use world::{Enemy, EnemyIntent, EnemyKind, Floor, Loot, LootKind, Tile};
/// Map width in tiles.
pub const WIDTH: i32 = 49;
/// Map height in tiles.
pub const HEIGHT: i32 = 23;
/// Floors in each expedition.
pub const FLOOR_COUNT: usize = 5;
/// Cells per floor.
pub const CELLS: usize = (WIDTH * HEIGHT) as usize;
/// Maximum retained player journal entries.
pub const JOURNAL_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A map coordinate.
pub struct Point {
    /// Horizontal map coordinate.
    pub x: i32,
    /// Vertical map coordinate.
    pub y: i32,
}
impl Point {
    /// Offset a coordinate with saturation; callers still validate map bounds.
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
        }
    }
    /// Return a row-major index only for in-bounds coordinates.
    pub fn index(self) -> Option<usize> {
        (self.x >= 0 && self.x < WIDTH && self.y >= 0 && self.y < HEIGHT)
            .then(|| (self.y * WIDTH + self.x) as usize)
    }
    /// Chebyshev distance without overflowing malformed coordinates.
    pub fn distance(self, other: Self) -> i32 {
        self.x
            .abs_diff(other.x)
            .max(self.y.abs_diff(other.y))
            .min(i32::MAX as u32) as i32
    }
}
/// SplitMix64 with its complete resumable state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng(pub u64);
impl Rng {
    /// Draw a deterministic number below the bound; a zero bound is treated as one.
    pub fn below(&mut self, upper: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (z ^ (z >> 31)) % upper.max(1)
    }
}
/// UI navigation never enters the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Walk or attack a perceived enemy within weapon reach in that direction.
    Move(i32, i32),
    /// Spend breath to move one tile faster; never attacks.
    Sprint(i32, i32),
    /// Attack along a direction, using weapon reach.
    Attack(i32, i32),
    /// Catch breath for one ordinary interval.
    Wait,
    /// Dress the most urgently bleeding injury.
    Bandage,
    /// Eat a ration when useful.
    Eat,
    /// Perform one automatic care or recovery step.
    Rest,
    /// Use the stair underfoot, including early escape.
    Stairs,
    /// Explicitly take the ember or activate a fountain.
    Interact,
    /// Exchange active and spare weapons.
    SwapWeapon,
    /// Equip the indexed item from the ground observation.
    Equip(usize),
    /// Close an adjacent open door.
    CloseDoor(i32, i32),
    /// Treat the selected anatomical part.
    Treat(usize),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Terminal outcome, distinct from whether the ember was taken.
pub enum Outcome {
    /// Still exploring.
    Alive,
    /// Died, with the frozen cause.
    Dead(String),
    /// Returned alive, with or without the ember.
    Escaped,
}
/// Taking the ember permanently awakens the world; it cannot be dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// The dungeon remains dormant.
    Descent,
    /// Taking the ember has permanently awakened every floor.
    Awakened,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Semantic journal event category, also used by cosmetic presentation.
pub enum EventKind {
    /// Ordinary action feedback.
    Info,
    /// A perceived attack or creature death.
    Combat,
    /// The player suffers an injury.
    Injury,
    /// A perceived threat or environmental warning.
    Danger,
    /// Treatment or physiological recovery.
    Recovery,
    /// A place or item is discovered.
    Discovery,
    /// The expedition ends or its later life is recounted.
    Ending,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// One durable player-observable journal event.
pub struct JournalEntry {
    /// Stable identity within its saved collection.
    pub id: u64,
    /// Elapsed simulation time; ordinary walking costs 100.
    pub time: u64,
    /// Human-readable event description.
    pub text: String,
    /// Semantic event category.
    pub kind: EventKind,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
/// Action execution and interruption status for pacing recovery.
pub struct StepResult {
    /// Simulation time spent by this action.
    pub elapsed: u64,
    /// Whether the action changed saved state or feedback.
    pub changed: bool,
    /// Whether danger, injury, or completion interrupts automatic recovery.
    pub interrupted: bool,
}

/// Durable simulation. Renderers receive the observation-only RunView.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Original seed for reproducible expeditions.
    pub seed: u64,
    /// Complete generated and transformed dungeon.
    pub floors: Vec<Floor>,
    /// Current floor, zero based.
    pub depth: usize,
    /// Deepest floor visited, one based.
    pub deepest: usize,
    /// Current map position.
    pub position: Point,
    /// Known anatomy and physiological reserves.
    pub body: Body,
    /// Carried ordinary treatment and food supplies.
    pub supplies: Supplies,
    /// Equipped armor and the two-weapon kit.
    pub gear: Equipment,
    /// Coins currently carried.
    pub gold: u32,
    /// Creatures slain during this expedition.
    pub kills: u32,
    /// Number of completed player actions.
    pub turns: u64,
    /// Elapsed simulation time; ordinary walking costs 100.
    pub time: u64,
    /// Permanent descent or awakened phase.
    pub phase: Phase,
    /// Living, dead, or safely escaped.
    pub outcome: Outcome,
    /// Recent player-observable events, oldest first.
    pub journal: Vec<JournalEntry>,
    /// Frozen account of the character after this expedition.
    pub epilogue: String,
    /// Final score, present exactly when the expedition ends.
    pub final_score: Option<u32>,
    /// Result of the last committed action.
    pub last_step: StepResult,
    rng: Rng,
    next_event: u64,
    alert_until: u64,
    serious_wounds: u32,
    fountains_used: u8,
}
impl Run {
    /// Generate a fresh expedition using only its seed.
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng(seed);
        let fountain = if rng.below(4) == 0 {
            Some(1 + rng.below(3) as usize)
        } else {
            None
        };
        let floors: Vec<_> = (0..FLOOR_COUNT)
            .map(|d| world::generate(&mut rng, d, fountain == Some(d)))
            .collect();
        let position = floors[0].entrance;
        let mut run = Self {
            seed,
            floors,
            depth: 0,
            deepest: 1,
            position,
            body: Body::default(),
            supplies: Supplies::default(),
            gear: Equipment::default(),
            gold: 0,
            kills: 0,
            turns: 0,
            time: 0,
            phase: Phase::Descent,
            outcome: Outcome::Alive,
            journal: Vec::new(),
            epilogue: String::new(),
            final_score: None,
            last_step: StepResult::default(),
            rng,
            next_event: 1,
            alert_until: 0,
            serious_wounds: 0,
            fountains_used: 0,
        };
        run.say(
            EventKind::Info,
            "The Waiting Below. Returning alive is a victory.",
        );
        run.say(
            EventKind::Info,
            "The ember lies on floor five. Taking it awakens the dungeon.",
        );
        run.say(
            EventKind::Info,
            "Rest (r) tends wounds; uppercase movement sprints. Use g to interact.",
        );
        run.reveal();
        run
    }
    /// Read the current floor.
    pub fn floor(&self) -> &Floor {
        &self.floors[self.depth]
    }
    /// Read terrain, treating unknown or out-of-bounds cells as walls.
    pub fn tile(&self, p: Point) -> Tile {
        self.floor().tile(p)
    }
    /// Whether the expedition has reached an immutable ending.
    pub fn is_finished(&self) -> bool {
        self.outcome != Outcome::Alive
    }
    /// Whether taking the ember has permanently awakened the dungeon.
    pub fn has_ember(&self) -> bool {
        self.phase == Phase::Awakened
    }
    /// Execute an action and report whether saved state or feedback changed.
    pub fn act(&mut self, action: Action) -> bool {
        self.step(action).changed
    }
    fn say(&mut self, kind: EventKind, text: impl Into<String>) {
        let text = text.into();
        // Routine care advances physiology without filling the journal with
        // identical lines. Real intervening events and distinct treatments stay.
        if kind == EventKind::Recovery
            && self
                .journal
                .last()
                .is_some_and(|e| e.kind == kind && e.text == text)
        {
            return;
        }
        self.journal.push(JournalEntry {
            id: self.next_event,
            time: self.time,
            kind,
            text,
        });
        self.next_event = self.next_event.saturating_add(1);
        if self.journal.len() > JOURNAL_CAPACITY {
            self.journal.remove(0);
        }
    }
    /// Validate a save before indexed access or exposing a view.
    pub fn validate(&self) -> Result<(), String> {
        if self.floors.len() != FLOOR_COUNT
            || self.depth >= FLOOR_COUNT
            || !(self.depth + 1..=FLOOR_COUNT).contains(&self.deepest)
            || self.position.index().is_none()
            || !self.time.is_multiple_of(50)
            || self.time > u64::MAX - 100_000
            || self.turns > u64::MAX - 10_000
        {
            return Err("Invalid expedition coordinates or clock".into());
        }
        self.body.validate()?;
        self.gear.validate()?;
        if self.supplies.bandages > 128
            || self.supplies.splints > 128
            || self.supplies.food > 128
            || self.gold > 100_000
            || self.journal.len() > JOURNAL_CAPACITY
            || self.epilogue.len() > 2048
            || self
                .journal
                .iter()
                .any(|e| e.text.len() > 2048 || e.time > self.time)
            || self.journal.windows(2).any(|w| w[0].id >= w[1].id)
            || self.journal.last().is_some_and(|e| e.id >= self.next_event)
            || matches!(&self.outcome,Outcome::Dead(s) if s.len()>512)
        {
            return Err("Invalid supplies or journal".into());
        }
        let mut relics = 0;
        for (d, floor) in self.floors.iter().enumerate() {
            floor.validate()?;
            if floor.time > self.time || !floor.time.is_multiple_of(50) {
                return Err("Invalid floor clock".into());
            }
            if floor.tile(floor.entrance) != Tile::Up
                || (d + 1 < FLOOR_COUNT && floor.tile(floor.exit) != Tile::Down)
            {
                return Err("Invalid stairways".into());
            }
            for enemy in &floor.enemies {
                enemy.body.validate()?;
                enemy.gear.validate()?;
                if enemy.body.is_dead()
                    || enemy.position.index().is_none()
                    || enemy.target.is_some_and(|p| p.index().is_none())
                    || (d == self.depth && enemy.position == self.position)
                {
                    return Err("Invalid creature".into());
                }
            }
            for loot in &floor.loot {
                if loot.kind == LootKind::Relic {
                    relics += 1;
                    if d + 1 != FLOOR_COUNT || loot.position != floor.exit {
                        return Err("Invalid ember".into());
                    }
                }
            }
        }
        if !self.tile(self.position).walkable()
            || relics != usize::from(!self.has_ember())
            || self.outcome == Outcome::Alive && self.body.is_dead()
            || self.outcome == Outcome::Escaped
                && (self.depth != 0 || self.position != self.floor().entrance)
            || self.is_finished() != self.final_score.is_some()
        {
            return Err("Invalid expedition outcome".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
