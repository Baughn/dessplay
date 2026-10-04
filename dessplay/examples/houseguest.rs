//! The houseguest stage: pick one of Osaka's scenes and watch her do it
//! in the real UI's default layout, instead of waiting for luck.
//!
//! ```text
//! cargo run -p dessplay --example houseguest [seed]
//! ```
//!
//! ←/→ pick a scene · Enter play it · m a chat message arrives · ? a
//! chat message asking her something arrives (as the sata andagi plays,
//! she answers it) · f give her the next piece of furniture · x show why she does what she
//! does · v her next mood (lazy, busy, dreamy) · g goodbye · n new seed ·
//! [ ] slower / faster · t skip her clock to the next change in her
//! routine · d the next stage date (her calendar's days and seasons,
//! then today's, then none) · 1–6 make her sleepy,
//! restless, keen to tidy, mischievous, hungry, or keen to put right what
//! she's felt is wrong with her home · q quit. The bar shows her game
//! time and the part of her day it is (her clock runs at the stage's
//! speed times six), the stage date, and her needs. Her decisions and their reasons are logged to
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
use dessplay::ui::shell::{TERMINAL_STATE_PROLOGUE, select_image_picker, today};
use dessplay::ui::theme::ColorDepth;
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::CrosstermBackend;
use tuirealm::ratatui::style::{Modifier, Style};

const SPEEDS: [f64; 6] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0];

/// The stage's dates (`d` cycles them): her calendar's days and seasons
/// (exams, hay fever, summer, panic week, December), then today's, then
/// none.
#[derive(Clone, Copy)]
enum StageDate {
    On(u32, u32),
    Today,
    Off,
}

const DATES: [StageDate; 16] = [
    // Her starter calendar: what each date owes her, and her seasons.
    StageDate::On(1, 1),
    StageDate::On(1, 2),
    StageDate::On(2, 3),
    StageDate::On(2, 10),
    StageDate::On(3, 20),
    StageDate::On(4, 8),
    StageDate::On(7, 7),
    StageDate::On(7, 25),
    StageDate::On(8, 28),
    StageDate::On(9, 30),
    StageDate::On(10, 31),
    StageDate::On(12, 10),
    StageDate::On(12, 24),
    StageDate::On(12, 31),
    StageDate::Today,
    StageDate::Off,
];

impl StageDate {
    fn date(self) -> Option<chrono::NaiveDate> {
        match self {
            StageDate::On(month, day) => {
                use chrono::Datelike;
                let year = today().map_or(2026, |d| d.year());
                chrono::NaiveDate::from_ymd_opt(year, month, day)
            }
            StageDate::Today => today(),
            StageDate::Off => None,
        }
    }

    fn label(self) -> String {
        let date = self
            .date()
            .map_or_else(|| "no date".to_owned(), |d| d.format("%b %-d").to_string());
        match self {
            StageDate::Today => format!("{date} (today)"),
            _ => date,
        }
    }
}

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
    // The newest of those asks her something.
    let mut asked = false;
    let mut explain = false;
    // The real date she's told: today's to begin with.
    let mut date = DATES.len() - 2;
    // Her clock runs at the chosen speed.
    let mut now_ms = 0.0f64;
    let mut last = Instant::now();
    loop {
        let elapsed = last.elapsed().as_secs_f64() * 1000.0;
        last = Instant::now();
        now_ms += elapsed * SPEEDS[speed];
        let now = now_ms as u64;
        guest.set_date(DATES[date].date());
        guest.advance(now);
        terminal.draw(|frame| {
            ui.draw_with_renderer(frame, &mut renderer);
            let mut view = ui.idle_view(renderer.image_regions());
            view.chat_mark.synced += chats;
            if chats > 0 {
                view.chat_mark.synced_asks = asked;
            }
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
            // What she's playing, and which key of it.
            let playing = guest
                .playing(now)
                .map_or_else(String::new, |p| format!(" │ ▶ {p}"));
            // Her game time, and the stage date.
            let clock = guest
                .clock_label(now)
                .unwrap_or_else(|| "not met yet".to_owned());
            let menu = format!(
                " ◀ {} ▶  Enter play · f furnish · m chat · ? ask · x why · v mood · g bye · n seed {} · [ ] {}× · t skip · d date · 1-6 needs · q │ {} · {} │ {} │ {}{}",
                scene.name(),
                seed,
                SPEEDS[speed],
                clock,
                DATES[date].label(),
                mood,
                note,
                playing
            );
            let blank = " ".repeat(usize::from(area.width));
            // Why she's doing what she does, on the row above.
            // With her rooms, and what each is, and the rules of her
            // home they break (`*`: she has felt it this visit).
            if explain && let Some(why) = guest.explain() {
                let y = y.saturating_sub(1);
                let mut why = format!("{why} │ rooms: {}", guest.rooms());
                let broken = guest.broken();
                if !broken.is_empty() {
                    why.push_str(&format!(" │ broken: {broken}"));
                }
                // And how she'd put right the one she would mend.
                let repair = guest.repair();
                if !repair.is_empty() {
                    why.push_str(&format!(" │ mend: {repair}"));
                }
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
            KeyCode::Char('m') => {
                chats += 1;
                asked = false;
            }
            KeyCode::Char('?') => {
                chats += 1;
                asked = true;
            }
            KeyCode::Char('x') => explain = !explain,
            KeyCode::Char('v') => guest.next_mood(),
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
            KeyCode::Char('6') => guest.press(Want::Nesting),
            KeyCode::Char('t') => guest.skip_clock(now),
            KeyCode::Char('d') => date = (date + 1) % DATES.len(),
            KeyCode::Char('[') => speed = speed.saturating_sub(1),
            KeyCode::Char(']') => speed = (speed + 1).min(SPEEDS.len() - 1),
            _ => {}
        }
    }
}
