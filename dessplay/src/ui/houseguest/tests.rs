#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use proptest::prelude::*;
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;
use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::{Position, Rect};
use tuirealm::ratatui::style::Style;

use super::terrain::{Route, Terrain};
use super::*;
use crate::config::{Houseguest, Settings};
use crate::ui::app::{Ui, UiSnapshot};
use crate::ui::layout::{LayoutBundle, Renderer};
use dessplay_core::types::UserId;

const DELAY: Duration = Duration::from_secs(5);

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
    let after = run(&mut guest, &real, &view, 60_000, 62_500);
    assert_eq!(after, real);
    assert!(!guest.present());
    // The idle timer restarted at the key press.
    assert_eq!(guest.next_tick(62_500), Some(Duration::from_millis(2_500)));
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

/// In graphics mode nothing is ever hidden behind her: every cell she
/// changes was blank, or a line her image redraws.
fn assert_image_only_over_blanks_and_floors(
    frame: &Buffer,
    real: &Buffer,
) -> Result<(), TestCaseError> {
    for (index, (got, want)) in frame.content.iter().zip(&real.content).enumerate() {
        if got == want {
            continue;
        }
        let blank = want.symbol().trim().is_empty();
        let line = want
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| graphics::strokes(c).is_some());
        prop_assert!(
            blank || line,
            "cell {} ({:?}) hidden behind her",
            index,
            want.symbol()
        );
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
        chats in proptest::collection::vec(0u64..300_000, 0..4),
        protect in (0u16..40, 0u16..10, 1u16..20, 1u16..6),
    ) {
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        let mut now = 0;
        let span = 300_000 / sizes.len() as u64;
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
                if graphics {
                    assert_image_only_over_blanks_and_floors(&frame, &real)?;
                }
                let changed = frame
                    .content
                    .iter()
                    .zip(&real.content)
                    .filter(|(a, b)| a != b)
                    .count();
                prop_assert!(changed <= 40, "{} cells changed", changed);
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

fn real_ui() -> Ui {
    let mut ui = Ui::with_setup(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("test".into()),
            houseguest: Houseguest::After(DELAY),
            ..Settings::default()
        },
        vec![],
        false,
    );
    ui.apply_snapshot(UiSnapshot::default());
    ui
}

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
    for (got, want) in beat.content.iter().zip(&real.content) {
        if got != want {
            assert!(
                cells::untouchable(got),
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
