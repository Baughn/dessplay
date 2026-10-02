//! The houseguest stage: pick one of Osaka's scenes and watch her do it
//! in the real UI's default layout, instead of waiting for luck.
//!
//! ```text
//! cargo run -p dessplay --example houseguest [seed]
//! ```
//!
//! ←/→ pick a scene · Enter play it · m a chat message arrives ·
//! f give her the next piece of furniture · x show why she does what she
//! does · g goodbye · n new seed ·
//! [ ] slower / faster · 1–5 make her sleepy,
//! restless, keen to tidy, mischievous, or hungry · q quit. The bar shows her
//! needs. Her decisions and their reasons are logged to
//! `houseguest-stage.log` in the working directory (`tail -f` it beside
//! the stage).
//!
//! No client, network, or IRC: only the UI, drawn locally.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use dessplay::ui::houseguest::Guest;
use dessplay::ui::houseguest::stage::{Scene, Want, stage_ui};
use dessplay::ui::layout::{LayoutBundle, Renderer};
use dessplay::ui::shell::{TERMINAL_STATE_PROLOGUE, select_image_picker};
use dessplay::ui::theme::ColorDepth;
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::CrosstermBackend;
use tuirealm::ratatui::style::{Modifier, Style};

const SPEEDS: [f64; 6] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0];

fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), terminal::LeaveAlternateScreen, cursor::Show);
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let log = std::fs::File::create("houseguest-stage.log")?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log))
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "dessplay::ui::houseguest=debug",
        ))
        .init();
    let mut seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(rand::random);

    terminal::enable_raw_mode()?;
    execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    print!("{TERMINAL_STATE_PROLOGUE}");
    io::stdout().flush()?;
    // Queried before anything else reads stdin.
    let picker = select_image_picker();

    let result = run(&mut seed, picker);
    restore();
    println!("houseguest stage: last seed {seed}");
    result
}

fn run(seed: &mut u64, picker: ratatui_image::picker::Picker) -> color_eyre::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut ui = stage_ui();
    ui.set_color_depth(ColorDepth::detect());
    let mut renderer = Renderer::new(LayoutBundle::builtin()?);
    let fresh = |seed: u64| {
        let mut guest = Guest::new(seed);
        guest.set_picker(picker.clone());
        guest
    };
    let mut guest = fresh(*seed);
    let mut selected = 1; // pull a line
    guest.cue(Scene::Arrive);

    let mut speed = 3; // 1×
    let mut chats = 0;
    let mut explain = false;
    // Her clock runs at the chosen speed.
    let mut now_ms = 0.0f64;
    let mut last = Instant::now();
    loop {
        let elapsed = last.elapsed().as_secs_f64() * 1000.0;
        last = Instant::now();
        now_ms += elapsed * SPEEDS[speed];
        let now = now_ms as u64;
        guest.advance(now);
        terminal.draw(|frame| {
            ui.draw_with_renderer(frame, &mut renderer);
            let mut view = ui.idle_view(renderer.image_regions());
            view.chat_mark.synced += chats;
            let buf = frame.buffer_mut();
            guest.paint(buf, &view, now);
            // The menu goes on the keybinding bar: she never touches it.
            let area = buf.area;
            let y = area.height.saturating_sub(1);
            let scene = Scene::ALL[selected];
            let note = match guest.cue_note() {
                Some(Ok(what)) => what.clone(),
                Some(Err(why)) => format!("✗ {why}"),
                None => String::new(),
            };
            let mood = guest.mood().unwrap_or_default();
            let menu = format!(
                " ◀ {} ▶  Enter play · f furnish · m chat · x why · g bye · n seed {} · [ ] {}× · 1-5 needs · q │ {} │ {}",
                scene.name(),
                seed,
                SPEEDS[speed],
                mood,
                note
            );
            let blank = " ".repeat(usize::from(area.width));
            // Why she's doing what she does, on the row above.
            if explain && let Some(why) = guest.explain() {
                let y = y.saturating_sub(1);
                buf.set_string(0, y, &blank, Style::reset());
                buf.set_stringn(0, y, &why, usize::from(area.width), Style::reset());
            }
            buf.set_string(0, y, &blank, Style::reset());
            buf.set_stringn(
                0,
                y,
                &menu,
                usize::from(area.width),
                Style::reset().add_modifier(Modifier::REVERSED),
            );
        })?;

        let wait = guest
            .next_tick(now)
            .map_or(Duration::from_millis(250), |d| d.div_f64(SPEEDS[speed]))
            .clamp(Duration::from_millis(1), Duration::from_millis(250));
        if !event::poll(wait)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
            KeyCode::Left => selected = (selected + Scene::ALL.len() - 1) % Scene::ALL.len(),
            KeyCode::Right => selected = (selected + 1) % Scene::ALL.len(),
            KeyCode::Enter | KeyCode::Char(' ') => guest.cue(Scene::ALL[selected]),
            KeyCode::Char('m') => chats += 1,
            KeyCode::Char('x') => explain = !explain,
            KeyCode::Char('f') => {
                if let Some(item) = guest.wishlist() {
                    guest.give(item);
                }
            }
            KeyCode::Char('g') => guest.activity(now),
            KeyCode::Char('n') => {
                *seed = rand::random();
                guest = fresh(*seed);
                guest.cue(Scene::ALL[selected]);
            }
            KeyCode::Char('1') => guest.press(Want::Sleepy),
            KeyCode::Char('2') => guest.press(Want::Restless),
            KeyCode::Char('3') => guest.press(Want::Tidy),
            KeyCode::Char('4') => guest.press(Want::Mischief),
            KeyCode::Char('5') => guest.press(Want::Hungry),
            KeyCode::Char('[') => speed = speed.saturating_sub(1),
            KeyCode::Char(']') => speed = (speed + 1).min(SPEEDS.len() - 1),
            _ => {}
        }
    }
}
