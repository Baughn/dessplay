//! Generated floors and the ember's persistent, action-clock-driven awakening.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::anatomy::{
    ArmorMaterial, ArmorPiece, ArmorSlot, AttackProfile, Body, BodyKind, Equipment, WeaponKind,
};
use super::{CELLS, FLOOR_COUNT, HEIGHT, Point, Rng, WIDTH};

/// Hard ceiling on living creatures on a floor, including summoned creatures.
pub const MAX_ENEMIES: usize = 24;

/// Fixed terrain; memory stores this value rather than a visibility bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tile {
    /// Solid stone.
    Wall,
    /// Ordinary ground.
    Floor,
    /// Previous floor, or the surface.
    Up,
    /// Next floor.
    Down,
    /// An opaque door that can be opened with a movement action.
    DoorClosed,
    /// Open doorway.
    DoorOpen,
    /// Shallow water slows passage.
    Water,
    /// Loose stone slows passage.
    Rubble,
    /// One miraculous restoration, activated explicitly.
    Fountain,
    /// A spent fountain.
    FountainDry,
}

impl Tile {
    /// Whether a creature can enter without first opening anything.
    pub fn walkable(self) -> bool {
        !matches!(self, Self::Wall | Self::DoorClosed)
    }

    /// Symbol used for both current terrain and remembered terrain.
    pub fn glyph(self) -> char {
        match self {
            Self::Wall => '#',
            Self::Floor => '.',
            Self::Up => '<',
            Self::Down => '>',
            Self::DoorClosed => '+',
            Self::DoorOpen => '/',
            Self::Water => '~',
            Self::Rubble => ':',
            Self::Fountain => '&',
            Self::FountainDry => ';',
        }
    }
}

/// Creature behavior archetypes; their anatomy uses the same injury rules as ours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyKind {
    /// Quick scavenger that disengages after biting.
    Rat,
    /// Human remnant capable of calling others.
    Hollow,
    /// Armored guardian with a prepared heavy strike.
    Warden,
    /// Slow cavern dweller that follows sound.
    Brute,
}

impl EnemyKind {
    /// Name in observations and the journal.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rat => "ash rat",
            Self::Hollow => "hollow pilgrim",
            Self::Warden => "iron warden",
            Self::Brute => "cavern brute",
        }
    }

    /// Map glyph.
    pub fn glyph(self) -> char {
        match self {
            Self::Rat => 'r',
            Self::Hollow => 'h',
            Self::Warden => 'W',
            Self::Brute => 'B',
        }
    }
}

/// An enemy's saved commitment; UI telegraphs are derived from these deadlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyIntent {
    /// Able to choose an action.
    Idle,
    /// Preparing an attack at a fixed tile.
    Strike {
        /// Target retained even if the victim moves.
        target: Point,
        /// Local floor time at impact.
        at: u64,
    },
    /// Preparing a call that attracts other creatures.
    Calling {
        /// Local floor time at completion.
        at: u64,
    },
    /// Unable to start another attack yet.
    Recovering {
        /// Local floor time at recovery.
        until: u64,
    },
}

/// A creature with persistent anatomy and action scheduling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enemy {
    /// Floor-local stable identity, never reused by swarms.
    pub id: u64,
    /// Behavioral archetype.
    pub kind: EnemyKind,
    /// Current position.
    pub position: Point,
    /// Functional anatomy and physiology.
    pub body: Body,
    /// Weapons and region-specific protection.
    pub gear: Equipment,
    /// Earliest next action, on the local floor clock.
    pub next_action: u64,
    /// Committed windup or recovery.
    pub intent: EnemyIntent,
    /// Last perceived target location.
    pub target: Option<Point>,
    /// Crisis creatures produce no renewable rewards.
    pub summoned: bool,
}

impl Enemy {
    /// Construct a species and equipment loadout without consuming randomness.
    pub fn new(id: u64, kind: EnemyKind, position: Point, time: u64) -> Self {
        let body = Body::new(match kind {
            EnemyKind::Rat => BodyKind::Rat,
            EnemyKind::Brute => BodyKind::Brute,
            EnemyKind::Hollow | EnemyKind::Warden => BodyKind::Human,
        });
        let mut gear = Equipment {
            active: match kind {
                EnemyKind::Rat | EnemyKind::Brute => WeaponKind::Unarmed,
                EnemyKind::Hollow => WeaponKind::Knife,
                EnemyKind::Warden => WeaponKind::Mace,
            },
            spare: None,
            armor: [None; 6],
        };
        if kind == EnemyKind::Warden {
            for (index, slot) in [ArmorSlot::Head, ArmorSlot::Torso, ArmorSlot::Arms]
                .into_iter()
                .enumerate()
            {
                gear.armor[index] = Some(ArmorPiece {
                    slot,
                    material: ArmorMaterial::Iron,
                });
            }
        }
        Self {
            id,
            kind,
            position,
            body,
            gear,
            next_action: time.saturating_add(100),
            intent: EnemyIntent::Idle,
            target: None,
            summoned: false,
        }
    }
}

/// Finite floor supplies, equipment, treasure, and the expedition objective.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LootKind {
    /// Clean linen for one treatment.
    Bandage,
    /// Bone support for one fracture.
    Splint,
    /// One ration.
    Food,
    /// A pile of coins.
    Gold(u16),
    /// A weapon to equip explicitly.
    Weapon(WeaponKind),
    /// Regional protection to equip explicitly.
    Armor(ArmorPiece),
    /// The ember, taken only by explicit interaction.
    Relic,
}

impl LootKind {
    /// Map glyph.
    pub fn glyph(self) -> char {
        match self {
            Self::Bandage => '!',
            Self::Splint => '=',
            Self::Food => '%',
            Self::Gold(_) => '$',
            Self::Weapon(_) => ')',
            Self::Armor(_) => '[',
            Self::Relic => '*',
        }
    }

    /// Player-facing contents.
    pub fn name(self) -> String {
        match self {
            Self::Bandage => "clean linen".into(),
            Self::Splint => "a wooden splint".into(),
            Self::Food => "dried apples".into(),
            Self::Gold(coins) => format!("{coins} gold"),
            Self::Weapon(weapon) => weapon.name().into(),
            Self::Armor(piece) => piece.name(),
            Self::Relic => "the ember".into(),
        }
    }
}

/// One ground item; dropped gear may share a tile with other items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loot {
    /// Position in the floor.
    pub position: Point,
    /// Contents.
    pub kind: LootKind,
}

/// A cavern generated in advance and exposed by a later breach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cavern {
    /// Initially solid cells, including a tunnel into the playable floor.
    pub cells: Vec<Point>,
    /// The recognizable place where cracking stone gives warning.
    pub mouth: Point,
    /// Whether this cavern has been exposed already.
    pub opened: bool,
}

/// Alternating pressure and recoverable intervals after awakening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrisisPhase {
    /// No new hazards while the dungeon gathers itself.
    Lull,
    /// Advance warning of the next outbreak.
    Warning,
    /// Swarms and rockfalls are active.
    Surge,
}

/// A rockfall with a guaranteed advance warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collapse {
    /// Tile marked by dust and falling pebbles.
    pub position: Point,
    /// Earliest local floor time at impact.
    pub at: u64,
}

/// Complete persistent ascent cycle state; revisiting stairs never resets it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crisis {
    /// The ember has been disturbed somewhere in this expedition.
    pub awakened: bool,
    /// Current wave stage.
    pub phase: CrisisPhase,
    /// Next stage deadline on the local floor clock.
    pub next_event: u64,
    /// Completed outbreak count, capped for difficulty.
    pub cycle: u16,
    /// Announced pending rockfall.
    pub collapse: Option<Collapse>,
}

impl Default for Crisis {
    fn default() -> Self {
        Self {
            awakened: false,
            phase: CrisisPhase::Lull,
            next_event: 0,
            cycle: 0,
            collapse: None,
        }
    }
}

/// Generated world and remembered knowledge; inactive floors remain frozen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Floor {
    /// Actual row-major terrain.
    pub tiles: Vec<Tile>,
    /// Last observed terrain, never updated by hidden changes.
    pub remembered: Vec<Option<Tile>>,
    /// Current line-of-sight mask.
    pub visible: Vec<bool>,
    /// Living creatures.
    pub enemies: Vec<Enemy>,
    /// Uncollected finite supplies and equipment.
    pub loot: Vec<Loot>,
    /// Upward stair.
    pub entrance: Point,
    /// Downward stair, or ember location on the fifth floor.
    pub exit: Point,
    /// Action time spent on this floor only.
    pub time: u64,
    /// Pre-generated spaces revealed during the awakening.
    pub caverns: Vec<Cavern>,
    /// Saved warning, wave, and lull progress.
    pub crisis: Crisis,
    /// Next stable creature identity.
    pub next_enemy_id: u64,
}

impl Floor {
    /// Terrain at a position; off-map is solid rock.
    pub fn tile(&self, point: Point) -> Tile {
        point
            .index()
            .and_then(|i| self.tiles.get(i))
            .copied()
            .unwrap_or(Tile::Wall)
    }

    /// Traversable by walking or opening a door, useful for path planning.
    pub fn passable(&self, point: Point) -> bool {
        self.tile(point) != Tile::Wall
    }

    /// A diagonal may pass one blocked flank, but cannot squeeze between two.
    pub fn clear_corner(&self, from: Point, to: Point) -> bool {
        from.x == to.x
            || from.y == to.y
            || self.tile(Point { x: from.x, y: to.y }).walkable()
            || self.tile(Point { x: to.x, y: from.y }).walkable()
    }

    /// One-tile movement respecting destination terrain and diagonal clearance.
    pub fn step_allowed(&self, from: Point, to: Point) -> bool {
        from != to
            && from.distance(to) == 1
            && self.tile(to).walkable()
            && self.clear_corner(from, to)
    }

    /// Opaque doors and stone stop sight; sight does not leak through corners.
    pub fn sight(&self, from: Point, to: Point) -> bool {
        if from.index().is_none() || to.index().is_none() {
            return false;
        }
        let mut p = from;
        let dx = (to.x - from.x).abs();
        let dy = -(to.y - from.y).abs();
        let sx = (to.x - from.x).signum();
        let sy = (to.y - from.y).signum();
        let mut error = dx + dy;
        while p != to {
            let previous = p;
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                p.x += sx;
            }
            if twice <= dx {
                error += dx;
                p.y += sy;
            }
            if !self.clear_corner(previous, p) {
                return false;
            }
            if p == to {
                return true;
            }
            if !self.tile(p).walkable() {
                return false;
            }
        }
        true
    }

    /// Whether a warned environmental impact threatens this tile.
    pub fn threatened(&self, point: Point) -> bool {
        self.crisis.collapse.is_some_and(|c| c.position == point)
    }

    /// Validate save dimensions, positions, identities, connectivity, and anatomy.
    pub fn validate(&self) -> Result<(), String> {
        if self.tiles.len() != CELLS
            || self.remembered.len() != CELLS
            || self.visible.len() != CELLS
        {
            return Err("invalid floor dimensions".into());
        }
        if self.tile(self.entrance) != Tile::Up || !self.tile(self.exit).walkable() {
            return Err("invalid floor stairs".into());
        }
        if self.enemies.len() > MAX_ENEMIES || self.loot.len() > 256 || self.caverns.len() > 4 {
            return Err("floor entity bounds exceeded".into());
        }
        let reached = self.reachable();
        if self
            .tiles
            .iter()
            .enumerate()
            .any(|(i, t)| *t != Tile::Wall && !reached[i])
        {
            return Err("floor has inaccessible ground".into());
        }
        for (i, enemy) in self.enemies.iter().enumerate() {
            if !self.tile(enemy.position).walkable()
                || enemy.id >= self.next_enemy_id
                || enemy.target.is_some_and(|p| p.index().is_none())
                || self.enemies[..i]
                    .iter()
                    .any(|other| other.id == enemy.id || other.position == enemy.position)
            {
                return Err("invalid creature position or identity".into());
            }
            enemy.body.validate()?;
            enemy.gear.validate()?;
            if let EnemyIntent::Strike { target, .. } = enemy.intent
                && target.index().is_none()
            {
                return Err("invalid attack target".into());
            }
        }
        if self
            .loot
            .iter()
            .any(|item| !self.tile(item.position).walkable())
        {
            return Err("item embedded in stone or a closed door".into());
        }
        for cavern in &self.caverns {
            if cavern.cells.len() > CELLS
                || cavern.mouth.index().is_none()
                || cavern.cells.iter().any(|p| p.index().is_none())
            {
                return Err("invalid dormant cavern".into());
            }
        }
        if self
            .crisis
            .collapse
            .is_some_and(|c| c.position.index().is_none())
        {
            return Err("invalid rockfall position".into());
        }
        Ok(())
    }

    fn set(&mut self, point: Point, tile: Tile) {
        if let Some(index) = point.index() {
            self.tiles[index] = tile;
        }
    }

    fn reachable(&self) -> Vec<bool> {
        let mut reached = vec![false; CELLS];
        let Some(start) = self.entrance.index() else {
            return reached;
        };
        reached[start] = true;
        let mut pending = VecDeque::from([self.entrance]);
        while let Some(p) = pending.pop_front() {
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let n = p.offset(dx, dy);
                if let Some(i) = n.index()
                    && !reached[i]
                    && self.passable(n)
                {
                    reached[i] = true;
                    pending.push_back(n);
                }
            }
        }
        reached
    }
}

#[derive(Clone, Copy)]
struct Room {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Room {
    fn center(self) -> Point {
        Point {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }
    fn overlaps(self, other: Self) -> bool {
        self.x <= other.x + other.w
            && other.x <= self.x + self.w
            && self.y <= other.y + other.h
            && other.y <= self.y + self.h
    }
}

/// Generate a connected room graph with extra loops and optional reward branches.
pub fn generate(rng: &mut Rng, depth: usize, fountain: bool) -> Floor {
    let mut floor = Floor {
        tiles: vec![Tile::Wall; CELLS],
        remembered: vec![None; CELLS],
        visible: vec![false; CELLS],
        enemies: Vec::new(),
        loot: Vec::new(),
        entrance: Point { x: 2, y: 2 },
        exit: Point {
            x: WIDTH - 3,
            y: HEIGHT - 3,
        },
        time: 0,
        caverns: Vec::new(),
        crisis: Crisis::default(),
        next_enemy_id: 1,
    };
    let mut rooms = Vec::new();
    let desired = 8 + rng.below(4) as usize;
    for _ in 0..250 {
        let w = 4 + rng.below(6) as i32;
        let h = 3 + rng.below(4) as i32;
        let room = Room {
            x: 1 + rng.below((WIDTH - w - 2) as u64) as i32,
            y: 1 + rng.below((HEIGHT - h - 2) as u64) as i32,
            w,
            h,
        };
        if rooms.iter().copied().any(|other| room.overlaps(other)) {
            continue;
        }
        for y in room.y..room.y + room.h {
            for x in room.x..room.x + room.w {
                floor.set(Point { x, y }, Tile::Floor);
            }
        }
        rooms.push(room);
        if rooms.len() == desired {
            break;
        }
    }
    // Placement always has room for at least two rooms; this fallback also makes
    // the generator total if dimensions or room sizes are changed in the future.
    if rooms.len() < 2 {
        floor.tiles.fill(Tile::Wall);
        rooms = vec![
            Room {
                x: 2,
                y: 2,
                w: 5,
                h: 4,
            },
            Room {
                x: WIDTH - 8,
                y: HEIGHT - 7,
                w: 5,
                h: 4,
            },
        ];
        for room in &rooms {
            for y in room.y..room.y + room.h {
                for x in room.x..room.x + room.w {
                    floor.set(Point { x, y }, Tile::Floor);
                }
            }
        }
    }
    let mut connected = vec![false; rooms.len()];
    connected[0] = true;
    let mut edges = Vec::new();
    while connected.contains(&false) {
        let mut best = (i32::MAX, 0, 0);
        for a in 0..rooms.len() {
            if !connected[a] {
                continue;
            }
            for b in 0..rooms.len() {
                if connected[b] {
                    continue;
                }
                let p = rooms[a].center();
                let q = rooms[b].center();
                let distance = (p.x - q.x).abs() + (p.y - q.y).abs();
                if distance < best.0 {
                    best = (distance, a, b);
                }
            }
        }
        connected[best.2] = true;
        edges.push((best.1, best.2));
    }
    let mut extra = Vec::new();
    for a in 0..rooms.len() {
        for b in a + 1..rooms.len() {
            if !edges.contains(&(a, b)) && !edges.contains(&(b, a)) {
                extra.push((a, b));
            }
        }
    }
    for _ in 0..4.min(extra.len()) {
        let i = rng.below(extra.len() as u64) as usize;
        edges.push(extra.swap_remove(i));
    }
    for (a, b) in edges {
        carve_tunnel(
            &mut floor,
            rooms[a].center(),
            rooms[b].center(),
            rng.below(2) == 0,
        );
    }
    floor.entrance = rooms[0].center();
    floor.exit = rooms
        .iter()
        .map(|r| r.center())
        .max_by_key(|p| p.distance(floor.entrance))
        .unwrap_or(rooms[1].center());
    // Doors occur in narrow passages and never replace stairs or item cells.
    let mut door_candidates = Vec::new();
    for y in 2..HEIGHT - 2 {
        for x in 2..WIDTH - 2 {
            let p = Point { x, y };
            if floor.tile(p) != Tile::Floor || p == floor.entrance || p == floor.exit {
                continue;
            }
            let vertical = floor.passable(p.offset(0, -1))
                && floor.passable(p.offset(0, 1))
                && !floor.passable(p.offset(-1, 0))
                && !floor.passable(p.offset(1, 0));
            let horizontal = floor.passable(p.offset(-1, 0))
                && floor.passable(p.offset(1, 0))
                && !floor.passable(p.offset(0, -1))
                && !floor.passable(p.offset(0, 1));
            if vertical || horizontal {
                door_candidates.push(p);
            }
        }
    }
    for _ in 0..(3 + depth).min(door_candidates.len()) {
        let i = rng.below(door_candidates.len() as u64) as usize;
        let p = door_candidates.swap_remove(i);
        if ![-1, 1].iter().any(|d| {
            floor.tile(p.offset(*d, 0)) == Tile::DoorClosed
                || floor.tile(p.offset(0, *d)) == Tile::DoorClosed
        }) {
            floor.set(p, Tile::DoorClosed);
        }
    }
    if depth == 1 || rng.below(3) == 0 {
        let room = rooms[1 + rng.below((rooms.len() - 1) as u64) as usize];
        for y in room.y..room.y + room.h {
            for x in room.x..room.x + room.w {
                if rng.below(3) != 0 {
                    floor.set(Point { x, y }, Tile::Water);
                }
            }
        }
    }
    floor.set(floor.entrance, Tile::Up);
    floor.set(
        floor.exit,
        if depth + 1 == FLOOR_COUNT {
            Tile::Floor
        } else {
            Tile::Down
        },
    );
    if fountain {
        let room = rooms[1 + rng.below((rooms.len() - 1) as u64) as usize];
        let p = room.center().offset(-1, 0);
        if p != floor.entrance && p != floor.exit {
            floor.set(p, Tile::Fountain);
        }
    }
    for _ in 0..3 {
        add_cavern(&mut floor, rng);
    }
    let mut candidates: Vec<Point> = (0..CELLS)
        .map(point)
        .filter(|p| {
            matches!(floor.tile(*p), Tile::Floor | Tile::Water)
                && *p != floor.exit
                && p.distance(floor.entrance) > 3
        })
        .collect();
    let supply = [
        LootKind::Bandage,
        LootKind::Bandage,
        LootKind::Splint,
        LootKind::Food,
        LootKind::Gold(8 + rng.below(18) as u16),
        LootKind::Weapon(if depth.is_multiple_of(2) {
            WeaponKind::Spear
        } else {
            WeaponKind::Mace
        }),
        LootKind::Armor(ArmorPiece {
            slot: [
                ArmorSlot::Torso,
                ArmorSlot::Head,
                ArmorSlot::Legs,
                ArmorSlot::Arms,
                ArmorSlot::Feet,
            ][depth.min(4)],
            material: if depth < 2 {
                ArmorMaterial::Leather
            } else {
                ArmorMaterial::Iron
            },
        }),
    ];
    for kind in supply {
        if let Some(p) = choose_remove(&mut candidates, rng) {
            floor.loot.push(Loot { position: p, kind });
        }
    }
    // Reserve an observable route between the stairs that does not pass the
    // optional treasury guard. A player may still deliberately approach it.
    let stair_route = route_between_stairs(&floor);
    if rng.below(2) == 0 {
        let mut treasury_rooms: Vec<Point> = rooms
            .iter()
            .map(|room| room.center())
            .filter(|p| {
                let guard = p.offset(1, 0);
                floor.tile(guard).walkable()
                    && guard.distance(floor.entrance) >= 10
                    && stair_route.iter().all(|route| guard.distance(*route) >= 8)
            })
            .collect();
        if let Some(p) = choose_remove(&mut treasury_rooms, rng) {
            floor.loot.push(Loot {
                position: p,
                kind: LootKind::Gold(25 + rng.below(36) as u16),
            });
            let guard = p.offset(1, 0);
            spawn(
                &mut floor,
                match depth {
                    0 => EnemyKind::Rat,
                    1 => EnemyKind::Hollow,
                    _ => EnemyKind::Warden,
                },
                guard,
                false,
            );
            candidates.retain(|p| *p != guard);
        }
    }
    let encounters: &[EnemyKind] = match depth {
        0 => &[EnemyKind::Rat, EnemyKind::Rat],
        1 => &[EnemyKind::Rat, EnemyKind::Rat],
        2 => &[EnemyKind::Rat, EnemyKind::Rat, EnemyKind::Hollow],
        3 => &[EnemyKind::Rat, EnemyKind::Rat, EnemyKind::Hollow],
        _ => &[
            EnemyKind::Rat,
            EnemyKind::Rat,
            EnemyKind::Hollow,
            EnemyKind::Warden,
        ],
    };
    for &kind in encounters {
        let spacing = if depth < 2 { 8 } else { 6 };
        let mut positions: Vec<Point> = candidates
            .iter()
            .copied()
            .filter(|p| {
                p.distance(floor.entrance) >= if depth < 2 { 10 } else { 8 }
                    && floor
                        .enemies
                        .iter()
                        .all(|enemy| p.distance(enemy.position) >= spacing)
            })
            .collect();
        if let Some(p) = choose_remove(&mut positions, rng) {
            spawn(&mut floor, kind, p, false);
            candidates.retain(|candidate| *candidate != p);
        }
    }
    if depth + 1 == FLOOR_COUNT {
        floor.loot.push(Loot {
            position: floor.exit,
            kind: LootKind::Relic,
        });
    }
    floor
}

fn route_between_stairs(floor: &Floor) -> Vec<Point> {
    let mut previous = vec![None; CELLS];
    let mut queue = VecDeque::from([floor.entrance]);
    if let Some(i) = floor.entrance.index() {
        previous[i] = Some(floor.entrance);
    }
    while let Some(p) = queue.pop_front() {
        if p == floor.exit {
            break;
        }
        for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let next = p.offset(dx, dy);
            if let Some(i) = next.index()
                && floor.passable(next)
                && previous[i].is_none()
            {
                previous[i] = Some(p);
                queue.push_back(next);
            }
        }
    }
    let mut route = vec![floor.exit];
    let mut p = floor.exit;
    while p != floor.entrance {
        let Some(next) = p.index().and_then(|i| previous[i]) else {
            break;
        };
        route.push(next);
        p = next;
    }
    route
}

fn point(index: usize) -> Point {
    Point {
        x: index as i32 % WIDTH,
        y: index as i32 / WIDTH,
    }
}

fn choose_remove(points: &mut Vec<Point>, rng: &mut Rng) -> Option<Point> {
    if points.is_empty() {
        None
    } else {
        Some(points.swap_remove(rng.below(points.len() as u64) as usize))
    }
}

fn tunnel_points(mut p: Point, to: Point, horizontal_first: bool) -> Vec<Point> {
    let mut result = vec![p];
    for horizontal in [horizontal_first, !horizontal_first] {
        while if horizontal { p.x != to.x } else { p.y != to.y } {
            if horizontal {
                p.x += (to.x - p.x).signum();
            } else {
                p.y += (to.y - p.y).signum();
            }
            result.push(p);
        }
    }
    result
}

fn carve_tunnel(floor: &mut Floor, from: Point, to: Point, horizontal_first: bool) {
    for p in tunnel_points(from, to, horizontal_first) {
        floor.set(p, Tile::Floor);
    }
}

fn add_cavern(floor: &mut Floor, rng: &mut Rng) {
    let mut candidates = Vec::new();
    for y in 3..HEIGHT - 3 {
        for x in 3..WIDTH - 3 {
            let center = Point { x, y };
            if (-1..=1).all(|dy| {
                (-2..=2).all(|dx| {
                    let p = center.offset(dx, dy);
                    floor.tile(p) == Tile::Wall
                        && !floor.caverns.iter().any(|c| c.cells.contains(&p))
                })
            }) {
                candidates.push(center);
            }
        }
    }
    let Some(center) = choose_remove(&mut candidates, rng) else {
        return;
    };
    let mouth = (0..CELLS)
        .map(point)
        .filter(|p| floor.passable(*p))
        .min_by_key(|p| (p.x - center.x).abs() + (p.y - center.y).abs())
        .unwrap_or(floor.entrance);
    let mut cells = Vec::new();
    for dy in -1..=1 {
        for dx in -2..=2 {
            if (dx != -2 && dx != 2) || dy == 0 || rng.below(2) == 0 {
                cells.push(center.offset(dx, dy));
            }
        }
    }
    for p in tunnel_points(center, mouth, rng.below(2) == 0) {
        if floor.tile(p) == Tile::Wall && !cells.contains(&p) {
            cells.push(p);
        }
    }
    floor.caverns.push(Cavern {
        cells,
        mouth,
        opened: false,
    });
}

fn spawn(floor: &mut Floor, kind: EnemyKind, position: Point, summoned: bool) {
    if floor.enemies.len() >= MAX_ENEMIES
        || !floor.tile(position).walkable()
        || floor.enemies.iter().any(|e| e.position == position)
    {
        return;
    }
    let mut enemy = Enemy::new(floor.next_enemy_id, kind, position, floor.time);
    floor.next_enemy_id = floor.next_enemy_id.saturating_add(1);
    enemy.summoned = summoned;
    floor.enemies.push(enemy);
}

/// A simulation event filtered through player perception by the facade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldEvent {
    /// Sound/impact origin, or a general atmospheric event.
    pub position: Option<Point>,
    /// Player-facing description.
    pub message: String,
    /// Interrupt automatic care if perceived.
    pub danger: bool,
    /// The player was standing beneath a previously warned rockfall.
    pub impact: bool,
}

/// Permanently initialize this floor's crisis; repeated calls are idempotent.
pub fn awaken(floor: &mut Floor) {
    if !floor.crisis.awakened {
        floor.crisis.awakened = true;
        floor.crisis.phase = CrisisPhase::Lull;
        // The first action after pickup/arrival announces the awakening. Later
        // cycles provide full lulls, but no floor grants an initial free walk.
        floor.crisis.next_event = floor.time;
    }
}

/// Advance hazards after the caller advances this floor's action clock.
pub fn advance(floor: &mut Floor, rng: &mut Rng, player: Point) -> Vec<WorldEvent> {
    let mut events = Vec::new();
    if !floor.crisis.awakened {
        return events;
    }
    if let Some(collapse) = floor.crisis.collapse
        && floor.time >= collapse.at
    {
        floor.crisis.collapse = None;
        let p = collapse.position;
        let occupied = p == player
            || floor.enemies.iter().any(|e| e.position == p)
            || floor.loot.iter().any(|item| item.position == p);
        let original = floor.tile(p);
        // Only ordinary ground can be lost. Occupants are never embedded, and
        // closing one optional passage may not disconnect any other active cell.
        let anchors_dormant_cavern = floor
            .caverns
            .iter()
            .any(|cavern| !cavern.opened && cavern.mouth == p);
        let may_close = !occupied
            && !anchors_dormant_cavern
            && p != floor.entrance
            && p != floor.exit
            && matches!(original, Tile::Floor | Tile::Rubble | Tile::DoorOpen);
        if may_close {
            floor.set(p, Tile::Wall);
            let reached = floor.reachable();
            if floor
                .tiles
                .iter()
                .enumerate()
                .any(|(i, t)| *t != Tile::Wall && !reached[i])
            {
                floor.set(p, Tile::Rubble);
            }
        } else if p != floor.entrance
            && p != floor.exit
            && matches!(
                original,
                Tile::Floor | Tile::Rubble | Tile::DoorOpen | Tile::Water
            )
        {
            floor.set(p, Tile::Rubble);
        }
        events.push(WorldEvent {
            position: Some(p),
            message: if floor.tile(p) == Tile::Wall {
                "The ceiling crashes down, sealing a passage."
            } else {
                "Stone crashes down, leaving a slope of loose rubble."
            }
            .into(),
            danger: true,
            impact: p == player,
        });
        for enemy in floor.enemies.iter_mut().filter(|enemy| enemy.position == p) {
            let injury = enemy.body.hit(
                AttackProfile {
                    weapon: WeaponKind::Mace,
                    power: 35,
                },
                &enemy.gear,
                rng,
            );
            events.push(WorldEvent {
                position: Some(p),
                message: format!(
                    "Falling stone strikes the {}: {}",
                    enemy.kind.name(),
                    injury.message
                ),
                danger: true,
                impact: false,
            });
        }
    }
    if floor.time < floor.crisis.next_event {
        return events;
    }
    match floor.crisis.phase {
        CrisisPhase::Lull => {
            floor.crisis.phase = CrisisPhase::Warning;
            floor.crisis.next_event = floor.time.saturating_add(250);
            events.push(WorldEvent {
                position: None,
                message:
                    "The lull breaks. Scratching rises inside the walls; cracks shed pale dust."
                        .into(),
                danger: true,
                impact: false,
            });
            if let Some(cavern) = floor.caverns.iter().find(|c| !c.opened) {
                events.push(WorldEvent {
                    position: Some(cavern.mouth),
                    message: "A sealed cavern strains against its stone seam.".into(),
                    danger: true,
                    impact: false,
                });
            }
            let mut candidates: Vec<Point> = (0..CELLS)
                .map(point)
                .filter(|p| {
                    matches!(
                        floor.tile(*p),
                        Tile::Floor | Tile::DoorOpen | Tile::Water | Tile::Rubble
                    ) && p.distance(player) <= 6
                        && *p != floor.entrance
                        && *p != floor.exit
                })
                .collect();
            if let Some(p) = choose_remove(&mut candidates, rng) {
                floor.crisis.collapse = Some(Collapse {
                    position: p,
                    at: floor.time.saturating_add(300),
                });
                events.push(WorldEvent {
                    position: Some(p),
                    message:
                        "Pebbles fall from a splitting ceiling. Move away from the marked ground."
                            .into(),
                    danger: true,
                    impact: false,
                });
            }
        }
        CrisisPhase::Warning => {
            floor.crisis.phase = CrisisPhase::Surge;
            floor.crisis.next_event = floor.time.saturating_add(600);
            floor.crisis.cycle = floor.crisis.cycle.saturating_add(1).min(6);
            if let Some(i) = floor.caverns.iter().position(|c| !c.opened) {
                floor.caverns[i].opened = true;
                let cells = floor.caverns[i].cells.clone();
                let mouth = floor.caverns[i].mouth;
                for p in &cells {
                    if floor.tile(*p) == Tile::Wall {
                        floor.set(*p, Tile::Floor);
                    }
                }
                let mut positions: Vec<Point> = cells
                    .into_iter()
                    .filter(|p| *p != player && !floor.enemies.iter().any(|e| e.position == *p))
                    .collect();
                if let Some(p) = choose_remove(&mut positions, rng) {
                    spawn(floor, EnemyKind::Brute, p, true);
                }
                events.push(WorldEvent {
                    position: Some(mouth),
                    message: "The seam bursts open onto a cavern. Something enormous stirs within."
                        .into(),
                    danger: true,
                    impact: false,
                });
            }
            swarm(floor, rng, player, &mut events);
        }
        CrisisPhase::Surge => {
            swarm(floor, rng, player, &mut events);
            floor.crisis.phase = CrisisPhase::Lull;
            floor.crisis.next_event = floor.time.saturating_add(900);
            events.push(WorldEvent {
                position: None,
                message: "The stone settles into a lull. The creatures already loose remain."
                    .into(),
                danger: false,
                impact: false,
            });
        }
    }
    events
}

fn swarm(floor: &mut Floor, rng: &mut Rng, player: Point, events: &mut Vec<WorldEvent>) {
    let mut candidates: Vec<Point> = (0..CELLS)
        .map(point)
        .filter(|p| {
            floor.tile(*p).walkable()
                && p.distance(player) >= 6
                && p.distance(player) <= 14
                && *p != floor.entrance
                && *p != floor.exit
                && !floor.enemies.iter().any(|e| e.position == *p)
                && [(0, 1), (1, 0), (0, -1), (-1, 0)]
                    .iter()
                    .any(|(dx, dy)| floor.tile(p.offset(*dx, *dy)) == Tile::Wall)
        })
        .collect();
    let count = 2 + usize::from(floor.crisis.cycle.min(4));
    let before = floor.enemies.len();
    for _ in 0..count {
        let Some(p) = choose_remove(&mut candidates, rng) else {
            break;
        };
        spawn(floor, EnemyKind::Rat, p, true);
        if let Some(enemy) = floor.enemies.last_mut()
            && enemy.position == p
        {
            enemy.target = Some(player);
        }
    }
    if floor.enemies.len() > before {
        events.push(WorldEvent {
            position: Some(floor.enemies[before].position),
            message: "Ash rats pour through cracks in the stone, drawn toward the ember.".into(),
            danger: true,
            impact: false,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]
        #[test]
        fn generated_floors_and_every_awakening_remain_connected(seed in any::<u64>(), depth in 0usize..5) {
            let mut rng = Rng(seed);
            let mut floor = generate(&mut rng, depth, true);
            prop_assert!(floor.validate().is_ok(), "{:?}", floor.validate());
            let original = floor.clone();
            awaken(&mut floor);
            let player = floor.entrance;
            for _ in 0..180 {
                floor.time += 50;
                advance(&mut floor, &mut rng, player);
                prop_assert!(floor.validate().is_ok(), "{:?}", floor.validate());
                prop_assert!(floor.enemies.len() <= MAX_ENEMIES);
                prop_assert!(floor.tile(player).walkable());
            }
            prop_assert_eq!(&floor.remembered, &original.remembered);
            prop_assert_eq!(&floor.visible, &original.visible);
            prop_assert!(floor.caverns.iter().all(|c| c.opened));
        }
    }

    #[test]
    fn awakening_and_reentering_never_reset_a_warning() {
        let mut rng = Rng(71);
        let mut floor = generate(&mut rng, 2, false);
        let player = floor.entrance;
        awaken(&mut floor);
        floor.time = 1000;
        let warning = advance(&mut floor, &mut rng, player);
        assert!(warning.iter().any(|event| event.danger));
        assert_eq!(floor.crisis.phase, CrisisPhase::Warning);
        let saved = floor.clone();
        awaken(&mut floor);
        assert_eq!(floor, saved);
        let restored: Floor =
            serde_json::from_str(&serde_json::to_string(&floor).unwrap()).unwrap();
        assert_eq!(floor, restored);
        assert!(floor.crisis.collapse.unwrap().at >= floor.time + 200);
    }

    #[test]
    fn rockfall_cannot_embed_player_or_destroy_stairs_or_an_escape_bridge() {
        let mut rng = Rng(22);
        let mut floor = generate(&mut rng, 0, false);
        awaken(&mut floor);
        let victim = floor
            .tiles
            .iter()
            .position(|t| *t == Tile::Floor)
            .map(point)
            .unwrap();
        floor.crisis.collapse = Some(Collapse {
            position: victim,
            at: 300,
        });
        floor.time = 300;
        let events = advance(&mut floor, &mut rng, victim);
        assert!(events.iter().any(|e| e.impact));
        assert_eq!(floor.tile(victim), Tile::Rubble);
        assert!(floor.validate().is_ok());
        for p in [floor.entrance, floor.exit] {
            let tile = floor.tile(p);
            floor.crisis.collapse = Some(Collapse {
                position: p,
                at: floor.time,
            });
            advance(&mut floor, &mut rng, victim);
            assert_eq!(floor.tile(p), tile);
        }
        // An actual one-cell bridge must remain passable even when unoccupied.
        floor.tiles.fill(Tile::Wall);
        floor.enemies.clear();
        floor.loot.clear();
        floor.caverns.clear();
        floor.entrance = Point { x: 2, y: 2 };
        floor.exit = Point { x: 6, y: 2 };
        for x in 2..=6 {
            floor.set(Point { x, y: 2 }, Tile::Floor);
        }
        floor.set(floor.entrance, Tile::Up);
        floor.set(floor.exit, Tile::Down);
        let bridge = Point { x: 4, y: 2 };
        floor.crisis.collapse = Some(Collapse {
            position: bridge,
            at: floor.time,
        });
        let player = floor.entrance;
        advance(&mut floor, &mut rng, player);
        assert_eq!(floor.tile(bridge), Tile::Rubble);
        assert!(floor.validate().is_ok());
    }

    #[test]
    fn warned_rockfalls_injure_creatures_as_well_as_the_explorer() {
        let mut rng = Rng(42);
        let mut floor = generate(&mut rng, 0, false);
        floor.enemies.clear();
        let victim = floor
            .tiles
            .iter()
            .position(|t| *t == Tile::Floor)
            .map(point)
            .unwrap();
        spawn(&mut floor, EnemyKind::Rat, victim, true);
        let before = floor.enemies[0].body.clone();
        awaken(&mut floor);
        floor.crisis.next_event = 1000;
        floor.crisis.collapse = Some(Collapse {
            position: victim,
            at: 300,
        });
        floor.time = 300;
        let player = floor.entrance;
        let events = advance(&mut floor, &mut rng, player);
        assert_ne!(floor.enemies[0].body, before);
        assert!(floor.tile(victim).walkable());
        assert!(events.iter().any(|event| event.message.contains("ash rat")));
        assert!(events.iter().all(|event| !event.impact));
    }

    #[test]
    fn floors_vary_and_contain_routes_beyond_a_spanning_tree() {
        let a = generate(&mut Rng(1), 0, false);
        let b = generate(&mut Rng(2), 0, false);
        assert_ne!(a.tiles, b.tiles);
        // There are removable corridor cells, not merely cycles within rooms.
        for seed in 0..16 {
            let mut floor = generate(&mut Rng(seed), 0, false);
            let mut optional_corridors = 0;
            for i in 0..CELLS {
                let p = point(i);
                if floor.tile(p) != Tile::Floor || p == floor.exit {
                    continue;
                }
                let adjacent = [(0, 1), (1, 0), (0, -1), (-1, 0)]
                    .iter()
                    .filter(|(dx, dy)| floor.passable(p.offset(*dx, *dy)))
                    .count();
                if adjacent != 2 {
                    continue;
                }
                floor.set(p, Tile::Wall);
                let reached = floor.reachable();
                if floor
                    .tiles
                    .iter()
                    .enumerate()
                    .all(|(i, t)| *t == Tile::Wall || reached[i])
                {
                    optional_corridors += 1;
                }
                floor.set(p, Tile::Floor);
            }
            assert!(
                optional_corridors > 0,
                "seed {seed} has no alternate corridor"
            );
        }
    }

    #[test]
    fn closed_doors_and_diagonal_corners_block_sight_and_movement() {
        let mut floor = generate(&mut Rng(9), 0, false);
        let p = Point { x: 10, y: 10 };
        for dy in -1..=1 {
            for dx in -1..=2 {
                floor.set(p.offset(dx, dy), Tile::Floor);
            }
        }
        floor.set(p.offset(1, 0), Tile::DoorClosed);
        assert!(!floor.sight(p, p.offset(2, 0)));
        assert!(floor.sight(p, p.offset(1, 1)));
        assert!(floor.step_allowed(p, p.offset(1, 1)));
        floor.set(p.offset(0, 1), Tile::Wall);
        assert!(!floor.sight(p, p.offset(1, 1)));
        assert!(!floor.step_allowed(p, p.offset(1, 1)));
        assert!(floor.sight(p, p.offset(1, 0)));
        floor.set(p.offset(1, 0), Tile::DoorOpen);
        assert!(floor.step_allowed(p, p.offset(1, 1)));
        assert!(floor.sight(p, p.offset(2, 0)));
    }
}
