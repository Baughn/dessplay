//! Observation-only stdin play harness. `--json` emits the same player view as the TUI.
use dessplay::roguelike::{Action, EventKind, HEIGHT, Point, Run, RunView, WIDTH};
use std::io::{self, BufRead, Write};

fn direction(c: char) -> Option<(i32, i32)> {
    match c.to_ascii_lowercase() {
        'h' | '4' => Some((-1, 0)),
        'j' | '2' => Some((0, 1)),
        'k' | '8' => Some((0, -1)),
        'l' | '6' => Some((1, 0)),
        'y' | '7' => Some((-1, -1)),
        'u' | '9' => Some((1, -1)),
        'b' | '1' => Some((-1, 1)),
        'n' | '3' => Some((1, 1)),
        _ => None,
    }
}
fn show(view: &RunView, json: bool, since: u64) {
    if json {
        if let Ok(text) = serde_json::to_string(view) {
            println!("{text}");
        }
        return;
    }
    println!("   0000000000111111111122222222223333333333444444444");
    println!("   0123456789012345678901234567890123456789012345678");
    for y in 0..HEIGHT {
        println!(
            "{y:02} {}",
            (0..WIDTH)
                .map(|x| view.glyph(Point { x, y }))
                .collect::<String>()
        );
    }
    println!(
        "Floor {} | ({},{}) | Blood {} Breath {} Nutrition {} Bleed {} Pain {} | Linen {} Splints {} Food {} | {} actions / {} time | {} points",
        view.depth + 1,
        view.position.x,
        view.position.y,
        view.body.blood,
        view.body.stamina,
        view.body.hunger,
        view.body.bleeding(),
        view.body.pain(),
        view.supplies.bandages,
        view.supplies.splints,
        view.supplies.food,
        view.turns,
        view.time,
        view.score
    );
    println!("{}", view.objective());
    println!("{}", view.gear.lines().join(" | "));
    println!("{}", view.movement_summary());
    for line in view.body.condition_lines() {
        println!("  {line}");
    }
    for enemy in &view.enemies {
        println!(
            "{} ({},{}): {}; {}",
            enemy.name, enemy.position.x, enemy.position.y, enemy.intent, enemy.condition
        );
    }
    for (i, item) in view.ground.iter().enumerate() {
        println!("Ground {i}: {item:?} (equip {i}, or g to activate)");
    }
    for event in view.journal.iter().filter(|e| e.id > since) {
        println!("[{}] {}", event.time, event.text);
    }
    println!(
        "hjkl/yubn move; uppercase sprint; f+direction attack; c+direction close; . wait; r care; a bandage; e eat; g interact; x swap; <> stairs; i/v/p inspect; equip N; treat N; q quit"
    );
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seed = 42;
    let mut json = false;
    let mut transcript = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--transcript" => {
                let path = args.next().ok_or("--transcript requires a path")?;
                transcript = Some(std::fs::File::create(path)?);
            }
            _ => seed = arg.parse()?,
        }
    }
    let mut run = Run::new(seed);
    if let Some(file) = &mut transcript {
        writeln!(file, "{}", serde_json::json!({"seed":seed}))?;
    }
    show(&run.view(), json, 0);
    io::stdout().flush()?;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let before = run.journal.last().map_or(0, |e| e.id);
        if line.trim() == "q" {
            if !json {
                println!("{}", run.summary());
            }
            break;
        }
        let actions = if let Some(index) = line.strip_prefix("equip ") {
            vec![Action::Equip(index.trim().parse()?)]
        } else if let Some(index) = line.strip_prefix("treat ") {
            vec![Action::Treat(index.trim().parse()?)]
        } else {
            let mut actions = Vec::new();
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                let action = if let Some((dx, dy)) = direction(c) {
                    if c.is_ascii_uppercase() {
                        Some(Action::Sprint(dx, dy))
                    } else {
                        Some(Action::Move(dx, dy))
                    }
                } else {
                    match c {
                        'f' | 'c' => chars.next().and_then(direction).map(|(dx, dy)| {
                            if c == 'f' {
                                Action::Attack(dx, dy)
                            } else {
                                Action::CloseDoor(dx, dy)
                            }
                        }),
                        '.' | '5' => Some(Action::Wait),
                        'r' => Some(Action::Rest),
                        'a' => Some(Action::Bandage),
                        'e' => Some(Action::Eat),
                        'g' => Some(Action::Interact),
                        'x' => Some(Action::SwapWeapon),
                        '<' | '>' => Some(Action::Stairs),
                        _ => None,
                    }
                };
                if let Some(action) = action {
                    actions.push(action);
                }
            }
            actions
        };
        for action in actions {
            let old = run.view();
            let old_id = run.journal.last().map_or(0, |e| e.id);
            let result = run.step(action);
            run.validate()
                .map_err(|s| format!("seed {seed}, action {action:?}: {s}"))?;
            let view = run.view();
            if let Some(file) = &mut transcript {
                writeln!(
                    file,
                    "{}",
                    serde_json::json!({"action":action,"time":view.time,"position":view.position,"body":view.body,"supplies":view.supplies,"events":view.journal.iter().filter(|e|e.id>old_id).collect::<Vec<_>>()})
                )?;
                file.flush()?;
            }
            let new_danger = old.depth != view.depth
                || view
                    .enemies
                    .iter()
                    .any(|e| !old.enemies.iter().any(|o| o.id == e.id))
                || view.journal.iter().any(|e| {
                    e.id > old_id && matches!(e.kind, EventKind::Danger | EventKind::Injury)
                });
            if run.is_finished() || new_danger || !result.changed {
                break;
            }
        }
        show(
            &run.view(),
            json,
            if line.trim() == "p" { 0 } else { before },
        );
        io::stdout().flush()?;
        if run.is_finished() {
            if !json {
                println!("{}", run.summary());
            }
            break;
        }
    }
    Ok(())
}
