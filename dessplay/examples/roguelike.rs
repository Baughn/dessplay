//! Plain stdin play harness: `cargo run -p dessplay --example roguelike -- 42`.
//! Enter vi/numpad moves, `a` bandage, `e` eat, `r` rest, `>` stairs, `q` quit.
//! A line can contain several commands for reproducible scripted expeditions.

use dessplay::roguelike::{Action, HEIGHT, Point, Run, WIDTH};
use std::io::{self, BufRead};

fn show(run: &Run) {
    for y in 0..HEIGHT {
        println!(
            "{}",
            (0..WIDTH)
                .map(|x| run.glyph(Point { x, y }))
                .collect::<String>()
        );
    }
    println!(
        "Floor {} | Blood {} | Stamina {} | Nutrition {} | Bleed {} | Linen {} Food {} | Weapon {} Armor {} | {} turns",
        run.depth + 1,
        run.body.blood,
        run.body.stamina,
        run.body.hunger,
        run.body.bleeding(),
        run.bandages,
        run.food,
        run.weapon,
        run.armor,
        run.turns
    );
    for message in run.log.iter().rev().take(4).rev() {
        println!("{message}");
    }
    println!("hjkl yubn / numpad move; . wait; a bandage; e eat; r rest; > stairs; q quit");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(42);
    let mut run = Run::new(seed);
    show(&run);
    for line in io::stdin().lock().lines() {
        for key in line?.chars() {
            let action = match key {
                'h' | '4' => Action::Move(-1, 0),
                'j' | '2' => Action::Move(0, 1),
                'k' | '8' => Action::Move(0, -1),
                'l' | '6' => Action::Move(1, 0),
                'y' | '7' => Action::Move(-1, -1),
                'u' | '9' => Action::Move(1, -1),
                'b' | '1' => Action::Move(-1, 1),
                'n' | '3' => Action::Move(1, 1),
                '.' | '5' => Action::Wait,
                'a' => Action::Bandage,
                'e' => Action::Eat,
                'r' => Action::Rest,
                '<' | '>' => Action::Stairs,
                'q' => {
                    println!("{}", run.summary());
                    return Ok(());
                }
                _ => continue,
            };
            run.act(action);
            if run.is_finished() {
                break;
            }
        }
        show(&run);
        if run.is_finished() {
            println!("{}", run.summary());
            break;
        }
    }
    Ok(())
}
