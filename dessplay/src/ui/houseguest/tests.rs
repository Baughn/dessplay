#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use proptest::prelude::*;
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;
use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::{Position, Rect};
use tuirealm::ratatui::style::Style;

use super::osaka::{self, Osaka};
use super::terrain::{Route, Terrain};
use super::*;
use crate::ui::app::Ui;
use crate::ui::layout::{LayoutBundle, Renderer};

use super::stage::{DELAY, Scene, chatty_ui, real_ui, stage_ui};

fn view(protected: Vec<Rect>) -> IdleView {
    IdleView {
        delay: Some(DELAY),
        busy: None,
        chat_mark: ChatMark::default(),
        chat: Rect::new(0, 0, 30, 20),
        protected,
        truecolor: false,
    }
}

/// A simple two-pane screen: a tall left box and two stacked right
/// boxes, with a protected strip along the bottom.
fn rooms(width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let boxes = [
        Rect::new(0, 0, width / 2, height - 3),
        Rect::new(width / 2, 0, width - width / 2, (height - 3) / 2),
        Rect::new(
            width / 2,
            (height - 3) / 2,
            width - width / 2,
            height - 3 - (height - 3) / 2,
        ),
    ];
    for area in boxes {
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            area,
            &mut buf,
        );
    }
    buf.set_string(0, height - 2, "Tab Next pane | Enter Send", Style::new());
    buf
}

fn bottom_strip(width: u16, height: u16) -> Vec<Rect> {
    vec![Rect::new(0, height - 3, width, 3)]
}

/// Paint one frame over a copy of `real`.
fn paint(guest: &mut Guest, real: &Buffer, view: &IdleView, now: u64) -> Buffer {
    let mut buf = real.clone();
    guest.paint(&mut buf, view, now);
    buf
}

/// Run ticks as the shell would, up to `until`, painting after each.
fn run(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64, until: u64) -> Buffer {
    let mut now = from;
    let mut last = paint(guest, real, view, now);
    while now < until {
        let step = guest
            .next_tick(now)
            .map_or(until - now, |d| d.as_millis() as u64)
            .clamp(1, until - now);
        now += step;
        if guest.advance(now) {
            last = paint(guest, real, view, now);
        }
    }
    last
}

#[test]
fn she_arrives_only_after_the_idle_delay() {
    let real = rooms(80, 24);
    let view = view(bottom_strip(80, 24));
    let mut guest = Guest::new(7);
    assert_eq!(paint(&mut guest, &real, &view, 0), real);
    assert_eq!(guest.next_tick(0), Some(DELAY));
    assert!(!guest.advance(4_999));
    assert!(guest.advance(5_000));
    let frame = paint(&mut guest, &real, &view, 5_000);
    assert!(guest.present());
    // Walking in from an edge may still be off-screen; give her time.
    let later = run(&mut guest, &real, &view, 5_000, 20_000);
    assert!(guest.present());
    assert!(frame != real || later != real, "she is visible");
}

#[test]
fn a_busy_client_gets_no_visit() {
    let real = rooms(80, 24);
    for busy in [Busy::Playing, Busy::Overlay, Busy::Selection] {
        let view = IdleView {
            busy: Some(busy),
            ..view(bottom_strip(80, 24))
        };
        let mut guest = Guest::new(7);
        paint(&mut guest, &real, &view, 0);
        assert_eq!(guest.next_tick(0), None);
        assert!(!guest.advance(3_600_000));
        assert_eq!(paint(&mut guest, &real, &view, 3_600_000), real);
    }
}

#[test]
fn switched_off_means_no_visit_and_no_goodbye() {
    let real = rooms(80, 24);
    let on = view(bottom_strip(80, 24));
    let mut guest = Guest::new(3);
    run(&mut guest, &real, &on, 0, 30_000);
    assert!(guest.present());
    let off = IdleView { delay: None, ..on };
    assert_eq!(paint(&mut guest, &real, &off, 30_001), real);
    assert!(!guest.present());
}

#[test]
fn local_activity_dissolves_back_to_the_real_frame_in_time() {
    let real = rooms(80, 24);
    let view = view(bottom_strip(80, 24));
    let mut guest = Guest::new(11);
    let before = run(&mut guest, &real, &view, 0, 60_000);
    assert!(guest.present());
    assert_ne!(before, real, "she is on screen before the key press");
    guest.activity(60_000);
    let during = paint(&mut guest, &real, &view, 60_000);
    assert_ne!(during, real, "the dissolve starts from her");
    let after = run(
        &mut guest,
        &real,
        &view,
        60_000,
        60_000 + dissolve::DURATION_MS,
    );
    assert_eq!(after, real);
    assert!(!guest.present());
    // The idle timer restarted at the key press.
    let end = 60_000 + dissolve::DURATION_MS;
    assert_eq!(
        guest.next_tick(end),
        Some(DELAY - Duration::from_millis(dissolve::DURATION_MS))
    );
}

#[test]
fn a_chat_message_makes_her_look_not_leave() {
    let real = rooms(80, 24);
    let quiet = view(bottom_strip(80, 24));
    let mut guest = Guest::new(5);
    run(&mut guest, &real, &quiet, 0, 60_000);
    // Wait until she's standing on screen.
    let mut now = 60_000;
    let mut frame = paint(&mut guest, &real, &quiet, now);
    let chatty = IdleView {
        chat_mark: ChatMark {
            synced: 1,
            newest: Some(42),
            irc: 0,
        },
        ..quiet.clone()
    };
    for _ in 0..40 {
        now += 500;
        frame = paint(&mut guest, &real, &chatty, now);
        if frame.content.iter().any(|cell| cell.symbol() == "!") {
            break;
        }
        // Mid-fall she lands before looking; let time pass.
        guest.advance(now);
    }
    assert!(guest.present(), "a message never sends her away");
    let _ = frame;
}

#[test]
fn a_resize_during_the_dissolve_ends_it_at_once() {
    let real = rooms(80, 24);
    let view = view(bottom_strip(80, 24));
    let mut guest = Guest::new(2);
    run(&mut guest, &real, &view, 0, 60_000);
    guest.activity(60_000);
    paint(&mut guest, &real, &view, 60_000);
    let resized = rooms(90, 30);
    assert_eq!(paint(&mut guest, &resized, &view, 60_100), resized);
    assert!(!guest.present());
}

#[test]
fn a_tiny_terminal_gets_no_visit() {
    let real = rooms(50, 16);
    let view = view(bottom_strip(50, 16));
    let mut guest = Guest::new(1);
    let frame = run(&mut guest, &real, &view, 0, 60_000);
    assert_eq!(frame, real);
    assert!(!guest.present());
}

/// Cells that must never change, whatever she does.
fn assert_untouched(
    frame: &Buffer,
    real: &Buffer,
    protected: &[Rect],
) -> Result<(), TestCaseError> {
    let area = real.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let position = Position::new(x, y);
            let (Some(got), Some(want)) = (frame.cell(position), real.cell(position)) else {
                continue;
            };
            if protected.iter().any(|r| r.contains(position)) || cells::untouchable(want) {
                prop_assert_eq!(got, want, "protected cell ({}, {}) changed", x, y);
            }
            if cells::width(got) > 1 && !cells::untouchable(got) && x + 1 < area.right() {
                prop_assert_eq!(
                    frame.cell((x + 1, y)).unwrap().symbol(),
                    " ",
                    "half a wide glyph"
                );
            }
        }
    }
    Ok(())
}

/// A Ghostty-like picker: kitty protocol, 9×19 px cells. (The fixed
/// font size constructor is the only deterministic one.)
#[allow(deprecated)]
fn kitty() -> ratatui_image::picker::Picker {
    let mut picker = ratatui_image::picker::Picker::from_fontsize((9, 19).into());
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    picker
}

/// Nothing is ever hidden behind her: her image cells (placeholders)
/// cover only blanks and lines her image redraws; every other changed
/// cell belongs to the text layer (a hole or a moved glyph) or is a
/// bubble on a blank cell.
fn assert_nothing_hidden(
    frame: &Buffer,
    real: &Buffer,
    layer: &[(u16, u16)],
) -> Result<(), TestCaseError> {
    let width = real.area.width as usize;
    for (index, (got, want)) in frame.content.iter().zip(&real.content).enumerate() {
        if got == want {
            continue;
        }
        let at = ((index % width) as u16, (index / width) as u16);
        let blank = want.symbol().trim().is_empty();
        let line = want
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| graphics::strokes(c).is_some());
        if cells::untouchable(got) {
            prop_assert!(
                blank || line,
                "cell {:?} ({:?}) hidden behind her image",
                at,
                want.symbol()
            );
        } else {
            prop_assert!(
                blank || layer.contains(&at),
                "cell {:?} ({:?}) changed outside the text layer",
                at,
                want.symbol()
            );
        }
    }
    Ok(())
}

fn scatter(buf: &mut Buffer, text: &[(u16, u16, String)], skips: &[(u16, u16)]) {
    for (x, y, s) in text {
        buf.set_string(*x, *y, s, Style::new());
    }
    for &(x, y) in skips {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol("?")
                .set_diff_option(tuirealm::ratatui::buffer::CellDiffOption::Skip);
        }
    }
    cells::sanitize(buf);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(24)))]

    /// A long visit over arbitrary layouts, resizes, chat arrivals and a
    /// final key press: protected rectangles and image cells are never
    /// painted, no frame holds half a wide glyph, the overlay stays
    /// small, and the dissolve always lands exactly on the real frame.
    #[test]
    fn long_visits_never_touch_what_is_protected(
        seed in any::<u64>(),
        graphics in any::<bool>(),
        sizes in proptest::collection::vec((60u16..130, 18u16..45), 1..4),
        text in proptest::collection::vec((0u16..60, 0u16..18, "[a-z漢─│ ]{1,6}"), 0..20),
        skips in proptest::collection::vec((0u16..60, 0u16..18), 0..6),
        chats in proptest::collection::vec(0u64..200_000, 0..4),
        protect in (0u16..40, 0u16..10, 1u16..20, 1u16..6),
    ) {
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        let mut now = 0;
        let span = 200_000 / sizes.len() as u64;
        let mut mark = ChatMark::default();
        for &(w, h) in &sizes {
            let mut real = rooms(w, h);
            scatter(&mut real, &text, &skips);
            let (px, py, pw, ph) = protect;
            let mut protected = bottom_strip(w, h);
            protected.push(Rect::new(px, py, pw, ph).intersection(real.area));
            let until = now + span;
            while now < until {
                let step = guest
                    .next_tick(now)
                    .map_or(until - now, |d| d.as_millis() as u64)
                    .clamp(1, until - now);
                now += step;
                if chats.iter().any(|&c| c <= now && c > now - step) {
                    mark.synced += 1;
                }
                let view = IdleView { chat_mark: mark, ..view(protected.clone()) };
                guest.advance(now);
                let frame = paint(&mut guest, &real, &view, now);
                assert_untouched(&frame, &real, &protected)?;
                let layer: Vec<(u16, u16)> = match &guest.state {
                    State::Visiting(visit) => visit.layer.cells().collect(),
                    _ => Vec::new(),
                };
                if graphics {
                    assert_nothing_hidden(&frame, &real, &layer)?;
                }
                // Besides text she moved: her box and the floor row
                // under it, and one bubble of at most 24 characters.
                let width = usize::from(frame.area.width);
                let changed = frame
                    .content
                    .iter()
                    .zip(&real.content)
                    .enumerate()
                    .filter(|(i, (a, b))| {
                        let at = ((i % width) as u16, (i / width) as u16);
                        a != b && !layer.contains(&at)
                    })
                    .count();
                let most = (sprite::WIDTH * (sprite::HEIGHT + 1)) as usize + 24;
                prop_assert!(changed <= most, "{} cells changed", changed);
            }
        }
        let &(w, h) = sizes.last().unwrap();
        let mut real = rooms(w, h);
        scatter(&mut real, &text, &skips);
        let view = view(bottom_strip(w, h));
        guest.activity(now);
        let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
        prop_assert_eq!(end, real);
    }
}

// ---- Against the real default layout ----

fn real_frame(ui: &mut Ui, width: u16, height: u16) -> (Buffer, IdleView) {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut view = IdleView::default();
    let completed = terminal
        .draw(|frame| {
            ui.draw_with_renderer(frame, &mut renderer);
            view = ui.idle_view(renderer.image_regions());
        })
        .unwrap();
    (completed.buffer.clone(), view)
}

/// The terrain the default layout offers, drawn over the screen:
/// `=` standable floor, `#` protected, `^`/`v` climb and drop starts.
fn terrain_map(width: u16, height: u16) -> String {
    terrain_map_in(width, height, false)
}

fn terrain_map_in(width: u16, height: u16, graphics: bool) -> String {
    let mut ui = real_ui();
    let (buf, view) = real_frame(&mut ui, width, height);
    let terrain = Terrain::read(&buf, &view.protected, graphics);
    let mut rows: Vec<Vec<char>> = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| {
                    let position = Position::new(x, y);
                    if view.protected.iter().any(|r| r.contains(position)) {
                        '#'
                    } else {
                        buf.cell(position)
                            .and_then(|c| c.symbol().chars().next())
                            .unwrap_or(' ')
                    }
                })
                .collect()
        })
        .collect();
    for platform in &terrain.platforms {
        for x in platform.x0..=platform.x1 {
            rows[platform.y as usize][x as usize] = '=';
        }
    }
    for link in &terrain.links {
        let from = terrain.platforms[link.from];
        let mark = match link.route {
            Route::Climb if terrain.platforms[link.to].y < from.y => '^',
            Route::Climb => 'v',
            Route::Drop { .. } => '>',
        };
        rows[from.y as usize][link.x as usize] = mark;
    }
    rows.into_iter()
        .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn default_layout_terrain_100x30() {
    insta::assert_snapshot!(terrain_map(100, 30));
}

/// Line-art mode: only blank headroom and straight floor glyphs.
#[test]
fn default_layout_terrain_graphics_100x30() {
    insta::assert_snapshot!(terrain_map_in(100, 30, true));
}

#[test]
fn default_layout_terrain_80x24() {
    insta::assert_snapshot!(terrain_map(80, 24));
}

/// Seed 7, a minute and a half into a visit, over the real default
/// layout.
#[test]
fn osaka_at_home_seed_7() {
    let mut ui = real_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(7);
    let frame = run(&mut guest, &real, &view, 0, 95_000);
    assert!(guest.present());
    let text: Vec<String> = (0..frame.area.height)
        .map(|y| {
            (0..frame.area.width)
                .map(|x| frame.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    insta::assert_snapshot!(text.join("\n"));
}

#[test]
fn the_real_ui_reports_idle_and_protects_input_status_and_keybar() {
    let mut ui = real_ui();
    let (_, view) = real_frame(&mut ui, 100, 30);
    assert!(view.open());
    assert_eq!(view.delay, Some(DELAY));
    assert_eq!(view.protected.len(), 3, "{:?}", view.protected);
    assert!(view.protected.iter().all(|r| !r.is_empty()));
}

/// Reading the terrain runs on every frame she is on screen.
#[test]
fn terrain_read_is_cheap() {
    let mut ui = real_ui();
    let (buf, view) = real_frame(&mut ui, 200, 60);
    let started = std::time::Instant::now();
    for _ in 0..100 {
        std::hint::black_box(Terrain::read(&buf, &view.protected, true));
    }
    let per_read = started.elapsed() / 100;
    eprintln!("terrain read: {per_read:?}");
    if !cfg!(debug_assertions) {
        assert!(per_read < Duration::from_millis(2), "{per_read:?}");
    }
}

/// The first placement of a frame carries its image data; later frames
/// with the same look only place it (the terminal keeps the image).
#[test]
fn a_frame_is_transmitted_once_then_only_placed() {
    let mut ui = real_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(7);
    guest.set_picker(kitty());
    let transmits = |buf: &Buffer| {
        buf.content
            .iter()
            .filter(|cell| cell.symbol().contains("\x1b_G"))
            .count()
    };
    let mut now = 0;
    paint(&mut guest, &real, &view, now);
    let mut seen_image = false;
    let mut total = 0;
    let mut frames = 0;
    while now < 120_000 {
        let step = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .max(1);
        now += step;
        if guest.advance(now) {
            let frame = paint(&mut guest, &real, &view, now);
            let placed = frame
                .content
                .iter()
                .any(|cell| cell.symbol().contains('\u{10EEEE}'));
            seen_image |= placed;
            total += transmits(&frame);
            frames += usize::from(placed);
        }
    }
    assert!(seen_image, "she was drawn as line art");
    assert!(total < frames, "{total} transmits over {frames} frames");
}

/// Standing on a floor, the floor row joins her image (feet on the line).
#[test]
fn standing_on_a_floor_draws_the_floor_row_into_the_image() {
    let mut ui = real_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(7);
    guest.set_picker(kitty());
    let frame = run(&mut guest, &real, &view, 0, 95_000);
    let rows: Vec<u16> = (0..frame.area.height)
        .filter(|&y| {
            (0..frame.area.width).any(|x| {
                frame
                    .cell((x, y))
                    .is_some_and(|c| c.symbol().contains('\u{10EEEE}'))
            })
        })
        .collect();
    assert_eq!(rows.len(), 5, "4 body rows + the floor row: {rows:?}");
    let floor = *rows.last().unwrap();
    let under: String = (0..real.area.width)
        .filter_map(|x| real.cell((x, floor)))
        .map(|c| c.symbol().to_string())
        .collect();
    assert!(
        under.contains('─'),
        "the image's last row is a floor: {under}"
    );
}

/// Her line-art goodbye: for the first beats the image stays and the
/// dissolve writes nothing else; if anything lands in her space (a modal,
/// typing) the image is dropped at once rather than painted over it.
#[test]
fn the_goodbye_waves_as_line_art_but_never_over_new_content() {
    let placeholder = |buf: &Buffer| {
        buf.content
            .iter()
            .any(|cell| cell.symbol().contains('\u{10EEEE}'))
    };
    let mut ui = real_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(7);
    guest.set_picker(kitty());
    let before = run(&mut guest, &real, &view, 0, 95_000);
    assert!(placeholder(&before), "she is line art before the key press");
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let (x, y) = (visit.osaka.x as u16, visit.osaka.y as u16);
    guest.activity(95_000);

    let beat = paint(&mut guest, &real, &view, 95_100);
    assert!(placeholder(&beat), "startled, still line art");
    // Moved text stays as it was until the rain; nothing else changes.
    for ((got, want), was) in beat.content.iter().zip(&real.content).zip(&before.content) {
        if got != want {
            assert!(
                cells::untouchable(got) || got == was,
                "only image cells change during the beats"
            );
        }
    }

    let mut changed = real.clone();
    changed.set_string(x, y - 2, "modal", Style::new());
    let covered = paint(&mut guest, &changed, &view, 95_200);
    assert!(
        !placeholder(&covered),
        "the image is dropped over new content"
    );
    assert_eq!(covered.cell((x, y - 2)), changed.cell((x, y - 2)));
}

/// After the wave she bursts into letters that rain away: her box shows
/// noise for a while, then the real frame. (The first line-art build
/// showed no rain — the dissolve saw her own image as "the UI changed
/// here" and settled every cell at once.)
#[test]
fn line_art_bursts_into_letters_that_rain_away() {
    let mut ui = real_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(7);
    guest.set_picker(kitty());
    run(&mut guest, &real, &view, 0, 95_000);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let (x, y) = (visit.osaka.x, visit.osaka.y);
    guest.activity(95_000);
    let mut rained = 0;
    let mut now = 95_000;
    let end = now + dissolve::DURATION_MS;
    while now < end {
        let frame = paint(&mut guest, &real, &view, now);
        let letters = (-2..=2)
            .flat_map(|dx| (1..=4).map(move |dy| (x + dx, y - dy)))
            .filter(|&(cx, cy)| {
                frame
                    .cell((cx as u16, cy as u16))
                    .is_some_and(|c| c.symbol().chars().all(|ch| ch.is_ascii_alphanumeric()))
            })
            .count();
        rained = rained.max(letters);
        now += dissolve::FRAME_MS;
    }
    assert!(rained >= 10, "her box rained letters (peak {rained} cells)");
    assert_eq!(paint(&mut guest, &real, &view, end), real);
}

/// The newest message's rows are protected: she looks, never touches.
#[test]
fn the_newest_chat_message_is_protected() {
    let mut ui = chatty_ui(3);
    let (buf, view) = real_frame(&mut ui, 100, 30);
    let newest = (0..buf.area.height)
        .find(|&y| {
            (0..buf.area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
                .contains("line 2")
        })
        .expect("the newest line is on screen");
    let x = (0..buf.area.width)
        .find(|&x| buf.cell((x, newest)).unwrap().symbol() == "l")
        .unwrap();
    assert!(
        view.protected
            .iter()
            .any(|r| r.contains(Position::new(x, newest))),
        "{:?}",
        view.protected
    );
}

/// Start a visit with her standing at `(x, y)` (skipping the idle wait
/// and her wandering).
fn visiting_at(guest: &mut Guest, real: &Buffer, view: &IdleView, (x, y): (i32, i32)) {
    paint(guest, real, view, 0);
    let mut rng = Rng(1);
    guest.state = State::Visiting(Box::new(Visit {
        osaka: Osaka::standing_at(x, y, 0, &mut rng),
        terrain: Terrain::default(),
        painted: Vec::new(),
        image: None,
        layer: layer::TextLayer::default(),
        chances: osaka::Chances::default(),
        size: (real.area.width, real.area.height),
    }));
}

/// With a full log of short chat lines she tidies one: it moves right,
/// in both drawing modes; the goodbye rain puts everything back exactly.
#[test]
fn she_pulls_a_chat_line_and_the_goodbye_puts_it_back() {
    // On the chat floor itself, and on the Playlist's floor across the
    // screen (she has to make her way over).
    for (graphics, start) in [
        (false, (30, 21)),
        (true, (30, 21)),
        (false, (70, 24)),
        (true, (70, 24)),
    ] {
        let mut ui = chatty_ui(40);
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut found = None;
        for seed in 0..20u64 {
            let mut guest = Guest::new(seed);
            if graphics {
                guest.set_picker(kitty());
            }
            visiting_at(&mut guest, &real, &view, start);
            let mut now = 0;
            while now < 240_000 && found.is_none() {
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .max(1);
                if guest.advance(now) {
                    let frame = paint(&mut guest, &real, &view, now);
                    // A pull: three or more glyphs moved along their row
                    // (a sneeze pops them up; a swap is two).
                    if let State::Visiting(visit) = &guest.state
                        && visit.layer.entries().len() >= 3
                        && visit.layer.entries().iter().all(|d| d.at.1 == d.source.1)
                    {
                        found = Some((seed, frame));
                    }
                }
            }
            if found.is_some() {
                guest.activity(now);
                let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
                assert_eq!(end, real, "graphics={graphics}: the rain restores the text");
                break;
            }
        }
        let (seed, frame) = found.unwrap_or_else(|| panic!("graphics={graphics}: no pull"));
        let moved = (15..21u16).any(|y| {
            let row = |b: &Buffer| {
                (1..48u16)
                    .map(|x| b.cell((x, y)).unwrap().symbol().to_string())
                    .collect::<String>()
            };
            let (before, after) = (row(&real), row(&frame));
            before.contains("line") && after.starts_with(' ') && after.contains("line")
        });
        assert!(
            moved,
            "graphics={graphics} seed={seed}: a chat line moved right"
        );
    }
}

/// She keeps herself busy: across visits she does on-the-spot activities
/// (sitting, lying, calisthenics…) and spends little time just standing
/// and staring at the viewer.
#[test]
fn she_mostly_does_things_rather_than_stare() {
    use super::sprite::Pose;
    let real = rooms(100, 30);
    let view = view(bottom_strip(100, 30));
    let (mut staring, mut total, mut activities) = (0u64, 0u64, 0usize);
    for seed in 0..4u64 {
        let mut guest = Guest::new(seed);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        let mut last = None;
        while now < 300_000 {
            let step = guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if let State::Visiting(visit) = &guest.state {
                let (pose, ..) = visit.osaka.appearance(now);
                total += step;
                if pose == Pose::Stand {
                    staring += step;
                }
                let active = matches!(
                    pose,
                    Pose::Sit
                        | Pose::LieBack(_)
                        | Pose::LieFront(_)
                        | Pose::Jack(_)
                        | Pose::ToeTouch(_)
                        | Pose::Stretch
                        | Pose::Gaze
                );
                if active && last != Some(std::mem::discriminant(&pose)) {
                    activities += 1;
                }
                last = Some(std::mem::discriminant(&pose));
            }
            now += step;
            if guest.advance(now) {
                paint(&mut guest, &real, &view, now);
            }
        }
    }
    assert!(total > 0);
    assert!(activities >= 4, "{activities} activities over four visits");
    assert!(staring * 4 < total, "stared {staring} of {total} ms");
}

/// The left room with a line of text on her box's hands row, ending
/// three cells left of her box when she stands at x = 15 on the floor at
/// y = 26.
fn words_room() -> (Buffer, IdleView) {
    let mut real = rooms(100, 30);
    real.set_string(2, 24, "the cat sat", Style::new());
    (real, view(bottom_strip(100, 30)))
}

fn row_text(buf: &Buffer, y: u16, xs: std::ops::Range<u16>) -> String {
    xs.map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
        .collect()
}

/// She swaps two letters of a word, and swaps them back on schedule —
/// no goodbye needed.
#[test]
fn a_letter_swap_undoes_itself() {
    for graphics in [false, true] {
        let (real, view) = words_room();
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (15, 26));
        paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        let swap = visit
            .chances
            .swaps
            .iter()
            .find(|s| s.x == 15 && s.y == 26)
            .cloned()
            .expect("a swap from where she stands");
        assert_eq!(swap.pair, (11, 12), "the last two letters of 'sat'");
        visit.osaka.swap_now(swap, 0);
        let frame = run(&mut guest, &real, &view, 0, 2000);
        assert_eq!(
            row_text(&frame, 24, 2..13),
            "the cat sta",
            "graphics={graphics}"
        );
        // Kept for 7–14 s, then undone where she stands.
        let mut now = 2000;
        let back = loop {
            now += 100;
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("still visiting");
            };
            if visit.layer.entries().is_empty() {
                break frame;
            }
            assert!(now < 16_000, "graphics={graphics}: never swapped back");
        };
        assert!(now >= 7000, "kept a while");
        assert_eq!(
            row_text(&back, 24, 2..13),
            "the cat sat",
            "graphics={graphics}"
        );
    }
}

/// A chat message arriving while a swap is out puts it right at once.
#[test]
fn a_chat_message_undoes_a_swap_at_once() {
    let (real, view) = words_room();
    let mut guest = Guest::new(3);
    visiting_at(&mut guest, &real, &view, (15, 26));
    paint(&mut guest, &real, &view, 0);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    let swap = visit.chances.swaps.first().cloned().expect("a swap");
    let at = (swap.x, swap.y);
    visiting_at(&mut guest, &real, &view, at);
    paint(&mut guest, &real, &view, 0);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.swap_now(swap, 0);
    run(&mut guest, &real, &view, 0, 2000);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    assert_eq!(visit.layer.entries().len(), 2, "swapped");
    let mut chatty = view.clone();
    chatty.chat_mark.synced += 1;
    guest.advance(2001);
    paint(&mut guest, &real, &chatty, 2001);
    run(&mut guest, &real, &chatty, 2001, 2100);
    let State::Visiting(visit) = &guest.state else {
        panic!("still visiting: a chat message never sends her away");
    };
    assert!(visit.layer.entries().is_empty(), "swapped back at once");
}

/// A sneeze knocks a few letters loose; they fall, and she puts every
/// one back — no goodbye needed.
#[test]
fn a_sneeze_scatters_letters_and_she_puts_them_back() {
    for graphics in [false, true] {
        let (real, view) = words_room();
        let mut guest = Guest::new(5);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (15, 26));
        paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        assert!(visit.chances.loose.len() >= 2, "{:?}", visit.chances.loose);
        visit.osaka.sneeze_now(0);
        let mut now = 0;
        let mut most = 0;
        let mut scattered = false;
        while now < 6000 {
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            if let State::Visiting(visit) = &guest.state {
                most = most.max(visit.layer.entries().len());
                scattered |= row_text(&frame, 24, 2..13) != "the cat sat";
            }
        }
        assert!(most >= 2, "graphics={graphics}: knocked {most}");
        assert!(scattered, "graphics={graphics}: the line was disturbed");
        let State::Visiting(visit) = &guest.state else {
            panic!("still visiting");
        };
        assert!(
            visit.layer.entries().is_empty(),
            "graphics={graphics}: all back"
        );
        let frame = paint(&mut guest, &real, &view, now);
        assert_eq!(row_text(&frame, 24, 2..13), "the cat sat");
    }
}

/// If the word changes before her swap lands, nothing is owed: no swap-
/// back stays queued, and she doesn't giggle at a swap that never was.
#[test]
fn a_refused_swap_leaves_nothing_owed() {
    let (real, view) = words_room();
    let mut guest = Guest::new(3);
    visiting_at(&mut guest, &real, &view, (15, 26));
    paint(&mut guest, &real, &view, 0);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    let swap = visit
        .chances
        .swaps
        .iter()
        .find(|s| s.x == 15 && s.y == 26)
        .cloned()
        .expect("a swap");
    visit.osaka.swap_now(swap, 0);
    // The chat scrolls: the word is gone by the time she swaps.
    let mut scrolled = rooms(100, 30);
    scrolled.set_string(2, 24, "the cat", Style::new());
    run(&mut guest, &scrolled, &view, 0, 1000);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    assert!(visit.layer.entries().is_empty());
    assert!(!visit.osaka.owes_anything(), "no swap-back queued");
    let (_, face, bubble) = visit.osaka.appearance(1000);
    assert_ne!(bubble, Some(osaka::Bubble::Hehe), "{face:?}: no giggle");
}

/// Every scene the stage offers finds a spot in the stage room at the
/// sizes the terrain snapshots pin, and visibly happens there.
#[test]
fn every_scene_has_a_spot_in_the_stage_room() {
    use super::sprite::Pose;
    for (width, height) in [(100, 30), (80, 24)] {
        for graphics in [false, true] {
            let mut ui = stage_ui();
            let (real, view) = real_frame(&mut ui, width, height);
            for scene in Scene::ALL {
                let mut guest = Guest::new(1);
                if graphics {
                    guest.set_picker(kitty());
                }
                guest.cue(scene);
                paint(&mut guest, &real, &view, 0);
                let at = format!("{scene:?} at {width}x{height} graphics={graphics}");
                let note = guest.cue_note().cloned();
                assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
                let State::Visiting(visit) = &guest.state else {
                    panic!("{at}: visiting");
                };
                let start_y = visit.osaka.y;
                let (mut moved, mut swapped, mut climbed, mut poses) =
                    (0, false, false, Vec::new());
                let mut said = false;
                let mut now = 0;
                while now < 10_000 {
                    now += guest
                        .next_tick(now)
                        .map_or(100, |d| d.as_millis() as u64)
                        .clamp(1, 100);
                    guest.advance(now);
                    paint(&mut guest, &real, &view, now);
                    let State::Visiting(visit) = &guest.state else {
                        panic!("{at}: still visiting");
                    };
                    let entries = visit.layer.entries();
                    moved = moved.max(entries.len());
                    swapped |= entries.iter().any(|a| {
                        entries
                            .iter()
                            .any(|b| a.at == b.source && b.at == a.source && a.source != b.source)
                    });
                    climbed |= visit.osaka.y != start_y;
                    let (pose, _, bubble) = visit.osaka.appearance(now);
                    said |= matches!(bubble, Some(osaka::Bubble::Say(_)));
                    poses.push(std::mem::discriminant(&pose));
                }
                let posed = |pose: Pose| poses.contains(&std::mem::discriminant(&pose));
                let happened = match scene {
                    Scene::Arrive => true,
                    Scene::Pull => moved >= 3,
                    Scene::Swap => swapped,
                    Scene::Sneeze => moved >= 2,
                    Scene::ClimbUp | Scene::ClimbDown | Scene::Drop => climbed,
                    Scene::Sit => posed(Pose::Sit),
                    Scene::LieBack => posed(Pose::LieBack(0)),
                    Scene::LieFront => posed(Pose::LieFront(0)),
                    Scene::Jacks => posed(Pose::Jack(0)),
                    Scene::ToeTouch => posed(Pose::ToeTouch(0)),
                    Scene::Stretch => posed(Pose::Stretch),
                    Scene::Gaze => posed(Pose::Gaze),
                    Scene::Muse => said,
                };
                assert!(happened, "{at}: {note:?}, moved {moved}");
            }
        }
    }
}

/// How often natural chat offers her text to tidy or play with: over
/// random logs (message lengths like real chat), the share of screens
/// with at least one pull and one swap. Prints; run by hand.
#[test]
#[ignore]
fn reach_census() {
    const WORDS: [&str; 12] = [
        "ok", "yes", "lol", "the", "episode", "is", "that", "what", "watching", "no", "cat", "brb",
    ];
    for (width, height) in [(100, 30), (80, 24)] {
        for graphics in [false, true] {
            let (mut pulls, mut swaps, mut spots) = (0, 0, 0);
            let screens = 60;
            for seed in 0..screens {
                let mut rng = Rng(seed);
                let lines: Vec<String> = (0..40)
                    .map(|_| {
                        let n = 1 + rng.below(7) as usize;
                        (0..n)
                            .map(|_| WORDS[rng.below(WORDS.len() as u64) as usize])
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect();
                let mut ui = super::stage::chat_ui(lines.into_iter());
                let (real, view) = real_frame(&mut ui, width, height);
                let terrain = Terrain::read(&real, &view.protected, graphics);
                // Only text in the chat pane counts.
                let chat = view.chat;
                let inside = |x: u16, y: u16| chat.contains(Position::new(x, y));
                let p: Vec<_> = super::scenes::pulls(&real, &terrain, &view.protected)
                    .into_iter()
                    .filter(|p| p.cells.iter().all(|&x| inside(x, p.row)))
                    .collect();
                let s: Vec<_> = super::scenes::swaps(&real, &terrain, &view.protected)
                    .into_iter()
                    .filter(|s| inside(s.pair.0, s.row))
                    .collect();
                pulls += usize::from(!p.is_empty());
                swaps += usize::from(!s.is_empty());
                spots += p.len();
            }
            eprintln!(
                "{width}x{height} graphics={graphics}: pulls on {pulls}/{screens} screens ({spots} spots), swaps on {swaps}/{screens}"
            );
        }
    }
}

/// A bubble goes up and to the side she faces; blocked there, the other
/// side; never over anything but blank, open cells.
#[test]
fn bubbles_find_a_blank_spot_around_her_head() {
    let mut rng = Rng(1);
    let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
    let empty = Buffer::empty(Rect::new(0, 0, 40, 12));
    let terrain = Terrain::read(&empty, &[], true);
    osaka.facing = sprite::Facing::Right;
    assert_eq!(bubble_spot(&empty, &terrain, &osaka, 4), Some((23, 5)));
    osaka.facing = sprite::Facing::Left;
    assert_eq!(bubble_spot(&empty, &terrain, &osaka, 4), Some((14, 5)));
    // Text on her left above her head: the other side.
    let mut buf = empty.clone();
    buf.set_string(15, 5, "busy", Style::new());
    assert_eq!(bubble_spot(&buf, &terrain, &osaka, 4), Some((23, 5)));
    // The whole row above her taken: beside her head.
    buf.set_string(0, 5, "x".repeat(40), Style::new());
    assert_eq!(bubble_spot(&buf, &terrain, &osaka, 4), Some((13, 6)));
    // Nowhere at all.
    for y in 0..12 {
        buf.set_string(0, y, "x".repeat(40), Style::new());
    }
    assert_eq!(bubble_spot(&buf, &terrain, &osaka, 4), None);
}

/// Something she says shows for 1.2 s + 60 ms a character, then goes.
#[test]
fn she_says_a_line_and_then_stops() {
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(2);
    guest.cue(Scene::Muse);
    paint(&mut guest, &real, &view, 0);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let (_, _, bubble) = visit.osaka.appearance(0);
    let Some(osaka::Bubble::Say(line)) = bubble else {
        panic!("saying something: {bubble:?}");
    };
    let shows = |frame: &Buffer| {
        (0..frame.area.height).any(|y| {
            (0..frame.area.width)
                .map(|x| frame.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
                .contains(line)
        })
    };
    let frame = run(&mut guest, &real, &view, 0, 500);
    assert!(shows(&frame), "{line:?} on screen");
    let gone = 1200 + 60 * line.chars().count() as u64;
    let frame = run(&mut guest, &real, &view, 500, gone + 50);
    assert!(!shows(&frame), "{line:?} gone after {gone} ms");
}

/// Over long visits her needs show: she dozes more in a visit's second
/// half than its first, no one thing takes over, and she still tidies.
#[test]
fn her_needs_shape_long_visits() {
    use super::brain::Kind;
    use super::sprite::Pose;
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let half = 10 * 60_000;
    let (mut early, mut late) = (0u64, 0u64);
    let mut choices: Vec<Kind> = Vec::new();
    for seed in 0..4u64 {
        let mut guest = Guest::new(seed);
        guest.cue(Scene::Arrive);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        while now < 2 * half {
            let step = guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if let State::Visiting(visit) = &guest.state {
                let (pose, ..) = visit.osaka.appearance(now);
                if matches!(pose, Pose::LieBack(_)) {
                    if now < half {
                        early += step;
                    } else {
                        late += step;
                    }
                }
            }
            now += step;
            if guest.advance(now) {
                paint(&mut guest, &real, &view, now);
            }
        }
        let State::Visiting(visit) = &guest.state else {
            panic!("seed {seed}: still visiting");
        };
        choices.extend(&visit.osaka.choices);
    }
    assert!(late > 2 * early, "dozing: {early} ms early, {late} ms late");
    let most = choices
        .iter()
        .map(|k| choices.iter().filter(|c| *c == k).count())
        .max()
        .unwrap_or(0);
    assert!(
        most * 2 < choices.len(),
        "one choice took over: {choices:?}"
    );
    assert!(choices.contains(&Kind::Pull), "she tidied");
}
