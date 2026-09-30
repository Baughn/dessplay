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
        resident: false,
        focus: None,
        chat_mark: ChatMark::default(),
        chat: Rect::new(0, 0, 30, 20),
        scrollback: None,
        protected,
        nooks: Vec::new(),
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

/// The quiet panes of [`rooms`]: the tall left box and the two stacked
/// right ones.
fn nooks(width: u16, height: u16) -> Vec<(Nook, Rect)> {
    vec![
        (Nook::List, Rect::new(0, 0, width / 2, height - 3)),
        (
            Nook::Users,
            Rect::new(width / 2, 0, width - width / 2, (height - 3) / 2),
        ),
        (
            Nook::Playlist,
            Rect::new(
                width / 2,
                (height - 3) / 2,
                width - width / 2,
                height - 3 - (height - 3) / 2,
            ),
        ),
    ]
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
        owned in proptest::collection::vec((0usize..4, 0usize..3, 0u16..=1000, any::<bool>()), 0..5),
    ) {
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        let nook = [Nook::List, Nook::Users, Nook::Playlist];
        for &(item, at, along, left) in &owned {
            // A second room offered a taken pane is refused, as in play.
            let _ = guest.ledger.home.add(
                nook[at],
                room::Prop {
                    item: Furniture::ALL[item],
                    at: along,
                    facing: if left { sprite::Facing::Left } else { sprite::Facing::Right },
                    boxed: false,
                },
            );
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
                let view = IdleView {
                    chat_mark: mark,
                    nooks: nooks(w, h),
                    ..view(protected.clone())
                };
                guest.advance(now);
                let frame = paint(&mut guest, &real, &view, now);
                assert_untouched(&frame, &real, &protected)?;
                let (layer, shown): (Vec<(u16, u16)>, Vec<Shown>) = match &guest.state {
                    State::Visiting(visit) => (visit.layer.cells().collect(), visit.shown.clone()),
                    _ => (Vec::new(), Vec::new()),
                };
                // A room's pieces share a pane, and a pane holds one room.
                for a in &shown {
                    for b in &shown {
                        prop_assert_eq!(
                            a.item.room() == b.item.room(),
                            a.nook == b.nook,
                            "{:?} and {:?}", a, b
                        );
                    }
                }
                // Her furniture stands on lines, over blank cells only,
                // clear of protected cells, text she moved, and her.
                for prop in &shown {
                    let (cols, _) = prop.item.footprint();
                    let floor = (prop.left..prop.left + i32::from(cols)).map(|x| (x, prop.floor, None));
                    for (x, y, _) in prop.cells().chain(floor) {
                        let at = (x as u16, y as u16);
                        let want = real.cell(at).unwrap();
                        prop_assert!(!cells::untouchable(want));
                        prop_assert!(!protected.iter().any(|r| r.contains(at.into())), "{:?} protected", at);
                        prop_assert!(!layer.contains(&at), "{:?} over moved text", at);
                        if y < prop.floor {
                            prop_assert!(want.symbol().trim().is_empty(), "{:?} over {:?}", at, want.symbol());
                        } else {
                            prop_assert!(want.symbol().chars().next().is_some_and(|c| graphics::strokes(c).is_some()));
                        }
                    }
                }
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
                // Her box (with its floor row), a bubble, her furniture,
                // and the blank cells an image of her with the pieces she
                // overlaps spans beyond them.
                let (furniture, spanned) = match &guest.state {
                    State::Visiting(visit) => {
                        let area = |r: Rect| usize::from(r.width) * usize::from(r.height);
                        let furniture: usize = shown.iter().map(|p| area(p.cover())).sum();
                        let her = Rect::new(
                            (visit.osaka.x - sprite::WIDTH / 2).max(0) as u16,
                            (visit.osaka.y - sprite::HEIGHT).max(0) as u16,
                            sprite::WIDTH as u16,
                            sprite::HEIGHT as u16 + 1,
                        );
                        let union = visit.with.iter().fold(her, |r, p| r.union(p.cover()));
                        (furniture, area(union))
                    }
                    _ => (0, 0),
                };
                let most = (sprite::WIDTH * (sprite::HEIGHT + 1)) as usize + 24 + furniture + spanned;
                let debug = match &guest.state {
                    State::Visiting(visit) => format!("{:?} with {:?} at ({}, {})", visit.shown, visit.with, visit.osaka.x, visit.osaka.y),
                    _ => String::new(),
                };
                prop_assert!(changed <= most, "{} cells changed (most {}): {}", changed, most, debug);
            }
        }
        let &(w, h) = sizes.last().unwrap();
        let mut real = rooms(w, h);
        scatter(&mut real, &text, &skips);
        let view = IdleView {
            nooks: nooks(w, h),
            ..view(bottom_strip(w, h))
        };
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
            Route::Clamber { .. } => '%',
            Route::Around { .. } => '@',
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
        fades: Vec::new(),
        osaka: Osaka::standing_at(x, y, 0, &mut rng),
        terrain: Terrain::default(),
        painted: Vec::new(),
        image: None,
        layer: layer::TextLayer::default(),
        chances: osaka::Chances::default(),
        shown: Vec::new(),
        with: Vec::new(),
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
        assert_eq!(
            (swap.a.at.0, swap.b.at.0),
            (11, 12),
            "the last two letters of 'sat'"
        );
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

/// A line she has pulled isn't pulled again, but its letters are still
/// hers to swap — likely the only ones in reach by then. The swap trades
/// them where they now sit, and undoing it leaves the line pulled.
#[test]
fn she_swaps_letters_on_a_line_she_pulled() {
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
        let pull = visit
            .chances
            .pulls
            .iter()
            .find(|p| p.row == 24 && p.side == scenes::Side::Left && p.cells.len() == 9)
            .cloned()
            .expect("the whole line is pullable from its end");
        visit.osaka.pursue(scenes::Job::Pull(pull), 0);
        // Until the pulled line has been still for a while (she admires
        // it for two seconds).
        let (mut now, mut still_since, mut last) = (0, 0, String::new());
        let pulled = loop {
            now += 100;
            assert!(now < 30_000, "graphics={graphics}: never finished pulling");
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let row = row_text(&frame, 24, 0..40);
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            if row != last || visit.layer.entries().len() != 9 {
                (still_since, last) = (now, row);
            } else if now - still_since >= 1500 {
                break row;
            }
        };
        assert!(!pulled.starts_with("  the"), "{pulled:?}: moved");
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        assert!(
            visit.chances.pulls.iter().all(|p| p.row != 24),
            "not pulled twice"
        );
        let shown: Vec<u16> = visit.layer.entries().iter().map(|d| d.at.0).collect();
        // Just the line: her picture beside it animates.
        let end = shown.iter().max().map_or(0, |&x| x + 1);
        let pulled: String = pulled.chars().take(usize::from(end)).collect();
        let swap = visit
            .chances
            .swaps
            .iter()
            .find(|s| s.row == 24 && shown.contains(&s.a.at.0) && shown.contains(&s.b.at.0))
            .cloned()
            .unwrap_or_else(|| panic!("graphics={graphics}: {:?}", visit.chances.swaps));
        let stand = (swap.x, swap.y);
        visit.osaka.place(stand.0, stand.1, now);
        visit.osaka.swap_now(swap.clone(), now);
        let frame = run(&mut guest, &real, &view, now, now + 2000);
        now += 2000;
        let swapped = row_text(&frame, 24, 0..end);
        let (a, b) = (usize::from(swap.a.at.0), usize::from(swap.b.at.0));
        let mut expected: Vec<char> = pulled.chars().collect();
        expected.swap(a, b);
        assert_eq!(
            swapped,
            expected.iter().collect::<String>(),
            "graphics={graphics}: traded where they sit"
        );
        // Undone on schedule, back to the pulled line — not home.
        let back = loop {
            now += 100;
            assert!(now < 60_000, "graphics={graphics}: never swapped back");
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let row = row_text(&frame, 24, 0..end);
            if row != swapped {
                break row;
            }
        };
        assert_eq!(back, pulled, "graphics={graphics}");
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        assert_eq!(visit.layer.entries().len(), 9, "still pulled");
        assert!(!visit.osaka.owes_anything());
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
                let start = (visit.osaka.x, visit.osaka.y);
                let (mut went_out, mut doored, mut moved_on, mut gone) =
                    (false, false, false, false);
                let (mut moved, mut swapped, mut climbed, mut poses) =
                    (0, false, false, Vec::new());
                let mut said = false;
                let mut used = None;
                let mut now = 0;
                let mut happened = false;
                // Stop as soon as the scene has visibly happened: running
                // all 116 cases to the 10s cap costs ~50s in a debug build.
                while !happened && now < 10_000 {
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
                    went_out |= !(0..i32::from(width)).contains(&visit.osaka.x);
                    doored |= visit.osaka.door(now).is_some();
                    gone |= visit.osaka.hidden(now);
                    moved_on |= (visit.osaka.x, visit.osaka.y) != start;
                    used = used.or(visit.osaka.using());
                    let (pose, _, bubble) = visit.osaka.appearance(now);
                    said |= matches!(bubble, Some(osaka::Bubble::Say(_)));
                    poses.push(std::mem::discriminant(&pose));
                    let posed = |pose: Pose| poses.contains(&std::mem::discriminant(&pose));
                    happened = match scene {
                        Scene::Arrive => true,
                        Scene::Pull => moved >= 3,
                        Scene::Swap => swapped,
                        Scene::Sneeze => moved >= 2,
                        Scene::ClimbUp | Scene::ClimbDown | Scene::Drop => climbed,
                        Scene::Clamber => climbed,
                        Scene::StepOut => went_out,
                        Scene::Door => doored && moved_on,
                        Scene::Sit => posed(Pose::Sit),
                        Scene::LieBack => posed(Pose::LieBack(0)),
                        Scene::LieFront => posed(Pose::LieFront(0)),
                        Scene::Jacks => posed(Pose::Jack(0)),
                        Scene::ToeTouch => posed(Pose::ToeTouch(0)),
                        Scene::Stretch => posed(Pose::Stretch),
                        Scene::Gaze => posed(Pose::Gaze),
                        Scene::Muse => said,
                        Scene::Lounge => posed(Pose::Lounge),
                        Scene::Nap => posed(Pose::Nap(0)),
                        Scene::Sleep => posed(Pose::Sleep(0)),
                        Scene::Homework => posed(Pose::Homework(0)),
                        Scene::Watch => used == Some(Furniture::Tv),
                        Scene::Parcel => guest.ledger.home.props.iter().any(|p| !p.boxed),
                        Scene::Shopping => guest.ledger.ordered.is_some(),
                        Scene::Work => went_out || gone,
                        Scene::Read => posed(Pose::Read(0)),
                        Scene::Snack => posed(Pose::Eat(0)),
                        Scene::Pet => posed(Pose::Pet(0)),
                    };
                }
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
                let s: Vec<_> = super::scenes::swaps(
                    &real,
                    &terrain,
                    &view.protected,
                    &layer::TextLayer::default(),
                )
                .into_iter()
                .filter(|s| inside(s.a.at.0, s.row))
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
    assert_eq!(
        bubble_spot(&empty, &terrain, &[], &osaka, Pose::Stand, 4),
        Some((23, 5))
    );
    osaka.facing = sprite::Facing::Left;
    assert_eq!(
        bubble_spot(&empty, &terrain, &[], &osaka, Pose::Stand, 4),
        Some((14, 5))
    );
    // Text on her left above her head: the other side.
    let mut buf = empty.clone();
    buf.set_string(15, 5, "busy", Style::new());
    assert_eq!(
        bubble_spot(&buf, &terrain, &[], &osaka, Pose::Stand, 4),
        Some((23, 5))
    );
    // The whole row above her taken: beside her head.
    buf.set_string(0, 5, "x".repeat(40), Style::new());
    assert_eq!(
        bubble_spot(&buf, &terrain, &[], &osaka, Pose::Stand, 4),
        Some((13, 6))
    );
    // Nowhere at all.
    for y in 0..12 {
        buf.set_string(0, y, "x".repeat(40), Style::new());
    }
    assert_eq!(
        bubble_spot(&buf, &terrain, &[], &osaka, Pose::Stand, 4),
        None
    );
}

/// Lying down or sitting, her bubble is by her head wherever it is, not
/// up at the top of her box — and never inside the box, where it would
/// scribble over her.
#[test]
fn bubbles_follow_her_head_when_she_is_down() {
    let mut rng = Rng(1);
    let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
    let empty = Buffer::empty(Rect::new(0, 0, 40, 12));
    let terrain = Terrain::read(&empty, &[], true);
    let spot = |osaka: &Osaka, pose, facing, len| {
        let mut osaka = osaka.clone();
        osaka.facing = facing;
        bubble_spot(&empty, &terrain, &[], &osaka, pose, len)
    };
    use sprite::Facing::{Left, Right};
    // On her back facing right, head on the floor at x=18: "zzz" just
    // up and behind it.
    assert_eq!(spot(&osaka, Pose::LieBack(0), Right, 3), Some((15, 8)));
    assert_eq!(spot(&osaka, Pose::LieBack(0), Left, 3), Some((23, 8)));
    // On her front, head at the end she faces.
    assert_eq!(spot(&osaka, Pose::LieFront(0), Right, 1), Some((23, 8)));
    assert_eq!(spot(&osaka, Pose::LieFront(0), Left, 1), Some((17, 8)));
    // Touching her toes, head low on the side she faces.
    assert_eq!(spot(&osaka, Pose::ToeTouch(1), Right, 4), Some((23, 6)));
    // Sitting: level with the box's top row, just outside it.
    assert_eq!(spot(&osaka, Pose::Sit, Right, 3), Some((23, 6)));
    // Both diagonals taken (chat to either side): level with her head
    // rather than way up over her box.
    let mut busy = empty.clone();
    busy.set_string(0, 8, "x".repeat(40), Style::new());
    let mut lying = osaka.clone();
    lying.facing = sprite::Facing::Right;
    assert_eq!(
        bubble_spot(&busy, &terrain, &[], &lying, Pose::LieBack(0), 3),
        Some((14, 9))
    );
    // Whatever the pose, never inside her box.
    osaka.x = 20;
    for pose in [
        Pose::LieBack(1),
        Pose::LieFront(1),
        Pose::Sit,
        Pose::ToeTouch(1),
    ] {
        for facing in [Left, Right] {
            let (start, row) = spot(&osaka, pose, facing, 5).expect("room");
            let inside = (6..10).contains(&row) && start <= 22 && start + 5 > 18;
            assert!(!inside, "{pose:?} {facing:?}: ({start}, {row})");
        }
    }
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
    // The direction only: how much more is the brain's to say (brain.rs,
    // `sleepiness_draws_her_to_lie_down`); four whole visits are too few
    // to pin a ratio that any change to her paths reshuffles.
    assert!(late > early, "dozing: {early} ms early, {late} ms late");
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

// ---- Her room ----

/// Given a sofa in the stage's room, she places it on a quiet pane's
/// floor, keeps it through the visit, and her goodbye takes it along:
/// the rain lands exactly on the real frame. In both drawing modes.
#[test]
fn her_sofa_stands_in_a_quiet_pane_and_leaves_with_her() {
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        guest.give(Furniture::Sofa);
        let frame = run(&mut guest, &real, &view, 0, 30_000);
        assert!(
            matches!(guest.cue_note(), Some(Ok(_))),
            "{:?}",
            guest.cue_note()
        );
        let State::Visiting(visit) = &guest.state else {
            panic!("she left");
        };
        let [sofa] = visit.shown[..] else {
            panic!("graphics={graphics}: {:?}", visit.shown);
        };
        let rect = sofa.rect();
        assert!(
            view.nooks
                .iter()
                .any(|&(_, pane)| pane.intersection(rect) == rect),
            "inside a quiet pane"
        );
        let drawn = |x: u16, y: u16| frame.cell((x, y)) != real.cell((x, y));
        assert!(
            (rect.x..rect.right()).any(|x| drawn(x, rect.bottom() - 1)),
            "graphics={graphics}: the sofa is on screen"
        );
        assert!(guest.ledger.home.owns(Furniture::Sofa));
        guest.activity(30_000);
        let end = run(
            &mut guest,
            &real,
            &view,
            30_000,
            30_000 + dissolve::DURATION_MS,
        );
        assert_eq!(
            end, real,
            "graphics={graphics}: the rain restores the frame"
        );
        // She still owns it next visit.
        assert!(guest.ledger.home.owns(Furniture::Sofa));
    }
}

/// Two quiet panes side by side over one floor (the bottom borders
/// meet), with the keybar strip below: a home she can walk all of.
fn home_screen() -> (Buffer, IdleView) {
    let (width, height) = (100u16, 20u16);
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let users = Rect::new(0, 8, 50, 9);
    let playlist = Rect::new(50, 8, 50, 9);
    for area in [users, playlist] {
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            area,
            &mut buf,
        );
    }
    buf.set_string(0, height - 2, "Tab Next pane | Enter Send", Style::new());
    let view = IdleView {
        nooks: vec![(Nook::Users, users), (Nook::Playlist, playlist)],
        ..view(bottom_strip(width, height))
    };
    (buf, view)
}

/// A furnished home over long visits in line art: she uses her things
/// (and sleeps in her bed more than on a border once she has one), goes
/// to work at most once a visit, her
/// image with the pieces she overlaps never hides text, and the distinct
/// images stay within the frame cache.
#[test]
fn a_furnished_home_gets_used_and_stays_cheap() {
    use super::brain::Kind;
    use super::room::Use;
    let (real, view) = home_screen();
    let mut choices: Vec<Kind> = Vec::new();
    for seed in 0..2u64 {
        let mut guest = Guest::new(seed);
        guest.set_picker(kitty());
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        // Her living room and bedroom (the screen has two panes).
        let pieces = [
            Furniture::Sofa,
            Furniture::Tv,
            Furniture::Bed,
            Furniture::Desk,
        ];
        for item in pieces {
            guest.give(item);
            paint(&mut guest, &real, &view, 0);
        }
        assert_eq!(
            pieces
                .iter()
                .filter(|&&item| guest.ledger.home.owns(item))
                .count(),
            4,
            "seed {seed}: {:?}",
            guest.cue_note()
        );
        let mut now = 0;
        while now < 20 * 60_000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if guest.advance(now) {
                let frame = paint(&mut guest, &real, &view, now);
                let State::Visiting(visit) = &guest.state else {
                    panic!("seed {seed}: still visiting");
                };
                let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
                assert_nothing_hidden(&frame, &real, &layer)
                    .unwrap_or_else(|e| panic!("seed {seed} at {now}: {e}"));
            }
        }
        let State::Visiting(visit) = &guest.state else {
            panic!("seed {seed}: still visiting");
        };
        let shifts = visit
            .osaka
            .choices
            .iter()
            .filter(|&&k| k == Kind::Work)
            .count();
        assert!(
            shifts <= 1,
            "seed {seed}: to work {shifts} times in one visit"
        );
        choices.extend(&visit.osaka.choices);
        let cached = guest.graphics.as_ref().map_or(0, |g| g.cached());
        assert!(cached < 200, "seed {seed}: {cached} distinct images");
    }
    let count = |kind: Kind| choices.iter().filter(|&&k| k == kind).count();
    for what in [Use::Lounge, Use::Nap, Use::Sleep, Use::Homework, Use::Watch] {
        assert!(
            count(Kind::Use(what)) > 0,
            "she never chose {what:?}: {choices:?}"
        );
    }
    assert!(
        count(Kind::Use(Use::Sleep)) >= count(Kind::Idle(osaka::Activity::LieBack)),
        "the bed beats a border: {choices:?}"
    );
    assert!(count(Kind::Work) > 0, "she never went to work: {choices:?}");
}

/// A pit: a room whose walls are text to the ceiling on the left and
/// protected screen on the right and below, beside her home (a quiet
/// pane up on the right). Returns the screen, its view, and the pit's
/// floor row.
fn pit_screen() -> (Buffer, IdleView, u16) {
    let (width, height) = (100u16, 30u16);
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let pit = Rect::new(0, 0, 50, 20);
    let home = Rect::new(50, 0, 50, 13);
    for area in [pit, home] {
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            area,
            &mut buf,
        );
    }
    for y in 1..=14 {
        buf.set_string(1, y, "x".repeat(48), Style::new());
    }
    let mut protected = bottom_strip(width, height);
    protected.push(Rect::new(50, 13, 50, 14));
    protected.push(Rect::new(0, 0, 1, height));
    let view = IdleView {
        nooks: vec![(Nook::Users, home)],
        ..view(protected)
    };
    (buf, view, pit.bottom() - 1)
}

/// In line art the pit has no way out — no climb past the text, no
/// drop, no screen edge — but with her home next door she still gets
/// there, through a door in space. (In ASCII her body may overlap text,
/// so she can climb out; she only has to get out.)
#[test]
fn she_gets_out_of_a_pit_through_a_door() {
    for graphics in [false, true] {
        let (real, view, floor) = pit_screen();
        let mut guest = Guest::new(5);
        if graphics {
            guest.set_picker(kitty());
            let terrain = Terrain::read(&real, &view.protected, true);
            let pit = terrain
                .platforms
                .iter()
                .position(|p| p.y == i32::from(floor))
                .expect("the pit's floor");
            assert!(
                terrain.links.iter().all(|l| l.from != pit),
                "{:?}",
                terrain.links
            );
        }
        visiting_at(&mut guest, &real, &view, (20, i32::from(floor)));
        guest.give(Furniture::Sofa);
        let (mut out, mut doored) = (false, false);
        let mut now = 0;
        while now < 10 * 60_000 && !out {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if guest.advance(now) {
                let frame = paint(&mut guest, &real, &view, now);
                if graphics && let State::Visiting(visit) = &guest.state {
                    let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
                    assert_nothing_hidden(&frame, &real, &layer)
                        .unwrap_or_else(|e| panic!("at {now}: {e}"));
                }
            }
            let State::Visiting(visit) = &guest.state else {
                panic!("graphics={graphics}: still visiting");
            };
            doored |= visit.osaka.door(now).is_some();
            out |= visit.osaka.y < i32::from(floor)
                && visit
                    .terrain
                    .platform_at(visit.osaka.x, visit.osaka.y)
                    .is_some();
        }
        assert!(guest.ledger.home.owns(Furniture::Sofa));
        assert!(out, "graphics={graphics}: still in the pit after {now} ms");
        assert!(doored || !graphics, "graphics={graphics}: out by a door");
    }
}

/// Stepping out and back in: she walks back on from the screen edge
/// without stumbling (half on screen is still walking in), in both
/// drawing modes.
#[test]
fn she_steps_out_and_walks_back_in_on_her_feet() {
    use super::sprite::Pose;
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(2);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::StepOut);
        paint(&mut guest, &real, &view, 0);
        assert!(
            matches!(guest.cue_note(), Some(Ok(_))),
            "{:?}",
            guest.cue_note()
        );
        let (mut out, mut back) = (false, false);
        let mut now = 0;
        while now < 30_000 && !back {
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            if guest.advance(now) {
                paint(&mut guest, &real, &view, now);
            }
            let State::Visiting(visit) = &guest.state else {
                panic!("graphics={graphics}: still visiting");
            };
            let (pose, ..) = visit.osaka.appearance(now);
            assert!(
                !matches!(pose, Pose::Dazed),
                "graphics={graphics}: stumbled at {now}"
            );
            let on_screen = (0..100).contains(&visit.osaka.x);
            out |= !on_screen;
            back |= out
                && visit
                    .terrain
                    .platform_at(visit.osaka.x, visit.osaka.y)
                    .is_some();
        }
        assert!(back, "graphics={graphics}: out {out}, back {back}");
    }
}

// ---- Her record ----

/// Her home outlives the process: the ledger handed out for saving
/// restores a guest who, on her next visit, has the same furniture in
/// the same pane; each visit counts and draws from its own seed.
#[test]
fn her_home_outlives_a_restart() {
    let (real, view) = home_screen();
    let mut guest = Guest::new(4);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    guest.give(Furniture::Sofa);
    paint(&mut guest, &real, &view, 0);
    let ledger = guest.ledger_to_save().expect("changed");
    assert!(guest.ledger_to_save().is_none(), "handed out once");
    assert_eq!(ledger.visits, 1);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let [before] = visit.shown[..] else {
        panic!("{:?}", visit.shown);
    };

    // A restart: the record round-trips through its stored form.
    let stored = Ledger::from_json(&ledger.to_json()).unwrap();
    let mut guest = Guest::restore(stored);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting again");
    };
    assert!(visit.shown.contains(&before), "the sofa where she left it");
    let ledger = guest.ledger_to_save().expect("a new visit");
    assert_eq!(ledger.visits, 2);
    assert_ne!(ledger.visit_seed(0), ledger.visit_seed(1));
}

/// "Osaka moved out" wipes her home and record (and she leaves at once);
/// a record that couldn't be read is never saved over.
#[test]
fn moving_out_wipes_her_record_and_an_unreadable_one_is_kept() {
    let (real, view) = home_screen();
    let mut guest = Guest::new(4);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    guest.give(Furniture::Sofa);
    paint(&mut guest, &real, &view, 0);
    let _ = guest.ledger_to_save();
    guest.move_out(77);
    assert!(!guest.present());
    let ledger = guest.ledger_to_save().expect("the wipe is saved");
    assert_eq!(ledger, Ledger::new(77));

    let mut guest = Guest::new(4);
    guest.keep_unsaved();
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    guest.give(Furniture::Sofa);
    paint(&mut guest, &real, &view, 0);
    assert!(guest.ledger_to_save().is_none());
}

// ---- Deliveries and the shopping channel ----

/// One whole visit of `minutes`, ending with a key press and the goodbye.
fn one_visit(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64, minutes: u64) -> u64 {
    guest.cue(Scene::Arrive);
    paint(guest, real, view, from);
    let until = from + minutes * 60_000;
    let mut now = from;
    while now < until {
        now += guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        if guest.advance(now) {
            paint(guest, real, view, now);
        }
    }
    guest.activity(now);
    let _ = run(guest, real, view, now, now + dissolve::DURATION_MS);
    now + dissolve::DURATION_MS
}

/// Her home fills up over visits, across a restart: the TV comes boxed
/// on her second visit and she unpacks it; after that the shopping
/// channel sells her one piece at a time, at most once every three
/// visits, and each arrives boxed on a later visit.
#[test]
fn her_home_fills_up_over_visits() {
    let (real, view) = home_screen();
    let mut guest = Guest::new(11);
    let mut now = 0;
    let mut bought: Vec<(u64, Furniture)> = Vec::new();
    for visit in 1..=12u64 {
        if visit == 6 {
            // A restart halfway.
            let ledger = guest
                .ledger_to_save()
                .unwrap_or_else(|| guest.ledger.clone());
            guest = Guest::restore(Ledger::from_json(&ledger.to_json()).unwrap());
        }
        let ordered = guest.ledger.ordered;
        now = one_visit(&mut guest, &real, &view, now, 4);
        assert_eq!(guest.ledger.visits, visit);
        if let Some(item) = guest.ledger.ordered
            && ordered != Some(item)
        {
            bought.push((visit, item));
        }
        if visit == 1 {
            assert!(guest.ledger.home.props.is_empty(), "a first meeting");
        }
        if visit == 2 {
            assert!(guest.ledger.home.owns(Furniture::Tv), "the TV came");
        }
    }
    let home = &guest.ledger.home;
    assert!(
        home.props
            .iter()
            .any(|p| p.item == Furniture::Tv && !p.boxed),
        "she unpacked her TV: {home:?}"
    );
    assert!(
        CATALOGUE.iter().any(|&item| home.owns(item)),
        "she bought something: {bought:?}"
    );
    for pair in bought.windows(2) {
        assert!(pair[1].0 >= pair[0].0 + SHOP_EVERY, "too often: {bought:?}");
    }
    // Nothing turns up that she didn't buy.
    for prop in &home.props {
        assert!(
            prop.item == Furniture::Tv || bought.iter().any(|&(_, item)| item == prop.item),
            "{:?} unbought: {bought:?}",
            prop.item
        );
    }
}

/// A parcel she's interrupted unpacking is still boxed next visit, and
/// she unpacks it then.
#[test]
fn an_interrupted_unpacking_waits_for_her() {
    let (real, view) = home_screen();
    let mut guest = Guest::new(3);
    guest.cue(Scene::Parcel);
    paint(&mut guest, &real, &view, 0);
    let item = guest
        .ledger
        .home
        .props
        .first()
        .map(|p| p.item)
        .expect("a parcel");
    // Straight away: not enough time to unpack.
    let _ = run(&mut guest, &real, &view, 0, 1500);
    assert!(
        guest
            .ledger
            .home
            .props
            .iter()
            .any(|p| p.item == item && p.boxed)
    );
    guest.activity(1500);
    let now = 1500 + dissolve::DURATION_MS;
    let _ = run(&mut guest, &real, &view, 1500, now);
    let now = one_visit(&mut guest, &real, &view, now, 2);
    let _ = now;
    assert!(
        guest
            .ledger
            .home
            .props
            .iter()
            .any(|p| p.item == item && !p.boxed),
        "unpacked on the next visit: {:?}",
        guest.ledger.home
    );
}

// ---- Her part-time job ----

/// Off to work: she's gone for one to three minutes while her room stands
/// furnished; a chat message meanwhile doesn't fetch her; she comes back
/// with her shopping ("I'm home!"). Both ways out — a screen edge, and
/// the door when her floor reaches none — in both drawing modes.
#[test]
fn her_room_stands_furnished_while_she_works() {
    use super::sprite::Pose;
    let edge = {
        let mut ui = stage_ui();
        real_frame(&mut ui, 100, 30)
    };
    for (label, (real, view)) in [("edge", edge), ("door", home_screen())] {
        for graphics in [false, true] {
            let at = format!("{label} graphics={graphics}");
            let mut guest = Guest::new(6);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::Work);
            paint(&mut guest, &real, &view, 0);
            assert!(
                matches!(guest.cue_note(), Some(Ok(_))),
                "{at}: {:?}",
                guest.cue_note()
            );
            let mut view = view.clone();
            let (mut gone_since, mut home, mut nudged) = (None::<u64>, false, false);
            let mut now = 0;
            while now < 4 * 60_000 && !home {
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                if gone_since.is_some_and(|t| now > t + 10_000) && !nudged {
                    // A friend says something while she's out.
                    view.chat_mark.synced += 1;
                    nudged = true;
                }
                guest.advance(now);
                let frame = paint(&mut guest, &real, &view, now);
                let State::Visiting(visit) = &guest.state else {
                    panic!("{at}: still visiting");
                };
                let out = visit.osaka.hidden(now)
                    || !(0..i32::from(real.area.width)).contains(&visit.osaka.x);
                if out && gone_since.is_none() {
                    gone_since = Some(now);
                }
                if out {
                    // Her room is still there.
                    let sofa = visit.shown.iter().find(|s| s.item == Furniture::Sofa);
                    let sofa = sofa.unwrap_or_else(|| panic!("{at}: the sofa is shown"));
                    let r = sofa.rect();
                    assert!(
                        (r.x..r.right())
                            .any(|x| frame.cell((x, r.bottom() - 1))
                                != real.cell((x, r.bottom() - 1))),
                        "{at}: the sofa is drawn"
                    );
                }
                let (pose, _, bubble) = visit.osaka.appearance(now);
                home = matches!(pose, Pose::Carry(_))
                    && bubble == Some(osaka::Bubble::Say("I'm home!"));
            }
            let gone = gone_since.unwrap_or_else(|| panic!("{at}: she never left"));
            assert!(home, "{at}: not back by {now}");
            assert!(nudged, "{at}: the chat came while she was out");
            assert!(
                now - gone >= 55_000,
                "{at}: back after only {} ms",
                now - gone
            );
        }
    }
}

// ---- The rest of the catalogue ----

/// Cue `scene` in the two-pane home with `pieces` (a bedroom and a living
/// room), and run until `until` holds of her, the piece shown and the
/// time; returns whether it ever did.
fn watch_scene(
    scene: Scene,
    pieces: &[Furniture],
    graphics: bool,
    until: impl Fn(&Visit, u64) -> bool,
) -> bool {
    let (real, view) = home_screen();
    let mut guest = Guest::new(8);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    for &item in pieces {
        guest.give(item);
        paint(&mut guest, &real, &view, 0);
        assert!(
            guest.ledger.home.owns(item),
            "{item:?}: {:?}",
            guest.cue_note()
        );
    }
    guest.cue(scene);
    let mut now = 0;
    paint(&mut guest, &real, &view, now);
    while now < 20_000 {
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        let frame = paint(&mut guest, &real, &view, now);
        let State::Visiting(visit) = &guest.state else {
            panic!("{scene:?}: still visiting");
        };
        if graphics {
            let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
            assert_nothing_hidden(&frame, &real, &layer)
                .unwrap_or_else(|e| panic!("{scene:?} at {now}: {e}"));
        }
        if until(visit, now) {
            return true;
        }
    }
    false
}

/// The state `item` is in right now, as drawn.
fn state_of(visit: &Visit, item: Furniture, cat: bool, now: u64) -> Option<art::PieceState> {
    let piece = visit.shown.iter().find(|s| s.item == item)?;
    Some(piece_state(piece, &visit.osaka, cat, now))
}

/// Her lamp goes dark while she sleeps in her bed, the fridge stands open
/// as she looks in, and the cat bites at the end of a petting — each in
/// both drawing modes, never hiding text.
#[test]
fn her_things_answer_what_she_does() {
    use art::PieceState;
    for graphics in [false, true] {
        assert!(
            watch_scene(
                Scene::Sleep,
                &[Furniture::Bed, Furniture::Lamp],
                graphics,
                |v, now| { state_of(v, Furniture::Lamp, false, now) == Some(PieceState::LampOff) }
            ),
            "graphics={graphics}: the lamp went off"
        );
        assert!(
            watch_scene(Scene::Snack, &[], graphics, |v, now| {
                state_of(v, Furniture::Fridge, false, now) == Some(PieceState::FridgeOpen)
            }),
            "graphics={graphics}: the fridge opened"
        );
        assert!(
            watch_scene(Scene::Pet, &[Furniture::Sofa], graphics, |v, now| {
                let (pose, _, bubble) = v.osaka.appearance(now);
                state_of(v, Furniture::CatBed, true, now) == Some(PieceState::CatBiting)
                    && matches!(pose, super::sprite::Pose::Pet(1))
                    && bubble == Some(osaka::Bubble::Say("Ow!"))
            }),
            "graphics={graphics}: the cat bit"
        );
    }
}

// ---- Resident Osaka ----

/// [`rooms`] as a resident sees it during playback: the tall left box is
/// the chat, the two right ones her quiet panes.
fn resident_view(width: u16, height: u16, focus: Option<Rect>) -> IdleView {
    let panes = nooks(width, height);
    IdleView {
        busy: Some(Busy::Playing),
        resident: true,
        focus,
        chat: panes[0].1,
        nooks: panes[1..].to_vec(),
        ..view(bottom_strip(width, height))
    }
}

/// A resident doesn't leave when someone's at the keys.
#[test]
fn a_resident_stays_through_local_input() {
    let real = rooms(100, 30);
    let view = resident_view(100, 30, None);
    let mut guest = Guest::new(11);
    run(&mut guest, &real, &view, 0, 30_000);
    assert!(guest.present());
    for press in [30_000, 30_500, 31_000] {
        guest.activity(press);
        guest.advance(press);
        paint(&mut guest, &real, &view, press);
        assert!(
            matches!(guest.state, State::Visiting(_)),
            "still visiting after a key press"
        );
    }
    run(&mut guest, &real, &view, 31_000, 60_000);
    assert!(matches!(guest.state, State::Visiting(_)));
    // Before she's arrived, input still restarts the idle wait.
    let mut early = Guest::new(11);
    paint(&mut early, &real, &view, 0);
    early.activity(4_000);
    assert!(!early.advance(5_000), "the idle wait restarted");
}

/// A swap out in the chat when someone presses a key: the letters are
/// back at once, nothing is owed, and she stays. Out of the chat, the
/// same swap is left alone.
#[test]
fn a_key_press_shakes_off_what_she_moved_in_the_chat() {
    for in_chat in [true, false] {
        let (real, _) = words_room();
        let mut view = resident_view(100, 30, None);
        if !in_chat {
            view.chat = view.nooks[0].1;
        }
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
            .expect("a swap from where she stands");
        visit.osaka.swap_now(swap, 0);
        let frame = run(&mut guest, &real, &view, 0, 2000);
        assert_eq!(row_text(&frame, 24, 2..13), "the cat sta");
        guest.activity(2001);
        guest.advance(2001);
        let frame = paint(&mut guest, &real, &view, 2001);
        let State::Visiting(visit) = &guest.state else {
            panic!("a resident stays");
        };
        if in_chat {
            assert_eq!(row_text(&frame, 24, 2..13), "the cat sat", "put back");
            assert!(visit.layer.entries().is_empty());
            assert!(!visit.osaka.owes_anything(), "nothing left to undo");
        } else {
            assert_eq!(row_text(&frame, 24, 2..13), "the cat sta", "not the chat");
            assert!(visit.osaka.owes_anything(), "still to swap back");
        }
    }
}

/// Focusing the pane she's in: what of her was there rains away at once
/// (no startled beat), the pane is itself again once the rain is done,
/// and she steps out of her door somewhere else ("Where was I?").
#[test]
fn focusing_her_pane_rains_her_out_and_she_steps_out_elsewhere() {
    for graphics in [false, true] {
        let real = rooms(100, 30);
        let panes = nooks(100, 30);
        let mut guest = Guest::new(21);
        if graphics {
            guest.set_picker(kitty());
        }
        let quiet = resident_view(100, 30, None);
        let mut now = 0;
        paint(&mut guest, &real, &quiet, now);
        // Until she's on screen, standing in one of the panes.
        let (pane, last) = loop {
            now += 250;
            guest.advance(now);
            let last = paint(&mut guest, &real, &quiet, now);
            if let State::Visiting(visit) = &guest.state
                && !visit.osaka.hidden(now)
                && visit.osaka.standing()
                && let Some(&(_, pane)) = panes
                    .iter()
                    .find(|(_, pane)| osaka::box_meets(*pane, (visit.osaka.x, visit.osaka.y)))
                && last != real
            {
                break (pane, last);
            }
            assert!(now < 120_000, "graphics={graphics}: she never stood still");
        };
        let hers = |frame: &Buffer| {
            (pane.top()..pane.bottom())
                .flat_map(|y| (pane.left()..pane.right()).map(move |x| (x, y)))
                .filter(|&at| frame.cell(at) != real.cell(at))
                .count()
        };
        assert!(hers(&last) > 0, "graphics={graphics}: she's in the pane");
        let focused = resident_view(100, 30, Some(pane));
        let landed = now + 1;
        now = landed;
        guest.advance(now);
        let first = paint(&mut guest, &real, &focused, now);
        assert!(
            hers(&first) > 0,
            "graphics={graphics}: the rain starts at once"
        );
        let mut out = false;
        while now < landed + 8000 {
            now += 50;
            guest.advance(now);
            let frame = paint(&mut guest, &real, &focused, now);
            if now >= landed + dissolve::DURATION_MS {
                assert_eq!(
                    hers(&frame),
                    0,
                    "graphics={graphics}: the pane is itself again"
                );
            }
            let State::Visiting(visit) = &guest.state else {
                panic!("graphics={graphics}: a resident stays");
            };
            if !visit.osaka.hidden(now) && visit.osaka.door(now).is_none() {
                assert!(
                    !osaka::box_meets(pane, (visit.osaka.x, visit.osaka.y)),
                    "graphics={graphics}: she's out of the focused pane"
                );
                out = true;
            }
        }
        assert!(out, "graphics={graphics}: she stepped out somewhere else");
    }
}

/// The focused pane is off-limits only while the client is in use. Left
/// alone, she may stay in it; a key press rains her out; after an idle
/// delay's quiet it's hers again.
#[test]
fn the_focused_pane_is_hers_again_when_the_client_is_left_alone() {
    let real = rooms(100, 30);
    let panes = nooks(100, 30);
    let mut guest = Guest::new(21);
    let alone = |focus| IdleView {
        busy: None,
        ..resident_view(100, 30, focus)
    };
    let mut now = 0;
    paint(&mut guest, &real, &alone(None), now);
    let pane = loop {
        now += 250;
        guest.advance(now);
        paint(&mut guest, &real, &alone(None), now);
        if let State::Visiting(visit) = &guest.state
            && !visit.osaka.hidden(now)
            && visit.osaka.standing()
            && let Some(&(_, pane)) = panes
                .iter()
                .find(|(_, pane)| osaka::box_meets(*pane, (visit.osaka.x, visit.osaka.y)))
        {
            break pane;
        }
        assert!(now < 120_000, "she never stood still");
    };
    let at = |guest: &Guest| match &guest.state {
        State::Visiting(visit) => (visit.osaka.x, visit.osaka.y),
        _ => panic!("a resident stays"),
    };
    let view = alone(Some(pane));
    // Nobody's at the keys: focusing her pane changes nothing.
    let spot = at(&guest);
    now += 1;
    guest.advance(now);
    paint(&mut guest, &real, &view, now);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    assert!(
        visit.fades.is_empty(),
        "no rain while the client is left alone"
    );
    assert_eq!(at(&guest), spot, "she stays where she is");
    assert_eq!(guest.gate(&view, now).focus, None);
    // A key press: the pane is protected, and she rains out of it.
    guest.activity(now);
    guest.advance(now);
    paint(&mut guest, &real, &view, now);
    let State::Visiting(visit) = &guest.state else {
        panic!("a resident stays");
    };
    assert!(!visit.fades.is_empty(), "she rains out of the focused pane");
    let pressed = now;
    assert_eq!(guest.gate(&view, now).focus, Some(pane));
    // A delay's quiet later, it's hers again.
    run(
        &mut guest,
        &real,
        &view,
        now,
        pressed + DELAY.as_millis() as u64,
    );
    now = pressed + DELAY.as_millis() as u64;
    assert_eq!(guest.gate(&view, now).focus, None);
    assert!(!guest.gate(&view, now).protected.contains(&pane));
}

/// Whether she stands in `rect`.
fn osaka_in(visit: &Visit, rect: Rect) -> bool {
    let (x, y) = (visit.osaka.x, visit.osaka.y);
    x >= 0 && y >= 0 && rect.contains((x as u16, y as u16).into())
}

/// Over long visits a resident spends far less of her time in the chat
/// than a visitor does (the chat's offers are a tenth as likely).
#[test]
fn a_resident_mostly_keeps_out_of_the_chat() {
    let real = rooms(100, 30);
    let chat = nooks(100, 30)[0].1;
    let mut share = [0.0; 2];
    for (i, resident) in [false, true].into_iter().enumerate() {
        let view = IdleView {
            resident,
            busy: None,
            ..resident_view(100, 30, None)
        };
        let (mut inside, mut total) = (0u64, 0u64);
        for seed in 0..4 {
            let mut guest = Guest::new(seed);
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            while now < 15 * 60_000 {
                now += 1000;
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
                if let State::Visiting(visit) = &guest.state
                    && !visit.osaka.hidden(now)
                {
                    total += 1;
                    inside += u64::from(osaka_in(visit, chat));
                }
            }
        }
        share[i] = inside as f64 / total.max(1) as f64;
    }
    let [visitor, resident] = share;
    eprintln!("time in the chat: visitor {visitor:.2}, resident {resident:.2}");
    assert!(
        resident < visitor * 0.5,
        "visitor {visitor:.2}, resident {resident:.2}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    /// A resident through focus changes and key presses: she never
    /// leaves; a newly focused pane shows only the rain over what of
    /// hers was in it, and nothing of hers once that's done; the
    /// protected strip is never touched.
    #[test]
    fn a_resident_keeps_out_of_the_focused_pane(
        seed in any::<u64>(),
        graphics in any::<bool>(),
        (w, h) in (70u16..130, 24u16..45),
        text in proptest::collection::vec((0u16..60, 0u16..18, "[a-z ]{1,6}"), 0..20),
        focuses in proptest::collection::vec((0u64..120_000, proptest::option::of(0usize..3)), 1..8),
        presses in proptest::collection::vec(0u64..120_000, 0..10),
        owned in proptest::collection::vec((0usize..4, 0usize..2, 0u16..=1000, any::<bool>()), 0..4),
    ) {
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        let panes = nooks(w, h);
        let quiet = [Nook::Users, Nook::Playlist];
        for &(item, at, along, left) in &owned {
            let _ = guest.ledger.home.add(
                quiet[at],
                room::Prop {
                    item: Furniture::ALL[item],
                    at: along,
                    facing: if left { sprite::Facing::Left } else { sprite::Facing::Right },
                    boxed: false,
                },
            );
        }
        let mut real = rooms(w, h);
        scatter(&mut real, &text, &[]);
        let base = bottom_strip(w, h);
        let mut focuses = focuses;
        focuses.sort_unstable();
        // The resident is on screen from her arrival.
        let mut arrived = false;
        let mut focus: Option<Rect> = None;
        // When the current focus landed, and what of hers was in it.
        let mut landed = 0;
        let mut hers_there: Vec<(u16, u16)> = Vec::new();
        let mut last = real.clone();
        let mut now = 0;
        let until = 120_000;
        while now < until {
            let step = guest
                .next_tick(now)
                .map_or(until - now, |d| d.as_millis() as u64)
                .clamp(1, 1000.min(until - now));
            now += step;
            if presses.iter().any(|&p| p <= now && p > now - step) {
                guest.activity(now);
            }
            let next = focuses
                .iter()
                .rev()
                .find(|(at, _)| *at <= now)
                .and_then(|(_, pane)| pane.map(|i| panes.get(i).map_or(panes[0].1, |p| p.1)));
            if next != focus {
                focus = next;
                landed = now;
                hers_there = focus.map_or_else(Vec::new, |rect| {
                    last.content
                        .iter()
                        .zip(&real.content)
                        .enumerate()
                        .filter(|(_, (a, b))| a != b)
                        .map(|(i, _)| ((i % usize::from(w)) as u16, (i / usize::from(w)) as u16))
                        .filter(|&(x, y)| rect.contains((x, y).into()))
                        .collect()
                });
            }
            let view = resident_view(w, h, focus);
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            assert_untouched(&frame, &real, &base)?;
            arrived |= guest.present();
            if arrived {
                prop_assert!(
                    matches!(guest.state, State::Visiting(_)),
                    "a resident never leaves (at {})", now
                );
            }
            if let (Some(rect), State::Visiting(visit)) = (focus, &guest.state) {
                let osaka = &visit.osaka;
                prop_assert!(
                    osaka.hidden(now)
                        || osaka.door(now).is_some()
                        || !osaka::box_meets(rect, (osaka.x, osaka.y)),
                    "she's in the focused pane at ({}, {}) at {}: {:?}", osaka.x, osaka.y, now, osaka
                );
            }
            // She may stand on a line of the focused pane (its top
            // border, her body in the pane above): her image redraws the
            // floor under her feet, as on any protected line.
            let feet = match &guest.state {
                State::Visiting(visit) if visit.osaka.standing() => {
                    Some((visit.osaka.x, visit.osaka.y))
                }
                _ => None,
            };
            if let Some(rect) = focus {
                for y in rect.top()..rect.bottom() {
                    for x in rect.left()..rect.right() {
                        let (got, want) = (frame.cell((x, y)).unwrap(), real.cell((x, y)).unwrap());
                        if got == want {
                            continue;
                        }
                        let underfoot = feet.is_some_and(|(fx, fy)| {
                            i32::from(y) == fy && (i32::from(x) - fx).abs() <= sprite::WIDTH / 2
                        }) && want.symbol().chars().next().is_some_and(|c| graphics::strokes(c).is_some());
                        if underfoot {
                            continue;
                        }
                        prop_assert!(
                            now < landed + dissolve::DURATION_MS && hers_there.contains(&(x, y)),
                            "({}, {}) in the focused pane shows {:?} at {} (focused at {})",
                            x, y, got.symbol(), now, landed
                        );
                    }
                }
            }
            last = frame;
        }
    }
}

/// [`rooms`] with the tall left box as the chat, scrolled back: its
/// bottom border is the accordion, `unseen` counted in its middle (as
/// the chat pane draws it), and protected like the real one.
fn accordion_room(width: u16, height: u16, unseen: usize) -> (Buffer, Rect) {
    let mut buf = rooms(width, height);
    let accordion = Rect::new(1, height - 4, width / 2 - 2, 1);
    for (i, x) in (accordion.left()..accordion.right()).enumerate() {
        buf[(x, accordion.y)].set_symbol(if i % 2 == 0 { "╱" } else { "╲" });
    }
    let label = format!(" ↓ {unseen} new ");
    let at = accordion.x + (accordion.width - label.chars().count() as u16) / 2;
    buf.set_string(at, accordion.y, &label, Style::new());
    (buf, accordion)
}

fn scrolled_back(view: IdleView, accordion: Rect, unseen: usize) -> IdleView {
    let mut view = view;
    view.scrollback = Some(Scrollback { accordion, unseen });
    view.protected.push(accordion);
    view
}

/// What the errand looked like, frame by frame.
#[derive(Debug, Default)]
struct ErrandSeen {
    /// When she first appeared.
    arrived: Option<u64>,
    /// When she started poking, and where she stood.
    poked: Option<(u64, (i32, i32))>,
    /// When the painted accordion first differed from the real one.
    shook: Option<u64>,
    /// When she was gone again (leaving or absent).
    gone: Option<u64>,
}

fn watch_errand(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    accordion: Rect,
    until: u64,
) -> ErrandSeen {
    let mut seen = ErrandSeen::default();
    let mut now = 0;
    paint(guest, real, view, now);
    while now < until {
        now += 50;
        guest.advance(now);
        let frame = paint(guest, real, view, now);
        match &guest.state {
            State::Visiting(visit) => {
                seen.arrived.get_or_insert(now);
                let poking = visit.osaka.appearance(now).0 == sprite::Pose::ToeTouch(0)
                    || visit.osaka.appearance(now).0 == sprite::Pose::ToeTouch(1);
                if poking && guest.errand.is_some() && seen.poked.is_none() {
                    seen.poked = Some((now, (visit.osaka.x, visit.osaka.y)));
                }
            }
            State::Leaving(_) | State::Absent => {
                if seen.arrived.is_some() {
                    seen.gone.get_or_insert(now);
                }
            }
            State::Arriving => {}
        }
        let moved = (accordion.left()..accordion.right())
            .any(|x| frame[(x, accordion.y)] != real[(x, accordion.y)]);
        if moved {
            seen.shook.get_or_insert(now);
        }
    }
    seen
}

/// Scrolled back with messages unseen for a minute, she arrives even
/// while the video plays — as a visitor, who then leaves, or a resident,
/// for whom even the focused chat pane is no obstacle — stands on the
/// accordion and pokes it, and it shakes. Nothing happens before the
/// minute is up.
#[test]
fn unseen_messages_bring_her_to_poke_the_accordion() {
    for resident in [false, true] {
        let (real, accordion) = accordion_room(100, 30, 2);
        let chat = nooks(100, 30)[0].1;
        let base = IdleView {
            busy: Some(Busy::Playing),
            resident,
            focus: resident.then_some(chat),
            chat,
            ..view(bottom_strip(100, 30))
        };
        let view = scrolled_back(base, accordion, 2);
        let mut guest = Guest::new(5);
        let seen = watch_errand(&mut guest, &real, &view, accordion, 90_000);
        // A resident was here all along; a visitor comes for it.
        let arrived = seen.arrived.expect("she came");
        let expected = if resident {
            DELAY.as_millis() as u64..nudge::NUDGE_MS
        } else {
            nudge::NUDGE_MS..nudge::NUDGE_MS + 100
        };
        assert!(expected.contains(&arrived), "resident={resident}: {seen:?}");
        let (poked, (_, y)) = seen.poked.expect("she poked it");
        assert_eq!(y, i32::from(accordion.y), "standing on the accordion");
        let shook = seen.shook.expect("it shook");
        assert!(shook >= poked && shook < poked + 500, "{seen:?}");
        if resident {
            // Done, she's out of the focused pane.
            let State::Visiting(visit) = &guest.state else {
                panic!("a resident stays");
            };
            assert!(
                visit.osaka.hidden(90_000)
                    || !osaka::box_meets(chat, (visit.osaka.x, visit.osaka.y)),
                "out of the focused chat after her errand"
            );
        } else {
            let gone = seen.gone.expect("a visitor leaves: the video's playing");
            assert!(gone > poked, "{seen:?}");
        }
    }
}

/// Visits off: the accordion shakes by itself, once a minute while more
/// messages keep arriving, and not again when none do.
#[test]
fn with_visits_off_the_accordion_shakes_by_itself() {
    let (real, accordion) = accordion_room(100, 30, 1);
    let off = IdleView {
        delay: None,
        ..view(bottom_strip(100, 30))
    };
    let mut guest = Guest::new(5);
    let seen = watch_errand(
        &mut guest,
        &real,
        &scrolled_back(off.clone(), accordion, 1),
        accordion,
        nudge::NUDGE_MS + 5_000,
    );
    assert!(seen.arrived.is_none());
    let shook = seen.shook.expect("it shook");
    assert!((nudge::NUDGE_MS..nudge::NUDGE_MS + 200).contains(&shook));
    // Nothing new: no more shaking.
    let quiet = scrolled_back(off.clone(), accordion, 1);
    let mut now = nudge::NUDGE_MS + 5_000;
    while now < 3 * nudge::NUDGE_MS {
        now += 250;
        guest.advance(now);
        paint(&mut guest, &real, &quiet, now);
        assert!(!guest.nudge.shaking(now), "at {now}");
    }
    // Another message: a minute after the last poke.
    let more = scrolled_back(off, accordion, 2);
    now += 1;
    paint(&mut guest, &real, &more, now);
    assert!(guest.next_tick(now).is_some());
    now += 1000;
    guest.advance(now);
    paint(&mut guest, &real, &more, now);
    assert!(
        guest.nudge.shaking(now),
        "due again (the last was over a minute ago)"
    );
}

/// A key press while a visitor pokes the accordion: she finishes, then
/// leaves.
#[test]
fn a_visitor_finishes_poking_before_she_leaves() {
    let (real, accordion) = accordion_room(100, 30, 3);
    let chat = nooks(100, 30)[0].1;
    let view = scrolled_back(
        IdleView {
            chat,
            ..view(bottom_strip(100, 30))
        },
        accordion,
        3,
    );
    let mut guest = Guest::new(9);
    // Someone's typing, now and then: no ordinary visit, but the errand.
    let mut now = 0;
    paint(&mut guest, &real, &view, now);
    while guest.errand.as_ref().is_none_or(|errand| !errand.poked) {
        now += 50;
        if now % 3000 == 0 {
            guest.activity(now);
        }
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert!(now < nudge::NUDGE_MS + 20_000, "she never poked it");
    }
    guest.activity(now);
    let pressed = now;
    while now < pressed + 1500 {
        now += 50;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert!(
            matches!(guest.state, State::Visiting(_)),
            "still poking at {now}"
        );
    }
    while now < pressed + 3000 {
        now += 50;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
    }
    assert!(
        matches!(guest.state, State::Leaving(_) | State::Absent),
        "gone once done"
    );
}

/// Back at the newest line mid-errand: it's off.
#[test]
fn following_the_newest_line_calls_the_errand_off() {
    let (real, accordion) = accordion_room(100, 30, 1);
    let chat = nooks(100, 30)[0].1;
    let base = IdleView {
        busy: Some(Busy::Playing),
        chat,
        ..view(bottom_strip(100, 30))
    };
    let back = scrolled_back(base.clone(), accordion, 1);
    let mut guest = Guest::new(5);
    let mut now = 0;
    paint(&mut guest, &real, &back, now);
    while guest.errand.is_none() {
        now += 50;
        guest.advance(now);
        paint(&mut guest, &real, &back, now);
        assert!(now < nudge::NUDGE_MS + 1000);
    }
    let plain = rooms(100, 30);
    now += 50;
    guest.advance(now);
    paint(&mut guest, &plain, &base, now);
    assert!(guest.errand.is_none(), "called off");
    assert!(
        matches!(guest.state, State::Leaving(_) | State::Absent),
        "a visitor, and the video's playing"
    );
    assert!(!guest.nudge.shaking(now), "nothing left to shake");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(8)))]

    /// Scrolled back over arbitrary sizes, message arrivals, key presses
    /// and playback, visitor or resident: nothing protected changes but
    /// the accordion, which only ever shows as itself or slid a column;
    /// every errand ends; and after a last key press with nothing new
    /// arriving, a visitor is soon gone and the frame is the real one.
    #[test]
    fn errands_end_and_touch_only_the_accordion(
        seed in any::<u64>(),
        graphics in any::<bool>(),
        (w, h) in (60u16..130, 18u16..45),
        resident in any::<bool>(),
        playing in any::<bool>(),
        arrivals in proptest::collection::vec(0u64..60_000, 1..6),
        presses in proptest::collection::vec(0u64..130_000, 0..20),
    ) {
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        let chat = nooks(w, h)[0].1;
        let mut now = 0;
        let mut errand_since: Option<u64> = None;
        let view_at = |unseen: usize| {
            let (real, accordion) = accordion_room(w, h, unseen);
            let base = IdleView {
                busy: playing.then_some(Busy::Playing),
                resident,
                focus: resident.then_some(chat),
                chat,
                nooks: nooks(w, h)[1..].to_vec(),
                ..view(bottom_strip(w, h))
            };
            (real, accordion, scrolled_back(base, accordion, unseen))
        };
        let mut paints = 0;
        while now < 130_000 {
            let step = guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            now += step;
            paints += 1;
            prop_assert!(paints < 20_000, "busy at {}: {:?} errand={} next={:?}", now, guest.nudge, guest.errand.is_some(), guest.next_tick(now));
            if presses.iter().any(|&p| p <= now && p > now - step) {
                guest.activity(now);
            }
            let unseen = arrivals.iter().filter(|&&a| a <= now).count();
            let (real, accordion, view) = view_at(unseen);
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let others: Vec<Rect> = view
                .protected
                .iter()
                .copied()
                .filter(|&r| r != accordion)
                .collect();
            assert_untouched(&frame, &real, &others)?;
            let row = |buf: &Buffer| -> Vec<String> {
                (accordion.left()..accordion.right())
                    .map(|x| buf[(x, accordion.y)].symbol().to_owned())
                    .collect()
            };
            let (got, want) = (row(&frame), row(&real));
            let slid = |by: usize| {
                let mut r = want.clone();
                r.rotate_right(by);
                r
            };
            prop_assert!(
                got == want || got == slid(1) || got == slid(want.len() - 1),
                "the accordion at {}: {:?}", now, got.concat()
            );
            match (&guest.errand, errand_since) {
                (Some(_), None) => errand_since = Some(now),
                (Some(_), Some(since)) => {
                    prop_assert!(now - since < 60_000, "an errand since {} still on at {}", since, now);
                }
                (None, _) => errand_since = None,
            }
        }
        // Back to the newest line, and typing away: the errand's off,
        // and a visitor is soon gone.
        let real = rooms(w, h);
        let view = IdleView {
            busy: playing.then_some(Busy::Playing),
            resident,
            chat,
            nooks: nooks(w, h)[1..].to_vec(),
            ..view(bottom_strip(w, h))
        };
        let mut end = paint(&mut guest, &real, &view, now);
        for _ in 0..30 {
            now += 1000;
            guest.activity(now);
            guest.advance(now);
            end = paint(&mut guest, &real, &view, now);
        }
        prop_assert!(guest.errand.is_none());
        if !resident {
            prop_assert!(matches!(guest.state, State::Absent));
            prop_assert_eq!(end, real);
        }
    }
}

/// The real default layout: scrolled back (PgUp) with two messages
/// arriving below, the chat reports its accordion with the count, and
/// it's a floor she can stand on to poke it.
#[test]
fn the_real_chat_scrolled_back_offers_her_a_spot_on_its_accordion() {
    use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};
    let chat = |n: usize| {
        let view = dessplay_core::state::StateView {
            chat: (0..n)
                .map(|i| dessplay_core::types::ChatMessage {
                    timestamp: dessplay_core::types::SharedTimestamp(1_000 + i as u64 * 60_000),
                    sender: dessplay_core::types::UserId::new("kim"),
                    text: format!("line {i}"),
                })
                .collect(),
            ..Default::default()
        };
        crate::ui::app::UiSnapshot {
            view: std::sync::Arc::new(view),
            ..Default::default()
        }
    };
    let mut ui = real_ui();
    ui.apply_snapshot(chat(40));
    let (_, view) = real_frame(&mut ui, 100, 30);
    assert_eq!(view.scrollback, None, "following the newest line");
    ui.handle(Event::Keyboard(KeyEvent {
        code: Key::PageUp,
        modifiers: KeyModifiers::NONE,
    }));
    real_frame(&mut ui, 100, 30);
    ui.apply_snapshot(chat(42));
    let (buf, view) = real_frame(&mut ui, 100, 30);
    let back = view.scrollback.expect("scrolled back");
    assert_eq!(back.unseen, 2);
    assert!(view.chat.contains(back.accordion.as_position()));
    assert!(view.protected.contains(&back.accordion));
    let terrain = Terrain::read(&buf, &view.protected, false);
    let spot = accordion_spot(&terrain, back.accordion).expect("a floor on it");
    assert_eq!(spot.1, i32::from(back.accordion.y));
}
