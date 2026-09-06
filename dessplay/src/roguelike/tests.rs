#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use proptest::prelude::*;

fn scripted(code: u8) -> Action {
    match code % 23 {
        0 => Action::Move(1, 0),
        1 => Action::Move(-1, 0),
        2 => Action::Move(0, 1),
        3 => Action::Move(0, -1),
        4 => Action::Move(1, 1),
        5 => Action::Move(-1, -1),
        6 => Action::Sprint(1, 0),
        7 => Action::Sprint(-1, 0),
        8 => Action::Sprint(0, 1),
        9 => Action::Sprint(0, -1),
        10 => Action::Attack(1, 0),
        11 => Action::Attack(0, -1),
        12 => Action::Wait,
        13 => Action::Rest,
        14 => Action::Bandage,
        15 => Action::Eat,
        16 => Action::Stairs,
        17 => Action::Interact,
        18 => Action::SwapWeapon,
        19 => Action::Equip(0),
        20 => Action::CloseDoor(1, 0),
        21 => Action::Treat(0),
        _ => Action::Treat(6),
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]
    #[test]
    fn walking_never_restores_breath(seed in any::<u64>()) {
        let mut run=Run::new(seed);
        run.floors[0].enemies.clear();run.body.stamina=20;
        let (dx,dy)=[(1,0),(-1,0),(0,1),(0,-1)].into_iter().find(|&(dx,dy)|run.floor().step_allowed(run.position,run.position.offset(dx,dy))).unwrap();
        run.act(Action::Move(dx,dy));
        prop_assert!(run.body.stamina<=20);
    }
    #[test]
    fn arbitrary_actions_and_saves_preserve_exact_future(seed in any::<u64>(),codes in prop::collection::vec(any::<u8>(),0..200),split in any::<usize>()) {
        let mut run=Run::new(seed);run.validate().unwrap();
        let split=split%(codes.len()+1);
        for &code in &codes[..split] {run.act(scripted(code));run.validate().unwrap();}
        let mut resumed:Run=serde_json::from_slice(&serde_json::to_vec(&run).unwrap()).unwrap();
        for &code in &codes[split..] {
            prop_assert_eq!(run.act(scripted(code)),resumed.act(scripted(code)));
            prop_assert_eq!(&run,&resumed);run.validate().unwrap();
        }
    }
    #[test]
    fn corrupt_coordinates_are_rejected_without_panicking(x in any::<i32>(),y in any::<i32>()) {
        let point=Point{x,y};prop_assume!(point.index().is_none());
        let mut run=Run::new(42);run.position=point;prop_assert!(run.validate().is_err());
        let mut run=Run::new(42);run.floors[0].entrance=point;prop_assert!(run.validate().is_err());
    }
    #[test]
    fn finished_runs_are_immutable(seed in any::<u64>(),codes in prop::collection::vec(any::<u8>(),0..100)) {
        let mut run=Run::new(seed);run.act(Action::Stairs);prop_assert_eq!(&run.outcome,&Outcome::Escaped);
        let before=run.clone();for code in codes {prop_assert!(!run.act(scripted(code)));}prop_assert_eq!(run,before);
    }
}
#[test]
fn early_escape_is_a_victory_and_ember_is_explicit() {
    let mut run = Run::new(20260906);
    assert!(run.act(Action::Stairs));
    assert_eq!(run.outcome, Outcome::Escaped);
    assert!(!run.has_ember());
    assert!(run.summary().contains("escaped alive without"));
    run.validate().unwrap();
    let mut run = Run::new(7);
    run.depth = 4;
    run.deepest = 5;
    run.position = run.floor().exit;
    run.reveal();
    assert!(!run.has_ember());
    run.act(Action::Interact);
    assert!(run.has_ember());
    assert!(!run.is_finished());
    run.validate().unwrap();
}
#[test]
fn sprint_uses_half_the_time_and_no_teleportation() {
    let mut run = Run::new(7);
    run.floors[0].enemies.clear();
    let (dx, dy) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find(|&(dx, dy)| {
            run.floor()
                .step_allowed(run.position, run.position.offset(dx, dy))
                && run.tile(run.position.offset(dx, dy)) == Tile::Floor
        })
        .unwrap();
    let mut sprint = run.clone();
    let walking = run.step(Action::Move(dx, dy));
    let running = sprint.step(Action::Sprint(dx, dy));
    assert_eq!(run.position, sprint.position);
    assert_eq!(walking.elapsed, 100);
    assert_eq!(running.elapsed, 50);
    assert!(sprint.body.stamina < run.body.stamina);
}
#[test]
fn changing_unseen_terrain_does_not_rewrite_memory() {
    let mut run = Run::new(42);
    let i = run.floor().visible.iter().position(|v| !*v).unwrap();
    run.floors[0].remembered[i] = Some(Tile::Wall);
    let before = run.view();
    run.floors[0].tiles[i] = Tile::Floor;
    let after = run.view();
    assert_eq!(before.cells, after.cells);
    assert_eq!(after.cells[i].terrain, Some(Tile::Wall));
}
#[test]
fn awakened_save_resumes_every_warning_and_floor_schedule() {
    let mut run = Run::new(7);
    run.depth = 4;
    run.deepest = 5;
    run.position = run.floor().exit;
    run.reveal();
    run.act(Action::Interact);
    let mut resumed: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    for _ in 0..100 {
        run.act(Action::Wait);
        resumed.act(Action::Wait);
        assert_eq!(run, resumed);
        run.validate().unwrap();
    }
}

#[test]
fn named_enemy_windups_are_observed_and_interrupt_recovery() {
    // Sample honest movement/waits; an enemy may see us outside our circular
    // field of view, but cannot donate its private intent to our journal.
    for seed in 0..20 {
        let mut run = Run::new(seed);
        for code in (0..100).map(|i| {
            if i < 10 {
                1
            } else {
                ((i * 17 + seed) % 13) as u8
            }
        }) {
            let id = run.journal.last().map_or(0, |e| e.id);
            run.act(scripted(code));
            let view = run.view();
            for event in view
                .journal
                .iter()
                .filter(|e| e.id > id && e.text == "The hollow pilgrim draws breath for a call.")
            {
                assert!(
                    view.enemies.iter().any(|e| e.name == "hollow pilgrim"),
                    "seed {seed}, time {}, {event:?}",
                    view.time
                );
                assert!(view.danger, "perceived windup must interrupt recovery");
            }
        }
    }
}

#[test]
fn playtest_call_pursuit_never_stacks_creatures() {
    #[derive(serde::Deserialize)]
    struct Replay {
        seed: u64,
        actions: Vec<Action>,
    }
    let replays: Vec<Replay> =
        serde_json::from_str(include_str!("replay-regressions.json")).unwrap();
    assert_eq!(replays.len(), 3);
    for replay in replays {
        let mut run = Run::new(replay.seed);
        for action in replay.actions {
            run.act(action);
            assert!(
                run.validate().is_ok(),
                "seed {}, action {action:?}, time {}, error {:?}",
                replay.seed,
                run.time,
                run.validate()
            );
        }
    }
}

#[test]
fn dropped_equipment_cannot_be_closed_inside_a_door() {
    let mut run = Run::new(42);
    run.floors[0].enemies.clear();
    let (dx, dy) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find(|&(dx, dy)| {
            run.floor()
                .step_allowed(run.position, run.position.offset(dx, dy))
        })
        .unwrap();
    let p = run.position.offset(dx, dy);
    run.floors[0].tiles[p.index().unwrap()] = Tile::DoorOpen;
    run.floors[0].loot.push(Loot {
        position: p,
        kind: LootKind::Weapon(WeaponKind::Knife),
    });
    run.validate().unwrap();
    run.act(Action::CloseDoor(dx, dy));
    run.validate().unwrap();
    assert_eq!(run.tile(p), Tile::DoorOpen);
}

#[test]
fn invalid_directions_do_not_mutate_a_committed_run() {
    let mut run = Run::new(42);
    run.act(Action::Wait);
    for action in [
        Action::Move(0, 0),
        Action::Sprint(9, 9),
        Action::Attack(0, 0),
        Action::CloseDoor(9, 9),
    ] {
        let before = run.clone();
        assert!(!run.act(action));
        assert_eq!(run, before);
    }
}

fn arena() -> Run {
    let mut run = Run::new(42);
    let floor = &mut run.floors[0];
    floor.enemies.clear();
    floor.loot.clear();
    floor.caverns.clear();
    for y in 1..HEIGHT - 1 {
        for x in 1..WIDTH - 1 {
            floor.tiles[Point { x, y }.index().unwrap()] = Tile::Floor;
        }
    }
    floor.entrance = Point { x: 1, y: 1 };
    floor.exit = Point {
        x: WIDTH - 2,
        y: HEIGHT - 2,
    };
    floor.tiles[floor.entrance.index().unwrap()] = Tile::Up;
    floor.tiles[floor.exit.index().unwrap()] = Tile::Down;
    floor.next_enemy_id = 10;
    run.position = Point { x: 20, y: 18 };
    run.reveal();
    run
}

#[test]
fn a_complete_stair_journey_banks_the_ember_and_freezes_its_ending() {
    // A short, connected scenario isolates the complete descent/return contract
    // from difficulty tuning. It is not evidence of winning a generated dungeon.
    let mut run = arena();
    let up = Point { x: 20, y: 18 };
    let down = up.offset(1, 0);
    let mut floor = run.floors[0].clone();
    floor.tiles[floor.entrance.index().unwrap()] = Tile::Floor;
    floor.tiles[floor.exit.index().unwrap()] = Tile::Floor;
    floor.entrance = up;
    floor.exit = down;
    floor.tiles[up.index().unwrap()] = Tile::Up;
    floor.tiles[down.index().unwrap()] = Tile::Down;
    floor.remembered.fill(None);
    floor.visible.fill(false);
    run.floors = vec![floor; FLOOR_COUNT];
    run.floors[FLOOR_COUNT - 1].tiles[down.index().unwrap()] = Tile::Floor;
    run.floors[FLOOR_COUNT - 1].loot.push(Loot {
        position: down,
        kind: LootKind::Relic,
    });
    run.position = up;
    run.reveal();
    run.validate().unwrap();
    for depth in 0..FLOOR_COUNT - 1 {
        assert!(run.act(Action::Move(1, 0)));
        assert!(run.act(Action::Stairs));
        assert_eq!(run.depth, depth + 1);
    }
    run.act(Action::Move(1, 0));
    assert!(!run.has_ember());
    run.act(Action::Interact);
    assert!(run.has_ember());
    let mut resumed: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    for depth in (0..FLOOR_COUNT).rev() {
        for action in [Action::Move(-1, 0), Action::Stairs] {
            assert!(run.act(action));
            assert!(resumed.act(action));
            assert_eq!(run, resumed);
            run.validate().unwrap();
        }
        assert_eq!(run.depth, depth.saturating_sub(1));
    }
    assert_eq!(run.outcome, Outcome::Escaped);
    assert!(run.score() >= 10_500);
    assert!(run.summary().contains("escaped with the ember"));
    assert!(!run.epilogue.is_empty());
    let ending = run.clone();
    assert!(!run.act(Action::Wait));
    assert_eq!(run, ending);
}

#[test]
fn pursuit_respects_an_occupied_goal_even_without_sight_of_player() {
    for kind in [
        EnemyKind::Rat,
        EnemyKind::Hollow,
        EnemyKind::Warden,
        EnemyKind::Brute,
    ] {
        let mut run = arena();
        let goal = Point { x: 10, y: 8 };
        let mut caller = Enemy::new(0, EnemyKind::Hollow, goal, 0);
        caller.intent = EnemyIntent::Recovering { until: 10_000 };
        caller.next_action = 10_000;
        let mut pursuer = Enemy::new(1, kind, goal.offset(1, 0), 0);
        pursuer.target = Some(goal);
        run.floors[0].enemies = vec![caller, pursuer];
        for _ in 0..10 {
            run.act(Action::Wait);
            run.validate().unwrap();
        }
    }
}

#[test]
fn an_injuring_hit_interrupts_a_pilgrims_call() {
    let mut run = arena();
    run.gear.active = WeaponKind::Spear;
    let mut caller = Enemy::new(0, EnemyKind::Hollow, run.position.offset(2, 0), 0);
    caller.intent = EnemyIntent::Calling { at: 200 };
    caller.next_action = 200;
    run.floors[0].enemies.push(caller);
    run.reveal();
    run.act(Action::Attack(1, 0));
    assert!(matches!(
        run.floor().enemies[0].intent,
        EnemyIntent::Recovering { .. }
    ));
    assert!(
        run.journal
            .iter()
            .any(|e| e.text == "The pilgrim's call breaks into silence.")
    );
    run.validate().unwrap();
}

#[test]
fn review_call_responders_approach_an_occupied_goal_without_entering_it() {
    for kind in [
        EnemyKind::Rat,
        EnemyKind::Hollow,
        EnemyKind::Warden,
        EnemyKind::Brute,
    ] {
        let mut run = arena();
        let goal = Point { x: 10, y: 8 };
        let start = goal.offset(0, -5);
        let mut caller = Enemy::new(0, EnemyKind::Hollow, goal, 0);
        caller.intent = EnemyIntent::Calling { at: 50 };
        caller.next_action = 50;
        let mut responder = Enemy::new(1, kind, start, 0);
        responder.next_action = 50;
        run.floors[0].enemies = vec![caller, responder];
        run.act(Action::Wait);
        let responder = run.floor().enemies.iter().find(|e| e.id == 1).unwrap();
        assert!(
            responder.position.distance(goal) < start.distance(goal),
            "{kind:?} did not approach the cry"
        );
        for _ in 0..8 {
            run.act(Action::Wait);
            run.validate().unwrap();
        }
        let responder = run.floor().enemies.iter().find(|e| e.id == 1).unwrap();
        assert_eq!(responder.position.distance(goal), 1);
        assert!(run.floor().step_allowed(responder.position, goal));
    }
}

#[test]
fn review_opening_and_closing_doors_both_respect_diagonal_corners() {
    for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
        for block_x in [true, false] {
            for blocker in [Tile::Wall, Tile::DoorClosed] {
                let mut run = arena();
                let door = run.position.offset(dx, dy);
                let flank = if block_x {
                    run.position.offset(dx, 0)
                } else {
                    run.position.offset(0, dy)
                };
                run.floors[0].tiles[flank.index().unwrap()] = blocker;
                for (tile, action) in [
                    (Tile::DoorOpen, Action::CloseDoor(dx, dy)),
                    (Tile::DoorClosed, Action::Move(dx, dy)),
                ] {
                    run.floors[0].tiles[door.index().unwrap()] = tile;
                    run.reveal();
                    let before = run.clone();
                    assert!(
                        !run.step(action).changed,
                        "{action:?} reached through {blocker:?}"
                    );
                    assert_eq!(run, before);
                }
                run.floors[0].tiles[flank.index().unwrap()] = Tile::Floor;
                run.floors[0].tiles[door.index().unwrap()] = Tile::DoorOpen;
                assert_eq!(run.step(Action::CloseDoor(dx, dy)).elapsed, 100);
                assert_eq!(run.tile(door), Tile::DoorClosed);
                assert_eq!(run.step(Action::Move(dx, dy)).elapsed, 100);
                assert_eq!(run.tile(door), Tile::DoorOpen);
            }
        }
    }
}

#[test]
fn review_blind_reach_attacks_give_contact_without_unseen_anatomy() {
    let mut run = arena();
    run.gear.active = WeaponKind::Spear;
    run.body.eyes = [0, 0];
    let mut caller = Enemy::new(0, EnemyKind::Hollow, run.position.offset(2, 0), 0);
    caller.intent = EnemyIntent::Calling { at: 200 };
    caller.next_action = 200;
    let before_body = caller.body.clone();
    run.floors[0].enemies.push(caller);
    run.reveal();
    assert!(run.view().enemies.is_empty());
    let journal_start = run.journal.len();
    assert_eq!(run.step(Action::Attack(1, 0)).elapsed, 150);
    assert_ne!(run.floor().enemies[0].body, before_body);
    assert!(matches!(
        run.floor().enemies[0].intent,
        EnemyIntent::Recovering { .. }
    ));
    let combat: Vec<_> = run.journal[journal_start..]
        .iter()
        .filter(|e| e.kind == EventKind::Combat)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        combat,
        [
            "Your spear strikes something beyond sight.",
            "The call breaks off."
        ]
    );
    run.validate().unwrap();
}

#[test]
fn review_dodged_heavy_attacks_spend_breath_and_enter_normal_recovery() {
    for kind in [EnemyKind::Warden, EnemyKind::Brute] {
        let mut run = arena();
        let mut attacker = Enemy::new(0, kind, run.position.offset(1, 0), 0);
        attacker.next_action = 50;
        run.floors[0].enemies.push(attacker);
        run.reveal();
        run.act(Action::Wait);
        let before = &run.floor().enemies[0];
        let EnemyIntent::Strike { target, at } = before.intent else {
            panic!("heavy attack was not telegraphed")
        };
        assert_eq!(target, run.position);
        let breath = before.body.stamina;
        let mut body = run.body.clone();
        body.tick();
        run.act(Action::Move(-1, 0));
        let attacker = &run.floor().enemies[0];
        assert_eq!(run.body, body, "the dodged blow must miss");
        assert_eq!(attacker.body.stamina, breath - 20);
        assert_eq!(attacker.intent, EnemyIntent::Recovering { until: at + 200 });
        assert_eq!(attacker.next_action, at + 200);
        run.validate().unwrap();
    }
}
