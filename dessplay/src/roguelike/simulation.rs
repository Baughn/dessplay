//! Explicit actions and stable 50-unit scheduling. Other floors freeze in place.
use super::narration::{AttackSource, Victim};
use super::*;
use std::collections::VecDeque;

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
fn direction(dx: i32, dy: i32) -> bool {
    (-1..=1).contains(&dx) && (-1..=1).contains(&dy) && (dx != 0 || dy != 0)
}

impl Run {
    /// Execute one action and every simulation event due before the next player decision.
    pub fn step(&mut self, action: Action) -> StepResult {
        if self.is_finished() {
            return StepResult::default();
        }
        let before_id = self.next_event;
        let before_time = self.time;
        let previous_step = self.last_step.clone();
        self.last_step = StepResult::default();
        let duration = self.perform(action);
        if duration > 0 {
            self.turns = self.turns.saturating_add(1);
            self.reveal();
            for _ in 0..duration / 50 {
                if self.is_finished() {
                    break;
                }
                self.time += 50;
                self.floors[self.depth].time += 50;
                // Physiology is charged by elapsed time, never by input count.
                if self.time.is_multiple_of(100) {
                    self.body.tick();
                    self.check_death(None);
                }
                if self.is_finished() {
                    break;
                }
                if self.floor().time.is_multiple_of(100) {
                    for e in &mut self.floors[self.depth].enemies {
                        e.body.tick();
                    }
                    self.remove_dead();
                }
                let events =
                    world::advance(&mut self.floors[self.depth], &mut self.rng, self.position);
                for event in events {
                    let distance = event.position.map_or(0, |p| p.distance(self.position));
                    let visible = event
                        .position
                        .is_none_or(|p| p.index().is_some_and(|i| self.floor().visible[i]));
                    if visible || distance <= 12 {
                        let text = if visible {
                            event.message
                        } else {
                            format!(
                                "Stone shifts to the {}.",
                                self.sound_direction(event.position.unwrap_or(self.position))
                            )
                        };
                        self.say(
                            if event.danger {
                                EventKind::Danger
                            } else {
                                EventKind::Info
                            },
                            text,
                        );
                        if event.danger {
                            self.alert_until = self.time + 200;
                            self.last_step.interrupted = true;
                        }
                    }
                    if event.impact {
                        self.hurt(
                            AttackProfile {
                                weapon: WeaponKind::Mace,
                                power: 35,
                            },
                            AttackSource::FallingStone,
                        );
                    }
                    if self.is_finished() {
                        break;
                    }
                }
                if self.is_finished() {
                    break;
                }
                self.remove_dead();
                self.reveal();
                let now = self.floor().time;
                let mut due: Vec<_> = self
                    .floor()
                    .enemies
                    .iter()
                    .filter(|e| e.next_action <= now)
                    .map(|e| e.id)
                    .collect();
                due.sort_unstable();
                for id in due {
                    let Some(i) = self.floor().enemies.iter().position(|e| e.id == id) else {
                        continue;
                    };
                    let mut enemy = self.floors[self.depth].enemies.remove(i);
                    self.enemy_action(&mut enemy);
                    self.floors[self.depth].enemies.push(enemy);
                    if self.is_finished() {
                        break;
                    }
                }
                self.reveal();
            }
        }
        self.last_step.elapsed = self.time - before_time;
        self.last_step.changed = duration > 0 || self.next_event != before_id;
        self.last_step.interrupted |= self.danger() || self.is_finished();
        let result = self.last_step.clone();
        if !result.changed {
            self.last_step = previous_step;
        }
        result
    }
    fn perform(&mut self, action: Action) -> u64 {
        match action {
            Action::Move(dx, dy) => self.move_player(dx, dy, false),
            Action::Sprint(dx, dy) => self.move_player(dx, dy, true),
            Action::Attack(dx, dy) => self.attack(dx, dy),
            Action::Wait => {
                self.body.wait();
                100
            }
            Action::Rest => {
                if self.danger() {
                    self.say(
                        EventKind::Info,
                        "Danger interrupts your rest. Find cover first.",
                    );
                    return 0;
                }
                let result = self.body.care_step(&mut self.supplies);
                self.say(EventKind::Recovery, result.message);
                if result.changed { 100 } else { 0 }
            }
            Action::Bandage | Action::Treat(_) => {
                let result = if let Action::Treat(i) = action {
                    self.body.treat(&mut self.supplies, i)
                } else {
                    self.body.bandage(&mut self.supplies, None)
                };
                self.say(EventKind::Recovery, result.message);
                if result.changed { 100 } else { 0 }
            }
            Action::Eat => {
                let result = self.body.eat(&mut self.supplies);
                self.say(EventKind::Recovery, result.message);
                if result.changed { 100 } else { 0 }
            }
            Action::Stairs => self.stairs(),
            Action::Interact => self.interact(),
            Action::SwapWeapon => {
                if let Some(spare) = self.gear.spare {
                    self.gear.spare = Some(self.gear.active);
                    self.gear.active = spare;
                    self.say(EventKind::Info, format!("You ready the {}.", spare.name()));
                    100
                } else {
                    self.say(EventKind::Info, "You have no spare weapon.");
                    0
                }
            }
            Action::Equip(index) => self.equip(index),
            Action::CloseDoor(dx, dy) => {
                if !direction(dx, dy) {
                    return 0;
                }
                let p = self.position.offset(dx, dy);
                if !self.floor().clear_corner(self.position, p) {
                    return 0;
                }
                if self.tile(p) != Tile::DoorOpen {
                    self.say(EventKind::Info, "There is no open door there.");
                    return 0;
                }
                if self.floor().enemies.iter().any(|e| e.position == p) {
                    self.say(EventKind::Info, "A creature blocks the door.");
                    return 0;
                }
                if self.floor().loot.iter().any(|item| item.position == p) {
                    self.say(EventKind::Info, "Loose equipment blocks the door.");
                    return 0;
                }
                if let Some(i) = p.index() {
                    self.floors[self.depth].tiles[i] = Tile::DoorClosed;
                }
                self.say(EventKind::Info, "You close the door.");
                100
            }
        }
    }
    fn move_player(&mut self, dx: i32, dy: i32, sprint: bool) -> u64 {
        if !direction(dx, dy) {
            return 0;
        }
        let p = self.position.offset(dx, dy);
        if self.tile(p) == Tile::DoorClosed && !sprint {
            // Opening also obeys diagonal wall-corner constraints.
            if !self.floor().clear_corner(self.position, p) {
                return 0;
            }
            if let Some(i) = p.index() {
                self.floors[self.depth].tiles[i] = Tile::DoorOpen;
            }
            self.say(EventKind::Info, "You open the door.");
            self.noise(5);
            return 100;
        }
        if !self.floor().step_allowed(self.position, p) {
            self.say(EventKind::Info, "The way is blocked.");
            return 0;
        }
        if !sprint {
            let (_, target) = self.attack_target(dx, dy, self.body.effective_weapon(&self.gear));
            if target.is_some_and(|i| {
                let enemy = &self.floor().enemies[i];
                enemy.position == p
                    || enemy
                        .position
                        .index()
                        .is_some_and(|index| self.floor().visible[index])
            }) {
                return self.attack(dx, dy);
            }
        } else if self.floor().enemies.iter().any(|e| e.position == p) {
            self.say(EventKind::Info, "A creature blocks your sprint.");
            return 0;
        }
        if sprint {
            let cost = self.body.sprint_cost(&self.gear);
            if self.body.stamina < cost || cost > 100 {
                self.say(
                    EventKind::Info,
                    "You cannot sprint: catch your breath or tend your legs.",
                );
                return 0;
            }
            self.body.stamina -= cost;
        }
        let mut cost = self.body.movement_cost(sprint, &self.gear);
        if matches!(self.tile(p), Tile::Water | Tile::Rubble) {
            cost += 100;
        }
        self.position = p;
        self.noise(if sprint { 10 } else { 3 });
        self.collect();
        cost.max(50)
    }
    // Automatic thrusts and explicit attacks trace the same first target.
    fn attack_target(&self, dx: i32, dy: i32, weapon: WeaponKind) -> (Point, Option<usize>) {
        let mut p = self.position;
        let mut target = None;
        for _ in 0..weapon.reach() {
            let next = p.offset(dx, dy);
            if !self.floor().step_allowed(p, next) {
                break;
            }
            p = next;
            if let Some(i) = self.floor().enemies.iter().position(|e| e.position == p) {
                target = Some(i);
                break;
            }
        }
        (p, target)
    }
    fn attack(&mut self, dx: i32, dy: i32) -> u64 {
        if !direction(dx, dy) {
            return 0;
        }
        let weapon = self.body.effective_weapon(&self.gear);
        let (p, target) = self.attack_target(dx, dy, weapon);
        let power = self.body.attack_power(weapon.power());
        self.body.stamina = self.body.stamina.saturating_sub(weapon.breath_cost());
        self.noise(12);
        if let Some(i) = target {
            let now = self.floor().time;
            let visible = self.floor().enemies[i]
                .position
                .index()
                .is_some_and(|index| self.floor().visible[index]);
            let e = &mut self.floors[self.depth].enemies[i];
            let before = e.body.clone();
            let report = e
                .body
                .hit(AttackProfile { weapon, power }, &e.gear, &mut self.rng);
            let interrupted = matches!(e.intent, EnemyIntent::Calling { .. }) && e.body != before;
            if interrupted {
                e.intent = EnemyIntent::Recovering { until: now + 300 };
                e.next_action = now + 300;
            }
            let victim = Victim::Enemy(e.kind);
            self.say(
                EventKind::Combat,
                if visible {
                    report.narrate(AttackSource::Player(weapon), victim)
                } else {
                    format!("Your {} strikes something beyond sight.", weapon.name())
                },
            );
            if interrupted {
                self.say(
                    EventKind::Combat,
                    if visible {
                        "The pilgrim's call breaks into silence."
                    } else {
                        "The call breaks off."
                    },
                );
            }
            self.remove_dead();
        } else {
            self.say(EventKind::Combat, "Your attack meets empty air.");
        }
        let close_penalty = if weapon == WeaponKind::Spear && self.position.distance(p) == 1 {
            50
        } else {
            0
        };
        weapon.cost() + close_penalty
    }
    fn collect(&mut self) {
        let mut retained = Vec::new();
        let loot = std::mem::take(&mut self.floors[self.depth].loot);
        for l in loot {
            if l.position != self.position {
                retained.push(l);
                continue;
            }
            let text = match l.kind {
                LootKind::Bandage if self.supplies.bandages < 127 => {
                    self.supplies.bandages += 2;
                    Some("You collect two clean linen bandages.".into())
                }
                LootKind::Splint if self.supplies.splints < 128 => {
                    self.supplies.splints += 1;
                    Some("You collect a wooden splint.".into())
                }
                LootKind::Food if self.supplies.food < 128 => {
                    self.supplies.food += 1;
                    Some("You collect a ration of dried apples.".into())
                }
                LootKind::Gold(amount) => {
                    self.gold = self.gold.saturating_add(u32::from(amount)).min(100_000);
                    Some(format!("You collect {amount} gold."))
                }
                _ => None,
            };
            if let Some(text) = text {
                self.say(EventKind::Discovery, text);
            } else {
                if matches!(l.kind, LootKind::Weapon(_) | LootKind::Armor(_)) {
                    self.say(
                        EventKind::Discovery,
                        "Equipment lies here. Inspect it with i; choose what to carry.",
                    );
                }
                if l.kind == LootKind::Relic {
                    self.say(EventKind::Danger,"The ember waits. Taking it (g) will awaken the dungeon. You may still leave alive.");
                }
                retained.push(l);
            }
        }
        self.floors[self.depth].loot = retained;
        if self.tile(self.position) == Tile::Fountain {
            self.say(
                EventKind::Discovery,
                "A healing fountain. Drink (g) to restore your entire body once.",
            );
        }
    }
    fn equip(&mut self, index: usize) -> u64 {
        let Some(actual) = self
            .floor()
            .loot
            .iter()
            .enumerate()
            .filter(|(_, l)| l.position == self.position)
            .nth(index)
            .map(|(i, _)| i)
        else {
            return 0;
        };
        let item = self.floor().loot[actual].kind;
        let replacement = match item {
            LootKind::Weapon(w) => {
                if self.gear.spare.is_none() {
                    self.gear.spare = Some(self.gear.active);
                    self.gear.active = w;
                    None
                } else {
                    let old = self.gear.active;
                    self.gear.active = w;
                    Some(LootKind::Weapon(old))
                }
            }
            LootKind::Armor(piece) => {
                let old = self.gear.armor[piece.slot.index()];
                self.gear.armor[piece.slot.index()] = Some(piece);
                old.map(LootKind::Armor)
            }
            _ => {
                self.say(EventKind::Info, "Use g to interact with this instead.");
                return 0;
            }
        };
        self.floors[self.depth].loot.remove(actual);
        if let Some(kind) = replacement {
            self.floors[self.depth].loot.push(Loot {
                position: self.position,
                kind,
            });
        }
        self.say(
            EventKind::Info,
            "You change equipment; the comparison reflects your new kit.",
        );
        100
    }
    fn interact(&mut self) -> u64 {
        if let Some(i) = self
            .floor()
            .loot
            .iter()
            .position(|l| l.position == self.position && l.kind == LootKind::Relic)
        {
            self.floors[self.depth].loot.remove(i);
            self.phase = Phase::Awakened;
            for floor in &mut self.floors {
                world::awaken(floor);
            }
            self.say(
                EventKind::Danger,
                "You lift the ember. Stone answers. The dungeon is awake: flee to daylight!",
            );
            self.alert_until = self.time + 300;
            self.last_step.interrupted = true;
            return 100;
        }
        if self.tile(self.position) == Tile::Fountain {
            self.body.restore();
            self.fountains_used += 1;
            if let Some(i) = self.position.index() {
                self.floors[self.depth].tiles[i] = Tile::FountainDry;
            }
            self.say(
                EventKind::Recovery,
                "The fountain restores flesh, bone, eyes and breath. Its water turns to dust.",
            );
            return 100;
        }
        if matches!(self.tile(self.position), Tile::Up | Tile::Down) {
            return self.stairs();
        }
        self.say(
            EventKind::Info,
            "There is nothing here to activate. Equipment choices are in i.",
        );
        0
    }
    fn stairs(&mut self) -> u64 {
        match self.tile(self.position) {
            Tile::Up if self.depth == 0 => {
                self.finish(Outcome::Escaped);
                100
            }
            Tile::Up => {
                let next = self.depth - 1;
                let p = self.floors[next].exit;
                if self.floors[next].enemies.iter().any(|e| e.position == p) {
                    self.say(EventKind::Danger, "Something blocks the stair above.");
                    return 0;
                }
                self.depth = next;
                self.position = p;
                self.say(EventKind::Info, format!("You climb to floor {}.", next + 1));
                100
            }
            Tile::Down if self.depth + 1 < FLOOR_COUNT => {
                let next = self.depth + 1;
                let p = self.floors[next].entrance;
                if self.floors[next].enemies.iter().any(|e| e.position == p) {
                    self.say(EventKind::Danger, "Something blocks the stair below.");
                    return 0;
                }
                self.depth = next;
                self.deepest = self.deepest.max(next + 1);
                self.position = p;
                self.say(
                    EventKind::Discovery,
                    format!(
                        "Floor {}: {}.",
                        next + 1,
                        [
                            "the vestibule",
                            "the drowned pantry",
                            "the pilgrim cells",
                            "the iron choir",
                            "the ember vault"
                        ][next]
                    ),
                );
                100
            }
            _ => {
                self.say(EventKind::Info, "Stand on < or > to use the stairs.");
                0
            }
        }
    }
    fn check_death(&mut self, cause: Option<String>) {
        if !self.is_finished() && self.body.is_dead() {
            self.finish(Outcome::Dead(
                cause.unwrap_or_else(|| format!("from {}", self.body.death_cause())),
            ));
        }
    }
    fn finish(&mut self, outcome: Outcome) {
        self.outcome = outcome;
        self.epilogue = if self.outcome == Outcome::Escaped {
            if self.body.vision_radius() == 1 {
                "You never saw the sea again. You bought a house beside it."
            } else if self.body.movement_cost(false, &self.gear) > 150 {
                "You learned the daylight paths slowly, with a stick and time enough to use it."
            } else if self.fountains_used > 0 {
                "For the rest of your life, you paused whenever you heard running water."
            } else if self.serious_wounds > 0 {
                "The old wounds ached in winter. You lived to complain about them."
            } else if self.gold >= 150 {
                "You bought a small inn. Its cellar door stayed locked."
            } else {
                "You grew old above ground, and called that enough."
            }
        } else {
            "Your lantern fell. The waiting below kept the rest."
        }
        .into();
        self.final_score = Some(self.score());
        self.say(
            EventKind::Ending,
            if self.outcome == Outcome::Escaped {
                if self.has_ember() {
                    "Daylight. The ember lives, and so do you."
                } else {
                    "Daylight. You have brought your life home. Victory."
                }
            } else {
                "The waiting below is over."
            },
        );
        self.say(EventKind::Ending, self.epilogue.clone());
    }
    fn hurt(&mut self, profile: AttackProfile, source: AttackSource) {
        let report = self.body.hit(profile, &self.gear, &mut self.rng);
        if report.serious {
            self.serious_wounds += 1;
        }
        self.last_step.interrupted = true;
        self.say(EventKind::Injury, report.narrate(source, Victim::Player));
        self.check_death(Some(format!(
            "to {} on floor {} ({})",
            source.death_source(),
            self.depth + 1,
            self.body.death_cause()
        )));
    }
    fn remove_dead(&mut self) {
        let mut dead = Vec::new();
        self.floors[self.depth].enemies.retain(|e| {
            if e.body.is_dead() {
                dead.push((e.kind, e.position));
                false
            } else {
                true
            }
        });
        for (kind, p) in dead {
            self.kills += 1;
            if p.index().is_some_and(|i| self.floor().visible[i]) {
                self.say(EventKind::Combat, format!("The {} falls.", kind.name()));
            }
        }
    }
    fn noise(&mut self, radius: i32) {
        let p = self.position;
        for e in &mut self.floors[self.depth].enemies {
            if e.position.distance(p) <= radius {
                e.target = Some(p);
            }
        }
    }
    fn sound_direction(&self, p: Point) -> String {
        let vertical = match (p.y - self.position.y).signum() {
            -1 => "north",
            1 => "south",
            _ => "",
        };
        let horizontal = match (p.x - self.position.x).signum() {
            -1 => "west",
            1 => "east",
            _ => "",
        };
        if vertical.is_empty() && horizontal.is_empty() {
            "nearby".into()
        } else {
            format!("{vertical}{horizontal}")
        }
    }
    fn enemy_action(&mut self, e: &mut Enemy) {
        let now = self.floor().time;
        let sees = e.position.distance(self.position) <= e.body.vision_radius()
            && self.floor().sight(e.position, self.position);
        if sees {
            e.target = Some(self.position);
        }
        let adjacent = e.position.distance(self.position) == 1
            && self.floor().step_allowed(e.position, self.position);
        let duration = e.body.movement_cost(false, &e.gear).max(100);
        e.next_action = now + duration;
        match e.intent {
            EnemyIntent::Strike { target, at } if now >= at => {
                if self.position == target && adjacent {
                    let profile = enemy_attack(e);
                    self.hurt(profile, AttackSource::Enemy(e.kind, profile.weapon));
                } else if e.position.index().is_some_and(|i| self.floor().visible[i]) {
                    self.say(
                        EventKind::Combat,
                        format!("The {} strikes empty stone.", e.kind.name()),
                    );
                }
                // Committing the swing costs exertion even when its target dodges.
                e.body.stamina = e.body.stamina.saturating_sub(20);
                e.intent = EnemyIntent::Recovering { until: now + 200 };
                e.next_action = now + 200;
                return;
            }
            EnemyIntent::Calling { at } if now >= at => {
                if e.body.attack_power(20) >= 8 && e.body.stamina >= 10 {
                    let p = e.position;
                    for other in &mut self.floors[self.depth].enemies {
                        if other.position.distance(p) <= 14 {
                            other.target = Some(p);
                        }
                    }
                    if p.distance(self.position) <= 14 {
                        self.say(
                            EventKind::Danger,
                            "A pilgrim's cry brings answering footsteps.",
                        );
                        self.alert_until = self.time + 200;
                    }
                    e.body.stamina = e.body.stamina.saturating_sub(10);
                } else if sees {
                    self.say(
                        EventKind::Combat,
                        "The wounded pilgrim's call breaks into silence.",
                    );
                }
                e.intent = EnemyIntent::Recovering { until: now + 300 };
                e.next_action = now + 300;
                return;
            }
            EnemyIntent::Strike { at, .. } | EnemyIntent::Calling { at } => {
                e.next_action = at.max(now + 50);
                return;
            }
            EnemyIntent::Recovering { until } if now < until => {
                e.next_action = until;
                return;
            }
            EnemyIntent::Recovering { .. } => {
                e.intent = EnemyIntent::Idle;
                if e.kind == EnemyKind::Rat && adjacent {
                    let retreat = DIRECTIONS
                        .iter()
                        .map(|&(dx, dy)| e.position.offset(dx, dy))
                        .filter(|&p| self.enemy_step(e.position, p))
                        .max_by_key(|p| p.distance(self.position));
                    if let Some(p) = retreat {
                        e.position = p;
                        return;
                    }
                }
            }
            EnemyIntent::Idle => {}
        }
        if e.body.stamina < 12 {
            e.body.wait();
            return;
        }
        if adjacent {
            if matches!(e.kind, EnemyKind::Warden | EnemyKind::Brute) {
                e.intent = EnemyIntent::Strike {
                    target: self.position,
                    at: now + 150,
                };
                e.next_action = now + 150;
                self.say(
                    EventKind::Danger,
                    format!(
                        "The {} {} toward ({},{}). Step away!",
                        e.kind.name(),
                        e.strike_windup(),
                        self.position.x,
                        self.position.y
                    ),
                );
            } else {
                let profile = enemy_attack(e);
                self.hurt(profile, AttackSource::Enemy(e.kind, profile.weapon));
                e.body.stamina = e.body.stamina.saturating_sub(12);
                e.intent = EnemyIntent::Recovering { until: now + 100 };
                e.next_action = now + 100;
            }
            return;
        }
        if e.kind == EnemyKind::Hollow && sees && e.target.is_some() && self.rng.below(4) == 0 {
            e.intent = EnemyIntent::Calling { at: now + 200 };
            e.next_action = now + 200;
            if e.position.index().is_some_and(|i| self.floor().visible[i]) {
                self.say(
                    EventKind::Danger,
                    "The hollow pilgrim draws breath for a call.",
                );
                self.alert_until = self.time + 200;
                self.last_step.interrupted = true;
            } else if e.position.distance(self.position) <= 4 {
                self.say(
                    EventKind::Danger,
                    format!(
                        "You hear an intake of breath to the {}.",
                        self.sound_direction(e.position)
                    ),
                );
                self.alert_until = self.time + 200;
                self.last_step.interrupted = true;
            }
            return;
        }
        if let Some(target) = e.target {
            if e.position == target {
                e.target = None;
                e.body.wait();
                return;
            }
            if let Some(next) = self.path_step(e.position, target) {
                if self.tile(next) == Tile::DoorClosed {
                    if e.kind == EnemyKind::Rat {
                        e.body.wait();
                        return;
                    }
                    if let Some(i) = next.index() {
                        self.floors[self.depth].tiles[i] = Tile::DoorOpen;
                    }
                    if next.distance(self.position) <= 10 {
                        self.say(EventKind::Danger, "A door opens with a scrape.");
                        self.alert_until = self.time + 200;
                    }
                } else {
                    e.position = next;
                }
                if !e.position.index().is_some_and(|i| self.floor().visible[i])
                    && e.position.distance(self.position) <= 5
                {
                    // At most one vague footstep per time instant; never disclose hidden identity or coordinates.
                    if !self
                        .journal
                        .last()
                        .is_some_and(|j| j.time == self.time && j.text.starts_with("Footsteps"))
                    {
                        self.say(
                            EventKind::Danger,
                            format!("Footsteps to the {}.", self.sound_direction(e.position)),
                        );
                    }
                    self.alert_until = self.time + 150;
                }
            } else {
                e.body.wait();
            }
        } else {
            e.body.wait();
        }
    }
    fn enemy_step(&self, from: Point, to: Point) -> bool {
        to != self.position
            && self.floor().step_allowed(from, to)
            && !matches!(self.tile(to), Tile::Up | Tile::Down)
            && !self.floor().enemies.iter().any(|e| e.position == to)
    }
    fn path_step(&self, start: Point, target: Point) -> Option<Point> {
        let occupied_goal = self.floor().enemies.iter().any(|e| e.position == target);
        let mut seen = vec![false; CELLS];
        let mut queue = VecDeque::from([(start, None)]);
        seen[start.index()?] = true;
        while let Some((p, first)) = queue.pop_front() {
            // A caller occupies the sound's origin. Reach a free neighboring
            // tile instead; stopping here also keeps adjacent listeners still.
            if occupied_goal
                && self.tile(p).walkable()
                && !matches!(self.tile(p), Tile::Up | Tile::Down)
                && self.floor().step_allowed(p, target)
            {
                return first.filter(|step| *step != self.position);
            }
            for (dx, dy) in DIRECTIONS {
                let next = p.offset(dx, dy);
                let Some(i) = next.index() else {
                    continue;
                };
                if seen[i] || !self.floor().passable(next) || !self.floor().clear_corner(p, next) {
                    continue;
                }
                // An occupied goal is still occupied. Calls may direct several
                // creatures to the caller's tile; no path may enter another actor.
                if self.floor().enemies.iter().any(|e| e.position == next)
                    || next != target && matches!(self.tile(next), Tile::Up | Tile::Down)
                {
                    continue;
                }
                let step = first.unwrap_or(next);
                if next == target {
                    return (step != self.position
                        && !matches!(self.tile(step), Tile::Up | Tile::Down))
                    .then_some(step);
                }
                seen[i] = true;
                queue.push_back((next, Some(step)));
            }
        }
        None
    }
}

// Natural teeth/weight are species attacks; tool users share exactly the
// player's hand-function gate, including during an already committed windup.
fn enemy_attack(enemy: &Enemy) -> AttackProfile {
    let (weapon, power) = match enemy.kind {
        EnemyKind::Rat => (WeaponKind::Knife, 8),
        EnemyKind::Brute => (WeaponKind::Mace, 42),
        EnemyKind::Hollow | EnemyKind::Warden => {
            let weapon = enemy.body.effective_weapon(&enemy.gear);
            let power = if weapon == WeaponKind::Unarmed {
                weapon.power()
            } else if enemy.kind == EnemyKind::Hollow {
                18
            } else {
                34
            };
            (weapon, power)
        }
    };
    AttackProfile {
        weapon,
        power: enemy.body.attack_power(power),
    }
}
