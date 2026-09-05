//! The Waiting Below: a deterministic, local, turn-based dungeon expedition.
//!
//! The complete simulation (including its PRNG and remembered map) is a save.
//! Only `Run::act` advances it; closing the modal or waiting for friends does not.

use serde::{Deserialize, Serialize};

/// Dungeon width in cells.
pub const WIDTH: i32 = 49;
/// Dungeon height in cells.
pub const HEIGHT: i32 = 23;
/// Number of floors in an expedition.
pub const FLOOR_COUNT: usize = 5;
/// Wound slots, in their display order.
pub const BODY_PARTS: [&str; 6] = [
    "Head",
    "Torso",
    "Left arm",
    "Right arm",
    "Left leg",
    "Right leg",
];
const CELLS: usize = (WIDTH * HEIGHT) as usize;

/// A dungeon coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: i32,
    /// Vertical coordinate.
    pub y: i32,
}

impl Point {
    fn offset(self, x: i32, y: i32) -> Self {
        Self {
            x: self.x + x,
            y: self.y + y,
        }
    }

    fn index(self) -> Option<usize> {
        (self.x >= 0 && self.x < WIDTH && self.y >= 0 && self.y < HEIGHT)
            .then(|| (self.y * WIDTH + self.x) as usize)
    }

    fn distance(self, other: Self) -> i32 {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }
}

/// Fixed terrain. Items and creatures are separate layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tile {
    /// Solid rock.
    Wall,
    /// Walkable ground.
    Floor,
    /// Stairway to the previous floor, or daylight on floor one.
    Up,
    /// Stairway to the next floor.
    Down,
}

/// A wound's structural injury and continuing blood loss per turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wound {
    /// Injury from zero to 100; 100 in the head or torso is fatal.
    pub severity: u16,
    /// Blood lost each turn until bandaged.
    pub bleeding: u16,
}

/// Physiology: injuries have consequences instead of sharing one hit-point bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    /// Circulating blood, zero to 1000; zero is fatal.
    pub blood: u16,
    /// Exertion reserve, zero to 100.
    pub stamina: u16,
    /// Nourishment, zero to 100; empty reserves cause wasting.
    pub hunger: u16,
    /// Head, torso, arms, then legs; see `BODY_PARTS`.
    pub wounds: [Wound; 6],
}

impl Body {
    /// Total bleeding per turn, stopped one wound at a time by bandaging.
    pub fn bleeding(&self) -> u16 {
        self.wounds.iter().map(|w| w.bleeding).sum()
    }

    /// Pain from all injuries, capped at 100.
    pub fn pain(&self) -> u16 {
        (self.wounds.iter().map(|w| w.severity).sum::<u16>() / 3).min(100)
    }
}

/// The creatures beneath the waiting room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyKind {
    /// Fragile, quick scavenger.
    Rat,
    /// A hungry human remnant.
    Hollow,
    /// Heavy, well-armored guardian.
    Warden,
}

impl EnemyKind {
    /// Short name used in combat messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rat => "ash rat",
            Self::Hollow => "hollow pilgrim",
            Self::Warden => "iron warden",
        }
    }

    /// Map symbol.
    pub fn glyph(self) -> char {
        match self {
            Self::Rat => 'r',
            Self::Hollow => 'h',
            Self::Warden => 'W',
        }
    }
}

/// A creature on one floor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enemy {
    /// Creature type.
    pub kind: EnemyKind,
    /// Current position.
    pub position: Point,
    /// Remaining endurance; dead creatures are removed immediately.
    pub health: u16,
}

/// Automatically collected supplies and treasure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LootKind {
    /// Two strips of clean linen.
    Bandage,
    /// One nourishing ration.
    Food,
    /// Coins contributing to the final score.
    Gold,
    /// A weapon upgrade, indexed from one.
    Weapon(u16),
    /// Armor upgrade, indexed from one.
    Armor(u16),
    /// The expedition's goal, carried back to the surface.
    Relic,
}

impl LootKind {
    /// Map symbol.
    pub fn glyph(self) -> char {
        match self {
            Self::Bandage => '!',
            Self::Food => '%',
            Self::Gold => '$',
            Self::Weapon(_) => ')',
            Self::Armor(_) => '[',
            Self::Relic => '*',
        }
    }
}

/// One item pile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loot {
    /// Item location.
    pub position: Point,
    /// Contents.
    pub kind: LootKind,
}

/// A generated floor; other floors freeze while the explorer is elsewhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Floor {
    /// Row-major terrain of `WIDTH * HEIGHT` cells.
    pub tiles: Vec<Tile>,
    /// Terrain remembered by the explorer.
    pub explored: Vec<bool>,
    /// Cells currently in sight; creatures and loot require current sight.
    pub visible: Vec<bool>,
    /// Surviving creatures.
    pub enemies: Vec<Enemy>,
    /// Uncollected items.
    pub loot: Vec<Loot>,
    /// Position of the upward staircase.
    pub entrance: Point,
    /// Position of the downward staircase (or the ember on floor five).
    pub exit: Point,
}

impl Floor {
    /// Terrain at a coordinate; outside the map is solid rock.
    pub fn tile(&self, point: Point) -> Tile {
        point
            .index()
            .and_then(|i| self.tiles.get(i))
            .copied()
            .unwrap_or(Tile::Wall)
    }

    fn step_allowed(&self, from: Point, to: Point) -> bool {
        self.tile(to) != Tile::Wall
            && (from.x == to.x
                || from.y == to.y
                || (self.tile(Point { x: from.x, y: to.y }) != Tile::Wall
                    && self.tile(Point { x: to.x, y: from.y }) != Tile::Wall))
    }

    fn sight(&self, from: Point, to: Point) -> bool {
        let mut p = from;
        let dx = (to.x - from.x).abs();
        let dy = -(to.y - from.y).abs();
        let sx = (to.x - from.x).signum();
        let sy = (to.y - from.y).signum();
        let mut error = dx + dy;
        while p != to {
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                p.x += sx;
            }
            if twice <= dx {
                error += dx;
                p.y += sy;
            }
            if p == to {
                return true;
            }
            if self.tile(p) == Tile::Wall {
                return false;
            }
        }
        true
    }
}

/// A turn request. Invalid actions can add feedback but never cost a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move one cell, or attack its occupant; diagonals are permitted.
    Move(i32, i32),
    /// Remain still for one turn and catch one's breath.
    Wait,
    /// Treat the most urgently bleeding or injured body part.
    Bandage,
    /// Consume one ration.
    Eat,
    /// Recover for one turn, only when no creature is in sight and not bleeding.
    Rest,
    /// Use the staircase underfoot, or escape with the ember.
    Stairs,
}

/// Terminal results are immutable: no action can revive or alter a finished run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Still exploring.
    Alive,
    /// Dead, with the cause suitable for the shared chat summary.
    Dead(String),
    /// Returned the ember to daylight.
    Escaped,
}

/// Complete resumable expedition, including deterministic random state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Original seed, useful for replaying an expedition.
    pub seed: u64,
    /// All five generated floors.
    pub floors: Vec<Floor>,
    /// Current floor, zero-based.
    pub depth: usize,
    /// Deepest floor reached, one-based.
    pub deepest: usize,
    /// Explorer location.
    pub position: Point,
    /// Physiology.
    pub body: Body,
    /// Available bandages.
    pub bandages: u16,
    /// Available rations.
    pub food: u16,
    /// Weapon tier, zero to five.
    pub weapon: u16,
    /// Armor tier, zero to five.
    pub armor: u16,
    /// Recovered coins.
    pub gold: u32,
    /// Creatures slain.
    pub kills: u32,
    /// Completed simulation turns.
    pub turns: u64,
    /// Whether the ember has been recovered.
    pub relic: bool,
    /// Current outcome.
    pub outcome: Outcome,
    /// Most recent 80 expedition messages, oldest first.
    pub log: Vec<String>,
    rng: Rng,
}

/// SplitMix64 with its entire state in the save, independent of `rand` versions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Rng(u64);

impl Rng {
    fn below(&mut self, upper: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (z ^ (z >> 31)) % upper
    }
}

impl Run {
    /// Generate a fresh expedition. Nothing depends on the clock or network.
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng(seed);
        let floors: Vec<_> = (0..FLOOR_COUNT)
            .map(|depth| generate(&mut rng, depth))
            .collect();
        let position = floors[0].entrance;
        let mut run = Self {
            seed,
            floors,
            depth: 0,
            deepest: 1,
            position,
            body: Body {
                blood: 1000,
                stamina: 100,
                hunger: 100,
                wounds: [Wound::default(); 6],
            },
            bandages: 4,
            food: 3,
            weapon: 0,
            armor: 0,
            gold: 0,
            kills: 0,
            turns: 0,
            relic: false,
            outcome: Outcome::Alive,
            rng,
            log: vec![
                "The Waiting Below".into(),
                "Bring the ember (*) from floor 5 back to this stair (<).".into(),
                "Bump enemies to fight. Gather ! linen, % food, ) weapons, [ armor.".into(),
                "Bleeding? Bandage. Rest in safety. Nothing moves while you are away.".into(),
            ],
        };
        run.reveal();
        run
    }

    /// Current floor.
    pub fn floor(&self) -> &Floor {
        &self.floors[self.depth]
    }

    /// Current floor's terrain.
    pub fn tile(&self, point: Point) -> Tile {
        self.floor().tile(point)
    }

    /// Whether an expedition has ended.
    pub fn is_finished(&self) -> bool {
        self.outcome != Outcome::Alive
    }

    /// Check a deserialized save before exposing its indexed map to the UI or
    /// simulation. Invalid data is reported without discarding the stored save.
    pub fn validate(&self) -> Result<(), String> {
        if self.floors.len() != FLOOR_COUNT
            || self.depth >= FLOOR_COUNT
            || !(self.depth + 1..=FLOOR_COUNT).contains(&self.deepest)
        {
            return Err("Invalid dungeon depth".into());
        }
        if self.body.blood > 1000
            || self.body.stamina > 100
            || self.body.hunger > 100
            || self
                .body
                .wounds
                .iter()
                .any(|w| w.severity > 100 || w.bleeding > 8)
            || self.weapon > 5
            || self.armor > 5
            || self.bandages > 64
            || self.food > 64
        {
            return Err("Invalid explorer condition or equipment".into());
        }
        if self.log.len() > 80
            || self.log.iter().any(|s| s.len() > 1024)
            || matches!(&self.outcome, Outcome::Dead(cause) if cause.len() > 512)
        {
            return Err("Invalid expedition log".into());
        }
        let mut relics = 0;
        for (depth, floor) in self.floors.iter().enumerate() {
            if floor.tiles.len() != CELLS
                || floor.explored.len() != CELLS
                || floor.visible.len() != CELLS
                || floor.enemies.len() > 8
                || floor.loot.len() > 9
            {
                return Err("Invalid floor dimensions or occupants".into());
            }
            if floor.entrance == floor.exit
                || floor.tile(floor.entrance) != Tile::Up
                || floor.tile(floor.exit)
                    != if depth + 1 == FLOOR_COUNT {
                        Tile::Floor
                    } else {
                        Tile::Down
                    }
            {
                return Err("Invalid stairways".into());
            }
            for (i, tile) in floor.tiles.iter().enumerate() {
                let point = Point {
                    x: i as i32 % WIDTH,
                    y: i as i32 / WIDTH,
                };
                if (point.x == 0 || point.y == 0 || point.x == WIDTH - 1 || point.y == HEIGHT - 1)
                    && *tile != Tile::Wall
                    || *tile == Tile::Up && point != floor.entrance
                    || *tile == Tile::Down && (point != floor.exit || depth + 1 == FLOOR_COUNT)
                    || floor.visible[i] && !floor.explored[i]
                {
                    return Err("Invalid floor terrain or visibility".into());
                }
            }
            for (i, enemy) in floor.enemies.iter().enumerate() {
                if floor.tile(enemy.position) != Tile::Floor
                    || enemy.health == 0
                    || enemy.health > 42
                    || floor.enemies[..i]
                        .iter()
                        .any(|e| e.position == enemy.position)
                    || depth == self.depth && enemy.position == self.position
                {
                    return Err("Invalid creature position or condition".into());
                }
            }
            for (i, loot) in floor.loot.iter().enumerate() {
                if floor.tile(loot.position) != Tile::Floor
                    || floor.loot[..i].iter().any(|l| l.position == loot.position)
                    || matches!(loot.kind, LootKind::Weapon(t) | LootKind::Armor(t) if !(1..=5).contains(&t))
                {
                    return Err("Invalid supplies".into());
                }
                if loot.kind == LootKind::Relic {
                    relics += 1;
                    if depth + 1 != FLOOR_COUNT || loot.position != floor.exit {
                        return Err("Invalid ember location".into());
                    }
                }
            }
        }
        if self.tile(self.position) == Tile::Wall
            || relics != usize::from(!self.relic)
            || self.outcome == Outcome::Escaped
                && (!self.relic || self.depth != 0 || self.position != self.floor().entrance)
            || self.outcome == Outcome::Alive
                && (self.body.blood == 0
                    || self.body.wounds[0].severity == 100
                    || self.body.wounds[1].severity == 100)
        {
            return Err("Invalid expedition position or outcome".into());
        }
        Ok(())
    }

    /// Short score and outcome for a chat announcement or memorial.
    pub fn summary(&self) -> String {
        let outcome = match &self.outcome {
            Outcome::Alive => "is exploring".to_owned(),
            Outcome::Dead(cause) => format!("died {cause}"),
            Outcome::Escaped => "escaped with the ember".to_owned(),
        };
        format!(
            "The Waiting Below: {outcome}; floor {}/{FLOOR_COUNT}, {} kills, {} gold, {} turns{}.",
            self.deepest,
            self.kills,
            self.gold,
            self.turns,
            if self.relic && self.outcome != Outcome::Escaped {
                ", carrying the ember"
            } else {
                ""
            }
        )
    }

    /// Map symbol with fog and live occupants applied.
    pub fn glyph(&self, point: Point) -> char {
        if point == self.position {
            return '@';
        }
        let Some(index) = point.index() else {
            return ' ';
        };
        let floor = self.floor();
        if !floor.explored[index] {
            return ' ';
        }
        if floor.visible[index] {
            if let Some(enemy) = floor.enemies.iter().find(|e| e.position == point) {
                return enemy.kind.glyph();
            }
            if let Some(loot) = floor.loot.iter().find(|l| l.position == point) {
                return loot.kind.glyph();
            }
        }
        match floor.tile(point) {
            Tile::Wall => '#',
            Tile::Floor => '.',
            Tile::Up => '<',
            Tile::Down => '>',
        }
    }

    /// Perform one action. Returns whether any saved state changed, including
    /// feedback for invalid commands. A finished expedition is always unchanged.
    pub fn act(&mut self, action: Action) -> bool {
        if self.is_finished() {
            return false;
        }
        let before_turn = self.turns;
        let before_log = self.log.clone();
        let mut rest = false;
        let spent = match action {
            Action::Move(dx, dy) => self.move_or_attack(dx, dy),
            Action::Wait => {
                self.body.stamina = (self.body.stamina + 12).min(100);
                true
            }
            Action::Bandage => self.bandage(),
            Action::Eat => {
                if self.food == 0 {
                    self.say("Your food pouch is empty.");
                    false
                } else if self.body.hunger > 75 {
                    self.say("Save your food: you are still well fed.");
                    false
                } else {
                    self.food -= 1;
                    self.body.hunger = (self.body.hunger + 50).min(100);
                    self.body.stamina = (self.body.stamina + 20).min(100);
                    self.say("You eat a ration. Warmth returns to your limbs.");
                    true
                }
            }
            Action::Rest => {
                if self.body.bleeding() > 0 {
                    self.say("Stop the bleeding before you rest. Use a bandage (a).");
                    false
                } else if self
                    .floor()
                    .enemies
                    .iter()
                    .any(|e| e.position.index().is_some_and(|i| self.floor().visible[i]))
                {
                    self.say("A creature is watching. Find cover before resting.");
                    false
                } else {
                    rest = true;
                    true
                }
            }
            Action::Stairs => self.stairs(),
        };
        if spent {
            self.turns = self.turns.saturating_add(1);
            if !self.is_finished() {
                self.physiology(rest);
                self.check_death(if self.body.hunger == 0 && self.body.bleeding() == 0 {
                    "from starvation"
                } else {
                    "from blood loss"
                });
                if !self.is_finished() {
                    self.enemies_turn();
                }
                self.reveal();
            }
        }
        before_turn != self.turns || before_log != self.log
    }

    fn say(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.log.last() != Some(&message) {
            self.log.push(message);
        }
        if self.log.len() > 80 {
            self.log.remove(0);
        }
    }

    fn move_or_attack(&mut self, dx: i32, dy: i32) -> bool {
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dy) || (dx == 0 && dy == 0) {
            return false;
        }
        let target = self.position.offset(dx, dy);
        if !self.floor().step_allowed(self.position, target) {
            self.say("Solid stone blocks your way.");
            return false;
        }
        if let Some(index) = self
            .floor()
            .enemies
            .iter()
            .position(|e| e.position == target)
        {
            let arm_penalty = (self.body.wounds[2].severity + self.body.wounds[3].severity) / 30;
            let exhaustion = if self.body.stamina < 12 { 4 } else { 0 };
            let damage = (7 + self.weapon * 3 + self.rng.below(5) as u16)
                .saturating_sub(arm_penalty + exhaustion)
                .max(2);
            self.body.stamina = self.body.stamina.saturating_sub(12);
            let enemy = &mut self.floors[self.depth].enemies[index];
            let kind = enemy.kind;
            enemy.health = enemy.health.saturating_sub(damage);
            if enemy.health == 0 {
                self.floors[self.depth].enemies.remove(index);
                self.kills += 1;
                self.gold += 2 + self.depth as u32;
                self.say(format!(
                    "You slay the {} (+{} gold).",
                    kind.name(),
                    2 + self.depth
                ));
            } else {
                self.say(format!("You strike the {} for {damage}.", kind.name()));
            }
        } else {
            self.position = target;
            let limp = (self.body.wounds[4].severity + self.body.wounds[5].severity) / 35;
            self.body.stamina = (self.body.stamina + 3).min(100).saturating_sub(limp);
            self.collect();
        }
        true
    }

    fn collect(&mut self) {
        while let Some(index) = self
            .floor()
            .loot
            .iter()
            .position(|l| l.position == self.position)
        {
            let loot = self.floors[self.depth].loot.remove(index);
            match loot.kind {
                LootKind::Bandage => {
                    self.bandages += 2;
                    self.say("You find two clean linen bandages (!).");
                }
                LootKind::Food => {
                    self.food += 1;
                    self.say("You pack a ration of dried apples (%).");
                }
                LootKind::Gold => {
                    let amount = 10 + self.rng.below(20) as u32;
                    self.gold += amount;
                    self.say(format!("You pocket {amount} old coins."));
                }
                LootKind::Weapon(tier) => {
                    self.weapon = self.weapon.max(tier);
                    self.say(format!(
                        "You equip a keener blade (weapon {}).",
                        self.weapon
                    ));
                }
                LootKind::Armor(tier) => {
                    self.armor = self.armor.max(tier);
                    self.say(format!(
                        "You buckle on stronger armor (armor {}).",
                        self.armor
                    ));
                }
                LootKind::Relic => {
                    self.relic = true;
                    self.say("The ember fits in your palm. Now return to the surface (<)!");
                }
            }
        }
    }

    fn bandage(&mut self) -> bool {
        if self.bandages == 0 {
            self.say("No linen remains. Find more (!) before the bleeding wins.");
            return false;
        }
        let target = self
            .body
            .wounds
            .iter()
            .enumerate()
            .filter(|(_, w)| w.severity > 0 || w.bleeding > 0)
            .max_by_key(|(_, w)| (w.bleeding, w.severity))
            .map(|(i, _)| i);
        let Some(index) = target else {
            self.say("You have no wounds to dress.");
            return false;
        };
        self.bandages -= 1;
        self.body.wounds[index].bleeding = 0;
        self.body.wounds[index].severity = self.body.wounds[index].severity.saturating_sub(8);
        self.say(format!(
            "You bind your {}. Its bleeding stops.",
            BODY_PARTS[index].to_lowercase()
        ));
        true
    }

    fn stairs(&mut self) -> bool {
        match self.tile(self.position) {
            Tile::Up if self.depth == 0 => {
                if self.relic {
                    self.outcome = Outcome::Escaped;
                    self.say("Daylight. The ember lives, and so do you.");
                    true
                } else {
                    self.say("The ember is still below, on floor 5. Your expedition awaits.");
                    false
                }
            }
            Tile::Up => {
                self.depth -= 1;
                self.position = self.floor().exit;
                self.say(format!("You climb to floor {}.", self.depth + 1));
                true
            }
            Tile::Down if self.depth + 1 < self.floors.len() => {
                self.depth += 1;
                self.deepest = self.deepest.max(self.depth + 1);
                self.position = self.floor().entrance;
                self.say(format!(
                    "Floor {}: {}.",
                    self.depth + 1,
                    [
                        "the vestibule",
                        "the drowned pantry",
                        "the pilgrim cells",
                        "the iron choir",
                        "the ember vault"
                    ][self.depth]
                ));
                true
            }
            _ => {
                self.say("Stand on < or > to use the stairs.");
                false
            }
        }
    }

    fn physiology(&mut self, rest: bool) {
        self.body.blood = self.body.blood.saturating_sub(self.body.bleeding());
        if self.turns.is_multiple_of(10) {
            self.body.hunger = self.body.hunger.saturating_sub(1);
        }
        if self.body.hunger == 0 {
            self.body.blood = self.body.blood.saturating_sub(3);
        }
        self.body.stamina = (self.body.stamina + 2).min(100);
        if rest {
            self.body.stamina = (self.body.stamina + 18).min(100);
            if self.body.hunger > 0 {
                self.body.blood = (self.body.blood + 12).min(1000);
                for wound in &mut self.body.wounds {
                    wound.severity = wound.severity.saturating_sub(1);
                }
                if self.turns.is_multiple_of(3) {
                    self.body.hunger = self.body.hunger.saturating_sub(1);
                }
                self.say("You rest, breathing slowly. Blood and bruises recover.");
            } else {
                self.say("Without food, rest cannot mend your wounds.");
            }
        }
    }

    fn enemies_turn(&mut self) {
        for index in 0..self.floor().enemies.len() {
            let enemy = &self.floor().enemies[index];
            let position = enemy.position;
            let kind = enemy.kind;
            let distance = position.distance(self.position);
            if distance <= 1 && self.floor().step_allowed(position, self.position) {
                // Wardens telegraph their weight by attacking only every other turn.
                if kind == EnemyKind::Warden && self.turns.is_multiple_of(2) {
                    continue;
                }
                if self.rng.below(100) < 18 {
                    self.say(format!("The {} misses you.", kind.name()));
                    continue;
                }
                let part = self.rng.below(6) as usize;
                let raw = 4
                    + self.depth as u16
                    + self.rng.below(5) as u16
                    + if kind == EnemyKind::Warden { 4 } else { 0 };
                let damage = raw.saturating_sub(self.armor * 2).max(1);
                let wound = &mut self.body.wounds[part];
                wound.severity = (wound.severity + damage).min(100);
                if damage >= 4 && self.rng.below(3) == 0 {
                    wound.bleeding = (wound.bleeding + 1 + damage / 6).min(8);
                }
                self.body.blood = self.body.blood.saturating_sub(damage * 3);
                self.body.stamina = self.body.stamina.saturating_sub(self.body.pain() / 8);
                self.say(format!(
                    "The {} wounds your {}{}.",
                    kind.name(),
                    BODY_PARTS[part].to_lowercase(),
                    if self.body.wounds[part].bleeding > 0 {
                        " (bleeding!)"
                    } else {
                        ""
                    }
                ));
                self.check_death(&format!(
                    "to the {} on floor {}",
                    kind.name(),
                    self.depth + 1
                ));
                if self.is_finished() {
                    break;
                }
            } else if distance <= 8 && self.floor().sight(position, self.position) {
                let dx = (self.position.x - position.x).signum();
                let dy = (self.position.y - position.y).signum();
                for next in [
                    position.offset(dx, dy),
                    position.offset(dx, 0),
                    position.offset(0, dy),
                ] {
                    if next != position
                        && next != self.position
                        && self.floor().tile(next) == Tile::Floor
                        && self.floor().step_allowed(position, next)
                        && !self.floor().enemies.iter().any(|e| e.position == next)
                    {
                        self.floors[self.depth].enemies[index].position = next;
                        break;
                    }
                }
            }
        }
    }

    fn check_death(&mut self, cause: &str) {
        if self.body.blood == 0
            || self.body.wounds[0].severity >= 100
            || self.body.wounds[1].severity >= 100
        {
            self.outcome = Outcome::Dead(cause.into());
            self.say("Your lantern falls. The waiting below is over.");
        }
    }

    fn reveal(&mut self) {
        let position = self.position;
        let floor = &mut self.floors[self.depth];
        floor.visible.fill(false);
        for y in (position.y - 8).max(0)..=(position.y + 8).min(HEIGHT - 1) {
            for x in (position.x - 8).max(0)..=(position.x + 8).min(WIDTH - 1) {
                let point = Point { x, y };
                if (x - position.x).pow(2) + (y - position.y).pow(2) <= 64
                    && floor.sight(position, point)
                {
                    let index = (y * WIDTH + x) as usize;
                    floor.visible[index] = true;
                    floor.explored[index] = true;
                }
            }
        }
    }
}

fn generate(rng: &mut Rng, depth: usize) -> Floor {
    let mut floor = Floor {
        tiles: vec![Tile::Wall; CELLS],
        explored: vec![false; CELLS],
        visible: vec![false; CELLS],
        enemies: Vec::new(),
        loot: Vec::new(),
        entrance: Point { x: 0, y: 0 },
        exit: Point { x: 0, y: 0 },
    };
    let mut centers = Vec::new();
    // Six disjoint room slots in a snake; connecting consecutive centers
    // constructively guarantees one connected walkable component.
    for (col, row) in [(0, 0), (1, 0), (2, 0), (2, 1), (1, 1), (0, 1)] {
        let x = 2 + col * 16 + rng.below(3) as i32;
        let y = 2 + row * 11 + rng.below(2) as i32;
        let width = 7 + rng.below(5) as i32;
        let height = 5 + rng.below(3) as i32;
        for yy in y..y + height {
            for xx in x..x + width {
                floor.tiles[(yy * WIDTH + xx) as usize] = Tile::Floor;
            }
        }
        let center = Point {
            x: x + width / 2,
            y: y + height / 2,
        };
        if let Some(&previous) = centers.last() {
            let mut point: Point = previous;
            let horizontal_first = rng.below(2) == 0;
            while point != center {
                if point.x != center.x && (horizontal_first || point.y == center.y) {
                    point.x += (center.x - point.x).signum();
                } else {
                    point.y += (center.y - point.y).signum();
                }
                floor.tiles[(point.y * WIDTH + point.x) as usize] = Tile::Floor;
            }
        }
        centers.push(center);
    }
    floor.entrance = centers[0];
    floor.exit = centers[5];
    floor.tiles[(floor.entrance.y * WIDTH + floor.entrance.x) as usize] = Tile::Up;
    floor.tiles[(floor.exit.y * WIDTH + floor.exit.x) as usize] = if depth == FLOOR_COUNT - 1 {
        Tile::Floor
    } else {
        Tile::Down
    };
    if depth == FLOOR_COUNT - 1 {
        floor.loot.push(Loot {
            position: floor.exit,
            kind: LootKind::Relic,
        });
    }
    let mut candidates: Vec<_> = floor
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(i, tile)| {
            let point = Point {
                x: i as i32 % WIDTH,
                y: i as i32 / WIDTH,
            };
            (*tile == Tile::Floor && point != floor.exit && point.distance(floor.entrance) > 3)
                .then_some(point)
        })
        .collect();
    // Sampling without replacement ensures every item and creature can be
    // reached and no spawn overlaps either a stair or another spawn.
    for i in (1..candidates.len()).rev() {
        let j = rng.below((i + 1) as u64) as usize;
        candidates.swap(i, j);
    }
    for kind in [
        LootKind::Weapon(depth as u16 + 1),
        LootKind::Armor(depth as u16 + 1),
        LootKind::Bandage,
        LootKind::Bandage,
        LootKind::Food,
        LootKind::Food,
        LootKind::Gold,
        LootKind::Gold,
    ] {
        if let Some(position) = candidates.pop() {
            floor.loot.push(Loot { position, kind });
        }
    }
    for index in 0..4 + depth {
        if let Some(position) = candidates.pop() {
            let kind = if depth >= 2 && index % 3 == 0 {
                EnemyKind::Warden
            } else if index % 2 == 0 {
                EnemyKind::Rat
            } else {
                EnemyKind::Hollow
            };
            let health = match kind {
                EnemyKind::Rat => 10,
                EnemyKind::Hollow => 17,
                EnemyKind::Warden => 30,
            } + depth as u16 * 3;
            floor.enemies.push(Enemy {
                position,
                kind,
                health,
            });
        }
    }
    floor
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use proptest::prelude::*;
    use std::collections::VecDeque;

    fn action(code: u8) -> Action {
        match code % 13 {
            0 => Action::Move(-1, -1),
            1 => Action::Move(0, -1),
            2 => Action::Move(1, -1),
            3 => Action::Move(-1, 0),
            4 => Action::Move(1, 0),
            5 => Action::Move(-1, 1),
            6 => Action::Move(0, 1),
            7 => Action::Move(1, 1),
            8 => Action::Wait,
            9 => Action::Bandage,
            10 => Action::Eat,
            11 => Action::Rest,
            _ => Action::Stairs,
        }
    }

    fn invariants(run: &Run) {
        run.validate().unwrap();
        assert!(run.depth < FLOOR_COUNT);
        assert_ne!(run.tile(run.position), Tile::Wall);
        assert!(run.body.blood <= 1000 && run.body.stamina <= 100 && run.body.hunger <= 100);
        assert!(
            run.body
                .wounds
                .iter()
                .all(|w| w.severity <= 100 && w.bleeding <= 8)
        );
        assert!(run.log.len() <= 80);
        for floor in &run.floors {
            assert_eq!(floor.tiles.len(), CELLS);
            assert!(
                floor
                    .enemies
                    .iter()
                    .all(|e| e.health > 0 && floor.tile(e.position) != Tile::Wall)
            );
            for (i, enemy) in floor.enemies.iter().enumerate() {
                assert!(
                    !floor.enemies[..i]
                        .iter()
                        .any(|e| e.position == enemy.position)
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]

        #[test]
        fn every_generated_cell_and_goal_is_reachable(seed in any::<u64>()) {
            let run = Run::new(seed);
            invariants(&run);
            for floor in &run.floors {
                let mut reached = vec![false; CELLS];
                let mut queue = VecDeque::from([floor.entrance]);
                reached[floor.entrance.index().unwrap()] = true;
                while let Some(p) = queue.pop_front() {
                    for (dx,dy) in [(1,0),(-1,0),(0,1),(0,-1)] {
                        let next = p.offset(dx,dy);
                        if floor.tile(next) != Tile::Wall {
                            let i = next.index().unwrap();
                            if !reached[i] { reached[i] = true; queue.push_back(next); }
                        }
                    }
                }
                for (i, tile) in floor.tiles.iter().enumerate() { prop_assert_eq!(*tile != Tile::Wall, reached[i]); }
                prop_assert!(reached[floor.exit.index().unwrap()]);
                prop_assert!(floor.loot.iter().all(|loot| reached[loot.position.index().unwrap()]));
            }
        }

        #[test]
        fn saving_at_any_turn_preserves_the_exact_future(seed in any::<u64>(), codes in prop::collection::vec(any::<u8>(), 0..350), split in any::<usize>()) {
            let mut uninterrupted = Run::new(seed);
            let split = split % (codes.len() + 1);
            for &code in &codes[..split] { uninterrupted.act(action(code)); }
            let mut restored: Run = serde_json::from_slice(&serde_json::to_vec(&uninterrupted).unwrap()).unwrap();
            for &code in &codes[split..] {
                prop_assert_eq!(uninterrupted.act(action(code)), restored.act(action(code)));
                prop_assert_eq!(&uninterrupted, &restored);
                invariants(&restored);
            }
        }

        #[test]
        fn finished_runs_are_immutable(seed in any::<u64>(), codes in prop::collection::vec(any::<u8>(), 0..100)) {
            for outcome in [Outcome::Escaped, Outcome::Dead("from blood loss".into())] {
                let mut run = Run::new(seed);
                run.outcome = outcome;
                let before = run.clone();
                for &code in &codes { prop_assert!(!run.act(action(code))); }
                prop_assert_eq!(run, before);
            }
        }

        #[test]
        fn corrupt_coordinates_are_rejected_without_panicking(x in any::<i32>(), y in any::<i32>()) {
            let point = Point { x, y };
            prop_assume!(point.index().is_none());
            let original = Run::new(42);
            let mut run = original.clone();
            run.position = point;
            prop_assert!(run.validate().is_err());
            let mut run = original.clone();
            run.floors[0].entrance = point;
            prop_assert!(run.validate().is_err());
            let mut run = original.clone();
            run.floors[0].enemies[0].position = point;
            prop_assert!(run.validate().is_err());
            let mut run = original;
            run.floors[0].loot[0].position = point;
            prop_assert!(run.validate().is_err());
        }
    }

    #[test]
    fn bleeding_rest_and_bandaging_obey_turn_rules() {
        let mut run = Run::new(42);
        run.floors[0].enemies.clear();
        run.body.wounds[2] = Wound {
            severity: 30,
            bleeding: 4,
        };
        let turns = run.turns;
        run.act(Action::Rest);
        assert_eq!(run.turns, turns);
        run.act(Action::Wait);
        assert_eq!(run.body.blood, 996);
        run.act(Action::Bandage);
        assert_eq!(run.body.bleeding(), 0);
        assert_eq!(run.bandages, 3);
        run.act(Action::Rest);
        assert_eq!(run.body.blood, 1000);
        assert!(run.body.wounds[2].severity < 30);
    }

    #[test]
    fn relic_requires_returning_to_daylight_and_stairs_preserve_floors() {
        let mut run = Run::new(7);
        for floor in &mut run.floors {
            floor.enemies.clear();
        }
        assert!(!run.act(Action::Stairs) || run.turns == 0);
        for depth in 0..FLOOR_COUNT - 1 {
            run.position = run.floor().exit;
            run.act(Action::Stairs);
            assert_eq!(run.depth, depth + 1);
        }
        run.position = run.floor().exit;
        run.collect();
        assert!(run.relic);
        assert!(!run.is_finished());
        for depth in (0..FLOOR_COUNT - 1).rev() {
            run.position = run.floor().entrance;
            run.act(Action::Stairs);
            assert_eq!(run.depth, depth);
        }
        run.position = run.floor().entrance;
        run.act(Action::Stairs);
        assert_eq!(run.outcome, Outcome::Escaped);
        assert_eq!(run.deepest, 5);
        assert!(run.summary().contains("escaped with the ember"));
    }

    /// A cautious explorer: all decisions use remembered terrain and currently
    /// visible occupants. It cannot inspect hidden cells, teleport, heal itself,
    /// or change the simulation except through the same actions as the modal.
    fn expedition_action(run: &Run) -> Action {
        let floor = run.floor();
        let visible_enemies: Vec<_> = floor
            .enemies
            .iter()
            .filter(|enemy| floor.visible[enemy.position.index().unwrap()])
            .collect();
        if let Some(enemy) = visible_enemies.iter().find(|enemy| {
            enemy.position.distance(run.position) == 1
                && floor.step_allowed(run.position, enemy.position)
        }) {
            return Action::Move(
                enemy.position.x - run.position.x,
                enemy.position.y - run.position.y,
            );
        }
        if run.body.bleeding() > 0 && run.bandages > 0 {
            return Action::Bandage;
        }
        if run.body.hunger <= 50 && run.food > 0 {
            return Action::Eat;
        }
        if visible_enemies.is_empty()
            && run.body.bleeding() == 0
            && run.body.hunger > 0
            && (run.body.blood < 990
                || run.body.stamina < 85
                || run.body.wounds.iter().any(|w| w.severity > 8))
        {
            return Action::Rest;
        }

        // Breadth-first search only the remembered walkable component.
        let mut routes = vec![None; CELLS];
        let mut queue = VecDeque::from([(run.position, 0_usize, (0, 0))]);
        routes[run.position.index().unwrap()] = Some((0_usize, (0, 0)));
        while let Some((point, distance, first)) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                let next = point.offset(dx, dy);
                let Some(index) = next.index() else {
                    continue;
                };
                if floor.explored[index]
                    && floor.tile(next) != Tile::Wall
                    && routes[index].is_none()
                {
                    let first = if distance == 0 { (dx, dy) } else { first };
                    routes[index] = Some((distance + 1, first));
                    queue.push_back((next, distance + 1, first));
                }
            }
        }
        let nearest = |targets: Vec<Point>| {
            targets
                .into_iter()
                .filter_map(|point| {
                    routes[point.index().unwrap()].filter(|(distance, _)| *distance > 0)
                })
                .min_by_key(|(distance, _)| *distance)
                .map(|(_, (dx, dy))| Action::Move(dx, dy))
        };
        if let Some(action) = nearest(visible_enemies.iter().map(|e| e.position).collect()) {
            return action;
        }
        if !run.relic {
            if let Some(action) = nearest(
                floor
                    .loot
                    .iter()
                    .filter(|l| floor.visible[l.position.index().unwrap()])
                    .map(|l| l.position)
                    .collect(),
            ) {
                return action;
            }
            let frontier = floor
                .explored
                .iter()
                .enumerate()
                .filter_map(|(i, explored)| {
                    let point = Point {
                        x: i as i32 % WIDTH,
                        y: i as i32 / WIDTH,
                    };
                    (*explored
                        && floor.tile(point) != Tile::Wall
                        && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
                            point
                                .offset(dx, dy)
                                .index()
                                .is_some_and(|j| !floor.explored[j])
                        }))
                    .then_some(point)
                })
                .collect();
            if let Some(action) = nearest(frontier) {
                return action;
            }
        }
        // Stair locations are consulted only after their terrain is remembered.
        let stair = if run.relic {
            floor.entrance
        } else {
            floor.exit
        };
        if run.position == stair {
            Action::Stairs
        } else {
            nearest(vec![stair]).expect("Exploration must uncover a route to the goal")
        }
    }

    #[test]
    fn cautious_explorer_can_retrieve_the_ember_and_return_through_real_actions() {
        for seed in [42, 0, 1, 2026, u64::MAX] {
            let mut run = Run::new(seed);
            for _ in 0..10_000 {
                if run.is_finished() {
                    break;
                }
                let action = expedition_action(&run);
                let before = run.turns;
                assert!(run.act(action), "seed {seed}: no change for {action:?}");
                assert!(
                    run.turns > before,
                    "seed {seed}: rejected {action:?}: {:?}",
                    run.log.last()
                );
                invariants(&run);
            }
            println!("seed {seed}: {}", run.summary());
            assert_eq!(
                run.outcome,
                Outcome::Escaped,
                "seed {seed}: {}",
                run.summary()
            );
        }
    }
}
