//! Diagnostic observation-limited policies, not a human win-rate estimate or CI gate.
//!
//! `cargo run -p dessplay --example roguelike_survey -- --seeds 100 --start 1`
//! emits JSON lines for individual expeditions followed by policy aggregates.
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use dessplay::roguelike::{
    Action, ArmorMaterial, CELLS, EventKind, FLOOR_COUNT, LootKind, Outcome, Point, Run, RunView,
    Tile, WIDTH, WeaponKind,
};
use serde_json::json;

const DIRECTIONS: [(i32, i32); 8] = [
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (1, 1),
    (-1, 1),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Cautious,
    Ember,
}

impl Policy {
    fn name(self) -> &'static str {
        match self {
            Self::Cautious => "cautious_retreat",
            Self::Ember => "committed_ember",
        }
    }
}

struct Memory {
    visits: Vec<Vec<u16>>,
    returning: bool,
    policy: Policy,
}

impl Memory {
    fn new(policy: Policy) -> Self {
        Self {
            visits: vec![vec![0; CELLS]; FLOOR_COUNT],
            returning: false,
            policy,
        }
    }
}

fn point(index: usize) -> Point {
    Point {
        x: index as i32 % WIDTH,
        y: index as i32 / WIDTH,
    }
}

fn index(p: Point) -> usize {
    (p.y * WIDTH + p.x) as usize
}

fn known_step(view: &RunView, from: Point, to: Point) -> bool {
    to.index().is_some()
        && view.tile(to) != Tile::Wall
        && (from.x == to.x
            || from.y == to.y
            || (view.tile(Point { x: from.x, y: to.y }).walkable()
                || view.tile(Point { x: to.x, y: from.y }).walkable()))
}

fn creature_at(view: &RunView, p: Point) -> bool {
    view.enemies.iter().any(|enemy| enemy.position == p)
}

/// Dijkstra uses only remembered terrain, currently perceived enemies and warnings.
fn routes(view: &RunView) -> (Vec<u32>, Vec<Option<Point>>) {
    let mut distance = vec![u32::MAX; CELLS];
    let mut first = vec![None; CELLS];
    let start = index(view.position);
    distance[start] = 0;
    let mut queue = BinaryHeap::from([Reverse((0_u32, start))]);
    while let Some(Reverse((cost, i))) = queue.pop() {
        if cost != distance[i] {
            continue;
        }
        let from = point(i);
        for (dx, dy) in DIRECTIONS {
            let to = from.offset(dx, dy);
            if !known_step(view, from, to) {
                continue;
            }
            let j = index(to);
            let danger = u32::from(view.cells[j].threatened) * 200
                + u32::from(creature_at(view, to)) * 70
                + view
                    .enemies
                    .iter()
                    .filter(|enemy| enemy.position.distance(to) == 1)
                    .count() as u32
                    * 12;
            let terrain = match view.tile(to) {
                Tile::Water | Tile::Rubble | Tile::DoorClosed => 15,
                _ => 10,
            };
            let next = cost + terrain + danger;
            if next < distance[j] {
                distance[j] = next;
                first[j] = if i == start { Some(to) } else { first[i] };
                queue.push(Reverse((next, j)));
            }
        }
    }
    (distance, first)
}

fn armor_rank(material: ArmorMaterial) -> u8 {
    match material {
        ArmorMaterial::Cloth => 0,
        ArmorMaterial::Leather => 1,
        ArmorMaterial::Iron => 2,
    }
}

fn desirable(view: &RunView, item: LootKind) -> bool {
    match item {
        LootKind::Weapon(weapon) => {
            view.body.can_wield(weapon)
                && (view.gear.active == WeaponKind::Unarmed
                    || (weapon == WeaponKind::Spear && view.gear.active != weapon)
                    || (weapon == WeaponKind::Knife && !view.body.can_wield(view.gear.active)))
        }
        LootKind::Armor(piece) => view.gear.armor[piece.slot.index()]
            .is_none_or(|equipped| armor_rank(piece.material) > armor_rank(equipped.material)),
        _ => false,
    }
}

fn movement(view: &RunView, to: Point, urgent: bool) -> Action {
    let dx = to.x - view.position.x;
    let dy = to.y - view.position.y;
    if urgent
        && view.tile(to).walkable()
        && !creature_at(view, to)
        && view.body.stamina >= view.body.sprint_cost(&view.gear).saturating_add(12)
    {
        Action::Sprint(dx, dy)
    } else {
        Action::Move(dx, dy)
    }
}

fn reachable_target(distances: &[u32], predicate: impl Fn(Point) -> bool) -> Option<Point> {
    (0..CELLS)
        .filter(|i| distances[*i] != u32::MAX && predicate(point(*i)))
        .min_by_key(|i| distances[*i])
        .map(point)
}

/// All decisions enter through the exact same player observation as the TUI.
fn choose(view: &RunView, memory: &mut Memory) -> Action {
    let here = index(view.position);
    memory.visits[view.depth][here] = memory.visits[view.depth][here].saturating_add(1);
    if view.relic
        || (memory.policy == Policy::Cautious
            && (view.deepest >= 2
                || view.turns >= 100
                || view.body.blood < 850
                || view.body.pain() >= 18))
    {
        memory.returning = true;
    }
    let (distance, first) = routes(view);
    let escape = reachable_target(&distance, |p| view.tile(p) == Tile::Up);
    let descent = reachable_target(&distance, |p| view.tile(p) == Tile::Down);
    let goal = if memory.returning { escape } else { descent };

    // Leave committed windups and rockfalls before spending a turn on maintenance.
    if view.cells[here].threatened
        && let Some(to) = DIRECTIONS
            .iter()
            .map(|(dx, dy)| view.position.offset(*dx, *dy))
            .filter(|p| {
                known_step(view, view.position, *p)
                    && view.tile(*p).walkable()
                    && !view.cells[index(*p)].threatened
                    && !creature_at(view, *p)
            })
            .max_by_key(|p| {
                let separation = view
                    .enemies
                    .iter()
                    .map(|enemy| enemy.position.distance(*p))
                    .min()
                    .unwrap_or(9);
                separation * 20 - goal.map_or(0, |g| p.distance(g))
            })
    {
        return movement(view, to, true);
    }
    if memory.returning && view.tile(view.position) == Tile::Up {
        return Action::Stairs;
    }
    if !memory.returning && view.ground.contains(&LootKind::Relic) {
        return Action::Interact;
    }
    if view.tile(view.position) == Tile::Fountain
        && (view.body.pain() >= 10 || view.body.blood < 750 || view.body.vision_radius() < 6)
    {
        return Action::Interact;
    }

    let closest = view
        .enemies
        .iter()
        .map(|enemy| enemy.position.distance(view.position))
        .min()
        .unwrap_or(99);
    if view.body.bleeding() > 0
        && view.supplies.bandages > 0
        && (closest > 1 || view.body.bleeding() >= 8 || view.body.blood < 500)
    {
        return Action::Bandage;
    }
    if view.body.hunger <= 45 && view.supplies.food > 0 && closest > 1 {
        return Action::Eat;
    }

    // A known route home takes precedence over elective fights.
    if memory.returning
        && closest <= 2
        && let Some(to) = goal.and_then(|p| first[index(p)])
        && !creature_at(view, to)
        && !view.cells[index(to)].threatened
    {
        return movement(view, to, true);
    }
    let weapon = view.body.effective_weapon(&view.gear);
    if let Some(enemy) = view
        .enemies
        .iter()
        .filter(|enemy| {
            let dx = enemy.position.x - view.position.x;
            let dy = enemy.position.y - view.position.y;
            let distance = view.position.distance(enemy.position);
            distance <= weapon.reach()
                && (dx == 0 || dy == 0 || dx.abs() == dy.abs())
                && (distance == 1
                    || known_step(
                        view,
                        view.position,
                        view.position.offset(dx.signum(), dy.signum()),
                    ))
        })
        .min_by_key(|enemy| {
            (
                usize::from(!enemy.intent.starts_with("calling")),
                view.position.distance(enemy.position),
            )
        })
    {
        if view.body.stamina < weapon.breath_cost() && !view.cells[here].threatened {
            return Action::Wait;
        }
        return Action::Attack(
            (enemy.position.x - view.position.x).signum(),
            (enemy.position.y - view.position.y).signum(),
        );
    }
    if closest > 2 {
        if let Some((i, _)) = view
            .ground
            .iter()
            .enumerate()
            .find(|(_, item)| desirable(view, **item))
        {
            return Action::Equip(i);
        }
        if view.can_rest
            && (view.body.stamina < 70
                || view.body.blood < 950
                || (view.body.pain() > 0 && memory.visits[view.depth][here] < 12))
        {
            return Action::Rest;
        }
        if view.body.stamina < 25 {
            return Action::Wait;
        }
    }
    if !memory.returning && view.tile(view.position) == Tile::Down {
        return Action::Stairs;
    }

    let fountain = reachable_target(&distance, |p| view.tile(p) == Tile::Fountain);
    if (view.body.pain() >= 20 || view.body.blood < 650)
        && let Some(to) = fountain.and_then(|p| first[index(p)])
    {
        return movement(view, to, closest <= 2);
    }
    if let Some(to) = goal.and_then(|p| first[index(p)]) {
        return movement(view, to, closest <= 2);
    }

    // Pick visible supplies/equipment close by before searching another frontier.
    let loot = reachable_target(&distance, |p| {
        p != view.position
            && view.cells[index(p)].visible
            && matches!(view.glyph(p), '!' | '=' | '%' | ')' | '[' | '*' | '$')
            && distance[index(p)] <= 90
            && memory.visits[view.depth][index(p)] < 3
    });
    if let Some(to) = loot.and_then(|p| first[index(p)]) {
        return movement(view, to, closest <= 2);
    }

    let frontier = (0..CELLS)
        .filter(|i| distance[*i] != u32::MAX && *i != here && view.tile(point(*i)) != Tile::Wall)
        .filter_map(|i| {
            let unknown = DIRECTIONS
                .iter()
                .filter(|(dx, dy)| {
                    // Standing here cannot reveal a diagonal behind either
                    // opaque corner. Such cells are not useful frontiers.
                    if *dx != 0
                        && *dy != 0
                        && (!view.tile(point(i).offset(*dx, 0)).walkable()
                            || !view.tile(point(i).offset(0, *dy)).walkable())
                    {
                        return false;
                    }
                    point(i)
                        .offset(*dx, *dy)
                        .index()
                        .is_some_and(|j| view.cells[j].terrain.is_none())
                })
                .count();
            (unknown > 0).then_some((
                distance[i] + u32::from(memory.visits[view.depth][i]) * 60,
                i,
            ))
        })
        .min();
    if let Some((_, i)) = frontier
        && let Some(to) = first[i]
    {
        return movement(view, to, closest <= 2);
    }

    // With no observable frontier left, leave rather than deliberately starve.
    memory.returning = true;
    if let Some(to) = escape.and_then(|p| first[index(p)]) {
        return movement(view, to, closest <= 2);
    }
    Action::Wait
}

#[derive(Default)]
struct Totals {
    runs: usize,
    escaped: usize,
    ember_escapes: usize,
    dead: usize,
    capped: usize,
    reached_ember: usize,
    actions: u64,
    max_depth: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seeds = 100_u64;
    let mut start = 1_u64;
    let mut max_actions = 3000_u64;
    let mut policies = vec![Policy::Cautious, Policy::Ember];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seeds" => seeds = args.next().ok_or("--seeds needs a count")?.parse()?,
            "--start" => start = args.next().ok_or("--start needs a seed")?.parse()?,
            "--max-actions" => {
                max_actions = args
                    .next()
                    .ok_or("--max-actions needs a count")?
                    .parse::<u64>()?
                    .min(3000)
            }
            "--policy" => {
                policies = match args.next().as_deref() {
                    Some("cautious") => vec![Policy::Cautious],
                    Some("ember") => vec![Policy::Ember],
                    Some("both") => vec![Policy::Cautious, Policy::Ember],
                    _ => return Err("--policy is cautious, ember, or both".into()),
                }
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    for policy in policies {
        let mut totals = Totals::default();
        for offset in 0..seeds {
            let seed = start.wrapping_add(offset);
            let mut run = Run::new(seed);
            let mut memory = Memory::new(policy);
            let mut attempts = 0_u64;
            let mut resting = 0_u64;
            let mut invalid = 0_u64;
            let mut injuries = 0_u64;
            let mut serious = 0_u64;
            let mut warnings = 0_u64;
            let mut supply_spent = [0_u64; 3];
            let mut last_event = 0;
            while attempts < max_actions {
                let view = run.view();
                if view.is_finished() {
                    break;
                }
                let action = choose(&view, &mut memory);
                if action == Action::Rest {
                    resting += 1;
                }
                let result = run.step(action);
                attempts += 1;
                let next = run.view();
                if result.elapsed == 0 && !next.is_finished() {
                    invalid += 1;
                }
                for (i, (before, after)) in [
                    (view.supplies.bandages, next.supplies.bandages),
                    (view.supplies.splints, next.supplies.splints),
                    (view.supplies.food, next.supplies.food),
                ]
                .into_iter()
                .enumerate()
                {
                    supply_spent[i] += u64::from(before.saturating_sub(after));
                }
                for entry in next.journal.iter().filter(|entry| entry.id > last_event) {
                    injuries += u64::from(entry.kind == EventKind::Injury);
                    warnings += u64::from(entry.kind == EventKind::Danger);
                    serious += u64::from(
                        entry.kind == EventKind::Injury
                            && (entry.text.contains("fractur")
                                || entry.text.contains("destroy")
                                || entry.text.contains("sever")),
                    );
                }
                last_event = next.journal.last().map_or(last_event, |entry| entry.id);
            }
            let view = run.view();
            let outcome = match view.outcome {
                Outcome::Alive => "action_cap",
                Outcome::Dead(_) => "dead",
                Outcome::Escaped => "escaped",
            };
            totals.runs += 1;
            totals.escaped += usize::from(view.outcome == Outcome::Escaped);
            totals.ember_escapes += usize::from(view.outcome == Outcome::Escaped && view.relic);
            totals.dead += usize::from(matches!(view.outcome, Outcome::Dead(_)));
            totals.capped += usize::from(view.outcome == Outcome::Alive);
            totals.reached_ember += usize::from(view.relic);
            totals.actions += view.turns;
            totals.max_depth = totals.max_depth.max(view.deepest);
            println!(
                "{}",
                json!({"type":"run", "policy":policy.name(),"seed":seed,"outcome":outcome,
                "cause":view.outcome,"ember":view.relic,"deepest":view.deepest,"actions":view.turns,
                "attempts":attempts,"invalid_actions":invalid,"elapsed":view.time,"score":view.score,
                "blood":view.body.blood,"pain":view.body.pain(),"nutrition":view.body.hunger,
                "supplies":view.supplies,"supplies_spent":supply_spent,"rest_actions":resting,
                "injury_events":injuries,"major_injury_words":serious,"danger_events":warnings,
                "gold":view.gold,"kills":view.kills})
            );
        }
        println!(
            "{}",
            json!({"type":"aggregate","policy":policy.name(),"runs":totals.runs,
            "escaped":totals.escaped,"ember_escapes":totals.ember_escapes,"dead":totals.dead,
            "capped":totals.capped,"reached_ember":totals.reached_ember,"max_depth":totals.max_depth,
            "mean_actions":totals.actions as f64 / totals.runs.max(1) as f64,
            "interpretation":"Diagnostic scripted-policy results; not human win rates or a CI gate."})
        );
    }
    Ok(())
}
