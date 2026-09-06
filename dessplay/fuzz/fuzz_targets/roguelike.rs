//! Arbitrary real action sequences and save/resume, plus independently exercised
//! world crisis transitions so coverage does not depend on random play reaching floor5.
#![no_main]
use dessplay::roguelike::{Action, Point, Rng, Run, world};
use libfuzzer_sys::fuzz_target;
#[derive(Debug, arbitrary::Arbitrary)]
struct Input {
    seed: u64,
    actions: Vec<u8>,
    split: u16,
}
fuzz_target!(|input: Input| {
    let mut run = Run::new(input.seed);
    let mut resumed = run.clone();
    let mut rng = Rng(input.seed);
    let mut floor = world::generate(&mut rng, (input.seed % 5) as usize, false);
    world::awaken(&mut floor);
    for (i, code) in input.actions.iter().take(512).enumerate() {
        let action = match code % 20 {
            0 => Action::Move(-1, 0),
            1 => Action::Move(1, 0),
            2 => Action::Move(0, -1),
            3 => Action::Move(0, 1),
            4 => Action::Move(1, 1),
            5 => Action::Move(-1, -1),
            6 => Action::Sprint(1, 0),
            7 => Action::Sprint(0, -1),
            8 => Action::Attack(1, 0),
            9 => Action::Attack(0, -1),
            10 => Action::Rest,
            11 => Action::Wait,
            12 => Action::Bandage,
            13 => Action::Eat,
            14 => Action::Stairs,
            15 => Action::Interact,
            16 => Action::SwapWeapon,
            17 => Action::Equip(usize::from(*code) / 20),
            18 => Action::Treat(usize::from(*code) / 20),
            _ => Action::CloseDoor(0, 1),
        };
        assert_eq!(run.act(action), resumed.act(action));
        assert_eq!(run, resumed);
        run.validate().unwrap();
        if i % 16 == usize::from(input.split) % 16 {
            resumed = serde_json::from_slice(&serde_json::to_vec(&run).unwrap()).unwrap();
            resumed.validate().unwrap();
        }
        let _ = run.view();
        // Each 100-unit environmental step is legal independent of player input.
        floor.time += 100;
        let entrance = floor.entrance;
        let _ = world::advance(&mut floor, &mut rng, entrance);
        floor.enemies.retain(|e| !e.body.is_dead());
        floor.validate().unwrap();
        assert!(floor.tile(entrance).walkable());
        let _: Option<usize> = Point {
            x: i32::from(*code),
            y: -1,
        }
        .index();
    }
});
