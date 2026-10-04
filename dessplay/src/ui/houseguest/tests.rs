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
            synced_asks: false,
            irc: 0,
            irc_asks: false,
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
    assert_untouched_but_feet(frame, real, protected, None)
}

/// [`assert_untouched`], except that standing at `feet` in line art, her
/// image redraws the line under her feet — she may stand on a protected
/// line (design.md, Houseguest: "though she may stand on a line inside
/// one").
fn assert_untouched_but_feet(
    frame: &Buffer,
    real: &Buffer,
    protected: &[Rect],
    feet: Option<(i32, i32)>,
) -> Result<(), TestCaseError> {
    let underfoot = |x: u16, y: u16, want: &tuirealm::ratatui::buffer::Cell| {
        feet.is_some_and(|(fx, fy)| {
            i32::from(y) == fy && (i32::from(x) - fx).abs() <= sprite::WIDTH / 2
        }) && want
            .symbol()
            .chars()
            .next()
            .is_some_and(|c| graphics::strokes(c).is_some())
    };
    let area = real.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let position = Position::new(x, y);
            let (Some(got), Some(want)) = (frame.cell(position), real.cell(position)) else {
                continue;
            };
            let redrawn = underfoot(x, y, want) && cells::untouchable(got);
            if (protected.iter().any(|r| r.contains(position)) && !redrawn)
                || cells::untouchable(want)
            {
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

/// A Ghostty-like picker: kitty protocol, 9×19 px cells.
fn kitty() -> ratatui_image::picker::Picker {
    kitty_cells(9, 19)
}

/// A kitty-protocol picker with `width`×`height`-pixel cells. (The fixed
/// font size constructor is the only deterministic one.)
#[allow(deprecated)]
fn kitty_cells(width: u16, height: u16) -> ratatui_image::picker::Picker {
    let mut picker = ratatui_image::picker::Picker::from_fontsize((width, height).into());
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    picker
}

/// Text her line art covers, and since when: she may pass in front of
/// text (it's derezzed into alien glyphs), but never stay over it.
#[derive(Default)]
struct Hidden(std::collections::BTreeMap<(u16, u16), u64>);

/// The longest any text stays hidden behind her image.
const HIDDEN_MS: u64 = 10_000;

impl Hidden {
    /// Check a frame: her image covers only blank cells, lines, and text
    /// in passing; anything else changed is her text layer.
    fn check(
        &mut self,
        frame: &Buffer,
        real: &Buffer,
        layer: &[(u16, u16)],
        now: u64,
    ) -> Result<(), TestCaseError> {
        self.check_raining(frame, real, layer, |_| false, now)
    }

    /// [`Hidden::check`], but for the cells a rain of hers is painting
    /// (`raining`): its frozen copy of what was there, whatever it was.
    fn check_raining(
        &mut self,
        frame: &Buffer,
        real: &Buffer,
        layer: &[(u16, u16)],
        raining: impl Fn((u16, u16)) -> bool,
        now: u64,
    ) -> Result<(), TestCaseError> {
        let width = real.area.width as usize;
        let mut hidden = std::collections::BTreeSet::new();
        for (index, (got, want)) in frame.content.iter().zip(&real.content).enumerate() {
            let at = ((index % width) as u16, (index / width) as u16);
            if got == want || raining(at) {
                continue;
            }
            let blank = want.symbol().trim().is_empty();
            let line = want
                .symbol()
                .chars()
                .next()
                .is_some_and(|c| graphics::strokes(c).is_some());
            if cells::untouchable(got) {
                prop_assert!(
                    cells::width(want) <= 1,
                    "wide glyph {:?} at {:?} behind her image",
                    want.symbol(),
                    at
                );
                if !blank && !line {
                    hidden.insert(at);
                }
            } else {
                prop_assert!(
                    blank || layer.contains(&at),
                    "cell {:?} ({:?}) changed outside the text layer",
                    at,
                    want.symbol()
                );
            }
        }
        self.0.retain(|at, _| hidden.contains(at));
        for at in hidden {
            let since = *self.0.entry(at).or_insert(now);
            prop_assert!(
                now - since <= HIDDEN_MS,
                "text at {:?} hidden behind her since {} (now {})",
                at,
                since,
                now
            );
        }
        Ok(())
    }
}

fn scatter(buf: &mut Buffer, text: &[(u16, u16, String)], skips: &[(u16, u16)]) {
    for (x, y, s) in text {
        // Off a small screen, it isn't there.
        if buf.area.contains((*x, *y).into()) {
            buf.set_string(*x, *y, s, Style::new());
        }
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
        let owned: Vec<(Furniture, usize, u16, bool)> = owned
            .into_iter()
            .map(|(item, at, along, left)| (Furniture::ALL[item], at, along, left))
            .collect();
        long_visit(seed, graphics, &sizes, &text, &skips, &chats, protect, &owned, 200_000)?;
    }

    /// [`long_visits_never_touch_what_is_protected`], shorter, with decor
    /// among her pieces (a poster hung over what stands, a plant beside
    /// it), owned before or after the rest.
    #[test]
    fn long_visits_with_decor_never_touch_what_is_protected(
        seed in any::<u64>(),
        graphics in any::<bool>(),
        sizes in proptest::collection::vec((60u16..130, 18u16..45), 1..3),
        text in proptest::collection::vec((0u16..60, 0u16..18, "[a-z漢─│ ]{1,6}"), 0..20),
        skips in proptest::collection::vec((0u16..60, 0u16..18), 0..6),
        chats in proptest::collection::vec(0u64..60_000, 0..3),
        protect in (0u16..40, 0u16..10, 1u16..20, 1u16..6),
        owned in proptest::collection::vec((0usize..4, 0usize..3, 0u16..=1000, any::<bool>()), 0..4),
        decor in proptest::collection::vec((any::<bool>(), 0usize..3, 0u16..=1000, any::<bool>()), 1..3),
        decor_first in any::<bool>(),
    ) {
        let mut owned: Vec<(Furniture, usize, u16, bool)> = owned
            .into_iter()
            .map(|(item, at, along, left)| (Furniture::ALL[item], at, along, left))
            .collect();
        let decor = decor.into_iter().map(|(poster, at, along, left)| {
            let item = if poster { Furniture::Poster } else { Furniture::Plant };
            (item, at, along, left)
        });
        if decor_first {
            owned.splice(0..0, decor);
        } else {
            owned.extend(decor);
        }
        long_visit(seed, graphics, &sizes, &text, &skips, &chats, protect, &owned, 60_000)?;
    }
}

/// One long visit, `span` ms over `sizes` in turn (see
/// [`long_visits_never_touch_what_is_protected`]), with her pieces
/// `owned` (each: what, which pane, how far along, facing left).
#[allow(clippy::too_many_arguments)]
fn long_visit(
    seed: u64,
    graphics: bool,
    sizes: &[(u16, u16)],
    text: &[(u16, u16, String)],
    skips: &[(u16, u16)],
    chats: &[u64],
    protect: (u16, u16, u16, u16),
    owned: &[(Furniture, usize, u16, bool)],
    span: u64,
) -> Result<(), TestCaseError> {
    long_visit_of(
        Guest::new(seed),
        graphics,
        sizes,
        text,
        skips,
        chats,
        protect,
        owned,
        span,
    )
}

/// [`long_visit`] for `guest` as made (her clock somewhere in her week,
/// say): visiting, or her home standing empty while she's out.
#[allow(clippy::too_many_arguments)]
fn long_visit_of(
    mut guest: Guest,
    graphics: bool,
    sizes: &[(u16, u16)],
    text: &[(u16, u16, String)],
    skips: &[(u16, u16)],
    chats: &[u64],
    protect: (u16, u16, u16, u16),
    owned: &[(Furniture, usize, u16, bool)],
    span: u64,
) -> Result<(), TestCaseError> {
    if graphics {
        guest.set_picker(kitty());
    }
    let nook = [Nook::List, Nook::Users, Nook::Playlist];
    for &(item, at, along, left) in owned {
        // A second room offered a taken pane is refused, as in play.
        let _ = guest.ledger.home.add(room::Prop::new(
            item,
            nook[at],
            along,
            if left {
                sprite::Facing::Left
            } else {
                sprite::Facing::Right
            },
        ));
    }
    let mut hidden = Hidden::default();
    let mut now = 0;
    let span = span / sizes.len() as u64;
    let mut mark = ChatMark::default();
    for &(w, h) in sizes {
        let mut real = rooms(w, h);
        scatter(&mut real, text, skips);
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
            let feet = match &guest.state {
                State::Visiting(visit) => feet(visit),
                // Her closed door stands on its floor (in line art, its
                // image redraws the line under it).
                State::Away(_) => guest.closed_door().map(|door| (door.x, door.y)),
                _ => None,
            };
            assert_untouched_but_feet(&frame, &real, &protected, feet)?;
            let (layer, shown): (Vec<(u16, u16)>, Vec<Shown>) = match &guest.state {
                // What she moved, and the flap a parcel just came in by
                // (swung in over her wall's line a moment).
                State::Visiting(visit) => {
                    let flap = visit.flap.into_iter().flat_map(|(flap, _)| {
                        (flap.rows.0..flap.rows.1).filter_map(move |y| {
                            Some((u16::try_from(flap.x).ok()?, u16::try_from(y).ok()?))
                        })
                    });
                    (
                        visit.layer.cells().chain(flap).collect(),
                        visit.shown.clone(),
                    )
                }
                State::Away(empty) => (Vec::new(), empty.shown.clone()),
                _ => (Vec::new(), Vec::new()),
            };
            // Each real piece stands on the floor of a strip that's
            // here this frame.
            let strips = room::strips(&view.nooks);
            for prop in shown.iter().filter(|s| s.scrap.is_none()) {
                let here = strips.iter().find(|(s, _)| Some(*s) == prop.strip);
                prop_assert!(
                    here.is_some_and(|(_, e)| e.floor == prop.floor),
                    "{:?}",
                    prop
                );
            }
            // Her furniture stands on lines (or hangs above them), over
            // blank cells only, clear of protected cells, text she
            // moved, and her.
            for prop in &shown {
                let (cols, _) = prop.size();
                let floor = (prop.left..prop.left + i32::from(cols))
                    .filter(|_| prop.lift() == 0)
                    .map(|x| (x, prop.floor, None));
                for (x, y, _) in prop.cells().chain(floor) {
                    let at = (x as u16, y as u16);
                    let want = real.cell(at).unwrap();
                    prop_assert!(!cells::untouchable(want));
                    prop_assert!(
                        !protected.iter().any(|r| r.contains(at.into())),
                        "{:?} protected",
                        at
                    );
                    prop_assert!(!layer.contains(&at), "{:?} over moved text", at);
                    if y < prop.floor {
                        prop_assert!(
                            want.symbol().trim().is_empty(),
                            "{:?} over {:?}",
                            at,
                            want.symbol()
                        );
                    } else {
                        prop_assert!(
                            want.symbol()
                                .chars()
                                .next()
                                .is_some_and(|c| graphics::strokes(c).is_some())
                        );
                    }
                }
            }
            if graphics {
                // What she moved raining out as she went out by her
                // routine shows it as it was, holes and all, a moment.
                hidden.check_raining(&frame, &real, &layer, |at| raining(&guest, at), now)?;
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
                    a != b && !layer.contains(&at) && !raining(&guest, at)
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
                    let with = visit.image.iter().flat_map(|i| &i.with);
                    let union = with.fold(her, |r, p| r.union(p.cover()));
                    (furniture, area(union))
                }
                // Her door's box, and the pieces its image takes in.
                State::Away(_) => {
                    let area = |r: Rect| usize::from(r.width) * usize::from(r.height);
                    let furniture: usize = shown.iter().map(|p| area(p.cover())).sum();
                    let spanned = guest.closed_door().map_or(0, |door| {
                        let box_ = Rect::new(
                            (door.x - sprite::WIDTH / 2).max(0) as u16,
                            (door.y - sprite::HEIGHT).max(0) as u16,
                            sprite::WIDTH as u16,
                            sprite::HEIGHT as u16 + 1,
                        );
                        let covers: Vec<Rect> = shown.iter().map(Shown::cover).collect();
                        let with = terrain::image(door.x, door.y, &covers).with;
                        let union = covers
                            .iter()
                            .zip(with)
                            .filter(|(_, with)| *with)
                            .fold(box_, |r, (c, _)| r.union(*c));
                        area(union)
                    });
                    (furniture, spanned)
                }
                _ => (0, 0),
            };
            let most = (sprite::WIDTH * (sprite::HEIGHT + 1)) as usize + 24 + furniture + spanned;
            let debug = match &guest.state {
                State::Visiting(visit) => format!(
                    "{:?} with {:?} at ({}, {})",
                    visit.shown,
                    visit.image.as_ref().map(|i| &i.with),
                    visit.osaka.x,
                    visit.osaka.y
                ),
                State::Away(empty) => {
                    format!("away: {:?} door {:?}", empty.shown, guest.closed_door())
                }
                _ => String::new(),
            };
            prop_assert!(
                changed <= most,
                "{} cells changed (most {}): {}",
                changed,
                most,
                debug
            );
        }
    }
    let &(w, h) = sizes.last().unwrap();
    let mut real = rooms(w, h);
    scatter(&mut real, text, skips);
    let view = IdleView {
        nooks: nooks(w, h),
        ..view(bottom_strip(w, h))
    };
    guest.activity(now);
    let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
    prop_assert_eq!(end, real);
    Ok(())
}

/// Whether a cell is what she moved and made raining out as she went
/// out by her routine (her home empty: the rain's cells are its own).
fn raining(guest: &Guest, (x, y): (u16, u16)) -> bool {
    match &guest.state {
        State::Away(empty) => empty.fades.iter().any(|fade| fade.painting(x, y)),
        _ => false,
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
    let (_, frame) = run_until_seen(&mut guest, &real, &view, 95_000);
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

/// Reading the terrain runs on every frame she is on screen: under 2 ms
/// (the fastest of 100, in a release build).
#[test]
fn terrain_read_is_cheap() {
    let mut ui = real_ui();
    let (buf, view) = real_frame(&mut ui, 200, 60);
    // The fastest of 100: what a read costs. Tests running alongside (a
    // deep pass loads every core) only ever make a run slower, so a mean
    // would measure the machine.
    let per_read = (0..100)
        .map(|_| {
            let started = std::time::Instant::now();
            std::hint::black_box(Terrain::read(&buf, &view.protected, true));
            started.elapsed()
        })
        .min()
        .unwrap_or_default();
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
/// Run a visit until `from`, then on until she's on screen and on her
/// feet (not through a door, stepped out or in the air), well inside
/// the screen's edges. Returns when, and the frame.
fn run_until_seen(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> (u64, Buffer) {
    let mut now = from;
    let mut frame = run(guest, real, view, 0, now);
    loop {
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        let osaka = &visit.osaka;
        let width = i32::from(real.area.width);
        let inside = osaka.x > 2 && osaka.x < width - 3 && osaka.y > 4;
        if !osaka.hidden(now) && osaka.door(now).is_none() && osaka.standing() && inside {
            return (now, frame);
        }
        assert!(now < from + 60_000, "never on screen");
        frame = run(guest, real, view, now, now + 500);
        now += 500;
    }
}

#[test]
fn standing_on_a_floor_draws_the_floor_row_into_the_image() {
    let mut ui = real_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(7);
    guest.set_picker(kitty());
    let (_, frame) = run_until_seen(&mut guest, &real, &view, 95_000);
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
    let (now, before) = run_until_seen(&mut guest, &real, &view, 95_000);
    assert!(placeholder(&before), "she is line art before the key press");
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let (x, y) = (visit.osaka.x as u16, visit.osaka.y as u16);
    guest.activity(now);

    let beat = paint(&mut guest, &real, &view, now + 100);
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
    let covered = paint(&mut guest, &changed, &view, now + 200);
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
    let (mut now, _) = run_until_seen(&mut guest, &real, &view, 95_000);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let (x, y) = (visit.osaka.x, visit.osaka.y);
    guest.activity(now);
    let mut rained = 0;
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
        kind: Kind::Normal,
        terrain: Terrain::default(),
        painted: Vec::new(),
        image: None,
        layer: layer::TextLayer::default(),
        chances: osaka::Chances::default(),
        shown: Vec::new(),
        apart: Vec::new(),
        made: Vec::new(),
        next_made: room::MadeId(0),
        reel: None,
        flap: None,
        broken: Vec::new(),
        repairs: Vec::new(),
        mending: None,
        set_down: None,
        judging: None,
        ghost: None,
        size: (real.area.width, real.area.height),
        tuck: false,
        looks: Looks::default(),
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
                        && visit.osaka.reeling().is_none()
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
    use script::Cue;
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
                let sofa = |guest: &Guest| {
                    guest
                        .ledger
                        .home
                        .props
                        .iter()
                        .find(|p| p.item == Furniture::Sofa)
                        .map(|p| p.facing)
                };
                let sofa_was = sofa(&guest);
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
                        Scene::Riddle => visit
                            .osaka
                            .plays()
                            .is_some_and(|p| p.own == script::ScriptId::Riddle),
                        Scene::Surf => visit
                            .osaka
                            .plays()
                            .is_some_and(|p| p.own == script::ScriptId::Surf),
                        Scene::Lounge => posed(Pose::Lounge),
                        Scene::Nap => posed(Pose::Nap(0)),
                        Scene::Sleep => posed(Pose::Sleep(0)),
                        Scene::Night => visit.osaka.sleeping(),
                        Scene::Homework => posed(Pose::Homework(0)),
                        Scene::Watch => used == Some(Furniture::Tv),
                        // Torn off and taking shape (the whole scene runs
                        // past the cap; see she_makes_furniture_of_text).
                        Scene::MakeSofa | Scene::MakeBed => visit
                            .made
                            .iter()
                            .any(|m| m.piece.scrap.is_some_and(|s| s.stage > 0)),
                        Scene::Parcel => guest.ledger.home.props.iter().any(|p| !p.boxed),
                        Scene::Shopping => guest.ledger.ordered.is_some(),
                        Scene::Work => went_out || gone,
                        Scene::Read => posed(Pose::Read(0)),
                        Scene::Snack => posed(Pose::Eat(0)),
                        Scene::Pet => posed(Pose::Pet(0)),
                        // Chosen as the use starts (the andagi plays
                        // after the snack, past the cap: see
                        // her_vignettes_play_at_her_things).
                        Scene::ChopsticksClean | Scene::ChopsticksBad | Scene::Andagi => {
                            let Some(Cue::Splice(id, branch)) = scene.cue() else {
                                panic!("{at}: {scene:?} cues no splice");
                            };
                            visit.osaka.plays().is_some_and(|p| {
                                [p.before, p.after]
                                    .into_iter()
                                    .flatten()
                                    .any(|s| s.splice == id && branch.is_none_or(|b| s.branch == b))
                            })
                        }
                        // Lifted, or (a turn) set down already.
                        Scene::Arrange => {
                            visit.osaka.carrying().is_some() || sofa(&guest) != sofa_was
                        }
                        Scene::DashIn | Scene::DashForgot => {
                            let Some(Cue::Script(id)) = scene.cue() else {
                                panic!("{at}: {scene:?} cues no script");
                            };
                            visit.osaka.plays().is_some_and(|p| p.own == id)
                        }
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
    use super::brain::Want;
    use super::sprite::Pose;
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let half = 10 * 60_000;
    let (mut early, mut late) = (0u64, 0u64);
    let mut choices: Vec<Want> = Vec::new();
    for seed in 0..4u64 {
        // Her needs alone: her routine would have her afternoon (16:00 to
        // 18:00, these twenty minutes) slow her sleepiness by design.
        let mut guest = Guest::new(seed).unfed();
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
                // Dozing: on the floor, or on something she's made.
                if matches!(pose, Pose::LieBack(_) | Pose::Nap(_) | Pose::Sleep(_)) {
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
    assert!(choices.contains(&Want::Pull), "she tidied");
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

/// The most distinct images a 20-minute visit to her furnished home may
/// encode in line art (9×19-pixel cells). The frame cache is sized for
/// visits far longer, so it bounds nothing here: this does. The most any
/// such visit encodes is 350 (busy, with her vignettes every chance),
/// and this is close to half again that, so a change that costs her
/// half as many images again (each encoded in two variants comes to
/// some 530) fails; and half the cache, so a visit this busy leaves the
/// other half free.
const VISIT_IMAGES: usize = 512;

/// A furnished home over long visits in line art, on a Saturday from
/// 10:00 (her part-time job's open: a day off, 10:00 to 17:00): she uses
/// each of her things (and sleeps in her bed more than on a border once
/// she has one), goes to work at most once a visit, her image with the
/// pieces she overlaps never hides text, and her images stay within the
/// frame cache and [`VISIT_IMAGES`]: none she needs again was dropped.
#[test]
fn a_furnished_home_gets_used_and_stays_cheap() {
    use super::brain::Want;
    use super::room::Use;
    let mut choices: Vec<Want> = Vec::new();
    for seed in 0..2u64 {
        let (mut guest, real, view) = furnished_home_in(on_saturday(seed), &[]);
        live_in(&mut guest, &real, &view, 20 * 60_000, seed);
        let visit = visit_of(&guest);
        let shifts = visit
            .osaka
            .choices
            .iter()
            .filter(|&&k| k == Want::Work)
            .count();
        assert!(
            shifts <= 1,
            "seed {seed}: to work {shifts} times in one visit"
        );
        choices.extend(&visit.osaka.choices);
        let counts = guest.graphics.as_ref().unwrap().counts();
        assert_eq!(counts.reencoded, 0, "seed {seed}: {counts:?}");
        assert!(
            counts.encoded <= VISIT_IMAGES,
            "seed {seed}: her images cost more: {counts:?}"
        );
    }
    let count = |want: Want| choices.iter().filter(|&&k| k == want).count();
    // Each piece gets used (a nap is one in fifty or so of her choices,
    // so the sofa's use may be either).
    for uses in [
        &[Use::Lounge, Use::Nap][..],
        &[Use::Sleep],
        &[Use::Homework],
        &[Use::Watch],
    ] {
        assert!(
            uses.iter().any(|&what| count(Want::Use(what)) > 0),
            "she never chose {uses:?}: {choices:?}"
        );
    }
    assert!(
        count(Want::Use(Use::Sleep)) >= count(Want::Idle(osaka::Activity::LieBack)),
        "the bed beats a border: {choices:?}"
    );
    assert!(count(Want::Work) > 0, "she never went to work: {choices:?}");
}

/// Busy about a furnished home, over long visits in line art: industrious
/// (her sofa, given, sits turned from where the TV landed, and she
/// arranges as she feels it), she walks and works and carries more than
/// any other mood, and the frame cache, sized for a visit far longer,
/// never fills: each image she shows is encoded once, and they come to
/// no more than [`VISIT_IMAGES`].
/// What it costs is counted here (and recorded in plan.md, for trials).
#[test]
fn a_busy_furnished_home_stays_cheap() {
    use super::brain::Mood;
    let mut acts = 0;
    for seed in 0..8u64 {
        let (mut guest, real, view) = furnished_home(seed);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.set_mood(Mood::Industrious);
        live_in(&mut guest, &real, &view, 20 * 60_000, seed);
        let graphics = guest.graphics.as_ref().unwrap();
        let (cached, counts) = (graphics.cached(), graphics.counts());
        let done = visit_of(&guest).osaka.home_acts();
        acts += done;
        eprintln!(
            "seed {seed}: {cached} cached, {counts:?}, {done} home acts ({} spots tried again), broken {}",
            visit_of(&guest).osaka.retried,
            guest.broken()
        );
        assert_eq!(
            (counts.evicted, counts.reencoded),
            (0, 0),
            "seed {seed}: her frame cache filled: {counts:?}"
        );
        assert!(
            counts.encoded <= VISIT_IMAGES,
            "seed {seed}: her images cost more: {counts:?}"
        );
    }
    assert!(acts >= 1, "she never set anything down");
}

/// Her vignettes as often as they may be (every splice that may wrap a
/// use she starts does, ten minutes apart) cost next to nothing and fit
/// in her frame cache with the rest of a busy visit: about a furnished
/// home with her fridge (industrious, as in
/// [`a_busy_furnished_home_stays_cheap`]; seeds 0 and 2 once outgrew the
/// old cache of 256, seed 2 dropping 111 images) for twenty minutes in
/// line art, the chopsticks before her homework and the andagi after
/// her snacks, the cache drops nothing and encodes no image twice, with
/// them or without, the visit stays within [`VISIT_IMAGES`], and the
/// vignettes add scarcely any images to it. (The two visits part ways
/// at the first vignette, so that is their cost give or take where she
/// happens to go. How the cache drops what it must is
/// `graphics::tests::a_full_cache_drops_what_she_showed_longest_ago`.)
#[test]
fn a_home_full_of_vignettes_stays_cheap() {
    use super::brain::Mood;
    use script::SpliceId;
    let mut played = std::collections::HashSet::new();
    for seed in [0u64, 2, 3] {
        let mut encoded = [0; 2];
        for sure in [false, true] {
            // The cost of her vignettes alone, unfed from the start: in
            // her afternoon (these twenty minutes) her routine boosts the
            // snack and the sofa over the homework the chopsticks wrap.
            let (mut guest, real, view) =
                furnished_home_in(Guest::new(seed).unfed(), &[Furniture::Fridge]);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("visiting");
            };
            visit.osaka.set_mood(Mood::Industrious);
            visit.osaka.splices_sure = sure;
            live_in_watching(&mut guest, &real, &view, 20 * 60_000, seed, |guest, _| {
                let plays = visit_of(guest).osaka.plays();
                for s in plays
                    .into_iter()
                    .flat_map(|p| [p.before, p.after])
                    .flatten()
                {
                    played.insert((sure, s.splice));
                }
            });
            let graphics = guest.graphics.as_ref().unwrap();
            let (cached, counts) = (graphics.cached(), graphics.counts());
            eprintln!("seed {seed}, vignettes {sure}: {cached} cached, {counts:?}");
            assert_eq!(
                (counts.evicted, counts.reencoded),
                (0, 0),
                "seed {seed}, vignettes {sure}: her frame cache filled: {counts:?}"
            );
            assert!(
                counts.encoded <= VISIT_IMAGES,
                "seed {seed}, vignettes {sure}: her images cost more: {counts:?}"
            );
            encoded[usize::from(sure)] = counts.encoded;
        }
        // Seen: 13 more, 2 more, 4 fewer.
        let [without, with] = encoded;
        assert!(
            with <= without + 32,
            "seed {seed}: her vignettes cost {with} images, {without} without them"
        );
    }
    for want in [SpliceId::Chopsticks, SpliceId::Andagi] {
        assert!(
            played.contains(&(true, want)),
            "{want:?} never played: {played:?}"
        );
    }
}

/// A furnished home on [`home_screen`] (her living room and bedroom, the
/// sofa, TV, bed and desk given), in line art, she arrived at `seed`.
fn furnished_home(seed: u64) -> (Guest, Buffer, IdleView) {
    furnished_home_with(seed, &[])
}

/// [`furnished_home`], with `more` given too.
fn furnished_home_with(seed: u64, more: &[Furniture]) -> (Guest, Buffer, IdleView) {
    furnished_home_in(Guest::new(seed), more)
}

/// A guest from `seed` meeting her for the first time, her clock
/// reading Saturday 10:00 (from her first visit on, as it would have
/// she met on a Saturday morning): a day off, her part-time job open.
fn on_saturday(seed: u64) -> Guest {
    let mut guest = Guest::new(seed);
    guest.ledger.clock = sat(10, 0).minutes();
    guest
}

/// [`furnished_home_with`], for `guest` as made (unfed, say).
fn furnished_home_in(mut guest: Guest, more: &[Furniture]) -> (Guest, Buffer, IdleView) {
    let (real, view) = home_screen();
    let seed = guest.ledger.master_seed;
    guest.set_picker(kitty());
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    let pieces: Vec<Furniture> = [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Bed,
        Furniture::Desk,
    ]
    .into_iter()
    .chain(more.iter().copied())
    .collect();
    for &item in &pieces {
        guest.give(item);
        paint(&mut guest, &real, &view, 0);
    }
    assert_eq!(
        pieces
            .iter()
            .filter(|&&item| guest.ledger.home.owns(item))
            .count(),
        pieces.len(),
        "seed {seed}: {:?}",
        guest.cue_note()
    );
    (guest, real, view)
}

/// Her visit going on until `until`, her image never hiding text.
fn live_in(guest: &mut Guest, real: &Buffer, view: &IdleView, until: u64, seed: u64) {
    live_in_watching(guest, real, view, until, seed, |_, _| {});
}

/// [`live_in`], `watch` seeing her after each step.
fn live_in_watching(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    until: u64,
    seed: u64,
    mut watch: impl FnMut(&Guest, u64),
) {
    let mut hidden = Hidden::default();
    let mut now = 0;
    while now < until {
        now += guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        if guest.advance(now) {
            let frame = paint(guest, real, view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("seed {seed}: still visiting");
            };
            let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
            hidden
                .check(&frame, real, &layer, now)
                .unwrap_or_else(|e| panic!("seed {seed} at {now}: {e}"));
            watch(guest, now);
        }
    }
}

/// A pit: a room whose walls are wide text to the ceiling on the left and
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
    // Wide glyphs: solid even in line art (she passes narrow text).
    for y in 1..=14 {
        buf.set_string(1, y, "漢".repeat(24), Style::new());
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

/// In line art the pit has no way out — no climb past the wide text, no
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
        let mut hidden = Hidden::default();
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
                    hidden
                        .check(&frame, &real, &layer, now)
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
    // The exit save still has it: a handout clears only the flag.
    assert_eq!(guest.final_ledger(0).as_ref(), Some(&ledger));
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
    assert!(guest.final_ledger(0).is_none(), "unchanged since restored");
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
    assert_eq!(guest.final_ledger(0), Some(Ledger::new(77)));

    let mut guest = Guest::new(4);
    guest.keep_unsaved();
    assert!(guest.final_ledger(0).is_none());
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    guest.give(Furniture::Sofa);
    paint(&mut guest, &real, &view, 0);
    assert!(guest.ledger_to_save().is_none());
    assert!(guest.final_ledger(0).is_none(), "nor at exit");
    // Moving out writes over it, at exit too: even a wipe drawn from the
    // seed she started from differs from a record that couldn't be read.
    guest.move_out(4);
    assert_eq!(guest.ledger_to_save(), Some(Ledger::new(4)));
    assert_eq!(guest.final_ledger(0), Some(Ledger::new(4)));
}

// ---- Her clock ----

/// A guest who has met you (her clock runs), at Monday 16:00.
fn met(seed: u64) -> Guest {
    let mut ledger = Ledger::new(seed);
    ledger.visits = 1;
    Guest::restore(ledger)
}

/// Her clock in game millis.
fn game_ms(guest: &Guest) -> u64 {
    guest.ledger.clock * 60_000 + guest.clock_rem
}

/// Her idle counter in real millis.
fn idle_ms(guest: &Guest) -> u64 {
    guest.ledger.idle_min * 60_000 + guest.idle_rem
}

/// The first tick only latches; time counts from it, six game millis to
/// the real one, and on its own changes nothing on screen.
#[test]
fn her_clock_counts_from_the_first_tick() {
    let mut guest = met(5);
    assert_eq!(guest.game_clock(42), Some(GameClock { at: 42, game: 0 }));
    assert!(!guest.advance(3_600_000), "time alone changes nothing");
    assert_eq!(game_ms(&guest), 0, "the first tick only latches");
    assert!(!guest.advance(3_610_000));
    assert_eq!((guest.ledger.clock, guest.clock_rem), (1, 0));
    assert!(!guest.advance(3_615_500));
    assert_eq!((guest.ledger.clock, guest.clock_rem), (1, 33_000));
    // Absent, she wakes only at her routine's next boundary (Monday
    // 18:00, 7_107_000 game ms on); unfed, at nothing.
    assert_eq!(
        guest.next_tick(3_615_500),
        Some(Duration::from_millis(1_184_500)),
        "and wakes for her routine's next boundary"
    );
    guest.set_feed_clock(false);
    assert_eq!(guest.next_tick(3_615_500), None, "unfed, no tick");
    guest.set_feed_clock(true);
    assert_eq!(
        guest.game_clock(0),
        Some(GameClock {
            at: 3_615_500,
            game: 93_000
        })
    );
    // Before she has met you it doesn't run, and there's no reading.
    let mut guest = Guest::new(5);
    guest.advance(0);
    assert!(!guest.advance(3_600_000));
    assert_eq!(game_ms(&guest), 0);
    assert_eq!(guest.game_clock(3_600_000), None);
    assert_eq!(guest.next_tick(3_600_000), None);
}

/// A step back in time (the stage slowed down, say) counts nothing and
/// panics nowhere; the next step forward counts only what's new.
#[test]
fn a_step_back_counts_nothing() {
    let mut guest = met(5);
    guest.open = true;
    guest.delay = Some(DELAY);
    guest.advance(1_000);
    guest.advance(500);
    assert_eq!((game_ms(&guest), idle_ms(&guest)), (0, 0));
    guest.advance(1_500);
    assert_eq!(game_ms(&guest), 6 * 500, "500 to 1500, once");
    guest.advance(DELAY.as_millis() as u64 + 2_000);
    guest.advance(DELAY.as_millis() as u64 + 1_000);
    guest.advance(DELAY.as_millis() as u64 + 3_000);
    assert_eq!(idle_ms(&guest), 3_000, "from the gate, once");
}

/// A clean exit hands out a ledger whose only change is her clock.
#[test]
fn the_exit_save_carries_time_alone() {
    let mut guest = met(5);
    assert_eq!(guest.final_ledger(0), None, "unchanged");
    guest.advance(0);
    guest.advance(50_000);
    assert_eq!(guest.ledger_to_save(), None, "under a batch");
    let ledger = guest.final_ledger(60_000).expect("her clock moved");
    assert_eq!(ledger.clock, 6);
    assert_eq!(
        Ledger { clock: 0, ..ledger },
        met(5).ledger,
        "and nothing else"
    );
}

/// A restored clock batches from where it was restored, not from zero.
#[test]
fn a_restored_clock_batches_from_where_it_was() {
    let mut ledger = Ledger::new(8);
    ledger.visits = 1;
    ledger.clock = 40;
    let mut guest = Guest::restore(ledger);
    guest.advance(0);
    for now in (5_000..300_000).step_by(5_000) {
        guest.advance(now);
        assert_eq!(
            guest.ledger_to_save(),
            None,
            "at {now}: {} game minutes",
            guest.ledger.clock
        );
    }
    guest.advance(300_000);
    assert_eq!(guest.ledger_to_save().map(|l| l.clock), Some(70));
}

/// Her clock stops at the top, and what it stops at is saved as itself.
#[test]
fn her_clock_stops_at_the_top() {
    let mut ledger = Ledger::new(8);
    ledger.visits = 1;
    ledger.clock = ledger::MINUTES_MAX - 1;
    ledger.idle_min = ledger::MINUTES_MAX - 1;
    let mut guest = Guest::restore(ledger);
    guest.open = true;
    guest.delay = Some(DELAY);
    guest.advance(0);
    guest.advance(3_600_000);
    assert_eq!(guest.ledger.clock, ledger::MINUTES_MAX);
    assert_eq!(guest.ledger.idle_min, ledger::MINUTES_MAX);
    let stored = Ledger::from_json(&guest.ledger.to_json()).unwrap();
    assert_eq!(stored, guest.ledger);
}

/// A reading of her clock answers for any moment: later at six times
/// the pace, earlier too (a catch-up decision), and never before zero.
#[test]
fn a_clock_reading_answers_earlier_and_later() {
    let clock = GameClock {
        at: 10_000,
        game: 30_000,
    };
    assert_eq!(clock.at(10_000), 30_000);
    assert_eq!(clock.at(11_000), 36_000);
    assert_eq!(clock.at(9_000), 24_000);
    assert_eq!(clock.at(0), 0, "saturates at zero");
    assert_eq!(clock.at(u64::MAX), u64::MAX, "and at the top");
}

/// The shell caps one step (a suspend `Instant` may count); without the
/// cap a step counts whole.
#[test]
fn a_capped_step_counts_only_the_cap() {
    let mut guest = met(5);
    guest.cap_steps(Some(600_000));
    guest.open = true;
    guest.delay = Some(Duration::ZERO);
    guest.advance(0);
    guest.advance(3_600_000);
    assert_eq!(guest.ledger.clock, 60, "ten real minutes, no more");
    assert_eq!(idle_ms(&guest), 600_000, "for the idle counter too");
    guest.advance(3_601_000);
    assert_eq!(game_ms(&guest), 60 * 60_000 + 6_000);
}

proptest! {
    /// Her clock and her idle counter come to the same however the time
    /// is cut into ticks: the clock from the tick she met you at, at six
    /// times real time; the idle counter from when the gate opened (or
    /// she met you, if later). The latch moves while the clock waits for
    /// her first meeting, so none of the wait counts.
    #[test]
    fn accrual_is_independent_of_how_ticks_fall(
        before in proptest::collection::vec(1u64..200_000, 0..8),
        after in proptest::collection::vec(1u64..200_000, 0..12),
        start in 0u64..100_000,
        quiet in 0u64..2_000_000,
        open in any::<bool>(),
    ) {
        let mut guest = Guest::new(9);
        guest.open = open;
        guest.delay = Some(DELAY);
        guest.quiet_since = quiet;
        let mut now = start;
        guest.advance(now);
        for gap in &before {
            now += gap;
            guest.advance(now);
        }
        prop_assert_eq!(game_ms(&guest), 0, "not before she met you");
        prop_assert_eq!(idle_ms(&guest), 0);
        // She meets you on a paint at `met_at`, after its tick.
        let met_at = now;
        guest.ledger.visits = 1;
        for gap in &after {
            now += gap;
            guest.advance(now);
        }
        prop_assert_eq!(game_ms(&guest), 6 * (now - met_at));
        let gate = quiet + DELAY.as_millis() as u64;
        let idle = if open { now.saturating_sub(gate.max(met_at)) } else { 0 };
        prop_assert_eq!(idle_ms(&guest), idle);
    }

    /// Her clock never runs back within a process, even when time does
    /// (the stage slowing down); a clean exit saves it to the
    /// millisecond's minute, and a crash, restored from the last handout,
    /// loses less than a batch.
    #[test]
    fn her_clock_survives_a_restart(
        gaps in proptest::collection::vec(-100_000i64..400_000, 1..40),
        events in proptest::collection::vec(any::<bool>(), 40),
        tail in 0u64..400_000,
    ) {
        let mut guest = met(3);
        let mut saved = guest.ledger.clone();
        let mut now = 1_000u64;
        // The latest moment yet: time counts from it.
        let mut high = now;
        guest.advance(now);
        let mut last = 0;
        for (i, &gap) in gaps.iter().enumerate() {
            now = now.saturating_add_signed(gap);
            high = high.max(now);
            guest.advance(now);
            if events[i] {
                // An event dirties the ledger at once.
                guest.send_parcel();
            }
            prop_assert!(guest.ledger.clock >= last, "never runs back");
            last = guest.ledger.clock;
            if let Some(ledger) = guest.ledger_to_save() {
                saved = ledger;
            }
            // A crash now: restored from the last handout.
            prop_assert!(
                guest.ledger.clock - saved.clock < 30,
                "a crash loses less than a batch: {} saved at {}",
                guest.ledger.clock,
                saved.clock
            );
        }
        // A clean exit: her clock up to the exit's moment, handed out
        // whenever it moved (the record restored had it at zero).
        let exit = high + tail;
        let want = (game_ms(&guest) + 6 * tail) / 60_000;
        let ledger = match guest.final_ledger(exit) {
            Some(ledger) => ledger,
            None => {
                prop_assert_eq!(want, 0, "her clock moved, and the exit save dropped it");
                guest.ledger.clone()
            }
        };
        prop_assert_eq!(ledger.clock, want);
        let restored = Guest::restore(Ledger::from_json(&ledger.to_json()).unwrap());
        prop_assert_eq!(restored.ledger.clock, want, "equal after the restart");
        prop_assert_eq!(restored.ledger.idle_min, ledger.idle_min);
    }
}

/// Time dirties the ledger in batches of thirty game minutes since the
/// last handout; an event hands it out at once and starts the batch
/// again.
#[test]
fn her_clock_is_saved_in_batches() {
    let mut guest = met(8);
    let mut now = 0;
    guest.advance(now);
    // Ten real seconds a game minute.
    let mut tick = |guest: &mut Guest| {
        now += 10_000;
        guest.advance(now);
        guest.ledger_to_save()
    };
    for minute in 1..30 {
        assert_eq!(tick(&mut guest), None, "minute {minute}");
    }
    let ledger = tick(&mut guest).expect("thirty game minutes");
    assert_eq!(ledger.clock, 30);
    for _ in 31..50 {
        assert_eq!(tick(&mut guest), None);
    }
    // An event at 16:50: handed out at once, and the batch restarts.
    guest.send_parcel();
    assert_eq!(tick(&mut guest).map(|l| l.clock), Some(50));
    for minute in 51..80 {
        assert_eq!(tick(&mut guest), None, "minute {minute}");
    }
    assert_eq!(tick(&mut guest).map(|l| l.clock), Some(80));
}

/// Moving out stops her clock back at Monday 16:00, counters and all;
/// it starts again when she next meets you.
#[test]
fn moving_out_resets_her_clock() {
    let mut guest = met(8);
    guest.open = true;
    guest.delay = Some(DELAY);
    guest.advance(0);
    guest.advance(3_605_500);
    assert_eq!((guest.ledger.clock, guest.clock_rem), (360, 33_000));
    assert!(guest.ledger.idle_min > 0);
    assert_ne!(guest.idle_rem, 0);
    guest.ledger.rare_at = 7;
    guest.ledger.legend_at = 9;
    guest.move_out(8);
    assert_eq!(guest.ledger_to_save(), Some(Ledger::new(8)));
    assert_eq!(
        (game_ms(&guest), idle_ms(&guest)),
        (0, 0),
        "no remainder carried over"
    );
    guest.advance(3_700_000);
    assert_eq!(game_ms(&guest), 0, "not until she meets you again");
    guest.ledger.visits = 1;
    guest.advance(3_710_000);
    assert_eq!(guest.ledger.clock, 1);
}

// ---- Her routine ----

/// Real millis for `minutes` game minutes.
fn real_ms(minutes: u64) -> u64 {
    minutes * 60_000 / CLOCK_SPEED
}

/// Real millis from her clock's start (Monday 16:00 of game day 0) to
/// `h:m` of game `day`.
fn real_at(day: u64, h: u64, m: u64) -> u64 {
    real_ms(day * 1440 + h * 60 + m - routine::START)
}

/// Advance an absent guest to `until` in steps of ten game minutes (her
/// clock, the latch and the slot batching run on every advance).
fn advance_to(guest: &mut Guest, from: u64, until: u64) -> u64 {
    let mut now = from;
    while now < until {
        now = (now + real_ms(10)).min(until);
        guest.advance(now);
    }
    now
}

fn date(y: i32, m: u32, d: u32) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::from_ymd_opt(y, m, d)
}

/// Unfed (the tests' [`Guest::unfed`]), the routine reaches nothing of
/// hers, whatever the date: `day` is `None` for the guest and for every
/// decision of a visit. Fed (the default), every decision carries the
/// routine at its own moment, and the explain line shows it.
#[test]
fn the_routine_reaches_her_only_when_fed() {
    let mut guest = met(5).unfed();
    guest.set_date(date(2026, 7, 25));
    let mut now = 0;
    guest.advance(now);
    for _ in 0..2 * 144 {
        now = advance_to(&mut guest, now, now + real_ms(10));
        assert_eq!(guest.day(now), None, "at {now}");
    }
    assert!(guest.clock_label(now).is_some(), "the stage still reads it");
    // Before she has met you there's nothing to feed.
    let mut fresh = Guest::new(5);
    fresh.set_feed_clock(true);
    fresh.advance(0);
    assert_eq!(fresh.day(0), None);
    assert_eq!(fresh.clock_label(0), None);

    for fed in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(5);
        guest.set_feed_clock(fed);
        guest.set_date(date(2026, 6, 17));
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        let mut now = 0;
        while now < 60_000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
        }
        let State::Visiting(visit) = &guest.state else {
            panic!("still visiting");
        };
        let decisions = &visit.osaka.decisions;
        assert!(decisions.len() > 3, "{} decisions", decisions.len());
        let clock = guest.game_clock(now).expect("met");
        for d in decisions {
            if fed {
                let day = d.day.expect("fed");
                // Her own clock, read at the decision's own moment.
                assert_eq!(day, routine::day_time(clock.at(d.at), false), "{d}");
                assert_eq!(day.slot, routine::Slot::Afternoon);
                assert!(day.school_day && !day.vacation);
            } else {
                assert_eq!(d.day, None, "{d}");
            }
        }
        let shown = guest.explain().expect("explained");
        assert_eq!(shown.contains(" @ Mon 16:0"), fed, "{shown}");
        assert_eq!(visit.osaka.day(now), guest.day(now));
    }
}

/// The vacation flag is read once a game day, at its first read: a real
/// date flipping in the middle of a game day changes nothing until the
/// next one, so the slots are a pure function of game time within it.
#[test]
fn the_vacation_latch_holds_through_a_game_day() {
    let mut guest = met(5);
    guest.set_feed_clock(true);
    guest.set_date(date(2026, 6, 17));
    guest.advance(0);
    // Tuesday 08:30, a school day: at school.
    let mut now = advance_to(&mut guest, 0, real_at(1, 8, 30));
    let day = guest.day(now).unwrap();
    assert_eq!(
        (day.weekday, day.slot),
        (chrono::Weekday::Tue, routine::Slot::Away)
    );
    // Summer starts (the real date flips at 09:00): she stays at school,
    // and the evening is still a school night's.
    guest.set_date(date(2026, 7, 25));
    for (h, m, slot) in [
        (9, 0, routine::Slot::Away),
        (12, 45, routine::Slot::Afternoon),
        (20, 0, routine::Slot::Homework),
        (22, 30, routine::Slot::Asleep),
    ] {
        now = advance_to(&mut guest, now, real_at(1, h, m));
        let day = guest.day(now).unwrap();
        assert_eq!(day.slot, slot, "Tue {h}:{m}");
        assert!(!day.vacation, "Tue {h}:{m}");
    }
    // Wednesday reads it afresh: vacation, a lie-in until 09:00.
    now = advance_to(&mut guest, now, real_at(2, 8, 30));
    let day = guest.day(now).unwrap();
    assert!(day.vacation && !day.school_day);
    assert_eq!(day.slot, routine::Slot::Asleep);
    assert_eq!(
        guest.clock_label(now).as_deref(),
        Some("Wed 08:30 Asleep (vacation)")
    );
    // The date goes away mid-day: Wednesday stays a vacation day.
    guest.set_date(None);
    now = advance_to(&mut guest, now, real_at(2, 10, 0));
    let day = guest.day(now).unwrap();
    assert!(day.vacation);
    assert_eq!(day.slot, routine::Slot::Morning);
    // Thursday, no calendar: school.
    now = advance_to(&mut guest, now, real_at(3, 9, 0));
    let day = guest.day(now).unwrap();
    assert!(!day.vacation && day.school_day);
    assert_eq!(day.slot, routine::Slot::Away);
}

/// While the clock is fed, crossing a slot boundary hands out her
/// record (in the batch's stead), so it's consistent at departures and
/// bedtimes; unfed, only the batch does.
#[test]
fn a_slot_change_saves_her_record_when_fed() {
    for fed in [false, true] {
        let mut guest = met(5);
        guest.set_feed_clock(fed);
        guest.advance(0);
        // 17:50, the record just handed out.
        let now = advance_to(&mut guest, 0, real_at(0, 17, 50));
        let _ = guest.ledger_to_save();
        // 18:00:30: the evening, ten and a half game minutes on.
        let now = advance_to(&mut guest, now, real_at(0, 18, 0) + 5_000);
        assert_eq!(guest.ledger_to_save().is_some(), fed, "fed: {fed}");
        // And not again within the slot (nor the batch, from 17:50).
        advance_to(&mut guest, now, real_at(0, 18, 15));
        assert_eq!(guest.ledger_to_save(), None, "fed: {fed}");
    }
}

/// A tick catching up over a long gap reads the routine at each
/// decision's own moment, not the tick's: decisions across a slot
/// boundary land in different minutes and slots.
#[test]
fn every_decision_reads_the_routine_at_its_own_moment() {
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(5);
    guest.set_feed_clock(true);
    guest.set_date(date(2026, 6, 17));
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    let mut now = 0;
    while now < 20_000 {
        now += 500;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
    }
    // 17:50, then one long gap to past 18:10 with no tick between.
    guest.ledger.clock = 110;
    guest.clock_rem = 0;
    guest.advance(now);
    let before = now;
    now += real_ms(25);
    // One tick runs at most 64 events: tick again at the same moment
    // until it has caught up.
    for _ in 0..8 {
        guest.advance(now);
    }
    let State::Visiting(visit) = &guest.state else {
        panic!("still visiting");
    };
    let clock = guest.game_clock(now).expect("met");
    let gap: Vec<_> = visit
        .osaka
        .decisions
        .iter()
        .filter(|d| d.at > before)
        .collect();
    for d in &gap {
        assert_eq!(d.day, Some(routine::day_time(clock.at(d.at), false)), "{d}");
    }
    let minutes: std::collections::HashSet<u16> = gap
        .iter()
        .filter_map(|d| d.day.map(|day| day.minute))
        .collect();
    let slots: std::collections::HashSet<routine::Slot> = gap
        .iter()
        .filter_map(|d| d.day.map(|day| day.slot))
        .collect();
    assert!(minutes.len() > 1, "{minutes:?}");
    assert_eq!(
        slots,
        [routine::Slot::Afternoon, routine::Slot::Evening].into(),
        "{gap:?}"
    );
}

/// Moving out on game day 0 drops the day's latch and slot with her
/// clock: the next meeting's day 0 latches from the date as it is then,
/// and its first slot is compared with nothing of the old home's.
#[test]
fn moving_out_forgets_the_days_latch_and_slot() {
    let mut guest = met(5);
    guest.set_feed_clock(true);
    guest.set_date(date(2026, 7, 25));
    guest.advance(0);
    // Monday 18:30, a vacation day: the evening.
    let now = advance_to(&mut guest, 0, real_at(0, 18, 30));
    let day = guest.day(now).unwrap();
    assert!(day.vacation);
    assert_eq!(day.slot, routine::Slot::Evening);
    guest.move_out(9);
    guest.set_date(date(2026, 6, 17));
    guest.ledger.visits = 1;
    let _ = guest.ledger_to_save();
    // Her first tick after the meeting.
    let now = now + 1_000;
    guest.advance(now);
    let day = guest.day(now).unwrap();
    assert_eq!((day.day, day.slot), (0, routine::Slot::Afternoon));
    assert!(!day.vacation, "day 0 latched afresh");
    assert_eq!(guest.ledger_to_save(), None, "no slot change to save");
}

/// The stage's `t`: her clock skips forward to the next boundary of her
/// routine, never back, and the record is handed out; nothing before she
/// has met you.
#[test]
fn skipping_her_clock_walks_her_routine_forward() {
    let mut fresh = Guest::new(5);
    fresh.advance(0);
    fresh.skip_clock(1_000);
    assert_eq!(fresh.ledger.clock, 0);
    assert_eq!(fresh.ledger_to_save(), None);

    let mut guest = met(5);
    guest.set_date(date(2026, 6, 17));
    guest.advance(0);
    // A little way into Monday afternoon.
    let mut now = advance_to(&mut guest, 0, real_ms(7) + 123);
    let mut labels = Vec::new();
    for _ in 0..12 {
        let before = game_ms(&guest);
        guest.skip_clock(now);
        assert!(game_ms(&guest) > before, "forward only");
        assert_eq!(guest.clock_rem, 0, "onto the minute");
        assert!(guest.ledger_to_save().is_some(), "handed out");
        labels.push(guest.clock_label(now).unwrap());
        // A while later (time runs on between skips).
        now += 1_000;
        guest.advance(now);
    }
    assert_eq!(
        labels[..10],
        [
            "Mon 18:00 Evening",
            "Mon 20:00 Homework",
            "Mon 22:30 Asleep",
            "Tue 07:00 Morning",
            "Tue 08:15 Away",
            "Tue 12:45 Afternoon",
            "Tue 18:00 Evening",
            "Tue 20:00 Homework",
            "Tue 22:30 Asleep",
            "Wed 07:00 Morning",
        ]
    );
    // A skip between ticks brings her clock up to its own moment first:
    // it lands on the boundary itself, and the next tick at the same
    // moment adds nothing.
    assert_eq!(labels[10..], ["Wed 08:15 Away", "Wed 12:45 Afternoon"]);
    let later = now + 5_000;
    guest.skip_clock(later);
    let evening = (2 * 1440 + 18 * 60 - routine::START) * 60_000;
    assert_eq!(game_ms(&guest), evening);
    assert_eq!(
        guest.clock_label(later).as_deref(),
        Some("Wed 18:00 Evening")
    );
    guest.advance(later);
    assert_eq!(game_ms(&guest), evening, "counted once");
    // At the clock's very end the boundary can't be reached: nothing
    // moves, not even back to the whole minute, and nothing is saved.
    guest.ledger.clock = ledger::MINUTES_MAX;
    guest.clock_rem = 12_345;
    guest.advance(later);
    let _ = guest.ledger_to_save();
    let end = game_ms(&guest);
    guest.skip_clock(later);
    assert_eq!(game_ms(&guest), end, "never back");
    assert_eq!(guest.ledger_to_save(), None);

    // A visiting Osaka reads the skipped clock at once.
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(5);
    guest.set_feed_clock(true);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    guest.advance(1_000);
    guest.skip_clock(1_000);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let day = visit.osaka.day(1_000).unwrap();
    assert_eq!((day.minute, day.slot), (18 * 60, routine::Slot::Evening));
}

/// Fed her routine, she arrives as her day has left her (D4 lever 3):
/// her first meeting, at 16:00, exactly as ever; at homework time
/// sleepier. Unfed, as ever whatever the time.
#[test]
fn she_arrives_as_her_day_has_left_her() {
    use super::brain::{Need, Needs};
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let arrived = |mut guest: Guest| {
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        *visit.osaka.needs()
    };
    for fed in [false, true] {
        let mut guest = Guest::new(5);
        guest.set_feed_clock(fed);
        assert_eq!(arrived(guest), Needs::default(), "fed: {fed}");
        // Monday 21:00: homework.
        let mut guest = met(5);
        guest.set_feed_clock(fed);
        guest.ledger.clock = 5 * 60;
        let needs = arrived(guest);
        if fed {
            assert_eq!(needs, Needs::arriving_in(routine::Slot::Homework));
            assert!(needs.get(Need::Sleepy) > Needs::default().get(Need::Sleepy));
        } else {
            assert_eq!(needs, Needs::default());
        }
    }
}

/// Fed her routine, her choices weigh homework by her day and the date
/// (D4 lever 1, through the decision's own reading of her clock): at
/// homework time three times as likely, in exam season twice that again;
/// unfed, as ever. (Monday 21:00 on: three real minutes stay inside
/// homework time.)
#[test]
fn her_choices_weigh_homework_by_her_day_and_the_date() {
    use super::brain::{BOOST, BOOST_STRONG, Want};
    use super::room::Use;
    let homework = Want::Use(Use::Homework);
    for (fed, on, times) in [
        (false, date(2027, 2, 1), 1.0),
        (true, date(2026, 6, 17), BOOST_STRONG),
        (true, date(2027, 2, 1), BOOST_STRONG * BOOST),
    ] {
        let (mut guest, real, view) = furnished_home(3);
        guest.set_feed_clock(fed);
        guest.set_date(on);
        guest.ledger.clock = 5 * 60;
        live_in(&mut guest, &real, &view, 3 * 60_000, 3);
        let visit = visit_of(&guest);
        let weighed: Vec<f64> = visit
            .osaka
            .factored
            .iter()
            .filter(|&&(want, into_chat, _)| want == homework && !into_chat)
            .map(|&(_, _, times)| times)
            .collect();
        assert!(!weighed.is_empty(), "fed {fed}: homework never weighed");
        assert!(
            weighed.iter().all(|&t| t == times),
            "fed {fed} on {on:?}: {weighed:?}"
        );
    }
}

/// Fed her routine, a visit across bedtime: skipping her clock onto
/// 22:30 makes the boundary due at once (the next tick handles it, cut
/// or not), and no tick ever leaves her due at or before its moment (no
/// spin), the next boundary (school) found as each is handled. Unfed,
/// no boundary is ever due.
#[test]
fn her_routine_cuts_in_at_bedtime_and_never_spins() {
    for fed in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(5);
        guest.set_feed_clock(fed);
        guest.set_date(date(2026, 6, 17));
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        let mut now = 0;
        let step = |guest: &mut Guest, now: &mut u64, until: u64| {
            while *now < until {
                *now += guest
                    .next_tick(*now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                guest.advance(*now);
                // (A paint may set her something due at once: the next
                // tick's.)
                let State::Visiting(visit) = &guest.state else {
                    panic!("still visiting");
                };
                assert!(visit.osaka.due() > *now, "due at {now}");
                paint(guest, &real, &view, *now);
            }
        };
        step(&mut guest, &mut now, 10_000);
        for _ in 0..3 {
            guest.skip_clock(now);
        }
        let State::Visiting(visit) = &guest.state else {
            panic!("still visiting");
        };
        assert_eq!(guest.clock_label(now).as_deref(), Some("Mon 22:30 Asleep"));
        assert_eq!(visit.osaka.cut_at(), fed.then_some(now), "fed: {fed}");
        guest.advance(now);
        let State::Visiting(visit) = &guest.state else {
            panic!("still visiting");
        };
        // Tuesday 08:15, 585 game minutes on.
        let school = now + real_ms(585);
        assert_eq!(visit.osaka.cut_at(), fed.then_some(school), "fed: {fed}");
        assert!(visit.osaka.due() > now);
        let until = now + 120_000;
        step(&mut guest, &mut now, until);
        let State::Visiting(visit) = &guest.state else {
            panic!("still visiting");
        };
        assert_eq!(visit.osaka.cut_at(), fed.then_some(school), "fed: {fed}");
    }
}

// ---- Her night ----

/// `h:m` of Monday, game day 0: a school night (bed at 22:30, up at 07:00
/// on Tuesday).
fn mon(h: u16, m: u16) -> routine::GameTime {
    routine::GameTime { day: 0, h, m }
}

/// `h:m` of Tuesday, game day 1: a school day.
fn tue(h: u16, m: u16) -> routine::GameTime {
    routine::GameTime { day: 1, h, m }
}

/// `h:m` of Saturday, game day 5: a day off (her part-time job's open
/// from 10:00 to 17:00).
fn sat(h: u16, m: u16) -> routine::GameTime {
    routine::GameTime { day: 5, h, m }
}

/// A visitor whose input ends a night visit gets a sleepy goodbye: her
/// first beat is a blink, not the startled face (round 1); by day it's
/// startled as ever. Tucked in, or up and about in the night, in both
/// drawing modes; the ASCII dissolve's face shows it.
#[test]
fn a_night_visit_ends_with_a_sleepy_goodbye() {
    use super::sprite::Face;
    let (real, view) = home_screen();
    let bed = [(Furniture::Bed, Nook::Playlist, 500)];
    for graphics in [false, true] {
        for (at, standing, face) in [
            (mon(23, 0), false, Face::Blink),
            (mon(23, 0), true, Face::Blink),
            (mon(16, 30), true, Face::Surprised),
        ] {
            let what = format!("{at:?} standing={standing} graphics={graphics}");
            let mut guest = home_at(3, at, &bed, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            while now < 20_000 {
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let night = face == Face::Blink;
            assert_eq!(asleep(&guest), night, "{what}");
            if standing {
                let State::Visiting(visit) = &mut guest.state else {
                    panic!("{what}: visiting");
                };
                let (x, y) = (visit.osaka.x, visit.osaka.y);
                visit.osaka.place(x, y, now);
                paint(&mut guest, &real, &view, now);
            }
            guest.activity(now + 1);
            let State::Leaving(leaving) = &guest.state else {
                panic!("{what}: leaving");
            };
            assert_eq!(leaving.startled, face, "{what}");
            let frame = paint(&mut guest, &real, &view, now + 50);
            if !graphics && standing {
                let text: Vec<String> = (0..frame.area.height)
                    .map(|y| row_text(&frame, y, 0..frame.area.width))
                    .collect();
                let shows = |face: Face| {
                    let glyphs: String = face.glyphs().iter().collect();
                    text.iter().any(|row| row.contains(&glyphs))
                };
                assert!(shows(face), "{what}: {text:#?}");
                assert!(
                    !shows(if night { Face::Surprised } else { Face::Blink }),
                    "{what}"
                );
            }
        }
    }
}

/// Out at work through a door in place, with her sofa in her box, she
/// isn't there: a visitor's key gets no goodbye from her (no startled
/// beat, no wave). While her door still stands it stays drawn until the
/// rain and only its glyphs rain from her box; once it's shut behind
/// her, nothing of her box rains out but the sofa. The sofa stays drawn
/// until the rain either way. (Phase 5b: the goodbye's image was taken
/// from her box whether she was in it or not.) In both drawing modes.
#[test]
fn a_goodbye_while_she_is_out_shows_nothing_of_her() {
    use super::graphics::Look;
    use super::sprite::Face;
    let (real, view) = home_screen();
    for graphics in [false, true] {
        // Through her door, it still standing; then shut behind her.
        for standing in [true, false] {
            let (mut guest, now, (x, y), piece) =
                out_at_work(&real, &view, graphics, 0, |osaka, now| {
                    osaka.hidden(now) && osaka.door(now).is_some() == standing
                });
            let what = format!("graphics={graphics} door={standing}");
            let frame = paint(&mut guest, &real, &view, now);
            assert!(drawn(&frame, &real, &piece), "{what}: sofa");
            // The sofa is in her box: an image of her box would hold it.
            assert_eq!(terrain::image(x, y, &[piece.cover()]).with, [true]);
            let State::Visiting(visit) = &guest.state else {
                panic!("{what}: visiting");
            };
            assert_eq!((visit.osaka.x, visit.osaka.y), (x, y), "{what}");
            // In line art the sofa is in her door's image while it
            // stands; shut, it's drawn on its own.
            if graphics {
                let with = visit.image.as_ref().map(|i| i.with.clone());
                if standing {
                    assert_eq!(with, Some(vec![piece]), "{what}: in her door's image");
                } else {
                    assert_eq!(with, None, "{what}: no image of her box");
                    assert!(visit.apart.contains(&piece), "{what}: on its own");
                }
            }
            // Her door's cells, with their glyphs.
            let door: Vec<(u16, u16, char)> = visit
                .osaka
                .door(now)
                .map(|frame| sprite::door_cells(frame as usize, visit.osaka.facing))
                .unwrap_or_default()
                .into_iter()
                .map(|c| ((x + c.dx) as u16, (y + c.dy) as u16, c.glyph))
                .collect();
            assert_eq!(door.is_empty(), !standing, "{what}");
            // What rains out of her box but the sofa: her door's glyphs
            // where it stands, and nothing else.
            let mine = |cx: u16, cy: u16| {
                let (cx, cy) = (i32::from(cx), i32::from(cy));
                (x - sprite::WIDTH / 2..=x + sprite::WIDTH / 2).contains(&cx)
                    && (y - sprite::HEIGHT..y).contains(&cy)
                    && !piece.cover().contains(Position::new(cx as u16, cy as u16))
            };
            let body: Vec<(u16, u16, char)> = visit
                .painted
                .iter()
                .filter(|c| mine(c.x, c.y))
                .map(|c| (c.x, c.y, c.glyph))
                .filter(|cell| !door.contains(cell))
                .collect();
            assert!(body.is_empty(), "{what}: her box rains {body:?}");
            // In ASCII, the door's glyphs as shown (outside the sofa).
            let shown: Vec<(u16, u16, char)> = door
                .iter()
                .copied()
                .filter(|&(cx, cy, glyph)| {
                    mine(cx, cy) && frame[(cx, cy)].symbol().starts_with(glyph)
                })
                .collect();
            if !graphics && standing {
                assert!(!shown.is_empty(), "{what}: the door shows");
            }
            if let Some(graphics) = &mut guest.graphics {
                graphics.take_looks();
            }
            guest.activity(now);
            assert!(matches!(guest.state, State::Leaving(_)), "{what}: leaving");
            let mut t = now;
            while t < now + dissolve::RAIN_FROM_MS {
                let frame = paint(&mut guest, &real, &view, t);
                let what = format!("{what} t={}", t - now);
                assert!(drawn(&frame, &real, &piece), "{what}: the sofa stays");
                let text: Vec<String> = (0..frame.area.height)
                    .map(|y| row_text(&frame, y, 0..frame.area.width))
                    .collect();
                for face in [Face::Surprised, Face::Blink] {
                    let glyphs: String = face.glyphs().iter().collect();
                    assert!(
                        !text.iter().any(|row| row.contains(&glyphs)),
                        "{what}: {face:?}: {text:#?}"
                    );
                }
                for &(cx, cy, glyph) in &shown {
                    assert!(
                        frame[(cx, cy)].symbol().starts_with(glyph),
                        "{what}: the door stays at ({cx}, {cy})"
                    );
                }
                if let Some(graphics) = &mut guest.graphics {
                    let looks = graphics.take_looks();
                    // What the frame painted was seen: the sofa, and her
                    // door while it stood.
                    assert!(
                        looks.iter().any(|look| matches!(
                            look,
                            Look::Prop(Furniture::Sofa, _) | Look::Piece(Furniture::Sofa, _)
                        )),
                        "{what}: the sofa is painted: {looks:?}"
                    );
                    assert_eq!(
                        looks.iter().any(|look| matches!(look, Look::Door(_))),
                        standing,
                        "{what}: the door stays: {looks:?}"
                    );
                    let her: Vec<&Look> = looks
                        .iter()
                        .filter(|look| matches!(look, Look::Pose(..) | Look::Wave(..)))
                        .collect();
                    assert!(her.is_empty(), "{what}: {her:?}");
                }
                t += dissolve::FRAME_MS;
            }
        }
    }
}

/// Through a door in place on a protected floor line (she may stand on
/// one), the only protected cells a frame changes are the floor's under
/// what stands in her box, which the frame's image records: her, or her
/// door while she's through it. In both drawing modes. (The oracles took
/// the feet from her placement alone, so a door's floor, redrawn as ever,
/// read as a protected cell touched.)
#[test]
fn a_door_on_a_protected_floor_redraws_only_its_floor() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let (_, _, (x, y), _) = out_at_work(&real, &view, graphics, 12, |_, _| true);
        let floor = Rect::new(
            (x - sprite::WIDTH / 2) as u16,
            y as u16,
            sprite::WIDTH as u16 + 1,
            1,
        );
        let mut protected = view.protected.clone();
        protected.push(floor);
        let view = IdleView {
            protected: protected.clone(),
            ..view.clone()
        };
        let (mut guest, mut now, at, _) = out_at_work(&real, &view, graphics, 12, |_, _| true);
        assert_eq!(at, (x, y), "graphics={graphics}");
        let (mut doors, mut redrawn) = (0, 0);
        loop {
            let frame = paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("graphics={graphics}: still visiting");
            };
            let (hidden, door) = (visit.osaka.hidden(now), visit.osaka.door(now));
            if hidden && door.is_some() {
                doors += 1;
                let floor_cells = (floor.x..floor.right()).map(|cx| (cx, floor.y));
                if floor_cells.into_iter().any(|at| frame[at] != real[at]) {
                    redrawn += 1;
                }
            }
            if let Err(e) = assert_untouched_but_feet(&frame, &real, &protected, feet(visit)) {
                panic!("graphics={graphics} t={now} hidden={hidden} {door:?}: {e}");
            }
            if hidden && door.is_none() {
                break;
            }
            assert!(now < 600_000, "graphics={graphics}: never through");
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            guest.advance(now);
        }
        assert!(doors > 0, "graphics={graphics}: her door stood");
        assert_eq!(
            redrawn > 0,
            graphics,
            "graphics={graphics}: its floor redrawn"
        );
    }
}

/// Her sofa's home (seed 3, a Saturday at 11:00), she visiting, set down
/// `dx` columns right of the middle of her sofa and sent to work through
/// a door in place there, then stepped as the shell would until `when`
/// holds of her: the guest, the time, where she stood, and her sofa.
fn out_at_work(
    real: &Buffer,
    view: &IdleView,
    graphics: bool,
    dx: i32,
    when: impl Fn(&Osaka, u64) -> bool,
) -> (Guest, u64, (i32, i32), Shown) {
    let sofa = [(Furniture::Sofa, Nook::Users, 300)];
    let mut guest = home_at(3, sat(11, 0), &sofa, graphics);
    let mut now = until_visiting(&mut guest, real, view, 0);
    paint(&mut guest, real, view, now);
    let piece = shown_piece(&guest, Furniture::Sofa).expect("her sofa is shown");
    let (x, y) = (piece.left + i32::from(piece.size().0) / 2 + dx, piece.floor);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.place(x, y, now);
    visit.osaka.go_to_work(None, now, &mut Rng(1));
    paint(&mut guest, real, view, now);
    loop {
        let State::Visiting(visit) = &guest.state else {
            panic!("graphics={graphics}: still visiting");
        };
        if when(&visit.osaka, now) {
            break;
        }
        assert!(now < 600_000, "graphics={graphics}: never");
        shell_step(&mut guest, real, view, &mut now, true);
    }
    (guest, now, (x, y), piece)
}

/// The floor cell under what stands in her box in the last frame's image
/// (her, or her door), if it stands: its line is redrawn under it.
fn feet(visit: &Visit) -> Option<(i32, i32)> {
    visit.image.as_ref().and_then(BoxArt::feet)
}

/// A home she has visited once, her clock at `at`, `pieces` placed in it
/// (each on its nook's floor, `x` of the way along in thousandths), on a
/// mid-June school day with nothing on the calendar: she's absent, and
/// arrives as soon as the idle gate opens.
fn home_at(
    seed: u64,
    at: routine::GameTime,
    pieces: &[(Furniture, Nook, u16)],
    graphics: bool,
) -> Guest {
    let mut ledger = Ledger::new_at(seed, at);
    for &(item, nook, x) in pieces {
        assert!(
            ledger
                .home
                .add(room::Prop::new(item, nook, x, sprite::Facing::Right)),
            "{item:?}"
        );
    }
    let mut guest = Guest::restore(ledger);
    guest.set_date(date(2026, 6, 17));
    if graphics {
        guest.set_picker(kitty());
    }
    guest
}

/// One step as the shell takes it from `now`: a tick, and a paint if it
/// says the screen could change (or `always`).
fn shell_step(guest: &mut Guest, real: &Buffer, view: &IdleView, now: &mut u64, always: bool) {
    *now += guest
        .next_tick(*now)
        .map_or(1000, |d| d.as_millis() as u64)
        .clamp(1, 1000);
    if guest.advance(*now) || always {
        paint(guest, real, view, *now);
    }
}

/// From `from`, until she's visiting (the gate opens after the idle
/// delay): when she arrived.
fn until_visiting(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> u64 {
    let mut now = from;
    paint(guest, real, view, now);
    while !matches!(guest.state, State::Visiting(_)) {
        assert!(now < from + 60_000, "she never came");
        shell_step(guest, real, view, &mut now, true);
    }
    now
}

/// The monotonic millis her clock reads `at` (as read at `now`).
fn real_of(guest: &Guest, now: u64, at: routine::GameTime) -> u64 {
    guest
        .game_clock(now)
        .expect("met")
        .when(at.minutes() * 60_000)
}

/// Whether she's asleep for the night.
fn asleep(guest: &Guest) -> bool {
    matches!(&guest.state, State::Visiting(visit) if visit.osaka.sleeping())
}

/// Her night's sleep, on each surface she spends it on (her bed; her
/// sofa, with no bed; a bed she makes of text, with neither; the floor,
/// with nothing to make one of): to bed at her first decision after
/// bedtime (not before), there soon after, posed as the surface has her
/// with the lamp going off as she settles, and one act until her wake
/// time (Tuesday 07:00, 85 real minutes on); then up beside it, a
/// stretch and good morning in the new day's mood. In both drawing
/// modes.
#[test]
fn her_night_on_each_surface_is_one_act_until_she_wakes() {
    use super::script::{ScriptId, Surface};
    use super::sprite::Pose;
    let homely = home_screen();
    let mut ui = stage_ui();
    let wordy = real_frame(&mut ui, 100, 30);
    for graphics in [false, true] {
        for (surface, made, pieces, (real, view)) in [
            (
                Surface::Bed,
                false,
                vec![
                    (Furniture::Bed, Nook::Playlist, 500),
                    (Furniture::Sofa, Nook::Users, 300),
                ],
                &homely,
            ),
            (
                Surface::Sofa,
                false,
                vec![(Furniture::Sofa, Nook::Users, 300)],
                &homely,
            ),
            (Surface::Bed, true, vec![], &wordy),
            (Surface::Floor, false, vec![], &homely),
        ] {
            let at = format!("{surface:?} made={made} graphics={graphics}");
            let mut guest = home_at(3, mon(22, 20), &pieces, graphics);
            let mut now = until_visiting(&mut guest, real, view, 0);
            let bedtime = real_of(&guest, now, mon(22, 30));
            while !asleep(&guest) {
                assert!(now < bedtime + 120_000, "{at}: not in bed by {now}");
                shell_step(&mut guest, real, view, &mut now, false);
            }
            let osaka = &visit_of(&guest).osaka;
            let first = osaka
                .decisions
                .iter()
                .find(|d| d.day.is_some_and(|day| day.slot == routine::Slot::Asleep))
                .expect("decided at night");
            assert!(first.method.starts_with("routine/bed"), "{at}: {first}");
            assert!(
                osaka
                    .decisions
                    .iter()
                    .filter(|d| d.at < bedtime)
                    .all(|d| !d.method.starts_with("routine/")),
                "{at}: to bed early"
            );
            let (since, play) = osaka.plays_since().expect("playing");
            assert!(since >= bedtime, "{at}");
            assert_eq!(play.own, ScriptId::Night, "{at}");
            assert_eq!(play.branch, surface.branch(false), "{at}");
            let (pose, ..) = osaka.appearance(now);
            let posed = match surface {
                Surface::Bed => matches!(pose, Pose::Sleep(_)),
                Surface::Sofa => matches!(pose, Pose::Nap(_)),
                Surface::Floor => matches!(pose, Pose::LieBack(_)),
            };
            assert!(posed, "{at}: {pose:?}");
            let on = osaka.seat();
            assert_eq!(
                on.map(|seat| seat.makeshift()),
                (surface != Surface::Floor).then_some(made),
                "{at}"
            );
            assert_eq!(
                on.map(|seat| seat.item),
                match surface {
                    Surface::Bed => Some(Furniture::Bed),
                    Surface::Sofa => Some(Furniture::Sofa),
                    Surface::Floor => None,
                },
                "{at}"
            );
            // One act until her wake time, the lamp off once she's settled.
            let wake = real_of(&guest, now, tue(7, 0));
            let mut woke = None;
            while woke.is_none() {
                assert!(now < wake + 5_000, "{at}: still asleep at {now}");
                shell_step(&mut guest, real, view, &mut now, false);
                let osaka = &visit_of(&guest).osaka;
                if osaka.sleeping() {
                    assert_eq!(osaka.plays_since().map(|(t, _)| t), Some(since), "{at}");
                    if now > since + script::LAMP_ON_MS {
                        assert_eq!(osaka.prop(now), Some(script::Prop::LampOff), "{at}");
                    }
                } else {
                    woke = Some(now);
                }
            }
            let woke = woke.unwrap_or_default();
            assert!(
                woke >= wake && woke < wake + 1_000,
                "{at}: woke at {woke}, not {wake}"
            );
            // Up: a stretch, good morning in her new day's mood, the lamp on.
            let osaka = &visit_of(&guest).osaka;
            let (pose, _, said) = osaka.appearance(woke);
            assert_eq!(pose, Pose::Stretch, "{at}");
            let mood = brain::Mood::of(brain::day_seed(3, 1));
            assert_eq!(osaka.mood(), mood, "{at}");
            assert_eq!(said, Some(osaka::Bubble::Say(mood.wake_line())), "{at}");
            assert_eq!(osaka.prop(woke), None, "{at}");
            assert_eq!(
                guest.day(woke).map(|day| day.slot),
                Some(routine::Slot::Morning),
                "{at}"
            );
            // Beside her bed or sofa, not in it.
            if let Some(seat) = on {
                assert_ne!(osaka.x, seat.x, "{at}: still in it");
            }
            paint(&mut guest, real, view, woke);
        }
    }
}

/// Awake at 02:00 is out of her reach, however lively the chat: a line
/// every ten seconds from 22:00 on, and her first decision after bedtime
/// still takes her to bed, where she's asleep by soon after; there she
/// only stirs at each line (a murmur, blinking, turned over, in the same
/// act), right through to 02:00. In both drawing modes.
#[test]
fn a_lively_chat_never_keeps_her_up() {
    use super::sprite::Pose;
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let mut guest = home_at(
            4,
            mon(22, 0),
            &[(Furniture::Bed, Nook::Playlist, 500)],
            graphics,
        );
        let mut mark = ChatMark::default();
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let (bedtime, two) = (
            real_of(&guest, now, mon(22, 30)),
            real_of(&guest, now, tue(2, 0)),
        );
        let mut next_line = now;
        let mut slept_since = None;
        let mut stirs = 0;
        while now < two {
            let chat = now >= next_line;
            if chat {
                mark.synced += 1;
                next_line = now + 10_000;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000)
                .min(next_line.saturating_sub(now).max(1));
            let changed = guest.advance(now);
            // Painted on each line, or (before she's down) on any change.
            if chat || changed && slept_since.is_none() {
                paint(&mut guest, &real, &view, now);
            }
            let osaka = &visit_of(&guest).osaka;
            match slept_since {
                None if osaka.sleeping() => {
                    assert!(now >= bedtime, "graphics={graphics}: to bed early");
                    assert!(
                        now < bedtime + 90_000,
                        "graphics={graphics}: in bed at {now}"
                    );
                    slept_since = osaka.plays_since().map(|(t, _)| t);
                }
                None => {}
                Some(since) => {
                    assert!(osaka.sleeping(), "graphics={graphics}: awake at {now}");
                    assert_eq!(osaka.plays_since().map(|(t, _)| t), Some(since));
                    if chat {
                        let (pose, face, said) = osaka.appearance(now);
                        assert_eq!(pose, Pose::Sleep(1), "graphics={graphics}: turned");
                        assert_eq!(face, sprite::Face::Blink);
                        assert_eq!(said, Some(osaka::Bubble::Say(osaka::MM)));
                        stirs += 1;
                    }
                }
            }
        }
        assert!(slept_since.is_some(), "graphics={graphics}: never in bed");
        assert!(stirs > 100, "graphics={graphics}: {stirs} stirs");
        let osaka = &visit_of(&guest).osaka;
        let first = osaka
            .decisions
            .iter()
            .find(|d| d.at >= bedtime)
            .expect("decided after bedtime");
        assert_eq!(
            first.day.map(|day| day.slot),
            Some(routine::Slot::Asleep),
            "graphics={graphics}: {first}"
        );
        assert!(
            first.method.starts_with("routine/bed"),
            "graphics={graphics}: her first decision after bedtime: {first}"
        );
        let decided = osaka.decisions.iter().filter(|d| d.at >= bedtime).count();
        assert!(
            decided < 30,
            "graphics={graphics}: {decided} decisions after bedtime"
        );
    }
}

/// Tucked in (A4): a visit beginning at night has her in her bed from its
/// first frame (else her sofa), asleep for the night, the lamp already
/// off; a chat line only stirs her. With neither, she arrives as ever and
/// her routine sends her to bed at her first decision. In both drawing
/// modes.
#[test]
fn a_visit_at_night_begins_tucked_in() {
    use super::script::{Prop, ScriptId, Surface};
    let (real, view) = home_screen();
    for graphics in [false, true] {
        for (pieces, surface) in [
            (
                vec![
                    (Furniture::Bed, Nook::Playlist, 500),
                    (Furniture::Sofa, Nook::Users, 300),
                ],
                Some(Surface::Bed),
            ),
            (
                vec![(Furniture::Sofa, Nook::Users, 300)],
                Some(Surface::Sofa),
            ),
            (vec![], None),
        ] {
            let at = format!("{surface:?} graphics={graphics}");
            let mut guest = home_at(5, mon(23, 0), &pieces, graphics);
            let arrived = until_visiting(&mut guest, &real, &view, 0);
            let osaka = &visit_of(&guest).osaka;
            match surface {
                Some(surface) => {
                    assert!(osaka.sleeping(), "{at}: tucked in");
                    let (since, play) = osaka.plays_since().unwrap();
                    assert_eq!(since, arrived, "{at}");
                    assert_eq!(play.own, ScriptId::Night, "{at}");
                    assert_eq!(
                        play.branch,
                        surface.branch(true),
                        "{at}: the lamp off at once"
                    );
                    assert_eq!(osaka.prop(arrived), Some(Prop::LampOff), "{at}");
                    assert_eq!(
                        osaka.appearance(arrived).2,
                        Some(osaka::Bubble::Zzz),
                        "{at}: no hello"
                    );
                    let wake = real_of(&guest, arrived, tue(7, 0));
                    assert_eq!(
                        osaka.use_span().map(|(_, _, until)| until),
                        Some(wake),
                        "{at}"
                    );
                    // A chat line: a stir, no more.
                    let mut now = arrived;
                    let mark = ChatMark {
                        synced: 1,
                        ..ChatMark::default()
                    };
                    let chatty = IdleView {
                        chat_mark: mark,
                        ..view.clone()
                    };
                    for _ in 0..5 {
                        shell_step(&mut guest, &real, &chatty, &mut now, true);
                    }
                    let osaka = &visit_of(&guest).osaka;
                    assert!(osaka.sleeping(), "{at}");
                    assert_eq!(osaka.plays_since().map(|(t, _)| t), Some(since), "{at}");
                    assert!(osaka.decisions.is_empty(), "{at}: {:?}", osaka.decisions);
                    assert!(osaka.said_lines().is_empty() && !osaka.looking(), "{at}");
                    assert!(!osaka.greeted(), "{at}: hello waits for the morning");
                    // Her hello is good morning, as she wakes (her bed's
                    // case, the night through).
                    if surface == Surface::Bed {
                        while asleep(&guest) {
                            assert!(now < wake + 5_000, "{at}: still asleep");
                            shell_step(&mut guest, &real, &view, &mut now, false);
                        }
                        let osaka = &visit_of(&guest).osaka;
                        let mood = brain::Mood::of(brain::day_seed(5, 1));
                        assert_eq!(osaka.mood(), mood, "{at}");
                        assert_eq!(
                            osaka.appearance(now).2,
                            Some(osaka::Bubble::Say(mood.wake_line())),
                            "{at}"
                        );
                        assert!(osaka.greeted(), "{at}");
                        // And no hello after it.
                        let woke = now;
                        while now < woke + 30_000 {
                            shell_step(&mut guest, &real, &view, &mut now, true);
                            let said = visit_of(&guest).osaka.appearance(now).2;
                            assert_ne!(
                                said,
                                Some(osaka::Bubble::Say(mood.greeting())),
                                "{at}: hello at {now}"
                            );
                        }
                    }
                }
                None => {
                    assert!(!osaka.sleeping(), "{at}: not tucked in");
                    assert!(osaka.decisions.is_empty(), "{at}: {:?}", osaka.decisions);
                    let mut now = arrived;
                    while !asleep(&guest) {
                        assert!(now < arrived + 60_000, "{at}: never to bed");
                        shell_step(&mut guest, &real, &view, &mut now, false);
                    }
                    let osaka = &visit_of(&guest).osaka;
                    assert!(
                        osaka
                            .decisions
                            .iter()
                            .all(|d| d.method.starts_with("routine/bed")),
                        "{at}"
                    );
                }
            }
        }
    }
}

/// Come on an errand at night (to poke the accordion), a visit isn't
/// tucked in: the errand is what she came for, and she comes groggy
/// (she was asleep). Come for nothing in particular, the same visit is
/// tucked in, and she isn't groggy.
#[test]
fn an_errand_at_night_is_never_tucked_in() {
    let (real, view) = home_screen();
    for errand in [false, true] {
        let mut guest = home_at(
            5,
            mon(23, 0),
            &[(Furniture::Bed, Nook::Playlist, 500)],
            false,
        );
        let terrain = Terrain::read(&real, &view.protected, false);
        let floor = terrain.platforms.first().cloned().expect("a floor");
        let (x, y) = ((floor.x0 + floor.x1) / 2, floor.y);
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(x, y, 0, &mut rng);
        if errand {
            osaka.errand((x, y), &terrain, 0);
        }
        assert_eq!(
            guest.day(0).map(|day| day.slot),
            Some(routine::Slot::Asleep)
        );
        guest.begin_visit(osaka, terrain, (100, 30), Kind::Normal, 0);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        assert_eq!(visit.tuck, !errand, "errand={errand}");
        assert_eq!(visit.osaka.groggy(), errand, "errand={errand}");
    }
}

/// The stage at night: cued to arrive, she comes tucked in if her bed
/// or sofa is shown (else she arrives, and her routine sends her to
/// bed); cued to a scene, she plays it (her routine takes her to bed
/// after).
#[test]
fn a_stage_cue_at_night_tucks_her_in_only_to_arrive() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        for (pieces, scene, tucked) in [
            (
                vec![(Furniture::Bed, Nook::Playlist, 500)],
                Scene::Arrive,
                true,
            ),
            (
                vec![(Furniture::Sofa, Nook::Users, 300)],
                Scene::Arrive,
                true,
            ),
            (vec![], Scene::Arrive, false),
            (
                vec![(Furniture::Bed, Nook::Playlist, 500)],
                Scene::Jacks,
                false,
            ),
            // Dashed home (the stage's, at night): for what she forgot.
            (
                vec![(Furniture::Bed, Nook::Playlist, 500)],
                Scene::DashIn,
                false,
            ),
        ] {
            let at = format!("{scene:?} {pieces:?} graphics={graphics}");
            let mut guest = home_at(8, mon(23, 0), &pieces, graphics);
            guest.advance(0);
            guest.cue(scene);
            paint(&mut guest, &real, &view, 0);
            assert_eq!(asleep(&guest), tucked, "{at}");
            if scene == Scene::Jacks {
                let mut now = 0;
                for _ in 0..3 {
                    shell_step(&mut guest, &real, &view, &mut now, true);
                }
                let (pose, ..) = visit_of(&guest).osaka.appearance(now);
                assert!(matches!(pose, sprite::Pose::Jack(_)), "{at}: {pose:?}");
            }
        }
    }
}

/// The lamp goes off for the night as she settles in, and stays off
/// whatever gets her up in the night (here the stage putting her
/// elsewhere: back to bed in the dark), until she wakes.
#[test]
fn the_lamp_stays_off_until_she_wakes() {
    let (real, view) = home_screen();
    let mut guest = home_at(
        6,
        tue(6, 30),
        &[(Furniture::Bed, Nook::Playlist, 500)],
        false,
    );
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    assert!(asleep(&guest));
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    let (x, y) = (visit.osaka.x, visit.osaka.y);
    visit.osaka.place(x + 30, y, now);
    assert!(!visit.osaka.sleeping());
    assert!(visit.osaka.dark(now), "up, in the dark");
    assert_eq!(visit.osaka.prop(now), None, "her act shows nothing");
    while !asleep(&guest) {
        assert!(visit_of(&guest).osaka.dark(now), "at {now}");
        assert!(now < 60_000 + 30_000, "never back to bed");
        shell_step(&mut guest, &real, &view, &mut now, true);
    }
    let wake = real_of(&guest, now, tue(7, 0));
    // Back in bed (settling in the dark: her act's lamp key a moment on
    // shows nothing over the dark).
    while asleep(&guest) {
        assert!(visit_of(&guest).osaka.dark(now), "at {now}");
        shell_step(&mut guest, &real, &view, &mut now, false);
    }
    assert!(now >= wake);
    let osaka = &visit_of(&guest).osaka;
    assert!(!osaka.dark(now), "on again as she wakes");
    assert_eq!(osaka.prop(now), None);
}

/// Her needs after the night (A11), a whole one from her evening by her
/// routine's bed reflex: as she wakes they're the morning's, exactly,
/// with nothing of the night kept to count again; and at her first
/// decision after, none is anywhere near saturated (a night at a quarter
/// of the pace would have the quick ones at 1). Her line budget, spent
/// in the evening, is the new day's, afresh.
#[test]
fn she_wakes_with_the_mornings_needs() {
    use super::brain::{Need, Needs};
    let (real, view) = home_screen();
    let mut guest = home_at(
        7,
        mon(22, 20),
        &[(Furniture::Bed, Nook::Playlist, 500)],
        false,
    );
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    assert!(!asleep(&guest));
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.note_beat(now);
    let bedtime = real_of(&guest, now, mon(22, 30));
    while !asleep(&guest) {
        assert!(now < bedtime + 120_000, "not in bed by {now}");
        shell_step(&mut guest, &real, &view, &mut now, false);
    }
    assert!(now >= bedtime);
    let spent = visit_of(&guest).osaka.line_budget();
    assert!(spent.is_some_and(|(d, n)| d == 0 && n > 0), "{spent:?}");
    let wake = real_of(&guest, now, tue(7, 0));
    while asleep(&guest) {
        assert!(now < wake + 5_000, "still asleep at {now}");
        shell_step(&mut guest, &real, &view, &mut now, false);
    }
    let osaka = &visit_of(&guest).osaka;
    let morning = Needs::arriving_in(routine::Slot::Morning);
    assert_eq!(*osaka.needs(), morning, "as she wakes");
    assert_eq!(osaka.slept_ms(), 0, "the night is behind her");
    assert_eq!(osaka.line_budget(), Some((1, 0)), "a new day's budget");
    let decided = osaka.decisions.len();
    while visit_of(&guest).osaka.decisions.len() == decided {
        shell_step(&mut guest, &real, &view, &mut now, true);
    }
    let needs = *visit_of(&guest).osaka.needs();
    for need in Need::ALL {
        let bound = morning.get(need) + 0.25;
        assert!(
            needs.get(need) <= bound.min(0.95),
            "{need:?}: {} after the night",
            needs.get(need)
        );
    }
}

/// The day is the unit (round-1b): a second visit the same game day
/// comes in that day's mood and says its hello; her mood changes with
/// the game day (as a visit begins on a new one, and as she wakes into
/// one), never within one; her line budget is the day's. Unfed, each
/// visit's own, as ever.
#[test]
fn her_mood_is_the_game_days() {
    let (real, view) = home_screen();
    let mood_of = |guest: &Guest| visit_of(guest).osaka.mood();
    let (mut changed, mut hellos) = (0, 0);
    for seed in 0..16u64 {
        // Two visits on Monday evening: one mood, one hello.
        let mut guest = home_at(seed, mon(18, 0), &[], false);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let first = mood_of(&guest);
        assert_eq!(
            first,
            brain::Mood::of(brain::day_seed(seed, 0)),
            "seed {seed}"
        );
        let later = now + 60_000;
        while now < later {
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        // A beat line said: the day's budget has something in it.
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.note_beat(now);
        // (Kept by the guest as she next steps, as one she said would be.)
        shell_step(&mut guest, &real, &view, &mut now, true);
        let spent = visit_of(&guest).osaka.line_budget();
        assert!(
            spent.is_some_and(|(day, n)| day == 0 && n > 0),
            "seed {seed}: {spent:?}"
        );
        guest.activity(now);
        while guest.present() {
            assert!(now < later + 60_000, "seed {seed}: never gone");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        // What she said that day up to leaving (the one noted, at least)
        // is the budget the next visit that day starts from.
        let today = guest.lines_today;
        assert!(
            today
                .zip(spent)
                .is_some_and(|((d, n), (_, m))| d == 0 && n >= m),
            "seed {seed}: {today:?} {spent:?}"
        );
        let mut now = until_visiting(&mut guest, &real, &view, now);
        assert_eq!(guest.ledger.visits, 3);
        assert_eq!(mood_of(&guest), first, "seed {seed}: the same day's mood");
        assert_eq!(
            visit_of(&guest).osaka.line_budget(),
            today,
            "seed {seed}: the day's budget"
        );
        // Her hello is the day's (or, dropping in, "I'm OK", which does
        // as well).
        let came = now;
        while !visit_of(&guest).osaka.greeted() {
            assert!(now < came + 120_000, "seed {seed}: no hello");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let said = visit_of(&guest).osaka.appearance(now).2;
        if said != Some(osaka::Bubble::Say(osaka::OK)) {
            assert_eq!(
                said,
                Some(osaka::Bubble::Say(first.greeting())),
                "seed {seed}"
            );
            hellos += 1;
        }
        // A visit on Tuesday: Tuesday's mood.
        let mut tuesday = home_at(seed, tue(18, 0), &[], false);
        until_visiting(&mut tuesday, &real, &view, 0);
        let next = mood_of(&tuesday);
        assert_eq!(
            next,
            brain::Mood::of(brain::day_seed(seed, 1)),
            "seed {seed}"
        );
        changed += usize::from(next != first);
        // Unfed: the visit's own.
        let mut unfed = home_at(seed, mon(18, 0), &[], false).unfed();
        until_visiting(&mut unfed, &real, &view, 0);
        assert_eq!(
            mood_of(&unfed),
            brain::Mood::of(unfed.ledger.visit_seed(1)),
            "seed {seed}"
        );
    }
    assert!(
        changed > 4,
        "her mood changed with the day {changed} times in 16"
    );
    assert!(hellos > 4, "the day's hello {hellos} times in 16");
}

// ---- Deliveries and the shopping channel ----

/// The channel sells her furniture first; decor when her room feeling
/// bare is what she needs most, or when there's no furniture left to
/// sell her.
#[test]
fn the_channel_sells_decor_when_her_room_feels_bare() {
    use super::brain::{Need, Needs};
    let mut ledger = Ledger::new(1);
    ledger.visits = 9;
    for item in [Furniture::Tv, Furniture::Sofa] {
        assert!(
            ledger
                .home
                .add(room::Prop::new(item, Nook::Users, 0, sprite::Facing::Right))
        );
    }
    let bare = Needs::with(&[(Need::Beauty, 0.9), (Need::Fun, 0.5)]);
    let fun = Needs::with(&[(Need::Beauty, 0.5), (Need::Fun, 0.9)]);
    assert_eq!(advert(&ledger, false, &fun), Some(Furniture::Bed));
    assert_eq!(advert(&ledger, false, &bare), Some(Furniture::Plant));
    assert!(ledger.home.add(room::Prop::new(
        Furniture::Plant,
        Nook::Users,
        0,
        sprite::Facing::Right
    )));
    assert_eq!(advert(&ledger, false, &bare), Some(Furniture::Poster));
    for item in CATALOGUE.iter().filter(|i| !i.decor()) {
        let _ = ledger.home.add(room::Prop::new(
            *item,
            Nook::Playlist,
            0,
            sprite::Facing::Right,
        ));
    }
    assert_eq!(
        advert(&ledger, false, &fun),
        Some(Furniture::Poster),
        "nothing else left"
    );
    assert!(ledger.home.add(room::Prop::new(
        Furniture::Poster,
        Nook::Users,
        0,
        sprite::Facing::Right
    )));
    assert_eq!(advert(&ledger, false, &bare), None, "she has it all");
}

/// [`home_screen`] with chat text above her panes.
fn busy_home_screen() -> (Buffer, IdleView) {
    let (mut real, view) = home_screen();
    for y in 1..7u16 {
        real.set_string(
            2 + (y * 7) % 30,
            y,
            "so what did you think of it",
            Style::new(),
        );
    }
    (real, view)
}

/// Watching the shopping channel in a room that feels bare to her, she
/// buys a potted plant (her room feeling bare is what she needs most);
/// otherwise, the next piece of furniture. In both drawing modes.
#[test]
fn a_bare_room_has_her_buy_decor() {
    use super::brain::Need;
    for graphics in [false, true] {
        for bare in [true, false] {
            let at = format!("graphics={graphics} bare={bare}");
            let (real, view) = busy_home_screen();
            let mut guest = Guest::new(3);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::Shopping);
            paint(&mut guest, &real, &view, 0);
            assert!(
                matches!(guest.cue_note(), Some(Ok(_))),
                "{at}: {:?}",
                guest.cue_note()
            );
            let mut now = 0;
            let advert = loop {
                assert!(now < 30_000, "{at}: never watched the shopping channel");
                if bare && let State::Visiting(visit) = &mut guest.state {
                    visit.osaka.press(Need::Beauty);
                }
                now += guest
                    .next_tick(now)
                    .map_or(100, |d| d.as_millis() as u64)
                    .clamp(1, 100);
                guest.advance(now);
                let State::Visiting(visit) = &guest.state else {
                    panic!("{at}: visiting");
                };
                if let Some((.., play)) = visit.osaka.playing()
                    && let Some(item) = play.bought
                {
                    // Sold: she watches the channel that sold it.
                    assert_eq!(play.own, script::ScriptId::Shopping, "{at}");
                    assert!(
                        matches!(
                            visit.osaka.prop(now),
                            Some(script::Prop::Tv(art::Channel::Shopping(_)))
                        ),
                        "{at}: {:?}",
                        visit.osaka.prop(now)
                    );
                    break item;
                }
                paint(&mut guest, &real, &view, now);
            };
            let want = if bare {
                Furniture::Plant
            } else {
                Furniture::Sofa
            };
            assert_eq!(advert, want, "{at}");
            assert_eq!(guest.ledger.ordered, Some(want), "{at}");
        }
    }
}

/// Her room feeling bare: it bothers her more and more while she's in a
/// plain room, and resting and using her things in a pretty one (a
/// poster in one room, a plant in the other) eases it. In both drawing
/// modes, with chat text about.
#[test]
fn a_pretty_room_eases_her_want_of_beauty() {
    use super::brain::Need;
    for graphics in [false, true] {
        let (real, view) = busy_home_screen();
        let level = |pretty: bool| {
            let mut guest = Guest::new(5);
            if graphics {
                guest.set_picker(kitty());
            }
            let mut pieces = vec![
                (Furniture::Sofa, Nook::Users, 0),
                (Furniture::Tv, Nook::Users, 600),
                (Furniture::Bed, Nook::Playlist, 0),
            ];
            if pretty {
                pieces.push((Furniture::Poster, Nook::Users, 300));
                pieces.push((Furniture::Plant, Nook::Playlist, 1000));
            }
            for (item, nook, at) in pieces {
                assert!(guest.ledger.home.add(room::Prop::new(
                    item,
                    nook,
                    at,
                    sprite::Facing::Right
                )));
            }
            guest.cue(Scene::Arrive);
            paint(&mut guest, &real, &view, 0);
            if graphics {
                live_in(&mut guest, &real, &view, 6 * 60_000, 5);
            } else {
                let _ = run(&mut guest, &real, &view, 0, 6 * 60_000);
            }
            let visit = visit_of(&guest);
            if pretty {
                assert!(
                    visit.shown.iter().filter(|s| s.item.decor()).count() == 2,
                    "graphics={graphics}: {:?}",
                    visit.shown
                );
            }
            visit.osaka.needs().get(Need::Beauty)
        };
        let (plain, pretty) = (level(false), level(true));
        assert!(plain > 0.5, "graphics={graphics}: plain {plain}");
        assert!(pretty < 0.3, "graphics={graphics}: pretty {pretty}");
    }
}

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

/// A parcel comes in through a flap in the wall at the screen's edge
/// and stands against it, boxed; the flap is open as it comes, then
/// shut. The next comes through the same flap and pushes the first
/// along. In both drawing modes.
#[test]
fn a_parcel_comes_in_through_a_flap_at_the_screens_edge() {
    for graphics in [false, true] {
        let (real, view) = home_screen();
        let (_, users) = view.nooks[0];
        let floor = i32::from(users.bottom()) - 1;
        let wall = |frame: &Buffer| {
            [floor - 2, floor - 1].map(|y| frame[(users.x, y as u16)].symbol().to_owned())
        };
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        // Standing across the screen, out of the flap's way (arriving,
        // she'd come in at that edge, in front of it).
        visiting_at(&mut guest, &real, &view, (75, floor));
        guest.ledger.visits = 1;
        guest.send_parcel();
        let frame = paint(&mut guest, &real, &view, 0);
        let parcel = guest.ledger.home.props[0];
        assert_eq!(parcel.item, Furniture::Tv);
        assert!(parcel.boxed);
        assert_eq!(parcel.strip, room::Strip::Bottom(Nook::Users));
        let shown = |guest: &Guest, item| match &guest.state {
            State::Visiting(visit) => visit.shown.iter().find(|s| s.item == item).copied(),
            _ => None,
        };
        let tv = shown(&guest, Furniture::Tv).expect("the parcel shows");
        assert_eq!(tv.left, i32::from(users.x) + 1, "against the wall");
        assert_eq!(wall(&frame), ["╲", "╲"], "the flap is open");
        let now = FLAP_MS + 100;
        assert!(guest.advance(now), "the flap shuts");
        let frame = paint(&mut guest, &real, &view, now);
        assert_eq!(wall(&frame), ["│", "│"]);
        guest.send_parcel();
        let frame = paint(&mut guest, &real, &view, now);
        assert_eq!(wall(&frame), ["╲", "╲"]);
        let sofa = shown(&guest, Furniture::Sofa).expect("the next parcel shows");
        let tv = shown(&guest, Furniture::Tv).expect("the TV still shows");
        assert_eq!(sofa.left, i32::from(users.x) + 1, "{sofa:?}");
        assert!(
            tv.left >= sofa.rect().right() as i32,
            "pushed along: {tv:?}"
        );
    }
}

/// The piece `item` as shown this frame, if it is.
fn shown_piece(guest: &Guest, item: Furniture) -> Option<Shown> {
    match &guest.state {
        State::Visiting(visit) => visit.shown.iter().find(|s| s.item == item).copied(),
        _ => None,
    }
}

/// Whether `piece` is drawn in `frame` (its glyphs, or an image over it).
fn drawn(frame: &Buffer, real: &Buffer, piece: &Shown) -> bool {
    let r = piece.rect();
    (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| frame[(x, y)] != real[(x, y)]))
}

/// Decor delivered: its parcel stands on the floor where it came in, she
/// unpacks it, and the plant stands there while the poster hangs on the
/// wall above, both drawn. In both drawing modes.
#[test]
fn she_unpacks_delivered_decor() {
    for item in [Furniture::Plant, Furniture::Poster] {
        for graphics in [false, true] {
            let at = format!("{item:?} graphics={graphics}");
            let (real, view) = home_screen();
            let (_, users) = view.nooks[0];
            let floor = i32::from(users.bottom()) - 1;
            let mut guest = Guest::new(3);
            if graphics {
                guest.set_picker(kitty());
            }
            visiting_at(&mut guest, &real, &view, (75, floor));
            guest.ledger.visits = 1;
            guest.ledger.ordered = Some(item);
            guest.ledger.bought_on = 0;
            paint(&mut guest, &real, &view, 0);
            let parcel = shown_piece(&guest, item).unwrap_or_else(|| panic!("{at}: delivered"));
            assert!(parcel.boxed, "{at}");
            assert_eq!(parcel.lane(), room::Lane::Floor, "{at}");
            let mut now = 0;
            while now < 10 * 60_000 && guest.ledger.home.boxed() {
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
            }
            assert!(!guest.ledger.home.boxed(), "{at}: never unpacked");
            assert!(
                !prop_of(&guest, item).settled,
                "{at}: unpacking settles nothing"
            );
            let frame = paint(&mut guest, &real, &view, now);
            let piece = shown_piece(&guest, item).unwrap_or_else(|| panic!("{at}: shown"));
            assert_eq!(piece.left, parcel.left, "{at}: where its parcel stood");
            let lane = match item {
                Furniture::Poster => room::Lane::Wall,
                _ => room::Lane::Floor,
            };
            assert_eq!(piece.lane(), lane, "{at}");
            assert!(drawn(&frame, &real, &piece), "{at}: drawn");
        }
    }
}

/// A delivery stands where it came in, which she never chose: it's
/// unsettled (until she sets it down herself). A gift the stage gives
/// her is settled where it lands. In both drawing modes.
#[test]
fn deliveries_are_unsettled_and_stage_gifts_settled() {
    for graphics in [false, true] {
        let (real, view) = home_screen();
        let (_, users) = view.nooks[0];
        let floor = i32::from(users.bottom()) - 1;
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (75, floor));
        guest.ledger.visits = 1;
        guest.ledger.ordered = Some(Furniture::Tv);
        guest.ledger.bought_on = 0;
        paint(&mut guest, &real, &view, 0);
        let tv = prop_of(&guest, Furniture::Tv);
        assert!(tv.boxed && !tv.settled, "graphics {graphics}: {tv:?}");
        guest.give(Furniture::Lamp);
        paint(&mut guest, &real, &view, 100);
        let lamp = prop_of(&guest, Furniture::Lamp);
        assert!(!lamp.boxed && lamp.settled, "graphics {graphics}: {lamp:?}");
        // Saved and read back, each is as it was.
        let back = ledger::Ledger::from_json(&guest.ledger.to_json()).unwrap();
        let settled: Vec<(Furniture, bool)> = back
            .home
            .props
            .iter()
            .map(|p| (p.item, p.settled))
            .collect();
        assert_eq!(
            settled,
            [(Furniture::Tv, false), (Furniture::Lamp, true)],
            "graphics {graphics}"
        );
    }
}

/// A poster hung over her sofa: it shows, drawn, while she goes about
/// her visit (she passes under it, and lounges under it), and nothing
/// hides text or touches what's protected. In both drawing modes.
#[test]
fn a_poster_hangs_over_her_sofa() {
    for graphics in [false, true] {
        let (real, view) = home_screen();
        let mut under = false;
        for seed in 0..3u64 {
            let at = format!("graphics={graphics} seed {seed}");
            let mut guest = Guest::new(seed);
            if graphics {
                guest.set_picker(kitty());
            }
            for item in [Furniture::Sofa, Furniture::Poster] {
                assert!(guest.ledger.home.add(room::Prop {
                    anchor: Some(room::Anchor {
                        side: room::Side::Left,
                        offset: 10,
                    }),
                    ..room::Prop::new(item, Nook::Users, 0, sprite::Facing::Right)
                }));
            }
            let mut hidden = Hidden::default();
            let mut now = 0;
            while now < 4 * 60_000 {
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                guest.advance(now);
                let frame = paint(&mut guest, &real, &view, now);
                let State::Visiting(visit) = &guest.state else {
                    continue;
                };
                let poster = shown_piece(&guest, Furniture::Poster)
                    .unwrap_or_else(|| panic!("{at} {now}: the poster shows"));
                let sofa = shown_piece(&guest, Furniture::Sofa)
                    .unwrap_or_else(|| panic!("{at} {now}: the sofa shows"));
                assert_eq!(poster.left, sofa.left, "{at}: over the sofa");
                // In line art: its box ends on the row above where the
                // layer is placed (see `graphics::Layer`), off the floor.
                let layer = prop_layer(
                    &poster,
                    graphics::Look::Prop(Furniture::Poster, art::Layer::Whole),
                );
                assert!(!layer.standing, "{at}");
                assert_eq!(layer.at.1, i32::from(poster.rect().bottom()), "{at}");
                assert!(drawn(&frame, &real, &poster), "{at} {now}: drawn");
                let r = poster.rect();
                let (x, y) = (visit.osaka.x, visit.osaka.y);
                let half = sprite::WIDTH / 2;
                if y == poster.floor
                    && x + half >= i32::from(r.x)
                    && x - half < i32::from(r.right())
                {
                    under = true;
                }
                let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
                assert_untouched(&frame, &real, &view.protected)
                    .unwrap_or_else(|e| panic!("{at} {now}: {e}"));
                if graphics {
                    hidden
                        .check(&frame, &real, &layer, now)
                        .unwrap_or_else(|e| panic!("{at} {now}: {e}"));
                }
            }
        }
        assert!(under, "graphics={graphics}: she never went under it");
    }
}

/// Text in the pane around a poster hung over her sofa — beside it, just
/// above it, just below it — leaves it hanging (and
/// the sofa standing) all visit; text over one cell of it closets the
/// poster alone. Either way nothing of hers hides text or touches what's
/// protected. In both drawing modes.
#[test]
fn text_near_a_hung_poster_closets_only_what_it_covers() {
    for graphics in [false, true] {
        for over in [false, true] {
            let at = format!("graphics={graphics} over={over}");
            let (mut real, view) = home_screen();
            // The poster's cells: columns 11..15, rows 10 and 11 (four
            // rows clear between it and the floor at 16); the sofa's
            // 11..20, rows 13..16.
            let mut text = vec![
                (11, 9, "top!".to_owned()),
                (6, 10, "ab".to_owned()),
                (15, 11, "zz".to_owned()),
                (24, 11, "and on".to_owned()),
                (11, 12, "low".to_owned()),
            ];
            if over {
                text.push((12, 10, "x".to_owned()));
            }
            scatter(&mut real, &text, &[]);
            let mut guest = Guest::new(2);
            if graphics {
                guest.set_picker(kitty());
            }
            for item in [Furniture::Sofa, Furniture::Poster] {
                assert!(guest.ledger.home.add(room::Prop {
                    anchor: Some(room::Anchor {
                        side: room::Side::Left,
                        offset: 10,
                    }),
                    ..room::Prop::new(item, Nook::Users, 0, sprite::Facing::Right)
                }));
            }
            let mut hidden = Hidden::default();
            let mut now = 0;
            let mut visited = false;
            while now < 3 * 60_000 {
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                guest.advance(now);
                let frame = paint(&mut guest, &real, &view, now);
                let State::Visiting(visit) = &guest.state else {
                    continue;
                };
                visited = true;
                let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
                let moved_into = |r: Rect| layer.iter().any(|&c| r.contains(c.into()));
                let poster = Rect::new(11, 10, 4, 2);
                let sofa = Rect::new(11, 13, 9, 3);
                let shows = |item| shown_piece(&guest, item).is_some();
                if !moved_into(poster) {
                    assert_eq!(shows(Furniture::Poster), !over, "{at} {now}: the poster");
                }
                if !moved_into(sofa) {
                    assert!(shows(Furniture::Sofa), "{at} {now}: the sofa");
                }
                if let Some(p) = shown_piece(&guest, Furniture::Poster) {
                    assert_eq!(p.rect(), poster, "{at}");
                }
                assert_untouched(&frame, &real, &view.protected)
                    .unwrap_or_else(|e| panic!("{at} {now}: {e}"));
                if graphics {
                    hidden
                        .check(&frame, &real, &layer, now)
                        .unwrap_or_else(|e| panic!("{at} {now}: {e}"));
                }
            }
            assert!(visited, "{at}: she never came");
        }
    }
}

/// A poster hung over her sofa, her beside it, as her line art draws
/// them, for review: `HOUSEGUEST_HUNG=/tmp/hung.png cargo test -p
/// dessplay --lib hung_poster_sheet -- --ignored`. At 9×19 px cells, then
/// HiDPI 18×38. (In play each piece is its own image; here they're
/// composed into one.)
#[test]
#[ignore = "writes a PNG for review"]
#[allow(deprecated)]
fn hung_poster_sheet() {
    use super::graphics::{Layer, Look};
    let path = std::env::var("HOUSEGUEST_HUNG").expect("HOUSEGUEST_HUNG");
    let (real, view) = home_screen();
    let mut images = Vec::new();
    for cell in [(9u16, 19u16), (18, 38)] {
        let mut picker = ratatui_image::picker::Picker::from_fontsize(cell.into());
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        let mut guest = Guest::new(0);
        guest.set_picker(picker);
        for item in [Furniture::Sofa, Furniture::Poster] {
            assert!(guest.ledger.home.add(room::Prop {
                anchor: Some(room::Anchor {
                    side: room::Side::Left,
                    offset: 10,
                }),
                ..room::Prop::new(item, Nook::Users, 0, sprite::Facing::Right)
            }));
        }
        visiting_at(&mut guest, &real, &view, (40, 16));
        paint(&mut guest, &real, &view, 0);
        let sofa = shown_piece(&guest, Furniture::Sofa).unwrap();
        let poster = shown_piece(&guest, Furniture::Poster).unwrap();
        let layers = [
            prop_layer(&sofa, Look::Prop(Furniture::Sofa, art::Layer::Whole)),
            prop_layer(&poster, Look::Prop(Furniture::Poster, art::Layer::Whole)),
            Layer {
                look: Look::Pose(
                    Pose::Stand,
                    sprite::Face::Vacant,
                    placement::InSight::assumed(),
                ),
                facing: sprite::Facing::Left,
                at: (sofa.left + 13, sofa.floor),
                standing: true,
            },
        ];
        let graphics = guest.graphics.as_ref().unwrap();
        images.push(
            graphics
                .canvas(&real, &layers, &|_, _| true)
                .expect("composes"),
        );
    }
    let pad = 18;
    let width = images.iter().map(|i| i.width()).max().unwrap() + 2 * pad;
    let height = images.iter().map(|i| i.height() + pad).sum::<u32>() + pad;
    let mut sheet = image::RgbaImage::from_pixel(width, height, image::Rgba([30, 33, 39, 255]));
    let mut y = pad;
    for image in &images {
        image::imageops::overlay(&mut sheet, image, i64::from(pad), i64::from(y));
        y += image.height() + pad;
    }
    sheet.save(path).unwrap();
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
    watch_scene_in(&home_screen(), scene, pieces, graphics, until)
}

/// [`watch_scene`], on `screen`.
fn watch_scene_in(
    screen: &(Buffer, IdleView),
    scene: Scene,
    pieces: &[Furniture],
    graphics: bool,
    until: impl Fn(&Visit, u64) -> bool,
) -> bool {
    let (real, view) = screen;
    let mut guest = Guest::new(8);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrive);
    paint(&mut guest, real, view, 0);
    for &item in pieces {
        guest.give(item);
        paint(&mut guest, real, view, 0);
        assert!(
            guest.ledger.home.owns(item),
            "{item:?}: {:?}",
            guest.cue_note()
        );
    }
    guest.cue(scene);
    let mut hidden = Hidden::default();
    let mut now = 0;
    paint(&mut guest, real, view, now);
    while now < 20_000 {
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        let frame = paint(&mut guest, real, view, now);
        let State::Visiting(visit) = &guest.state else {
            panic!("{scene:?}: still visiting");
        };
        if graphics {
            let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
            hidden
                .check(&frame, real, &layer, now)
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
    Some(piece_state(
        piece,
        visit.osaka.prop(now),
        visit.osaka.dark(now),
        cat,
    ))
}

/// What each piece shows for each thing her script can show on her
/// furniture, cat home or not, her lamp dark for the night or not: the
/// lamp off, the fridge open and the cat biting each only on their own
/// piece, the cat biting only when he's home, and in his bed when
/// nothing's going on; the lamp off in the dark whatever her script
/// shows, and the dark nothing else's; everything else plain (what's on
/// TV is drawn on its screen, not as a state).
#[test]
fn every_piece_shows_what_her_script_shows_on_it() {
    use art::{Channel, PieceState};
    use script::Prop;
    let props = [
        None,
        Some(Prop::LampOff),
        Some(Prop::FridgeOpen),
        Some(Prop::CatBiting),
        Some(Prop::Tv(Channel::Snow(0))),
        Some(Prop::Tv(Channel::Shopping(1))),
    ];
    for item in Furniture::ALL {
        let piece = Shown {
            item,
            facing: sprite::Facing::Right,
            boxed: false,
            strip: None,
            left: 40,
            floor: 20,
            scrap: None,
        };
        for prop in props {
            for cat in [false, true] {
                for dark in [false, true] {
                    let want = match (item, prop) {
                        (Furniture::Lamp, _) if dark => PieceState::LampOff,
                        (Furniture::Lamp, Some(Prop::LampOff)) => PieceState::LampOff,
                        (Furniture::Fridge, Some(Prop::FridgeOpen)) => PieceState::FridgeOpen,
                        (Furniture::CatBed, Some(Prop::CatBiting)) if cat => PieceState::CatBiting,
                        (Furniture::CatBed, _) if cat => PieceState::Cat,
                        _ => PieceState::Plain,
                    };
                    assert_eq!(
                        piece_state(&piece, prop, dark, cat),
                        want,
                        "{item:?} {prop:?} cat {cat} dark {dark}"
                    );
                }
            }
        }
    }
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

/// [`home_screen`], its panes full of text as a client's are (the users
/// and the playlist listed down them), so her image, held out or
/// reaching, has text to keep off.
fn wordy_home_screen() -> (Buffer, IdleView) {
    let (mut real, view) = home_screen();
    let users = [
        "kim        ready   Frieren 12  01:14:03",
        "bob        watching            00:24:10",
        "svein      paused  Frieren 12  01:14:03",
        "aoi        ready   Frieren 13  00:00:00",
        "mika       away                00:03:51",
        "ren        ready   Frieren 12  01:14:03",
    ];
    for (row, line) in (9u16..).zip(users) {
        real.set_string(2, row, line, Style::new());
    }
    for (row, ep) in (9u16..15).zip(12..) {
        let line = format!("{:02} Frieren ep {ep} [1080p]  23:40", row - 8);
        real.set_string(52, row, line, Style::new());
    }
    (real, view)
}

/// The two-pane home, quiet and text-dense, by name.
fn home_screens() -> [(&'static str, (Buffer, IdleView)); 2] {
    [("quiet", home_screen()), ("wordy", wordy_home_screen())]
}

/// Her vignettes, cued: the chopsticks split cleanly (a twinkle) or
/// badly (told off) at her desk before homework, and after a snack the
/// sata andagi, held up and named — each in both drawing modes, quiet
/// panes or text-dense, never hiding text.
#[test]
fn her_vignettes_play_at_her_things() {
    use super::sprite::Pose;
    use script::{HOLD_EM, SATA_ANDAGI};
    for (name, screen) in home_screens() {
        for graphics in [false, true] {
            for (scene, pieces, pose, said) in [
                (
                    Scene::ChopsticksClean,
                    &[Furniture::Desk][..],
                    Pose::Chopsticks(1),
                    osaka::Bubble::Sparkle,
                ),
                (
                    Scene::ChopsticksBad,
                    &[Furniture::Desk],
                    Pose::Chopsticks(2),
                    osaka::Bubble::Say(HOLD_EM),
                ),
                (
                    Scene::Andagi,
                    &[],
                    Pose::EatAndagi(0),
                    osaka::Bubble::Say(SATA_ANDAGI),
                ),
            ] {
                assert!(
                    watch_scene_in(&screen, scene, pieces, graphics, |v, now| {
                        let (shown, _, bubble) = v.osaka.appearance(now);
                        shown == pose && bubble == Some(said)
                    }),
                    "{name}, graphics={graphics}: {scene:?}"
                );
            }
        }
    }
}

/// A guest cued to the andagi after her snack on `screen` (graphics or
/// not), what she's seen of the chat `seen`, run (her image never hiding
/// text) until `at` holds of the use she plays (its play, since, until,
/// and the time), at most 30 s in: the guest, the text she's hidden so
/// far, and when.
fn andagi_until(
    screen: &(Buffer, IdleView),
    graphics: bool,
    seen: ChatMark,
    at: impl Fn(script::Play, u64, u64, u64) -> bool,
) -> (Guest, Hidden, u64) {
    let (real, view) = screen;
    let view = IdleView {
        chat_mark: seen,
        ..view.clone()
    };
    let mut guest = Guest::new(8);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrive);
    paint(&mut guest, real, &view, 0);
    guest.cue(Scene::Andagi);
    let mut hidden = Hidden::default();
    let mut now = 0;
    paint(&mut guest, real, &view, now);
    while now < 30_000 {
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        let frame = paint(&mut guest, real, &view, now);
        if graphics {
            let layer: Vec<(u16, u16)> = visit_of(&guest).layer.cells().collect();
            hidden
                .check(&frame, real, &layer, now)
                .unwrap_or_else(|e| panic!("graphics at {now}: {e}"));
        }
        let osaka = &visit_of(&guest).osaka;
        if let (Some(play), Some((_, since, until))) = (osaka.plays(), osaka.use_span())
            && at(play, since, until, now)
        {
            return (guest, hidden, now);
        }
    }
    panic!("graphics={graphics}: never there by {now}");
}

/// A chat line asking her something (ending in "?"), in chat or on IRC,
/// as the andagi plays after her snack: she turns to the chat and
/// answers "Sata andagi.", beaming, and plays the andagi on to its end,
/// neither stopped nor cut short, eating it at last (and, bitten, it
/// stays bitten). A line that asks nothing then, history compacted
/// while the newest line is a question, or a question during the snack
/// itself, she stops and looks at, as at any. In both drawing modes,
/// quiet panes or text-dense, never hiding text.
#[test]
fn asked_during_the_andagi_she_answers_and_plays_on() {
    use super::sprite::{Face, Facing, Pose};
    use script::{ANDAGI_FOUND_MS, SATA_ANDAGI, SpliceId};
    // What she's seen of the chat: a question its newest line.
    let seen = ChatMark {
        synced: 5,
        newest: Some(5),
        synced_asks: true,
        irc: 2,
        irc_asks: false,
    };
    let asked = ChatMark {
        synced: 6,
        newest: Some(6),
        ..seen
    };
    let cases = [
        ("asked", asked, true, true),
        (
            "asked on IRC",
            ChatMark {
                irc: 3,
                irc_asks: true,
                ..seen
            },
            true,
            true,
        ),
        (
            "told",
            ChatMark {
                synced_asks: false,
                ..asked
            },
            true,
            false,
        ),
        ("compacted", ChatMark { synced: 3, ..seen }, true, false),
        ("asked in the snack", asked, false, false),
    ];
    for (name, screen) in home_screens() {
        let (real, view) = &screen;
        for graphics in [false, true] {
            for (what, mark, in_coda, answers) in cases {
                let case = format!("{name}, graphics={graphics}, {what}");
                let (mut guest, mut hidden, now) =
                    andagi_until(&screen, graphics, seen, |play, since, until, now| {
                        let spliced = play.after.is_some_and(|s| s.splice == SpliceId::Andagi);
                        let end = play.body_end(since, until);
                        spliced
                            && if in_coda {
                                now >= end + ANDAGI_FOUND_MS + 300
                            } else {
                                (play.body_start(since) + 300..end).contains(&now)
                            }
                    });
                let span = visit_of(&guest).osaka.use_span();
                let (_, _, until) = span.unwrap();
                let chatty = IdleView {
                    chat_mark: mark,
                    ..view.clone()
                };
                let mut now = now + 1;
                guest.advance(now);
                paint(&mut guest, real, &chatty, now);
                let osaka = &visit_of(&guest).osaka;
                if !answers {
                    assert_ne!(osaka.use_span(), span, "{case}: she stopped to look");
                    continue;
                }
                assert_eq!(osaka.use_span(), span, "{case}: playing on");
                let (_, face, bubble) = osaka.appearance(now);
                assert_eq!(
                    (face, bubble),
                    (Face::Happy, Some(osaka::Bubble::Say(SATA_ANDAGI))),
                    "{case}"
                );
                let chat_x = i32::from(view.chat.x) + i32::from(view.chat.width) / 2;
                let facing = if chat_x < osaka.x {
                    Facing::Left
                } else {
                    Facing::Right
                };
                assert_eq!(osaka.facing, facing, "{case}: turned to the chat");
                // On to the end: the same use throughout, eating it at
                // last, and never whole again once bitten.
                let mut bitten = false;
                while now + 1 < until {
                    now += guest
                        .next_tick(now)
                        .map_or(100, |d| d.as_millis() as u64)
                        .clamp(1, 100)
                        .min(until - 1 - now);
                    guest.advance(now);
                    let frame = paint(&mut guest, real, &chatty, now);
                    if graphics {
                        let layer: Vec<(u16, u16)> = visit_of(&guest).layer.cells().collect();
                        hidden
                            .check(&frame, real, &layer, now)
                            .unwrap_or_else(|e| panic!("{case} at {now}: {e}"));
                    }
                    let osaka = &visit_of(&guest).osaka;
                    assert_eq!(osaka.use_span(), span, "{case} at {now}");
                    assert_eq!(osaka.facing, facing, "{case} at {now}: still turned");
                    let pose = osaka.appearance(now).0;
                    assert!(
                        !(bitten && pose == Pose::EatAndagi(0)),
                        "{case} at {now}: whole again"
                    );
                    bitten |= pose == Pose::EatAndagi(1);
                }
                assert!(bitten, "{case}: never bit it");
            }
        }
    }
}

/// One of her images as it's keyed: her pose and face, what she shows
/// on her furniture, and the way she faces.
type Seen = (
    super::sprite::Pose,
    super::sprite::Face,
    Option<script::Prop>,
    super::sprite::Facing,
);

/// The images the keys of a vignette `len` long can show her in from
/// `from` to `to` ms into it, facing `facing`: each pose (a bob's two
/// frames) with its face (or `face`, if given) and what it shows on her
/// furniture.
fn looks_of(
    keys: &[script::Key],
    len: u64,
    (from, to): (u64, u64),
    facing: super::sprite::Facing,
    face: Option<super::sprite::Face>,
) -> std::collections::HashSet<Seen> {
    let mut start = 0;
    let mut out = std::collections::HashSet::new();
    for key in keys {
        let end = key.span.end(Some(len));
        if start < to && from < end {
            let poses = match key.pose {
                script::Posed::Still(pose) => vec![pose],
                script::Posed::Bob(pose, _) => vec![pose(0), pose(1)],
                script::Posed::Host => panic!("a vignette has no host's pose"),
            };
            for pose in poses {
                out.insert((pose, face.unwrap_or(key.face), key.prop, facing));
            }
        }
        start = end;
    }
    out
}

/// Each vignette, cued in the two-pane home in line art, costs no more
/// images than its script has looks: those encoded from its first frame
/// to its last are (exactly) the looks its keys can show her in (each pose and
/// face, with what it shows on her furniture, the way she faces), less
/// those she'd shown before it. Asked something as the andagi plays,
/// she turns to answer: the keys up to then in the way she faced, the
/// rest turned, and those she says it over beaming.
#[test]
fn a_vignette_stays_within_its_image_budget() {
    use super::sprite::Face;
    use script::{ANDAGI_FOUND_MS, Cue, SATA_ANDAGI};
    for (scene, pieces, ask) in [
        (Scene::ChopsticksClean, &[Furniture::Desk][..], false),
        (Scene::ChopsticksBad, &[Furniture::Desk], false),
        (Scene::Andagi, &[], false),
        (Scene::Andagi, &[], true),
    ] {
        let Some(Cue::Splice(id, _)) = scene.cue() else {
            panic!("{scene:?}");
        };
        let case = format!("{scene:?}, asked {ask}");
        let (real, view) = home_screen();
        let chatty = IdleView {
            chat_mark: ChatMark {
                synced: 1,
                newest: Some(1),
                synced_asks: true,
                ..ChatMark::default()
            },
            ..view.clone()
        };
        let mut guest = Guest::new(8);
        guest.set_picker(kitty());
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        for &item in pieces {
            guest.give(item);
            paint(&mut guest, &real, &view, 0);
        }
        guest.cue(scene);
        let encoded = |guest: &Guest| guest.graphics.as_ref().unwrap().counts().encoded;
        // What she's shown before it; encoded before its first frame and
        // by its last; its branch, length, start and facing; and when
        // (into it) she turned to answer, and which way.
        let mut shown = std::collections::HashSet::new();
        let (mut from, mut to, mut over) = (None, None, false);
        let mut played = None;
        let mut answered = None;
        let mut now = 0;
        let mut on = &view;
        paint(&mut guest, &real, on, now);
        while now < 40_000 && !over {
            let before = encoded(&guest);
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            paint(&mut guest, &real, on, now);
            let osaka = &visit_of(&guest).osaka;
            let playing =
                osaka
                    .plays()
                    .zip(osaka.use_span())
                    .and_then(|(play, (_, since, until))| {
                        let s = play.spliced_at(since, until, now)?;
                        let start = if play.before == Some(s) {
                            since
                        } else {
                            play.body_end(since, until)
                        };
                        Some((s, start))
                    });
            match playing.filter(|(s, _)| s.splice == id) {
                Some((s, start)) => {
                    from.get_or_insert(before);
                    to = Some(encoded(&guest));
                    played.get_or_insert((s.branch, s.len, start, osaka.facing));
                    if ask && answered.is_none() && now >= start + ANDAGI_FOUND_MS + 300 {
                        on = &chatty;
                        paint(&mut guest, &real, on, now);
                        let osaka = &visit_of(&guest).osaka;
                        assert_eq!(osaka.appearance(now).1, Face::Happy, "{case}: answered");
                        answered = Some((now - start, osaka.facing));
                        to = Some(encoded(&guest));
                    }
                }
                None if from.is_none() => {
                    let (pose, face, _) = osaka.appearance(now);
                    shown.insert((pose, face, osaka.prop(now), osaka.facing));
                }
                None => over = true,
            }
        }
        let (Some(from), Some(to), Some((branch, len, _, facing))) = (from, to, played) else {
            panic!("{case}: never played");
        };
        let keys = id.script().keys(branch);
        let looks = match answered {
            None => looks_of(keys, len, (0, len), facing, None),
            Some((at, turned)) => {
                let mut looks = looks_of(keys, len, (0, at + 1), facing, None);
                looks.extend(looks_of(keys, len, (at, len), turned, None));
                let said = at + osaka::speech_ms(SATA_ANDAGI);
                looks.extend(looks_of(keys, len, (at, said), turned, Some(Face::Happy)));
                looks
            }
        };
        assert_eq!(ask, answered.is_some(), "{case}");
        let budget = looks.difference(&shown).count();
        eprintln!("{case}: {} encoded, budget {budget}", to - from);
        assert!(
            to - from <= budget,
            "{case}: {} images encoded over {budget} looks: {looks:?}",
            to - from,
        );
        // And the budget is the looks it shows, none to spare (so one
        // look more than its script has would be caught).
        assert_eq!(
            to - from,
            budget,
            "{case}: a look it never showed: {looks:?}"
        );
    }
}

/// Through a plain watch, painted only when she says something changed
/// (as the shell paints), the TV's screen keeps changing: what's on
/// moves at paint time, so her wakeups are what carry it on, and they
/// never leave it frozen. Its 400 ms frames sampled on the 1400 ms frame
/// grid change every second frame, so the screen changes at least every
/// two frames from the watch's start to its end. In both drawing modes,
/// among text.
#[test]
fn the_tv_screen_alternates_through_a_plain_watch() {
    use super::room::Use;
    use art::Channel;
    use script::{Prop, ScriptId};
    /// The longest the screen may hold: two frames, and a step's slack.
    const HOLD_MS: u64 = 2 * osaka::USE_FRAME_MS + 100;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = busy_home_screen();
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::Watch);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        assert!(
            matches!(guest.cue_note(), Some(Ok(_))),
            "{at}: {:?}",
            guest.cue_note()
        );
        let mut channels = Vec::new();
        // Each screen shown, and when it came on.
        let mut screens: Vec<(u64, Vec<String>)> = Vec::new();
        let mut watched = 0;
        let mut ended = None;
        while now < 90_000 {
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            if !guest.advance(now) {
                continue;
            }
            let frame = paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let Some((.., play)) = visit
                .osaka
                .playing()
                .filter(|(seat, ..)| seat.what == Use::Watch)
            else {
                if watched > 0 {
                    ended = Some(now);
                    break;
                }
                continue;
            };
            assert_eq!(
                (play.own, play.bought),
                (ScriptId::Watch, None),
                "{at}: a plain watch"
            );
            watched += 1;
            let prop = visit.osaka.prop(now);
            assert!(
                matches!(prop, Some(Prop::Tv(Channel::Snow(_)))),
                "{at}: {prop:?}"
            );
            if !channels.contains(&prop) {
                channels.push(prop);
            }
            let tv = visit
                .shown
                .iter()
                .find(|s| s.item == Furniture::Tv)
                .expect("the TV is shown");
            // ASCII: the screen's two cells. Line art: the image placed
            // over the TV (an image of another look is another image),
            // without the image data sent with its first placement (the
            // same image sent, then only placed, is one screen).
            let cover = tv.cover();
            let screen: Vec<String> = if graphics {
                cover
                    .positions()
                    .filter_map(|p| frame.cell(p))
                    .filter_map(|cell| {
                        let symbol = cell.symbol();
                        symbol.find("\x1b[s").map(|i| symbol[i..].to_owned())
                    })
                    .collect()
            } else {
                tv.screen()
                    .expect("the TV has a screen")
                    .iter()
                    .filter_map(|&(x, y)| frame.cell((x as u16, y as u16)))
                    .map(|cell| cell.symbol().to_owned())
                    .collect()
            };
            assert!(!screen.is_empty(), "{at}: the TV isn't drawn");
            assert!(
                screen.iter().all(|s| !s.contains("\x1b_G")),
                "{at}: image data left in {screen:?}"
            );
            if screens.last().is_none_or(|(_, last)| *last != screen) {
                screens.push((now, screen));
            }
        }
        assert!(watched > 5, "{at}: watched for {watched} frames");
        assert_eq!(channels.len(), 2, "{at}: {channels:?}");
        let ended = ended.expect("the watch ended");
        let distinct: std::collections::HashSet<_> = screens.iter().map(|(_, s)| s).collect();
        assert!(distinct.len() >= 2, "{at}: the screen froze: {screens:?}");
        // Never held longer than two frames, to the watch's end.
        let changes: Vec<u64> = screens.iter().map(|&(t, _)| t).chain([ended]).collect();
        for pair in changes.windows(2) {
            assert!(
                pair[1] - pair[0] <= HOLD_MS,
                "{at}: the screen held from {} to {}: {changes:?}",
                pair[0],
                pair[1]
            );
        }
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
/// and she steps out of her door somewhere else.
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
            let _ = guest.ledger.home.add(room::Prop::new(
                Furniture::ALL[item],
                quiet[at],
                along,
                if left { sprite::Facing::Left } else { sprite::Facing::Right },
            ));
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
                // What differed from the real frame, and what she painted
                // (a glyph she moved can land on its own twin, so a cell of
                // hers needn't differ).
                let painted: Vec<(u16, u16)> = match &guest.state {
                    State::Visiting(visit) => visit.painted.iter().map(|f| (f.x, f.y)).collect(),
                    _ => Vec::new(),
                };
                hers_there = focus.map_or_else(Vec::new, |rect| {
                    last.content
                        .iter()
                        .zip(&real.content)
                        .enumerate()
                        .filter(|(_, (a, b))| a != b)
                        .map(|(i, _)| ((i % usize::from(w)) as u16, (i / usize::from(w)) as u16))
                        .chain(painted)
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
/// the chat pane draws it), and protected like the real one. The log's
/// rows above it are full of text, as a scrolled-back log's are.
fn accordion_room(width: u16, height: u16, unseen: usize) -> (Buffer, Rect) {
    let mut buf = rooms(width, height);
    let accordion = Rect::new(1, height - 4, width / 2 - 2, 1);
    for y in accordion.y.saturating_sub(6)..accordion.y {
        let text = "so what did everyone think of that ending ".repeat(4);
        buf.set_stringn(1, y, &text, usize::from(accordion.width), Style::new());
    }
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
    /// When the accordion first shook.
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
            State::Arriving(_) | State::Away(_) => {}
        }
        // Shaking, and painted so (under her feet it's in her image).
        let moved = (accordion.left()..accordion.right())
            .any(|x| frame[(x, accordion.y)] != real[(x, accordion.y)]);
        if guest.nudge.shaking(now) && moved {
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
    for (resident, graphics) in [(false, false), (true, false), (false, true), (true, true)] {
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
        if graphics {
            guest.set_picker(kitty());
        }
        let seen = watch_errand(&mut guest, &real, &view, accordion, 90_000);
        // A resident was here all along; a visitor comes for it.
        let arrived = seen.arrived.expect("she came");
        let expected = if resident {
            DELAY.as_millis() as u64..nudge::NUDGE_MS
        } else {
            nudge::NUDGE_MS..nudge::NUDGE_MS + 100
        };
        assert!(
            expected.contains(&arrived),
            "resident={resident} graphics={graphics}: {seen:?}"
        );
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
            // Her image may stand on it (the cells under it are hers).
            let under_her = (accordion.left()..accordion.right())
                .any(|x| cells::untouchable(&frame[(x, accordion.y)]));
            let (got, want) = (row(&frame), row(&real));
            let slid = |by: usize| {
                let mut r = want.clone();
                r.rotate_right(by);
                r
            };
            prop_assert!(
                under_her || got == want || got == slid(1) || got == slid(want.len() - 1),
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

/// In line art she passes in front of text (it derezzes into alien
/// glyphs) but doesn't stay over it: placed standing in a block of
/// text, she moves off to a calm spot, and no text stays hidden behind
/// her for longer than walking past it takes.
#[test]
fn in_line_art_she_passes_text_but_does_not_stay_over_it() {
    for seed in 0..6 {
        let mut real = rooms(100, 30);
        // Text over the left half of the tall box's floor (row 26).
        for y in 20..26 {
            real.set_string(1, y, "lorem ipsum dolor sit amet", Style::new());
        }
        let view = view(bottom_strip(100, 30));
        let mut guest = Guest::new(seed);
        guest.set_picker(kitty());
        visiting_at(&mut guest, &real, &view, (12, 26));
        let mut hidden = Hidden::default();
        let (mut now, mut passed) = (0, false);
        while now < 90_000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 250);
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("seed {seed}: still visiting");
            };
            let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
            let mut strict = Hidden(hidden.0.clone());
            // Walking past a cell takes her box's width in steps.
            strict
                .check(&frame, &real, &layer, now)
                .and_then(|()| {
                    let longest = strict.0.values().map(|&since| now - since).max();
                    prop_assert!(
                        longest.is_none_or(|ms| ms <= 4_000),
                        "text hidden {:?} ms",
                        longest
                    );
                    Ok(())
                })
                .unwrap_or_else(|e| panic!("seed {seed} at {now}: {e}"));
            hidden = strict;
            passed |= !hidden.0.is_empty();
        }
        assert!(passed, "seed {seed}: the text was never passed");
    }
}

/// Run `guest` in the stage room until she's using the makeshift piece
/// `scene` makes, returning the time and frame; the whole scene, from
/// the walk to the line to sitting or lying on what she made of it.
fn make(guest: &mut Guest, scene: Scene, real: &Buffer, view: &IdleView) -> (u64, Buffer) {
    use super::sprite::Pose;
    // She reels the torn text in: it moves towards her hands.
    let mut reeled = false;
    guest.cue(scene);
    paint(guest, real, view, 0);
    assert!(
        matches!(guest.cue_note(), Some(Ok(_))),
        "{:?}",
        guest.cue_note()
    );
    let mut now = 0;
    loop {
        assert!(now < 30_000, "{scene:?}: never used what she made");
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        let frame = paint(guest, real, view, now);
        let State::Visiting(visit) = &guest.state else {
            panic!("{scene:?}: still visiting");
        };
        if let Some(build) = visit.osaka.reeling() {
            let hand = i32::from(build.hand());
            reeled |= visit.layer.entries().iter().any(|d| {
                let (from, to) = (i32::from(d.source.0), i32::from(d.at.0));
                (to - hand).abs() < (from - hand).abs()
            });
        }
        // She crumples it standing over it.
        if let Some(seat) = visit.osaka.seat()
            && seat.what == super::room::Use::Crumple
        {
            let made = visit
                .made
                .iter()
                .find(|m| m.piece.seat(seat.what, 0) == seat);
            let rect = made.expect("crumpling what she made").piece.rect();
            assert!(
                rect.contains((visit.osaka.x as u16, rect.y).into()),
                "{scene:?}: over it"
            );
        }
        let (pose, ..) = visit.osaka.appearance(now);
        let using = matches!(pose, Pose::Lounge | Pose::Nap(_) | Pose::Sleep(_))
            && visit.osaka.seat().is_some_and(|seat| seat.makeshift());
        if using {
            assert!(reeled, "{scene:?}: the torn text came to her hands");
            return (now, frame);
        }
    }
}

/// Without a sofa or a bed she makes one: she tears text off a line,
/// leaving its holes, crumples it into the piece and uses it — in both
/// drawing modes — and the goodbye rain mends the line exactly.
#[test]
fn she_makes_furniture_of_text() {
    for graphics in [false, true] {
        for scene in [Scene::MakeSofa, Scene::MakeBed] {
            let at = format!("{scene:?} graphics={graphics}");
            let mut ui = stage_ui();
            let (real, view) = real_frame(&mut ui, 100, 30);
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            let (now, frame) = make(&mut guest, scene, &real, &view);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let [made] = visit.made.as_slice() else {
                panic!("{at}: one piece, not {:?}", visit.made);
            };
            assert!(made.piece.scrap.is_some_and(|s| s.done()), "{at}");
            assert!(made.torn.len() >= scrap::MIN_GLYPHS, "{at}");
            for &(x, y) in &made.torn {
                assert!(!real[(x, y)].symbol().trim().is_empty(), "{at}: tore text");
                assert_eq!(frame[(x, y)].symbol(), " ", "{at}: its hole shows");
            }
            guest.activity(now);
            let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
            assert_eq!(end, real, "{at}: the rain mends the line");
        }
    }
}

/// Minor text changes go away when their pane is selected: focusing the
/// pane she tore the text from puts it back at once, and the piece goes.
#[test]
fn focusing_the_pane_puts_torn_text_back() {
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        let (now, _) = make(&mut guest, Scene::MakeBed, &real, &view);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        let torn = visit.made[0].torn.clone();
        let pane = view
            .nooks
            .iter()
            .map(|&(_, rect)| rect)
            .chain([view.chat])
            .find(|rect| rect.contains(torn[0].into()))
            .expect("a pane holds the torn text");
        let focused = IdleView {
            resident: true,
            busy: Some(Busy::Playing),
            focus: Some(pane),
            ..view.clone()
        };
        let frame = run(
            &mut guest,
            &real,
            &focused,
            now,
            now + dissolve::DURATION_MS,
        );
        let State::Visiting(visit) = &guest.state else {
            panic!("a resident stays");
        };
        assert!(visit.made.is_empty(), "graphics={graphics}: the piece went");
        for &(x, y) in &torn {
            assert_eq!(frame[(x, y)], real[(x, y)], "graphics={graphics}: mended");
        }
    }
}

/// She'd rather have the real thing: with a real sofa on offer as well
/// as makeshift ones, only about one choice in twenty is makeshift; with
/// no real one, the makeshift is always among her options; and she
/// doesn't make a second sofa while the first stands.
#[test]
fn a_real_piece_wins_nineteen_times_in_twenty() {
    use super::mind::{Place, Whims, places};
    use super::osaka::Chances;
    use super::room::{MadeId, PieceRef, Seat, Use};
    use super::scenes::{Build, Side};
    let seat = |x: i32, makeshift: bool| Seat {
        what: Use::Lounge,
        item: Furniture::Sofa,
        piece: if makeshift {
            PieceRef::Made(MadeId(0))
        } else {
            PieceRef::Real(Furniture::Sofa)
        },
        x,
        y: 20,
        facing: sprite::Facing::Right,
    };
    let piece = Shown {
        item: Furniture::Sofa,
        facing: sprite::Facing::Right,
        boxed: false,
        strip: None,
        left: 40,
        floor: 20,
        scrap: Some(scrap::Scrap::new(MadeId(1), &[], 0)),
    };
    let build = Build {
        x: 30,
        y: 20,
        row: 18,
        side: Side::Left,
        cells: vec![20, 21, 22, 23, 24],
        glyphs: "hello".to_owned(),
        piece,
        then: Use::Lounge,
    };
    let mut rng = super::Rng(4);
    let tally = |chances: &Chances, rng: &mut super::Rng| {
        let mut makeshift = 0;
        for _ in 0..4000 {
            let places = places(Use::Lounge, chances, Whims(rng.next()));
            let pick = places[rng.below(places.len() as u64) as usize];
            makeshift += usize::from(!matches!(pick, Place::Seat(s) if !s.makeshift()));
        }
        makeshift
    };
    let real_and_build = Chances {
        seats: vec![seat(10, false)],
        builds: vec![build.clone()],
        ..Chances::default()
    };
    let n = tally(&real_and_build, &mut rng);
    assert!((100..=320).contains(&n), "{n} of 4000 set about making one");
    let real_and_made = Chances {
        seats: vec![seat(10, false), seat(43, true)],
        ..Chances::default()
    };
    let n = tally(&real_and_made, &mut rng);
    assert!(
        (100..=320).contains(&n),
        "{n} of 4000 used the makeshift one"
    );
    let only_build = Chances {
        builds: vec![build.clone()],
        ..Chances::default()
    };
    assert_eq!(
        places(Use::Lounge, &only_build, Whims(rng.next())),
        vec![Place::Make(Furniture::Sofa)]
    );
    let made_and_build = Chances {
        seats: vec![seat(43, true)],
        builds: vec![build],
        ..Chances::default()
    };
    assert_eq!(
        places(Use::Lounge, &made_and_build, Whims(rng.next())),
        vec![Place::Seat(seat(43, true))]
    );
}

/// Wide glyphs tear off whole: with only CJK and mixed text in reach,
/// she still makes her pieces, never leaves half a wide glyph on screen,
/// and the goodbye mends it all exactly — in both drawing modes.
#[test]
fn she_tears_wide_glyphs_whole() {
    use super::stage::chat_ui;
    const LINES: [&str; 3] = [
        "葬送のフリーレン 12話",
        "猫が座った the cat sat",
        "おはよう osaka",
    ];
    for graphics in [false, true] {
        for scene in [Scene::MakeSofa, Scene::MakeBed] {
            let at = format!("{scene:?} graphics={graphics}");
            let mut ui = chat_ui((0..40).map(|i| LINES[i % LINES.len()].to_string()));
            let (real, view) = real_frame(&mut ui, 100, 30);
            let mut guest = Guest::new(2);
            if graphics {
                guest.set_picker(kitty());
            }
            let (now, frame) = make(&mut guest, scene, &real, &view);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let torn = &visit.made[0].torn;
            assert!(
                torn.iter().any(|&c| cells::width(&real[c]) > 1),
                "{at}: tore a wide glyph from {torn:?}"
            );
            assert_untouched(&frame, &real, &view.protected)
                .unwrap_or_else(|e| panic!("{at}: {e}"));
            guest.activity(now);
            let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
            assert_eq!(end, real, "{at}: the rain mends the line");
        }
    }
}

/// Interrupted while reeling torn text in (a chat message arrives), she
/// lets it go and it all goes back: no holes are left without a piece.
#[test]
fn an_interrupted_reel_puts_the_text_back() {
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::MakeSofa);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        // Until she has some of it in her hands.
        let build = loop {
            assert!(now < 20_000, "graphics={graphics}: never reeled");
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            if let Some(build) = visit.osaka.reeling()
                && visit.layer.holes().count() > 2
            {
                break build.clone();
            }
        };
        let mut chat = view.clone();
        chat.chat_mark.synced += 1;
        let frame = run(&mut guest, &real, &chat, now, now + 2_000);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        assert!(visit.made.is_empty(), "graphics={graphics}");
        assert!(
            visit.layer.is_empty(),
            "graphics={graphics}: {:?}",
            visit.layer.entries()
        );
        // The whole run it was moving along, as it was.
        let (lo, hi) = (build.cells.iter().min().unwrap(), build.hand());
        for x in *lo.min(&hi)..=*lo.max(&hi) {
            let at = (x, build.row);
            assert_eq!(frame[at], real[at], "graphics={graphics}: {at:?} back");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(24)))]

    /// Text that comes up near her after she's settled (a chat line, a
    /// pane redrawn) — under her, or under the image she shares with her
    /// furniture — is never hidden behind her for long: wherever she
    /// stays, she gets up and moves on.
    #[test]
    fn text_arriving_where_she_rests_is_never_hidden_for_long(
        seed in any::<u64>(),
        size in (60u16..130, 18u16..45),
        arrivals in proptest::collection::vec(
            (0u64..150_000, -9i32..10, -6i32..1, "[a-z]{1,8}"),
            1..12,
        ),
        owned in proptest::collection::vec((0usize..4, 0usize..3, 0u16..=1000, any::<bool>()), 0..5),
    ) {
        let (w, h) = size;
        let mut guest = Guest::new(seed);
        guest.set_picker(kitty());
        let nook = [Nook::List, Nook::Users, Nook::Playlist];
        for &(item, at, along, left) in &owned {
            let _ = guest.ledger.home.add(room::Prop::new(
                Furniture::ALL[item],
                nook[at],
                along,
                if left { sprite::Facing::Left } else { sprite::Facing::Right },
            ));
        }
        let protected = bottom_strip(w, h);
        let view = IdleView {
            nooks: nooks(w, h),
            ..view(protected.clone())
        };
        let mut real = rooms(w, h);
        let mut hidden = Hidden::default();
        let mut now = 0;
        let mut due = arrivals.clone();
        due.sort_by_key(|a| a.0);
        let mut due = due.into_iter().peekable();
        while now < 180_000 {
            let step = guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            now += step;
            // Text comes up around her, where she is when it does, in
            // blank cells clear of the protected strip.
            while let Some((_, dx, dy, s)) = due.next_if(|a| a.0 <= now) {
                if let State::Visiting(visit) = &guest.state {
                    let (x, y) = (visit.osaka.x + dx, visit.osaka.y + dy);
                    for (i, c) in s.chars().enumerate() {
                        let at = (x + i as i32, y);
                        let (Ok(ux), Ok(uy)) = (u16::try_from(at.0), u16::try_from(at.1)) else {
                            continue;
                        };
                        let free = real.cell((ux, uy)).is_some_and(|cell| cell.symbol() == " ")
                            && !protected.iter().any(|r| r.contains((ux, uy).into()));
                        if free {
                            real[(ux, uy)].set_char(c);
                        }
                    }
                }
            }
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let layer: Vec<(u16, u16)> = match &guest.state {
                State::Visiting(visit) => visit.layer.cells().collect(),
                _ => Vec::new(),
            };
            hidden.check(&frame, &real, &layer, now)?;
        }
    }
}

/// What she does to her home is hers the moment she does it: the visit
/// ending before the next paint (a key pressed right after the shopping
/// channel sold her something) doesn't lose the order.
#[test]
fn commits_survive_the_visit_ending() {
    let (real, view) = home_screen();
    let mut guest = Guest::new(3);
    guest.cue(Scene::Shopping);
    paint(&mut guest, &real, &view, 0);
    assert!(
        matches!(guest.cue_note(), Some(Ok(_))),
        "{:?}",
        guest.cue_note()
    );
    let mut now = 0;
    loop {
        assert!(now < 30_000, "never watched the shopping channel");
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        if visit
            .osaka
            .playing()
            .is_some_and(|(.., play)| play.bought.is_some())
        {
            break;
        }
        paint(&mut guest, &real, &view, now);
    }
    // Before any paint sees the purchase, someone's at the keys.
    guest.activity(now);
    let _ = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
    assert!(guest.ledger.ordered.is_some(), "the order was lost");
}

/// A sofa she made is a sofa: sleepy, she naps on it more often than
/// she lies down on the floor beside it.
#[test]
fn sleepy_she_naps_on_the_sofa_she_made() {
    use super::brain::{Need, Want};
    use super::osaka::Activity;
    use super::room::Use;
    let (mut nap, mut lie) = (0, 0);
    for seed in 0..4 {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(seed);
        let (mut now, _) = make(&mut guest, Scene::MakeSofa, &real, &view);
        let mut seen = 0;
        let end = now + 20 * 60_000;
        while now < end {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("seed {seed}: visiting");
            };
            // Kept sleepy: whatever she chose, she's sleepy again.
            if visit.osaka.choices.len() > seen {
                for kind in &visit.osaka.choices[seen..] {
                    nap += usize::from(*kind == Want::Use(Use::Nap));
                    lie += usize::from(*kind == Want::Idle(Activity::LieBack));
                }
                seen = visit.osaka.choices.len();
                visit.osaka.press(Need::Sleepy);
            }
        }
    }
    assert!(
        nap > lie,
        "napped on her sofa {nap} times, on the floor {lie}"
    );
}

/// What she means to make a piece *for* survives her being interrupted
/// while crumpling it: a chat line arrives mid-crumple (she looks up;
/// the heap stays unfinished), then she makes a bed. Whenever she does
/// finish the sofa, she sits on it within a few seconds.
#[test]
fn an_interrupted_crumple_keeps_its_purpose() {
    use super::room::Use;
    fn visit(guest: &Guest) -> &Visit {
        match &guest.state {
            State::Visiting(visit) => visit,
            _ => panic!("visiting"),
        }
    }
    for graphics in [false, true] {
        for seed in 0..4u64 {
            let at = format!("graphics={graphics} seed={seed}");
            let mut ui = stage_ui();
            let (real, mut view) = real_frame(&mut ui, 100, 30);
            let mut guest = Guest::new(seed);
            if graphics {
                guest.set_picker(kitty());
            }
            let mut now = 0;
            let step = |guest: &mut Guest, view: &IdleView, now: &mut u64| {
                *now += guest
                    .next_tick(*now)
                    .map_or(100, |d| d.as_millis() as u64)
                    .clamp(1, 100);
                guest.advance(*now);
                paint(guest, &real, view, *now);
            };
            guest.cue(Scene::MakeSofa);
            paint(&mut guest, &real, &view, now);
            while !visit(&guest)
                .osaka
                .seat()
                .is_some_and(|s| s.what == Use::Crumple && s.item == Furniture::Sofa)
            {
                assert!(now < 60_000, "{at}: never crumpled the sofa");
                step(&mut guest, &view, &mut now);
            }
            let until = now + 2_000;
            while now < until {
                step(&mut guest, &view, &mut now);
            }
            view.chat_mark.synced += 1;
            step(&mut guest, &view, &mut now);
            guest.cue(Scene::MakeBed);
            paint(&mut guest, &real, &view, now);
            // Live on until the sofa is finished, then give her 5 s.
            let sofa_done = |guest: &Guest| {
                visit(guest).made.iter().any(|m| {
                    m.piece.item == Furniture::Sofa && m.piece.scrap.is_some_and(|s| s.done())
                })
            };
            while !sofa_done(&guest) {
                assert!(now < 900_000, "{at}: never finished the sofa");
                step(&mut guest, &view, &mut now);
            }
            let finished = now;
            let mut sat = false;
            while now < finished + 5_000 && !sat {
                step(&mut guest, &view, &mut now);
                sat = visit(&guest).osaka.seat().is_some_and(|s| {
                    s.item == Furniture::Sofa && s.makeshift() && s.what != Use::Crumple
                });
            }
            assert!(
                sat,
                "{at}: finished the sofa at {finished} and didn't sit on it"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(8)))]

    /// Whatever interrupts her, a piece she made comes before anything
    /// new she'd choose: while one waits that she can get to and hasn't
    /// let be, she chooses nothing else; and while one waits, she keeps
    /// at it (another try, or progress) until it's used, gone (its text
    /// changed), or let be after a few tries. She makes a sofa, chat
    /// keeps arriving every 20-40 s, and she may be sent to make a bed.
    #[test]
    fn every_made_piece_is_used_or_let_go(
        seed in 0u64..1000,
        graphics in any::<bool>(),
        gaps in proptest::collection::vec(20_000u64..40_000, 12),
        bed_at in proptest::option::of(0u64..90_000),
    ) {
        use super::room::{MadeId, PieceRef, Use};
        // Between tries: a chat gap, the watch after it, and a walk.
        const BOUND: u64 = 90_000;
        let at = format!("seed {seed} graphics={graphics}");
        let mut ui = stage_ui();
        let (real, mut view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::MakeSofa);
        paint(&mut guest, &real, &view, 0);
        let mut chats = gaps.iter().scan(0, |t, gap| {
            *t += gap;
            Some(*t)
        }).peekable();
        let mut bed_at = bed_at;
        // Each piece she has made: where she is with it, and since when.
        let mut made: Vec<(MadeId, (bool, u8), u64)> = Vec::new();
        // Those she used, and those gone before she did.
        let (mut used, mut lost): (Vec<MadeId>, Vec<MadeId>) = (Vec::new(), Vec::new());
        let mut now = 0;
        while now < 180_000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if chats.next_if(|&t| t <= now).is_some() {
                view.chat_mark.synced += 1;
            }
            let chose = match &guest.state {
                State::Visiting(visit) => visit.osaka.choices.len(),
                _ => break,
            };
            guest.advance(now);
            let State::Visiting(visit) = &guest.state else {
                break;
            };
            // Chose something new: nothing she made was waiting that she
            // could get to (by what this frame offered her).
            if visit.osaka.choices.len() > chose {
                let chances = &visit.chances;
                for m in chances.mine.iter().filter(|m| !(m.done && m.used)) {
                    let next = if m.done { m.purpose } else { Use::Crumple };
                    let offered = chances
                        .seats
                        .iter()
                        .any(|s| s.piece == PieceRef::Made(m.id) && s.what == next);
                    prop_assert!(
                        !offered || visit.osaka.gave_up(m.id),
                        "{at}: at {now} chose {:?} over {m:?}",
                        visit.osaka.choices.last()
                    );
                }
            }
            if bed_at.is_some_and(|t| t <= now) {
                bed_at = None;
                guest.cue(Scene::MakeBed);
            }
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                break;
            };
            for m in visit.made.iter().filter_map(Made::mine) {
                let state = (m.done, visit.osaka.tries_at(m.id));
                match made.iter_mut().find(|(id, ..)| *id == m.id) {
                    Some((_, was, since)) if *was != state => (*was, *since) = (state, now),
                    Some(_) => {}
                    None => made.push((m.id, state, now)),
                }
            }
            // A piece gone before she used it is mourned (a beat owed).
            for &(id, ..) in &made {
                let here = visit.made.iter().filter_map(Made::mine).any(|m| m.id == id);
                if !here && !lost.contains(&id) && !used.contains(&id) {
                    lost.push(id);
                }
            }
            for m in visit.made.iter().filter_map(Made::mine).filter(|m| m.used) {
                if !used.contains(&m.id) {
                    used.push(m.id);
                }
            }
            let mourned = visit
                .osaka
                .beats
                .iter()
                .filter(|b| matches!(b.loss, super::mind::Loss::Piece(_)))
                .count();
            prop_assert!(
                mourned >= lost.len(),
                "{at}: lost {lost:?} unused, {mourned} mourned"
            );
            for &(id, state, since) in &made {
                let waiting = visit
                    .made
                    .iter()
                    .filter_map(Made::mine)
                    .any(|m| m.id == id && !m.used)
                    && !visit.osaka.gave_up(id);
                prop_assert!(
                    !waiting || now - since <= BOUND,
                    "{at}: {id:?} (done, tries) = {state:?} since {since}, still waiting at {now}"
                );
            }
        }
    }
}

/// Every decision is explained: which kind it was, by what, what she
/// set about, and — rolling — what she chose among (the choice one of
/// the best few). The stage shows the latest.
#[test]
fn her_decisions_explain_themselves() {
    use super::osaka::Bucket;
    let mut ui = stage_ui();
    let (real, mut view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(5);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    let mut now = 0;
    while now < 180_000 {
        now += guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        if now % 30_000 < 1000 {
            view.chat_mark.synced += 1;
        }
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
    }
    let State::Visiting(visit) = &guest.state else {
        panic!("still visiting");
    };
    let decisions = &visit.osaka.decisions;
    assert!(decisions.len() > 20, "{} decisions", decisions.len());
    for d in decisions {
        assert!(!d.act.is_empty(), "{d}");
        if let Some(want) = d.want {
            assert_eq!(d.bucket, Bucket::Normal, "{d}");
            assert!(d.top.iter().any(|&(w, _)| w == want), "{d}");
            assert!(d.top.len() <= 4, "{d}");
        }
    }
    for bucket in [Bucket::Reflex, Bucket::Normal] {
        assert!(
            decisions.iter().any(|d| d.bucket == bucket),
            "no {bucket:?}"
        );
    }
    let shown = guest.explain().expect("explained");
    assert_eq!(shown, decisions.last().unwrap().to_string());
}

/// A sofa she made, lost before she sat on it (a resize takes it), is
/// mourned: once she's free, she glances toward where it stood and says
/// "...my sofa.".
#[test]
fn a_lost_sofa_is_mourned() {
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::MakeSofa);
        paint(&mut guest, &real, &view, 0);
        // Run until the heap is there.
        let mut now = 0;
        loop {
            now += 100;
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            if !visit.made.is_empty() {
                break;
            }
            assert!(now < 60_000, "graphics={graphics}: never made it");
        }
        // The terminal grows: the heap is gone.
        let (real, view) = real_frame(&mut ui, 110, 30);
        let mut mourned = false;
        while now < 90_000 && !mourned {
            now += 100;
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            assert!(visit.made.is_empty(), "graphics={graphics}");
            mourned = visit.osaka.appearance(now).2 == Some(osaka::Bubble::Say("...my sofa."));
        }
        assert!(mourned, "graphics={graphics}: never mourned it");
    }
}

/// What she chose eases her needs by how much of it she did: a sleep
/// interrupted a few seconds in takes little off her sleepiness (she'll
/// go back to bed sooner), where choosing it used to take it all.
#[test]
fn an_interrupted_sleep_eases_only_what_she_slept() {
    use super::brain::Want;
    use super::room::Use;
    let (real, mut view) = home_screen();
    let mut checked = 0;
    for seed in 0..6u64 {
        let mut guest = Guest::new(seed);
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        guest.give(Furniture::Bed);
        paint(&mut guest, &real, &view, 0);
        let sleepy = |guest: &Guest| match &guest.state {
            State::Visiting(visit) => visit.osaka.needs().get(super::brain::Need::Sleepy),
            _ => panic!("visiting"),
        };
        let mut now = 0;
        // Sleepiness as she chose to sleep, and when she lay down.
        let mut chose: Option<f64> = None;
        let mut slept_since: Option<u64> = None;
        while now < 10 * 60_000 {
            if chose.is_none() {
                guest.press(super::stage::Want::Sleepy);
            }
            let before = sleepy(&guest);
            let decided = match &guest.state {
                State::Visiting(visit) => visit.osaka.decisions.len(),
                _ => 0,
            };
            now += guest
                .next_tick(now)
                .map_or(500, |d| d.as_millis() as u64)
                .clamp(1, 500);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            if chose.is_none()
                && visit.osaka.decisions[decided..]
                    .iter()
                    .any(|d| d.want == Some(Want::Use(Use::Sleep)))
            {
                chose = Some(before);
            }
            let sleeping = visit
                .osaka
                .use_span()
                .is_some_and(|(seat, ..)| seat.what == Use::Sleep);
            if chose.is_some() && sleeping {
                slept_since.get_or_insert(now);
            }
            // Five seconds in, a chat line wakes her.
            if let (Some(base), Some(since)) = (chose, slept_since)
                && now >= since + 5000
            {
                view.chat_mark.synced += 1;
                guest.advance(now + 1);
                paint(&mut guest, &real, &view, now + 1);
                let after = sleepy(&guest);
                assert!(
                    after > base - 0.15,
                    "seed {seed}: sleepy {base} at choosing, {after} after 5 s of sleep"
                );
                checked += 1;
                break;
            }
        }
    }
    assert!(checked >= 3, "only {checked} seeds chose to sleep");
}

/// Lint: every want with a spot of its own can be cued from the stage,
/// so it can be watched on demand. Standing and walking about are the
/// fillers, with no spot; travel is each of its ways.
#[test]
fn every_want_can_be_cued() {
    use super::brain::Want;
    use super::osaka::Activity;
    use super::room::Use;
    for want in Want::ALL {
        let scenes: &[Scene] = match want {
            Want::Stand | Want::Walk => &[],
            Want::SpaceOut => &[Scene::Muse],
            Want::Sneeze => &[Scene::Sneeze],
            Want::Idle(Activity::Sit) => &[Scene::Sit],
            Want::Idle(Activity::LieBack) => &[Scene::LieBack],
            Want::Idle(Activity::LieFront) => &[Scene::LieFront],
            Want::Idle(Activity::Jacks) => &[Scene::Jacks],
            Want::Idle(Activity::ToeTouch) => &[Scene::ToeTouch],
            Want::Idle(Activity::Stretch) => &[Scene::Stretch],
            Want::Idle(Activity::Gaze) => &[Scene::Gaze],
            Want::Travel => &[
                Scene::ClimbUp,
                Scene::ClimbDown,
                Scene::Drop,
                Scene::Clamber,
                Scene::StepOut,
                Scene::Door,
            ],
            Want::Pull => &[Scene::Pull],
            Want::Swap => &[Scene::Swap],
            Want::Work => &[Scene::Work],
            Want::Use(Use::Lounge) => &[Scene::Lounge],
            Want::Use(Use::Nap) => &[Scene::Nap],
            Want::Use(Use::Sleep) => &[Scene::Sleep],
            Want::Use(Use::Homework) => &[Scene::Homework],
            Want::Use(Use::Watch) => &[Scene::Watch, Scene::Shopping],
            Want::Use(Use::Unpack) => &[Scene::Parcel],
            Want::Use(Use::Read) => &[Scene::Read],
            Want::Use(Use::Snack) => &[Scene::Snack],
            Want::Use(Use::Pet) => &[Scene::Pet],
            Want::Use(Use::Crumple) => &[Scene::MakeSofa, Scene::MakeBed],
            Want::Arrange => &[Scene::Arrange],
        };
        assert!(
            matches!(want, Want::Stand | Want::Walk) || !scenes.is_empty(),
            "{want:?}"
        );
        for scene in scenes {
            assert!(Scene::ALL.contains(scene), "{want:?}: {scene:?}");
        }
    }
}

/// `scene` cued on a fresh guest in the stage room (graphics or not),
/// run until `done` says what she plays is what was wanted, for at
/// most 20 s: whether it did, and when it stopped.
fn cue_until(
    scene: Scene,
    graphics: bool,
    real: &Buffer,
    view: &IdleView,
    mut done: impl FnMut(Option<script::Play>) -> bool,
) -> (bool, u64) {
    let mut guest = Guest::new(1);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(scene);
    paint(&mut guest, real, view, 0);
    let mut now = 0;
    while now < 20_000 {
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        paint(&mut guest, real, view, now);
        if done(visit_of(&guest).osaka.plays()) {
            return (true, now);
        }
    }
    (false, now)
}

/// Lint: every script can be cued from the stage, and plays when it is
/// (in the stage room, in both drawing modes): its scene is on the
/// stage's menu, and cueing it has her playing that very script (not
/// another of the same piece's: a plain watch, not surfing) soon after.
/// Every script that shares its piece with another is forced by its
/// scene's cue, not left to her whims (which one guest's might happen
/// to match).
#[test]
fn every_script_can_be_cued() {
    use script::{Cue, ScriptId};
    for id in ScriptId::ALL {
        let shared = ScriptId::ALL
            .iter()
            .any(|&other| other != id && other.played_on() == id.played_on());
        if shared && id.played_on().is_some() {
            assert_eq!(id.scene().cue(), Some(Cue::Script(id)), "{id:?}");
        }
    }
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    for graphics in [false, true] {
        for id in ScriptId::ALL {
            let scene = id.scene();
            assert!(Scene::ALL.contains(&scene), "{id:?}: {scene:?}");
            // A splice's script is never a use's own: see
            // every_splice_can_be_cued.
            if id.host() == script::Host::Splice {
                assert!(
                    script::SpliceId::ALL
                        .iter()
                        .any(|s| s.script() == id && s.scenes().contains(&scene)),
                    "{id:?}: {scene:?}"
                );
                continue;
            }
            let at = format!("{id:?} ({scene:?}) graphics={graphics}");
            let (played, now) = cue_until(scene, graphics, &real, &view, |plays| {
                let played = plays.is_some_and(|p| p.own == id);
                // Another script of the piece she was cued to use: not
                // the one cued.
                if let Some(p) = plays
                    && !played
                    && scene.furniture().is_some()
                {
                    panic!("{at}: played {:?}", p.own);
                }
                played
            });
            assert!(played, "{at}: not played by {now}");
        }
    }
}

/// Lint: every splice row can be cued from the stage, and plays when it
/// is (in the stage room, in both drawing modes): each of its scenes is
/// on the stage's menu and cues it, every branch of it is some scene's
/// to force or left to her whims by one, and cueing it has her playing
/// that very splice (on the branch cued) round the next use soon after.
/// (The tests' own splices aren't rows, and have no scene.)
#[test]
fn every_splice_can_be_cued() {
    use script::{Cue, SpliceId};
    for id in [
        SpliceId::TestSnack,
        SpliceId::TestSleep,
        SpliceId::TestBedtime,
    ] {
        assert!(id.scenes().is_empty(), "{id:?}");
    }
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    for id in SpliceId::ALL {
        assert!(!id.scenes().is_empty(), "{id:?}: no scene");
        let branches = id.row().lens.len();
        let mut forced = vec![false; branches];
        for &scene in id.scenes() {
            assert!(Scene::ALL.contains(&scene), "{id:?}: {scene:?}");
            let Some(Cue::Splice(cued, branch)) = scene.cue() else {
                panic!("{id:?}: {scene:?} doesn't cue it");
            };
            assert_eq!(cued, id, "{scene:?}");
            match branch {
                Some(b) => forced[usize::from(b)] = true,
                None => forced.fill(true),
            }
            for graphics in [false, true] {
                let (played, now) = cue_until(scene, graphics, &real, &view, |plays| {
                    plays.is_some_and(|p| {
                        [p.before, p.after]
                            .into_iter()
                            .flatten()
                            .any(|s| s.splice == id && branch.is_none_or(|b| s.branch == b))
                    })
                });
                assert!(
                    played,
                    "{id:?} ({scene:?}) graphics={graphics}: not played by {now}"
                );
            }
        }
        assert!(forced.iter().all(|&f| f), "{id:?}: a branch no scene cues");
    }
}

/// What's on the TV looks different on each channel in each drawing
/// mode (in ASCII its two screen cells; in line art its image), and so
/// do the cat's, the lamp's and the fridge's states from plain in line
/// art, and the cat's from each other; in ASCII, the cat curled up,
/// biting, and not there (ASCII draws the cat only: the lamp and the
/// fridge look the same off or on, shut or open).
#[test]
fn each_prop_looks_distinct_in_each_mode() {
    use art::{Channel, PieceState};
    let channels = [
        vec![Channel::Snow(0), Channel::Snow(1)],
        vec![Channel::Shopping(0), Channel::Shopping(1)],
        vec![Channel::ColourBars],
        vec![Channel::Sunrise],
    ];
    let glyphs =
        |c: &[Channel]| -> Vec<[char; 2]> { c.iter().map(|&c| screen_glyphs(c)).collect() };
    let image = |c: Channel| {
        art::render_tv(c, sprite::Facing::Right, "#c9d1d9", 64, 48)
            .map(|i| i.into_raw())
            .unwrap()
    };
    for (i, a) in channels.iter().enumerate() {
        for b in &channels[i + 1..] {
            for ga in glyphs(a) {
                assert!(!glyphs(b).contains(&ga), "{a:?} and {b:?} in ASCII");
            }
            for &ca in a {
                for &cb in b {
                    assert_ne!(image(ca), image(cb), "{ca:?} and {cb:?} in line art");
                }
            }
        }
    }
    for (item, state) in [
        (Furniture::Lamp, PieceState::LampOff),
        (Furniture::Fridge, PieceState::FridgeOpen),
        (Furniture::CatBed, PieceState::Cat),
        (Furniture::CatBed, PieceState::CatBiting),
    ] {
        let render = |state| {
            art::render_piece(item, state, sprite::Facing::Right, "#c9d1d9", 64, 48)
                .map(|i| i.into_raw())
                .unwrap()
        };
        assert_ne!(
            render(state),
            render(PieceState::Plain),
            "{item:?} {state:?}"
        );
    }
    let cat = |state| {
        art::render_piece(
            Furniture::CatBed,
            state,
            sprite::Facing::Right,
            "#c9d1d9",
            64,
            48,
        )
        .map(|i| i.into_raw())
        .unwrap()
    };
    assert_ne!(
        cat(PieceState::Cat),
        cat(PieceState::CatBiting),
        "in line art"
    );
    // In ASCII, the cat curled up, biting, and not there.
    let ascii = [
        cat_glyphs(PieceState::Plain),
        cat_glyphs(PieceState::Cat),
        cat_glyphs(PieceState::CatBiting),
    ];
    for (i, a) in ascii.iter().enumerate() {
        for b in &ascii[i + 1..] {
            assert_ne!(a, b, "the cat in ASCII");
        }
    }
}

/// The sofa census (the migration's checkpoint bench): in the stage
/// room, cued to make a sofa, five-minute visits with chat every 37 s
/// (in a phase that differs by seed) or none, what became of each piece she made — used (and how long after
/// she made it), let be after her tries, lost (gone unused), or still
/// waiting when the visit ended. Prints; run by hand in release.
#[test]
#[ignore]
fn sofa_census() {
    use super::room::MadeId;
    const SEEDS: u64 = 100;
    for chat in [None, Some(37_000u64)] {
        for graphics in [false, true] {
            let (mut used, mut let_be, mut lost, mut waiting) = (0, 0, 0, 0);
            let mut waits: Vec<u64> = Vec::new();
            for seed in 0..SEEDS {
                let mut ui = stage_ui();
                let (real, mut view) = real_frame(&mut ui, 100, 30);
                let mut guest = Guest::new(seed);
                if graphics {
                    guest.set_picker(kitty());
                }
                guest.cue(Scene::MakeSofa);
                paint(&mut guest, &real, &view, 0);
                // Each piece: when it was made, and what became of it.
                let mut made: Vec<(MadeId, u64, Option<u64>, bool)> = Vec::new();
                let mut now = 0;
                while now < 300_000 {
                    let step = guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000);
                    now += step;
                    // Each seed's chat in another phase, so the first
                    // line lands anywhere from the tear to the sitting.
                    let shifted = |t: u64| t + seed * 373;
                    if chat.is_some_and(|every| shifted(now) / every != shifted(now - step) / every)
                    {
                        view.chat_mark.synced += 1;
                    }
                    guest.advance(now);
                    paint(&mut guest, &real, &view, now);
                    let State::Visiting(visit) = &guest.state else {
                        break;
                    };
                    let mine: Vec<_> = visit.made.iter().filter_map(Made::mine).collect();
                    for m in &mine {
                        if !made.iter().any(|(id, ..)| *id == m.id) {
                            made.push((m.id, now, None, false));
                        }
                    }
                    for (id, _, used_at, gone) in &mut made {
                        match mine.iter().find(|m| m.id == *id) {
                            Some(m) if m.used => {
                                used_at.get_or_insert(now);
                            }
                            Some(_) => {}
                            None => *gone = true,
                        }
                    }
                }
                let osaka = match &guest.state {
                    State::Visiting(visit) => Some(&visit.osaka),
                    _ => None,
                };
                for &(id, at, used_at, gone) in &made {
                    match used_at {
                        Some(when) => {
                            used += 1;
                            waits.push(when - at);
                        }
                        None if gone => lost += 1,
                        None if osaka.is_some_and(|o| o.gave_up(id)) => let_be += 1,
                        None => waiting += 1,
                    }
                }
            }
            waits.sort_unstable();
            let at = |q: usize| waits.get(waits.len() * q / 100).map_or(0, |ms| ms / 1000);
            eprintln!(
                "chat every {chat:?} graphics={graphics}: {} made, {used} used (made to used: median {} s, p90 {} s, max {} s), {let_be} let be, {lost} lost, {waiting} waiting at the end",
                used + let_be + lost + waiting,
                at(50),
                at(90),
                waits.last().map_or(0, |ms| ms / 1000),
            );
        }
    }
}

/// A chat line while she's clambering over a divider, hanging on its
/// pole, doesn't drop her: she gets over, and looks once she has, as
/// when climbing. Over the stage room's tallest divider.
#[test]
fn chat_mid_clamber_doesnt_drop_her() {
    use super::sprite::Pose;
    use super::terrain::Link;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut ui = stage_ui();
        let (real, mut view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        let floors = visit.terrain.platforms.clone();
        let link: Link = *visit
            .terrain
            .links
            .iter()
            .filter(|l| matches!(l.route, Route::Clamber { .. }))
            .max_by_key(|l| (floors[l.from].y - floors[l.to].y).abs())
            .expect("a divider to clamber over");
        let (from, to) = (floors[link.from], floors[link.to]);
        assert!((from.y - to.y).abs() >= 4, "{at}: a tall one: {link:?}");
        let start = if link.x > from.x0 {
            link.x - 1
        } else {
            link.x + 1
        };
        visit.osaka.place(start, from.y, 0);
        visit.osaka.travel(link, 0);
        let her = |guest: &Guest, now: u64| match &guest.state {
            State::Visiting(visit) => (visit.osaka.appearance(now).0, visit.osaka.x, visit.osaka.y),
            _ => panic!("visiting"),
        };
        let mut now = 0;
        // Halfway along the pole.
        let halfway = |y: i32| 2 * (y - from.y).abs() >= (to.y - from.y).abs();
        while !matches!(her(&guest, now), (Pose::Climb { .. }, _, y) if halfway(y)) {
            assert!(now < 30_000, "{at}: never on the pole");
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
        }
        view.chat_mark.synced += 1;
        let until = now + 10_000;
        while now < until {
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let (pose, ..) = her(&guest, now);
            assert!(
                !matches!(pose, Pose::Fall | Pose::Dazed),
                "{at}: {pose:?} at {now}"
            );
        }
        let (_, x, y) = her(&guest, now);
        assert!(
            y == to.y && (to.x0..=to.x1).contains(&x),
            "{at}: over onto {to:?}, but at ({x}, {y})"
        );
    }
}

/// Her sofa is where she watches the TV from when it faces it: on the
/// same strip, a few cells away, turned toward it, even across a floor
/// text splits; against either wall; drawn in text or in line art.
#[test]
fn a_sofa_facing_the_tv_is_watched_from() {
    use super::room::{Anchor, PieceRef, Prop, Side, Use};
    use sprite::Facing;
    // One pane, alone on its floor: nothing beyond its walls.
    let screen = || {
        let (width, height) = (100u16, 20u16);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let playlist = Rect::new(20, 8, 50, 9);
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            playlist,
            &mut buf,
        );
        buf.set_string(0, height - 2, "Tab Next pane | Enter Send", Style::new());
        let view = IdleView {
            nooks: vec![(Nook::Playlist, playlist)],
            ..view(bottom_strip(width, height))
        };
        (buf, view)
    };
    // The TV against the left wall or the right, the sofa a gap of so
    // many cells from it on the same strip, turned toward the TV or away
    // from it: only within 2..=14, turned toward it, is it watched from
    // (the TV is seen from the front, so its own way round doesn't
    // count). Something protected between them splits the floor, but
    // not the strip.
    for graphics in [false, true] {
        // (`tv_back`: the TV turned toward the sofa, its back to its
        // wall; else turned to its wall.)
        for (tv_side, gap, toward, tv_back, split, watched) in [
            (Side::Left, 2, true, true, false, true),
            (Side::Left, 2, true, false, false, true),
            (Side::Left, 8, true, false, true, true),
            (Side::Left, 14, true, true, false, true),
            (Side::Right, 2, true, false, false, true),
            (Side::Right, 8, true, true, true, true),
            (Side::Right, 8, true, false, true, true),
            (Side::Right, 14, true, false, false, true),
            (Side::Left, 2, false, false, false, false),
            (Side::Left, 8, false, true, true, false),
            (Side::Left, 14, false, false, false, false),
            (Side::Right, 2, false, true, false, false),
            (Side::Right, 8, false, false, true, false),
            (Side::Right, 14, false, true, false, false),
            (Side::Left, 0, true, true, false, false),
            (Side::Left, 1, true, false, false, false),
            (Side::Right, 15, true, true, false, false),
            (Side::Left, 30, true, false, false, false),
            (Side::Right, 15, false, false, false, false),
            (Side::Left, 30, false, true, false, false),
        ] {
            let (real, mut view) = screen();
            let (_, pane) = view.nooks[0];
            let (from, floor) = (pane.x + 1, pane.bottom() - 1);
            // Toward the wall the TV stands against.
            let toward_tv = match tv_side {
                Side::Left => Facing::Left,
                Side::Right => Facing::Right,
            };
            let sofa = if toward {
                toward_tv
            } else {
                match toward_tv {
                    Facing::Left => Facing::Right,
                    Facing::Right => Facing::Left,
                }
            };
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            // The TV turned either way: it doesn't count.
            let tv = match (tv_back, toward_tv) {
                (false, way) => way,
                (true, Facing::Left) => Facing::Right,
                (true, Facing::Right) => Facing::Left,
            };
            for (item, offset, facing) in [(Furniture::Tv, 0, tv), (Furniture::Sofa, 6 + gap, sofa)]
            {
                assert!(guest.ledger.home.add(Prop {
                    anchor: Some(Anchor {
                        side: tv_side,
                        offset,
                    }),
                    ..Prop::new(item, Nook::Playlist, 0, facing)
                }));
            }
            if split {
                let mid = match tv_side {
                    Side::Left => from + 6 + gap / 2,
                    Side::Right => pane.right() - 1 - 6 - gap / 2,
                };
                view.protected
                    .push(Rect::new(mid, floor.saturating_sub(4), 1, 4));
            }
            guest.cue(Scene::Arrive);
            paint(&mut guest, &real, &view, 0);
            let _ = run(&mut guest, &real, &view, 0, 2000);
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            let at = format!(
                "graphics {graphics}, TV at the {tv_side:?} wall turned {tv:?}, gap {gap}, sofa turned {sofa:?}"
            );
            let shown: Vec<_> = visit.shown.iter().map(|s| (s.item, s.left)).collect();
            assert_eq!(shown.len(), 2, "{at}: both stand: {shown:?}");
            let watch: Vec<_> = visit
                .chances
                .seats
                .iter()
                .filter(|s| s.what == Use::Watch && s.piece == PieceRef::Real(Furniture::Sofa))
                .collect();
            assert_eq!(
                !watch.is_empty(),
                watched,
                "{at}: {shown:?}, seats {:?}",
                visit.chances.seats
            );
            // Watching, she faces the way the sofa does: at the TV.
            for seat in watch {
                assert_eq!(seat.facing, toward_tv, "{at}: {seat:?}");
            }
        }
    }
}

/// The rules her home breaks are judged each frame on where her pieces
/// are laid out, and offered to her: text over a piece (which closets
/// it) neither breaks nor mends one. The stage shows them.
#[test]
fn the_rules_she_breaks_are_judged_on_her_layout() {
    use super::room::{Anchor, Prop, Side};
    use sprite::Facing;
    for graphics in [false, true] {
        let (width, height) = (100u16, 20u16);
        let mut real = Buffer::empty(Rect::new(0, 0, width, height));
        let playlist = Rect::new(20, 8, 50, 9);
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            playlist,
            &mut real,
        );
        real.set_string(0, height - 2, "Tab Next pane | Enter Send", Style::new());
        let view = IdleView {
            nooks: vec![(Nook::Playlist, playlist)],
            ..view(bottom_strip(width, height))
        };
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        // The TV against the left wall; the sofa 6 cells along, turned
        // away from it.
        for (item, offset, facing) in [
            (Furniture::Tv, 0, Facing::Right),
            (Furniture::Sofa, 12, Facing::Right),
        ] {
            assert!(guest.ledger.home.add(Prop {
                anchor: Some(Anchor {
                    side: Side::Left,
                    offset,
                }),
                ..Prop::new(item, Nook::Playlist, 0, facing)
            }));
        }
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        let _ = run(&mut guest, &real, &view, 0, 1000);
        let judged = |guest: &Guest| {
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            assert_eq!(visit.chances.broken, visit.broken);
            (
                visit.broken.clone(),
                visit.shown.iter().map(|s| s.item).collect::<Vec<_>>(),
            )
        };
        let (broken, shown) = judged(&guest);
        assert_eq!(
            shown,
            [Furniture::Tv, Furniture::Sofa],
            "graphics {graphics}"
        );
        let faces: Vec<_> = broken.iter().map(|b| b.pieces.clone()).collect();
        assert_eq!(
            faces,
            [vec![Furniture::Sofa, Furniture::Tv]],
            "graphics {graphics}"
        );
        assert_eq!(guest.broken(), "faces(sofa,TV)");
        // Text over the sofa closets it; the rule is as broken as before.
        let mut noisy = real.clone();
        noisy.set_string(
            playlist.x + 1 + 12 + 2,
            playlist.bottom() - 2,
            "hi",
            Style::new(),
        );
        paint(&mut guest, &noisy, &view, 1000);
        let (closeted, shown) = judged(&guest);
        assert_eq!(shown, [Furniture::Tv], "graphics {graphics}");
        assert_eq!(closeted, broken, "graphics {graphics}");
    }
}

/// Where she could make a sofa that faces the TV too, she mostly makes
/// it there: five times as likely as a spot that doesn't.
#[test]
fn a_made_sofa_mostly_faces_the_tv() {
    use super::mind::{Whims, pick_build};
    use super::osaka::Chances;
    use super::room::{MadeId, Use};
    use super::scenes::{Build, Side};
    let build = |x: i32, then: Use| Build {
        x,
        y: 20,
        row: 18,
        side: Side::Left,
        cells: vec![20, 21, 22, 23, 24],
        glyphs: "hello".to_owned(),
        piece: Shown {
            item: Furniture::Sofa,
            facing: sprite::Facing::Right,
            boxed: false,
            strip: None,
            left: x - 3,
            floor: 20,
            scrap: Some(scrap::Scrap::new(MadeId(0), &[], 0)),
        },
        then,
    };
    let chances = Chances {
        builds: vec![
            build(30, Use::Lounge),
            build(30, Use::Watch),
            build(60, Use::Lounge),
        ],
        ..Chances::default()
    };
    let mut rng = super::Rng(7);
    let facing = (0..6000)
        .filter(|_| {
            pick_build(Use::Lounge, Furniture::Sofa, &chances, Whims(rng.next()))
                .is_some_and(|b| b.x == 30)
        })
        .count();
    assert!(
        (4600..=5400).contains(&facing),
        "{facing} of 6000 faced the TV"
    );
}

/// A home on a playlist pane with chat-like text all around it (and on
/// the pane's top rows), her `pieces` anchored on its floor as given:
/// `(item, wall, offset, facing, settled)`.
fn rule_home(
    pieces: &[(Furniture, super::room::Side, u16, sprite::Facing, bool)],
    graphics: bool,
    seed: u64,
) -> (Guest, Buffer, IdleView) {
    use super::room::{Anchor, Prop};
    let (width, height) = (100u16, 20u16);
    let mut real = Buffer::empty(Rect::new(0, 0, width, height));
    let playlist = Rect::new(20, 8, 50, 9);
    tuirealm::ratatui::widgets::Widget::render(
        tuirealm::ratatui::widgets::Block::bordered(),
        playlist,
        &mut real,
    );
    for (y, line) in [
        (1, "alice: did anyone see the new episode yet"),
        (3, "bob: not yet, downloading it now"),
        (5, "carol: no spoilers please!!"),
        (6, "dave: brb, dinner"),
    ] {
        real.set_string(2, y, line, Style::new());
    }
    for (y, line) in [(9, "01 Frieren ep 12"), (10, "02 Frieren ep 13")] {
        real.set_string(playlist.x + 1, y, line, Style::new());
    }
    real.set_string(0, height - 2, "Tab Next pane | Enter Send", Style::new());
    let view = IdleView {
        nooks: vec![(Nook::Playlist, playlist)],
        ..view(bottom_strip(width, height))
    };
    let mut guest = Guest::new(seed);
    if graphics {
        guest.set_picker(kitty());
    }
    for &(item, side, offset, facing, settled) in pieces {
        assert!(guest.ledger.home.add(Prop {
            anchor: Some(Anchor { side, offset }),
            settled,
            ..Prop::new(item, Nook::Playlist, 0, facing)
        }));
    }
    (guest, real, view)
}

/// What she says, and how she looks, right now.
fn look_now(guest: &Guest, now: u64) -> (sprite::Face, Option<osaka::Bubble>) {
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let (_, face, bubble) = visit.osaka.appearance(now);
    (face, bubble)
}

fn nesting(guest: &Guest) -> f64 {
    match &guest.state {
        State::Visiting(visit) => visit.osaka.needs().get(super::brain::Need::Nesting),
        _ => panic!("visiting"),
    }
}

fn felt(guest: &Guest) -> Vec<rules::Grievance> {
    match &guest.state {
        State::Visiting(visit) => visit.osaka.felt().to_vec(),
        _ => panic!("visiting"),
    }
}

/// Step until `done` (checked after each painted tick), at most to
/// `until`; the time it was done at.
fn run_until(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    from: u64,
    until: u64,
    mut done: impl FnMut(&Guest, u64) -> bool,
) -> Option<u64> {
    let mut now = from;
    while now < until {
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        paint(guest, real, view, now);
        if done(guest, now) {
            return Some(now);
        }
    }
    None
}

/// Her sofa turned away from the TV: lounging on it, she cranes round
/// and says so for two frames, and has felt it once that has shown.
/// Nesting stays put until then, rises while the felt rule stays broken,
/// and stops once the home is put right; what she felt is forgotten
/// with the visit, and felt afresh on the next.
#[test]
fn she_feels_a_sofa_turned_away_from_the_tv() {
    use super::room::{Side, Use};
    use sprite::Facing;
    let line = rules::RULES[0].grievance;
    let grumbles = |guest: &Guest, now: u64| {
        look_now(guest, now) == (sprite::Face::Curious, Some(osaka::Bubble::Say(line)))
    };
    for graphics in [false, true] {
        let (mut guest, real, view) = rule_home(
            &[
                (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
            ],
            graphics,
            1,
        );
        let mut now = 0;
        for visit in 0..2 {
            guest.cue(Scene::Lounge);
            paint(&mut guest, &real, &view, now);
            let note = guest.cue_note().cloned();
            assert!(matches!(note, Some(Ok(_))), "graphics {graphics}: {note:?}");
            assert!(felt(&guest).is_empty(), "visit {visit}: a fresh visit");
            assert_eq!(guest.broken(), "faces(sofa,TV)");
            // Not felt (and no nesting) until she's said it.
            let from = run_until(&mut guest, &real, &view, now, now + 20_000, |guest, now| {
                assert!(felt(guest).is_empty(), "{now}: felt before saying so");
                assert_eq!(nesting(guest), 0.0, "{now}");
                grumbles(guest, now)
            })
            .unwrap_or_else(|| panic!("graphics {graphics}, visit {visit}: never said it"));
            let lounging = |guest: &Guest| match &guest.state {
                State::Visiting(visit) => visit.osaka.use_span().map(|(seat, since, _)| {
                    assert_eq!((seat.what, seat.item), (Use::Lounge, Furniture::Sofa));
                    since
                }),
                _ => None,
            };
            let since = lounging(&guest).expect("lounging as she says it");
            assert_eq!((from - since) % osaka::USE_FRAME_MS, 0, "on a frame");
            let to = run_until(
                &mut guest,
                &real,
                &view,
                from,
                from + 10_000,
                |guest, now| {
                    let grumbling = grumbles(guest, now);
                    assert_eq!(felt(guest).is_empty(), grumbling, "{now}");
                    !grumbling
                },
            )
            .expect("stops saying it");
            assert_eq!(to - from, osaka::GRIEVANCE_MS, "two frames");
            assert_eq!(lounging(&guest), Some(since), "still lounging");
            let key = rules::Grievance {
                row: 0,
                piece: Furniture::Sofa,
            };
            assert_eq!(felt(&guest), [key]);
            assert_eq!(guest.broken(), "faces(sofa,TV)*");
            assert_eq!(nesting(&guest), 0.0);
            // Felt and still broken: nesting rises, as she chooses on.
            now = run_until(&mut guest, &real, &view, to, to + 120_000, |guest, _| {
                nesting(guest) > 0.0
            })
            .unwrap_or_else(|| panic!("graphics {graphics}: nesting never rose"));
            // It isn't said again this visit: back on the sofa, she
            // lounges without a word about it.
            guest.cue(Scene::Lounge);
            paint(&mut guest, &real, &view, now);
            let note = guest.cue_note().cloned();
            assert!(matches!(note, Some(Ok(_))), "graphics {graphics}: {note:?}");
            let back = run_until(&mut guest, &real, &view, now, now + 20_000, |guest, now| {
                assert!(!grumbles(guest, now), "{now}: said twice in a visit");
                lounging(guest).is_some_and(|s| s != since)
            })
            .unwrap_or_else(|| panic!("graphics {graphics}: never lounged again"));
            let State::Visiting(v) = &guest.state else {
                panic!("visiting");
            };
            assert_eq!(v.osaka.grievance(), None, "graphics {graphics}: felt twice");
            now = back + osaka::USE_FRAME_MS * 3;
            let again = run_until(&mut guest, &real, &view, back, now, |guest, now| {
                grumbles(guest, now)
            });
            assert_eq!(again, None, "said twice in a visit");
            assert_eq!(felt(&guest), [key]);
            if visit == 0 {
                guest.activity(now);
                let _ = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
                now += dissolve::DURATION_MS;
                assert!(!guest.present());
            }
        }
        // Turned toward the TV, the sofa breaks nothing: nesting stops.
        let sofa = guest
            .ledger
            .home
            .props
            .iter_mut()
            .find(|p| p.item == Furniture::Sofa)
            .unwrap();
        sofa.facing = Facing::Left;
        paint(&mut guest, &real, &view, now);
        assert_eq!(guest.broken(), "", "graphics {graphics}");
        let rose = nesting(&guest);
        assert!(rose > 0.0);
        let decided = |guest: &Guest| match &guest.state {
            State::Visiting(visit) => visit.osaka.decisions.len(),
            _ => panic!("visiting"),
        };
        let before = decided(&guest);
        let _ = run(&mut guest, &real, &view, now, now + 90_000);
        assert!(decided(&guest) > before + 2, "she chose on");
        assert_eq!(nesting(&guest), rose, "graphics {graphics}");
    }
}

/// Once she has felt her sofa turned away from the TV, she works out
/// how she'd put it right: turn it round where it stands. Not before
/// she's felt it, not when her mood leaves her nothing to do about her
/// home, and not once it's right.
#[test]
fn she_works_out_how_to_turn_the_sofa_round() {
    use super::brain::Mood;
    use super::room::{Anchor, Side, Strip};
    use sprite::Facing;
    for graphics in [false, true] {
        for mood in [Mood::Ordinary, Mood::Industrious, Mood::Lazy] {
            let (mut guest, real, view) = rule_home(
                &[
                    (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                    (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
                ],
                graphics,
                1,
            );
            guest.cue(Scene::Lounge);
            paint(&mut guest, &real, &view, 0);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("visiting");
            };
            visit.osaka.set_mood(mood);
            let repairs = |guest: &Guest| match &guest.state {
                State::Visiting(visit) => visit.chances.repairs.clone(),
                _ => panic!("visiting"),
            };
            let at = run_until(&mut guest, &real, &view, 0, 30_000, |guest, now| {
                let felt = !felt(guest).is_empty();
                assert!(
                    felt || repairs(guest).is_empty(),
                    "{now}: before she felt it"
                );
                felt
            })
            .unwrap_or_else(|| panic!("graphics {graphics}, {mood:?}: never felt it"));
            paint(&mut guest, &real, &view, at);
            let found = repairs(&guest);
            if mood == Mood::Lazy {
                assert_eq!(found, [], "graphics {graphics}: lazy");
                assert_eq!(guest.repair(), "");
                continue;
            }
            let best = found
                .first()
                .unwrap_or_else(|| panic!("graphics {graphics}, {mood:?}"));
            assert_eq!(
                (best.piece, best.cost, best.tier),
                (Furniture::Sofa, 1, 0),
                "{best:?}"
            );
            assert_eq!(
                best.to,
                rules::Placement {
                    strip: Strip::Bottom(Nook::Playlist),
                    anchor: Anchor {
                        side: Side::Left,
                        offset: 12
                    },
                    facing: Facing::Left,
                }
            );
            assert_eq!(
                guest.repair(),
                "faces(sofa): sofa to Playlist L12 < (tier 0, 1 cells)"
            );
            // Turned round, there's nothing to put right.
            let sofa = guest
                .ledger
                .home
                .props
                .iter_mut()
                .find(|p| p.item == Furniture::Sofa)
                .unwrap();
            sofa.facing = Facing::Left;
            paint(&mut guest, &real, &view, at + 1);
            assert_eq!(repairs(&guest), [], "graphics {graphics}");
        }
    }
}

/// How she'd put her home right is worked out again when anything it
/// depends on changes — her home, the panes, the text she has moved,
/// which rule she'd put right — and otherwise at most once a second (text comes and goes), never on
/// every frame.
#[test]
fn the_repair_is_worked_out_again_only_when_it_may_have_changed() {
    use super::brain::Mood;
    use super::room::{Anchor, Prop, Side};
    use sprite::Facing;
    for graphics in [false, true] {
        let (mut guest, real, view) = rule_home(
            &[
                (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
                (Furniture::Fridge, Side::Left, 26, Facing::Right, true),
            ],
            graphics,
            1,
        );
        guest.cue(Scene::Arrive);
        paint(&mut guest, &real, &view, 0);
        let mending = |guest: &Guest| visit_of(guest).mending.map(|(_, at)| at);
        assert_eq!(mending(&guest), None, "nothing felt yet");
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.set_mood(Mood::Ordinary);
        let faces = rules::Grievance {
            row: rules::FACES_ROW,
            piece: Furniture::Sofa,
        };
        let wall = rules::Grievance {
            row: 2,
            piece: Furniture::Fridge,
        };
        assert!(matches!(
            wall.rule().map(|r| r.rule),
            Some(rules::Rule::AgainstWall(Furniture::Fridge))
        ));
        visit
            .osaka
            .feel(faces, (Furniture::Sofa, room::Use::Lounge));
        visit
            .osaka
            .feel(wall, (Furniture::Fridge, room::Use::Snack));
        let case = format!("graphics {graphics}");
        paint(&mut guest, &real, &view, 10);
        assert_eq!(mending(&guest), Some(10), "{case}: once felt");
        assert!(!visit_of(&guest).repairs.is_empty(), "{case}");
        assert!(
            visit_of(&guest).repairs.iter().all(|r| r.key == faces),
            "{case}"
        );
        // Nothing changed: not again within the second...
        paint(&mut guest, &real, &view, 500);
        paint(&mut guest, &real, &view, 1009);
        assert_eq!(mending(&guest), Some(10), "{case}: every frame");
        // ...and again once it's passed.
        paint(&mut guest, &real, &view, 1010);
        assert_eq!(mending(&guest), Some(1010), "{case}: the second passed");
        // Her home changed.
        assert!(guest.ledger.home.add(Prop {
            anchor: Some(Anchor {
                side: Side::Right,
                offset: 12,
            }),
            ..Prop::new(Furniture::Lamp, Nook::Playlist, 0, Facing::Left)
        }));
        paint(&mut guest, &real, &view, 1100);
        assert_eq!(mending(&guest), Some(1100), "{case}: her home");
        paint(&mut guest, &real, &view, 1150);
        assert_eq!(mending(&guest), Some(1100), "{case}");
        // The panes changed.
        let mut moved_pane = view.clone();
        moved_pane.nooks[0].1.width -= 1;
        paint(&mut guest, &real, &moved_pane, 1200);
        assert_eq!(mending(&guest), Some(1200), "{case}: the panes");
        paint(&mut guest, &real, &moved_pane, 1250);
        assert_eq!(mending(&guest), Some(1200), "{case}");
        // Text she has moved.
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        assert!(visit.layer.take(&real, &view.protected, (2, 1), (60, 2)));
        paint(&mut guest, &real, &moved_pane, 1300);
        assert_eq!(mending(&guest), Some(1300), "{case}: the text she moved");
        paint(&mut guest, &real, &moved_pane, 1350);
        assert_eq!(mending(&guest), Some(1300), "{case}");
        // The rule she'd put right: she lets go of the sofa, and would put
        // the fridge right instead.
        assert!(
            guest.broken().contains("wall(fridge)"),
            "{case}: {}",
            guest.broken()
        );
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.let_go_of(faces);
        paint(&mut guest, &real, &moved_pane, 1400);
        assert_eq!(mending(&guest), Some(1400), "{case}: the rule she'd mend");
        let repairs = &visit_of(&guest).repairs;
        assert!(
            !repairs.is_empty(),
            "{case}: the fridge has somewhere to go"
        );
        assert!(repairs.iter().all(|r| r.key == wall), "{case}: {repairs:?}");
    }
}

/// Cut short before it has all shown — a chat line, as she says it —
/// a grievance isn't felt: nesting stays put.
#[test]
fn a_grievance_cut_short_is_not_felt() {
    use super::room::Side;
    use sprite::Facing;
    let line = rules::RULES[0].grievance;
    for graphics in [false, true] {
        let (mut guest, real, mut view) = rule_home(
            &[
                (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
            ],
            graphics,
            2,
        );
        guest.cue(Scene::Lounge);
        paint(&mut guest, &real, &view, 0);
        let from = run_until(&mut guest, &real, &view, 0, 20_000, |guest, now| {
            look_now(guest, now).1 == Some(osaka::Bubble::Say(line))
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never said it"));
        let now = from + osaka::GRIEVANCE_MS / 2;
        let _ = run(&mut guest, &real, &view, from, now);
        view.chat_mark.synced += 1;
        guest.advance(now + 1);
        paint(&mut guest, &real, &view, now + 1);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        assert!(visit.osaka.use_span().is_none(), "she looked up");
        assert!(felt(&guest).is_empty(), "graphics {graphics}");
        assert_eq!(guest.broken(), "faces(sofa,TV)");
        assert_ne!(look_now(&guest, now + 1).1, Some(osaka::Bubble::Say(line)));
        let _ = run(&mut guest, &real, &view, now + 1, now + 3_000);
        assert_eq!(nesting(&guest), 0.0, "graphics {graphics}");
    }
}

/// A rule broken but not yet felt weighs nothing: with her sofa turned
/// away from the TV, she chooses on (her other needs rising) and
/// nesting stays put until she has felt it.
#[test]
fn an_unfelt_rule_raises_no_nesting() {
    use super::brain::Need;
    use super::room::Side;
    use sprite::Facing;
    for graphics in [false, true] {
        let (mut guest, real, view) = rule_home(
            &[
                (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
            ],
            graphics,
            4,
        );
        guest.cue(Scene::Stretch);
        paint(&mut guest, &real, &view, 0);
        let note = guest.cue_note().cloned();
        assert!(matches!(note, Some(Ok(_))), "graphics {graphics}: {note:?}");
        assert_eq!(guest.broken(), "faces(sofa,TV)");
        let hungry = |guest: &Guest| match &guest.state {
            State::Visiting(visit) => visit.osaka.needs().get(Need::Hungry),
            _ => panic!("visiting"),
        };
        let start = hungry(&guest);
        // Choices she makes before feeling it (her hunger rising shows
        // her needs moved on).
        let mut unfelt = 0;
        let mut last = start;
        let _ = run_until(&mut guest, &real, &view, 0, 60_000, |guest, now| {
            if !felt(guest).is_empty() {
                return true;
            }
            assert_eq!(nesting(guest), 0.0, "graphics {graphics}, {now}");
            let h = hungry(guest);
            if h > last {
                unfelt += 1;
                last = h;
            }
            false
        });
        assert!(
            unfelt >= 2,
            "graphics {graphics}: only {unfelt} choices before feeling it"
        );
    }
}

/// A TV she hasn't settled, in her bedroom: watching the shopping
/// channel on it, she has her home on her mind, not the advert, and
/// buys nothing.
#[test]
fn a_grievance_wins_over_the_shopping_channel() {
    use super::room::Side;
    use sprite::Facing;
    let line = rules::RULES[5].grievance;
    for graphics in [false, true] {
        let (mut guest, real, view) = rule_home(
            &[
                (Furniture::Bed, Side::Left, 0, Facing::Right, true),
                (Furniture::Tv, Side::Right, 0, Facing::Left, false),
            ],
            graphics,
            3,
        );
        guest.cue(Scene::Shopping);
        paint(&mut guest, &real, &view, 0);
        let note = guest.cue_note().cloned();
        assert!(matches!(note, Some(Ok(_))), "graphics {graphics}: {note:?}");
        assert_eq!(guest.broken(), "apart(TV,bed), belongs(TV)");
        let mut said = false;
        let watched = run_until(&mut guest, &real, &view, 0, 30_000, |guest, now| {
            let State::Visiting(visit) = &guest.state else {
                panic!("visiting");
            };
            assert_eq!(guest.ledger.ordered, None, "graphics {graphics}: bought");
            assert!(visit.chances.advert.is_some(), "the channel is on");
            said |= look_now(guest, now).1 == Some(osaka::Bubble::Say(line));
            visit
                .osaka
                .playing()
                .filter(|(seat, ..)| seat.what == room::Use::Watch)
                .is_some_and(|(_, since, play)| {
                    assert_eq!(play.bought, None, "graphics {graphics}");
                    now >= since + osaka::GRIEVANCE_MS * 2
                })
        });
        assert!(watched.is_some(), "graphics {graphics}: never watched");
        assert!(said, "graphics {graphics}: never said it");
        assert_eq!(guest.ledger.ordered, None, "graphics {graphics}: bought");
        assert_eq!(
            felt(&guest),
            [rules::Grievance {
                row: 5,
                piece: Furniture::Tv
            }]
        );
    }
}

// ---- She puts her home right ----

fn visit_of(guest: &Guest) -> &Visit {
    match &guest.state {
        State::Visiting(visit) => visit,
        _ => panic!("visiting"),
    }
}

/// The piece in her pocket, if one is.
fn carried(guest: &Guest) -> Option<Furniture> {
    match &guest.state {
        State::Visiting(visit) => visit.osaka.carrying(),
        _ => None,
    }
}

fn prop_of(guest: &Guest, item: Furniture) -> room::Prop {
    *guest
        .ledger
        .home
        .props
        .iter()
        .find(|p| p.item == item)
        .unwrap_or_else(|| panic!("she owns a {item:?}"))
}

/// Nothing she carries is drawn: the piece in her pocket is in none of
/// the lists a frame's furniture is drawn from (nor the goodbye's), and
/// in ASCII none of its glyphs is where it stood (but where she is, or
/// where a pane's rain still shows an earlier frame: a pane focused the
/// moment she lifts it rains out the frame it stood in, as it was on
/// screen).
fn carried_unseen(guest: &Guest, frame: &Buffer, real: &Buffer, view: &IdleView) {
    let State::Visiting(visit) = &guest.state else {
        return;
    };
    let Some(piece) = visit.osaka.carrying() else {
        return;
    };
    let real_one = |s: &Shown| s.item == piece && s.scrap.is_none();
    assert!(!visit.shown.iter().any(real_one), "{piece:?} shown");
    assert!(
        !visit.image.iter().flat_map(|i| &i.with).any(real_one),
        "{piece:?} in her image"
    );
    if visit.image.is_some() {
        return;
    }
    let mut home = guest.ledger.home.clone();
    let Some(at) = home
        .layout(&view.nooks)
        .into_iter()
        .find(|s| s.item == piece)
    else {
        return;
    };
    let (x, y) = (visit.osaka.x, visit.osaka.y);
    for (cx, cy, glyph) in at.cells() {
        let her = (cx - x).abs() <= sprite::WIDTH / 2 && (y - sprite::HEIGHT..=y).contains(&cy);
        let (Some(glyph), false) = (glyph, her) else {
            continue;
        };
        let cell = (cx as u16, cy as u16);
        if visit.fades.iter().any(|f| f.painting(cell.0, cell.1)) {
            continue;
        }
        let got = frame.cell(cell).unwrap().symbol();
        assert!(
            got == real.cell(cell).unwrap().symbol() || got == "." || !got.starts_with(glyph),
            "{piece:?}'s {glyph:?} drawn at {cell:?} while she carries it"
        );
    }
}

/// Step as the shell would, painting each tick, checking nothing she
/// carries is drawn, until `done` (after each paint) or `until`; when it
/// was done.
fn carry_until(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    from: u64,
    until: u64,
    mut done: impl FnMut(&Guest, u64) -> bool,
) -> Option<u64> {
    let mut now = from;
    while now < until {
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 100);
        guest.advance(now);
        let frame = paint(guest, real, view, now);
        carried_unseen(guest, &frame, real, view);
        if done(guest, now) {
            return Some(now);
        }
    }
    None
}

/// The methods she decided by, in order.
fn methods(guest: &Guest) -> Vec<&'static str> {
    visit_of(guest)
        .osaka
        .decisions
        .iter()
        .map(|d| d.method)
        .collect()
}

/// The whole of it: her sofa turned away from the TV, she lounges on it
/// and feels it; keen on her home, she sets about it, lifts the sofa
/// ("Hup!") into her pocket, where it shows nowhere, sets it down turned
/// round ("There!"), and sits back down on it to watch the TV. Once:
/// her mood's one thing about her home this visit.
#[test]
fn she_turns_her_sofa_round_and_watches_from_it() {
    use super::room::{Side, Use};
    use sprite::Facing;
    for graphics in [false, true] {
        let (mut guest, real, view) = rule_home(
            &[
                (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
            ],
            graphics,
            1,
        );
        guest.cue(Scene::Lounge);
        paint(&mut guest, &real, &view, 0);
        let felt_at = run_until(&mut guest, &real, &view, 0, 30_000, |guest, _| {
            !felt(guest).is_empty()
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never felt it"));
        assert_eq!(visit_of(&guest).osaka.mood(), super::brain::Mood::Ordinary);
        guest.press(stage::Want::Nesting);
        let mut hup = false;
        // (Turned where it stands, or tried a cell or so along first:
        // `she_tries_it_in_a_spot_or_two`.)
        let watches = |guest: &Guest| {
            visit_of(guest)
                .osaka
                .use_span()
                .is_some_and(|(seat, ..)| (seat.what, seat.item) == (Use::Watch, Furniture::Sofa))
        };
        let (mut turned, mut watched) = (false, false);
        let set = carry_until(&mut guest, &real, &view, felt_at, 600_000, |guest, now| {
            let (_, bubble) = look_now(guest, now);
            hup |= carried(guest).is_some() && bubble == Some(osaka::Bubble::Say("Hup!"));
            turned |= prop_of(guest, Furniture::Sofa).facing == Facing::Left;
            watched |= turned && watches(guest);
            turned && visit_of(guest).osaka.episode().is_none()
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never turned it round"));
        assert!(hup, "graphics {graphics}: lifted without a word");
        let sofa = prop_of(&guest, Furniture::Sofa);
        assert_eq!(
            (sofa.anchor.map(|a| a.side), sofa.strip, sofa.settled),
            (Some(Side::Left), room::Strip::Bottom(Nook::Playlist), true),
        );
        assert!(
            sofa.anchor.unwrap().offset.abs_diff(12) <= rules::TIE_CELLS as u16,
            "{sofa:?}"
        );
        assert_eq!(guest.broken(), "", "graphics {graphics}");
        assert_eq!(carried(&guest), None);
        assert_eq!(
            look_now(&guest, set).1,
            Some(osaka::Bubble::Say("There!")),
            "graphics {graphics}"
        );
        let ways = methods(&guest);
        let lift = ways.iter().position(|&m| m == "arrange/lift");
        let carry = ways.iter().position(|&m| m == "arrange/carry");
        assert!(
            lift.is_some() && lift < carry,
            "graphics {graphics}: {ways:?}"
        );
        let osaka = &visit_of(&guest).osaka;
        assert_eq!(osaka.home_acts(), 1);
        let lifted = osaka
            .decisions
            .iter()
            .find(|d| d.method == "arrange/lift")
            .unwrap();
        assert_eq!(lifted.bucket, osaka::Bucket::Normal, "chosen, not cued");
        // Sitting back down: to watch, from the sofa turned toward it
        // (trying it there, before she kept it; or after).
        let watching = watched
            || run_until(&mut guest, &real, &view, set, set + 30_000, |g, _| {
                watches(g)
            })
            .is_some();
        assert!(watching, "graphics {graphics}: {:?}", methods(&guest));
        assert!(methods(&guest).contains(&"arrange/use-it"));
        // The rule's right, and her mood's done its bit: no more.
        let _ = run(&mut guest, &real, &view, set + 30_000, set + 300_000);
        assert_eq!(visit_of(&guest).osaka.home_acts(), 1, "graphics {graphics}");
        assert_eq!(prop_of(&guest, Furniture::Sofa), sofa);
    }
}

/// Her sofa turned from the TV on a page of text (a few spots along its
/// strip as good as each other, turned round: [`wrong_home`] 0), having
/// felt it, keen on her home, in an ordinary mood.
fn sofa_to_try(graphics: bool, seed: u64) -> (Guest, Buffer, IdleView) {
    let (w, h) = (100, 30);
    let panes = nooks(w, h);
    let view = IdleView {
        chat: panes[0].1,
        nooks: panes[1..].to_vec(),
        ..view(bottom_strip(w, h))
    };
    let (guest, real) = sofa_to_try_in(graphics, seed, &view);
    (guest, real, view)
}

/// [`sofa_to_try`] on `view` (of [`nooks`] at 100×30).
fn sofa_to_try_in(graphics: bool, seed: u64, view: &IdleView) -> (Guest, Buffer) {
    use super::brain::Mood;
    use super::room::{Anchor, Prop};
    let real = wordy_rooms(100, 30);
    let mut guest = Guest::new(seed);
    if graphics {
        guest.set_picker(kitty());
    }
    for (item, nook, side, offset, facing, settled) in wrong_home(0, 12) {
        assert!(guest.ledger.home.add(Prop {
            anchor: Some(Anchor { side, offset }),
            settled,
            ..Prop::new(item, nook, 0, facing)
        }));
    }
    let faces = rules::Grievance {
        row: rules::FACES_ROW,
        piece: Furniture::Sofa,
    };
    assert_eq!(breaks(&guest.ledger.home, &view.nooks), vec![faces]);
    guest.cue(Scene::Lounge);
    paint(&mut guest, &real, view, 0);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.set_mood(Mood::Ordinary);
    guest.press(stage::Want::Nesting);
    (guest, real)
}

/// One tick as the shell would ([`carry_until`]) from `now`, with her
/// calm (so picky: a spot dearer than the cheapest she seldom keeps);
/// when it was.
fn calm_tick(guest: &mut Guest, real: &Buffer, view: &IdleView, now: u64) -> u64 {
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit
        .osaka
        .needs_mut()
        .serve(super::brain::Need::Restless, 1.0);
    carry_until(guest, real, view, now, now + 1, |_, _| true).unwrap_or(now + 1)
}

/// [`sofa_to_try`], with her calm: she lifts the sofa, sets it down in
/// one spot ("hmm..." as she sits on it), and keeps it there or tries
/// the next, up to three, keeping the last; for some visits two spots,
/// for some three. Every spot she sets it down in puts the rule right
/// and breaks nothing; however many, it's one thing about her home; and
/// in line art, no image she shows is encoded twice.
#[test]
fn she_tries_it_in_a_spot_or_two() {
    for graphics in [false, true] {
        let mut tried = std::collections::BTreeSet::new();
        for seed in 0..12 {
            let (mut guest, real, view) = sofa_to_try(graphics, seed);
            let case = format!("graphics {graphics}, seed {seed}");
            let mut spots: Vec<room::Prop> = Vec::new();
            let mut hmm = false;
            let mut was = prop_of(&guest, Furniture::Sofa);
            let mut now = 0;
            loop {
                assert!(now < 600_000, "{case}: never settled on a spot: {spots:?}");
                now = calm_tick(&mut guest, &real, &view, now);
                let is = prop_of(&guest, Furniture::Sofa);
                if is != was {
                    // Each spot puts it right, and breaks nothing else.
                    let now_broken = breaks(&guest.ledger.home, &view.nooks);
                    assert!(now_broken.is_empty(), "{case}: {now_broken:?} at {is:?}");
                    spots.push(is);
                    was = is;
                }
                hmm |= look_now(&guest, now).1 == Some(osaka::Bubble::Say("hmm..."));
                let osaka = &visit_of(&guest).osaka;
                assert!(osaka.home_acts() <= 1, "{case}");
                if osaka.home_acts() == 1 && osaka.episode().is_none() {
                    break;
                }
            }
            assert!((1..=3).contains(&spots.len()), "{case}: {spots:?}");
            if spots.len() >= 2 {
                assert!(hmm, "{case}: tried it without a thought");
            }
            // Kept where she last set it down: nothing more about her
            // home this visit.
            let _ = carry_until(&mut guest, &real, &view, now, now + 60_000, |_, _| false);
            assert_eq!(visit_of(&guest).osaka.home_acts(), 1, "{case}");
            assert_eq!(
                spots.last(),
                Some(&prop_of(&guest, Furniture::Sofa)),
                "{case}"
            );
            assert_eq!(
                visit_of(&guest).osaka.retried as usize + 1,
                spots.len(),
                "{case}"
            );
            // Each spot shows the piece anew: no image is encoded twice.
            if let Some(graphics) = &guest.graphics {
                let counts = graphics.counts();
                assert_eq!(counts.reencoded, 0, "{case}: {counts:?}");
            }
            tried.insert(spots.len());
            if tried.contains(&2) && tried.contains(&3) {
                break;
            }
        }
        assert!(
            tried.contains(&2) && tried.contains(&3),
            "graphics {graphics}: spots tried {tried:?}"
        );
    }
}

/// Lifted again to try in another spot, the sofa stays where she last
/// set it down if she says goodbye with it in her pocket.
#[test]
fn goodbye_mid_trial_leaves_the_last_spot() {
    for graphics in [false, true] {
        // (Seed 1 tries three spots: `she_tries_it_in_a_spot_or_two`.)
        let (mut guest, real, view) = sofa_to_try(graphics, 1);
        let mut now = 0;
        loop {
            assert!(now < 600_000, "graphics {graphics}: never lifted it again");
            now = calm_tick(&mut guest, &real, &view, now);
            let osaka = &visit_of(&guest).osaka;
            if osaka.episode().is_some_and(|e| e.tried > 0 && e.pocket) {
                break;
            }
        }
        let set = prop_of(&guest, Furniture::Sofa);
        assert_eq!(set.facing, sprite::Facing::Left, "set down turned round");
        guest.activity(now);
        let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
        assert!(!guest.present());
        assert!(
            end == real,
            "graphics {graphics}: the rain restores the frame"
        );
        assert_eq!(prop_of(&guest, Furniture::Sofa), set, "graphics {graphics}");
        assert!(breaks(&guest.ledger.home, &view.nooks).is_empty());
    }
}

/// Her bedroom upstairs (the Users pane: her bed, and the TV that came
/// in there, which she hasn't settled), her sofa downstairs (the
/// Playlist pane), and the chat beside them full of text.
fn two_rooms(graphics: bool, seed: u64) -> (Guest, Buffer) {
    use super::room::{Anchor, Prop, Side};
    use sprite::Facing;
    let (w, h) = (100, 30);
    let mut real = rooms(w, h);
    let text: Vec<(u16, u16, String)> = (0..12)
        .map(|i| {
            (
                2 + i * 5 % 20,
                2 + i * 2,
                "so what did you think of it".to_owned(),
            )
        })
        .collect();
    scatter(&mut real, &text, &[]);
    let mut guest = Guest::new(seed);
    if graphics {
        guest.set_picker(kitty());
    }
    for (item, nook, side, offset, facing, settled) in [
        (
            Furniture::Bed,
            Nook::Users,
            Side::Left,
            1,
            Facing::Right,
            true,
        ),
        (
            Furniture::Tv,
            Nook::Users,
            Side::Right,
            1,
            Facing::Left,
            false,
        ),
        (
            Furniture::Sofa,
            Nook::Playlist,
            Side::Left,
            2,
            Facing::Right,
            true,
        ),
    ] {
        assert!(guest.ledger.home.add(Prop {
            anchor: Some(Anchor { side, offset }),
            settled,
            ..Prop::new(item, nook, 0, facing)
        }));
    }
    (guest, real)
}

/// [`two_rooms`]' view: a visitor's, or a resident's with `focus`.
fn two_rooms_view(resident: bool, focus: Option<Rect>) -> IdleView {
    let (w, h) = (100, 30);
    if resident {
        return resident_view(w, h, focus);
    }
    let panes = nooks(w, h);
    IdleView {
        chat: panes[0].1,
        nooks: panes[1..].to_vec(),
        ..view(bottom_strip(w, h))
    }
}

/// In [`two_rooms`], she watches the TV and feels it doesn't belong in
/// her bedroom, then is keen on her home; when that is.
fn feel_the_tv(guest: &mut Guest, real: &Buffer, view: &IdleView) -> u64 {
    let line = rules::RULES[5].grievance;
    guest.cue(Scene::Watch);
    paint(guest, real, view, 0);
    let note = guest.cue_note().cloned();
    assert!(matches!(note, Some(Ok(_))), "{note:?}");
    assert!(guest.broken().contains("belongs(TV)"), "{}", guest.broken());
    let felt_at = run_until(guest, real, view, 0, 60_000, |guest, _| {
        !felt(guest).is_empty()
    })
    .expect("never felt it");
    assert_eq!(
        felt(guest),
        [rules::Grievance {
            row: 5,
            piece: Furniture::Tv
        }],
        "{line}"
    );
    guest.press(stage::Want::Nesting);
    felt_at
}

/// A TV she never settled, come in where her bed is: she feels it
/// doesn't belong, lifts it into her pocket and carries it downstairs
/// (another floor: a climb, a hop or a door) to her sofa's room, where
/// it makes a living room; the bedroom's a bedroom again.
#[test]
fn an_unsettled_tv_joins_the_sofas_room() {
    use super::room::{Role, Strip};
    for graphics in [false, true] {
        let (mut guest, real) = two_rooms(graphics, 5);
        let view = two_rooms_view(false, None);
        let from = feel_the_tv(&mut guest, &real, &view);
        let upstairs = |guest: &Guest| guest.ledger.home.rooms();
        assert_eq!(
            upstairs(&guest)
                .iter()
                .find(|(s, _)| *s == Strip::Bottom(Nook::Users))
                .map(|r| r.1),
            Some(Role::Den),
            "a TV and a bed: no bedroom"
        );
        let set = carry_until(&mut guest, &real, &view, from, 600_000, |guest, _| {
            prop_of(guest, Furniture::Tv).strip == Strip::Bottom(Nook::Playlist)
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never moved it: {:?}", methods(&guest)));
        let tv = prop_of(&guest, Furniture::Tv);
        assert!(tv.settled, "graphics {graphics}");
        let rooms = guest.ledger.home.rooms();
        assert!(
            rooms.contains(&(Strip::Bottom(Nook::Users), Role::Bedroom))
                && rooms.contains(&(Strip::Bottom(Nook::Playlist), Role::Living)),
            "graphics {graphics}: {rooms:?}"
        );
        // (Whether the sofa faces it is another rule, broken before.)
        let broken = guest.broken();
        assert!(
            !broken.contains("belongs") && !broken.contains("apart"),
            "{broken}"
        );
        let osaka = &visit_of(&guest).osaka;
        assert!(
            !osaka.headings.is_empty(),
            "graphics {graphics}: carried to another floor without heading there"
        );
        assert_eq!(osaka.home_acts(), 1);
        let shown = &visit_of(&guest).shown;
        assert!(
            shown.iter().any(|s| s.item == Furniture::Tv),
            "{set}: it shows"
        );
    }
}

/// Whether the TV is downstairs, set down; until it is, it's in her
/// pocket, every frame (never let go, or put back and lifted again).
fn tv_set_down_or_carried(guest: &Guest, now: u64, case: &str) -> bool {
    use super::room::Strip;
    if prop_of(guest, Furniture::Tv).strip == Strip::Bottom(Nook::Playlist) {
        return true;
    }
    assert_eq!(
        carried(guest),
        Some(Furniture::Tv),
        "{case}: let go at {now}"
    );
    assert!(
        visit_of(guest).osaka.episode().is_some_and(|e| e.pocket),
        "{case}: {now}"
    );
    false
}

/// She lifted a piece only the once.
fn lifted_once(guest: &Guest, case: &str) {
    let lifts = methods(guest)
        .iter()
        .filter(|&&m| m == "arrange/lift")
        .count();
    assert_eq!(lifts, 1, "{case}: {:?}", methods(guest));
}

/// Chat comes while the TV's in her pocket on its way downstairs: she
/// looks up at it, and then carries on with the TV, which shows nowhere
/// meanwhile.
#[test]
fn chat_mid_carry_and_she_carries_on() {
    for graphics in [false, true] {
        let (mut guest, real) = two_rooms(graphics, 5);
        let mut view = two_rooms_view(false, None);
        let from = feel_the_tv(&mut guest, &real, &view);
        let walking = carry_until(&mut guest, &real, &view, from, 600_000, |guest, _| {
            carried(guest).is_some() && visit_of(guest).osaka.act_name() == "Walk"
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never carried it off"));
        let before = methods(&guest).len();
        view.chat_mark.synced += 1;
        let _ = paint(&mut guest, &real, &view, walking);
        assert_eq!(
            visit_of(&guest).osaka.act_name(),
            "Look",
            "graphics {graphics}"
        );
        assert_eq!(carried(&guest), Some(Furniture::Tv), "still in her pocket");
        let case = format!("graphics {graphics}");
        let set = carry_until(&mut guest, &real, &view, walking, 600_000, |guest, now| {
            tv_set_down_or_carried(guest, now, &case)
        });
        assert!(set.is_some(), "graphics {graphics}: {:?}", methods(&guest));
        lifted_once(&guest, &case);
        let after = &methods(&guest)[before..];
        assert_eq!(
            after.first(),
            Some(&"watching chat"),
            "graphics {graphics}: {after:?}"
        );
        assert!(
            after.contains(&"arrange/carry"),
            "graphics {graphics}: {after:?}"
        );
    }
}

/// A busy chat while the TV's in her pocket: a line each time she's
/// walking with it again, more lines than she has tries. Each only
/// stops her a moment; none costs her the TV, and it gets downstairs.
#[test]
fn a_busy_chat_never_costs_her_the_piece() {
    for graphics in [false, true] {
        let (mut guest, real) = two_rooms(graphics, 5);
        let mut view = two_rooms_view(false, None);
        let from = feel_the_tv(&mut guest, &real, &view);
        let mut lines = 0;
        let mut now = from;
        let mut lifted = false;
        let set = loop {
            assert!(
                now < from + 600_000,
                "graphics {graphics}: never moved it: {:?}",
                methods(&guest)
            );
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            let osaka = &visit_of(&guest).osaka;
            if lines < 6 && carried(&guest).is_some() && osaka.act_name() == "Walk" {
                lines += 1;
                view.chat_mark.synced += 1;
            }
            let frame = paint(&mut guest, &real, &view, now);
            carried_unseen(&guest, &frame, &real, &view);
            if carried(&guest).is_some() {
                lifted = true;
            }
            if lifted && tv_set_down_or_carried(&guest, now, &format!("graphics {graphics}")) {
                break now;
            }
        };
        lifted_once(&guest, &format!("graphics {graphics}"));
        assert!(lines > 3, "graphics {graphics}: only {lines} lines");
        let osaka = &visit_of(&guest).osaka;
        assert!(
            !osaka
                .beats
                .iter()
                .any(|b| matches!(b.loss, mind::Loss::Moved(_))),
            "graphics {graphics}: let it go at some point before {set}: {:?}",
            osaka.beats
        );
        assert_eq!(osaka.home_acts(), 1);
    }
}

/// A resident with the TV in her pocket: the pane she's in is focused,
/// she's through her door with it, and carries on downstairs.
#[test]
fn evicted_mid_carry_she_carries_on() {
    for graphics in [false, true] {
        let (mut guest, real) = two_rooms(graphics, 5);
        let view = two_rooms_view(true, None);
        let from = feel_the_tv(&mut guest, &real, &view);
        let users = nooks(100, 30)[1].1;
        let upstairs = carry_until(&mut guest, &real, &view, from, 600_000, |guest, _| {
            let osaka = &visit_of(guest).osaka;
            carried(guest).is_some() && osaka::box_meets(users, (osaka.x, osaka.y))
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never lifted it"));
        let focused = two_rooms_view(true, Some(users));
        let _ = paint(&mut guest, &real, &focused, upstairs);
        let osaka = &visit_of(&guest).osaka;
        assert_eq!(
            osaka.act_name(),
            "Door",
            "graphics {graphics}: out of the pane"
        );
        assert_eq!(
            carried(&guest),
            Some(Furniture::Tv),
            "through her door with it"
        );
        let case = format!("graphics {graphics}");
        let set = carry_until(
            &mut guest,
            &real,
            &focused,
            upstairs,
            600_000,
            |guest, now| tv_set_down_or_carried(guest, now, &case),
        );
        assert!(set.is_some(), "graphics {graphics}: {:?}", methods(&guest));
        lifted_once(&guest, &case);
    }
}

/// The stage room, cued to turn the sofa round; when she has it in her
/// pocket (and is about to set it down, `setting`).
fn cued_carry(graphics: bool, setting: bool) -> (Guest, Buffer, IdleView, u64) {
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = Guest::new(1);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrange);
    paint(&mut guest, &real, &view, 0);
    let note = guest.cue_note().cloned();
    assert!(matches!(note, Some(Ok(_))), "{note:?}");
    let at = carry_until(&mut guest, &real, &view, 0, 30_000, |guest, _| {
        carried(guest).is_some() && (!setting || visit_of(guest).osaka.act_name() == "SetDown")
    })
    .expect("never lifted it");
    (guest, real, view, at)
}

/// What her record says of her home: the pieces, and what's on order
/// (not her clock, which runs on through the goodbye).
fn home_of(guest: &Guest) -> (room::Home, Option<Furniture>) {
    (guest.ledger.home.clone(), guest.ledger.ordered)
}

/// A goodbye with the sofa in her pocket, or set down but not yet taken
/// by a paint: her record is as it was (the sofa where it stood,
/// unturned), and next visit it's there.
#[test]
fn a_goodbye_mid_carry_leaves_the_piece_where_it_stood() {
    for graphics in [false, true] {
        for setting in [false, true] {
            let (mut guest, real, view, mut now) = cued_carry(graphics, setting);
            let record = home_of(&guest);
            let sofa = prop_of(&guest, Furniture::Sofa);
            if setting {
                // Set down, and the visit ends before any paint takes it.
                loop {
                    assert!(now < 30_000, "never set it down");
                    now += guest
                        .next_tick(now)
                        .map_or(100, |d| d.as_millis() as u64)
                        .clamp(1, 100);
                    guest.advance(now);
                    if visit_of(&guest).set_down.is_some() {
                        break;
                    }
                }
            }
            guest.activity(now);
            let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
            assert!(!guest.present());
            assert_eq!(
                end, real,
                "graphics {graphics}: the rain restores the frame"
            );
            assert_eq!(
                home_of(&guest),
                record,
                "graphics {graphics}, setting {setting}"
            );
            assert_eq!(prop_of(&guest, Furniture::Sofa), sofa);
        }
    }
}

/// The frame won't take the sofa where she set it down (the pane's
/// covered there now): it's back where it stood, unturned, and she
/// glances at it there.
#[test]
fn a_refused_set_down_puts_it_back_and_she_glances_at_it() {
    for graphics in [false, true] {
        let (mut guest, real, view, mut now) = cued_carry(graphics, true);
        let sofa = prop_of(&guest, Furniture::Sofa);
        loop {
            assert!(now < 30_000, "never set it down");
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 100);
            guest.advance(now);
            if visit_of(&guest).set_down.is_some() {
                break;
            }
        }
        let mut home = guest.ledger.home.clone();
        let at = home
            .layout(&view.nooks)
            .into_iter()
            .find(|s| s.item == Furniture::Sofa)
            .unwrap();
        let mut covered = view.clone();
        covered.protected.push(at.rect());
        let _ = paint(&mut guest, &real, &covered, now);
        assert_eq!(carried(&guest), None, "graphics {graphics}");
        assert_eq!(prop_of(&guest, Furniture::Sofa), sofa, "where it stood");
        let toward = (at.left + i32::from(at.size().0) / 2, at.floor);
        assert_eq!(
            visit_of(&guest).osaka.beats.last(),
            Some(&mind::Beat {
                loss: mind::Loss::Moved(Furniture::Sofa),
                toward
            })
        );
        // Uncovered, it shows again, and she glances at it.
        let glanced = carry_until(&mut guest, &real, &view, now, now + 10_000, |guest, _| {
            visit_of(guest).osaka.act_name() == "Glance"
        });
        assert!(glanced.is_some(), "graphics {graphics}");
        assert!(
            visit_of(&guest)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Sofa && s.scrap.is_none())
        );
        assert_eq!(visit_of(&guest).osaka.home_acts(), 0);
    }
}

/// Text comes up over the sofa as she bends to lift it (it's in the
/// closet): she's startled off it, and lifts it once it's back.
#[test]
fn a_piece_closeted_as_she_lifts_it_is_lifted_later() {
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::Arrange);
        paint(&mut guest, &real, &view, 0);
        let lifting = carry_until(&mut guest, &real, &view, 0, 30_000, |guest, _| {
            visit_of(guest).osaka.lifting().is_some()
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never bent to it"));
        let at = visit_of(&guest)
            .shown
            .iter()
            .find(|s| s.item == Furniture::Sofa)
            .copied()
            .unwrap();
        let mut covered = view.clone();
        covered.protected.push(at.rect());
        let _ = paint(&mut guest, &real, &covered, lifting);
        let osaka = &visit_of(&guest).osaka;
        assert_eq!(osaka.act_name(), "Look", "graphics {graphics}");
        assert_eq!(carried(&guest), None);
        assert!(osaka.episode().is_some(), "she still means to");
        let lifted = carry_until(
            &mut guest,
            &real,
            &view,
            lifting,
            lifting + 30_000,
            |guest, _| carried(guest).is_some(),
        );
        assert!(
            lifted.is_some(),
            "graphics {graphics}: {:?}",
            methods(&guest)
        );
    }
}

/// The image budget with a carry: in line art, busy about her home (her
/// sofa turned from the TV on a page of text; a TV she never settled in
/// her bedroom, carried downstairs), she sets at least one piece down
/// where it's right, and no image she shows is encoded twice.
/// Each piece pocketed and shown again is images encoded: what that
/// costs is counted here (and recorded in plan.md, for trials).
#[test]
fn a_carry_stays_within_the_image_budget() {
    use super::brain::Mood;
    use super::room::{Side, Strip};
    use sprite::Facing;
    for home in ["sofa", "tv"] {
        let (mut guest, real, view) = if home == "sofa" {
            rule_home(
                &[
                    (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                    (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
                ],
                true,
                1,
            )
        } else {
            let (guest, real) = two_rooms(true, 5);
            (guest, real, two_rooms_view(false, None))
        };
        let felt_at = if home == "sofa" {
            guest.cue(Scene::Lounge);
            paint(&mut guest, &real, &view, 0);
            let felt_at = run_until(&mut guest, &real, &view, 0, 30_000, |guest, _| {
                !felt(guest).is_empty()
            })
            .expect("never felt it");
            guest.press(stage::Want::Nesting);
            felt_at
        } else {
            feel_the_tv(&mut guest, &real, &view)
        };
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.set_mood(Mood::Industrious);
        let counts = |guest: &Guest| guest.graphics.as_ref().map(|g| g.counts()).unwrap();
        let before = counts(&guest);
        let mut lifted = None;
        let set = carry_until(
            &mut guest,
            &real,
            &view,
            felt_at,
            felt_at + 600_000,
            |guest, _| {
                if lifted.is_none() && carried(guest).is_some() {
                    lifted = Some(counts(guest));
                }
                visit_of(guest).osaka.home_acts() >= 1
            },
        )
        .unwrap_or_else(|| panic!("{home}: never set anything down: {:?}", methods(&guest)));
        if home == "tv" {
            assert_eq!(
                prop_of(&guest, Furniture::Tv).strip,
                Strip::Bottom(Nook::Playlist)
            );
        }
        // On a while: sitting back down, and whatever else she does.
        let _ = carry_until(&mut guest, &real, &view, set, set + 300_000, |_, _| false);
        let carrying = counts(&guest).encoded - lifted.expect("set down unlifted").encoded;
        let graphics = guest.graphics.as_ref().unwrap();
        let (counts, cached) = (graphics.counts(), graphics.cached());
        eprintln!(
            "{home}: set down at {set} ms; images encoded: {} from feeling it ({carrying} from lifting it to 5 min after setting it down), {} in all; {cached} cached; {} evicted",
            counts.encoded - before.encoded,
            counts.encoded,
            counts.evicted
        );
        assert_eq!(counts.reencoded, 0, "{home}: {counts:?}");
    }
}

/// Her alone on a floor, with the stage's lift of a sofa from where she
/// stands to `to` (or `others` for it); her, the floor, the move.
fn pocket_on_a_floor(
    graphics: bool,
) -> (
    Osaka,
    Terrain,
    rules::Repair,
    Vec<rules::Repair>,
    super::Rng,
) {
    use super::room::{Anchor, Side, Strip};
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    buf.set_string(0, 10, "─".repeat(60), Style::new());
    let terrain = Terrain::read(&buf, &[], graphics);
    let mut rng = Rng(7);
    let osaka = Osaka::standing_at(10, 10, 0, &mut rng);
    let repair = |offset: u16, left: i32| rules::Repair {
        key: rules::Grievance {
            row: rules::FACES_ROW,
            piece: Furniture::Sofa,
        },
        piece: Furniture::Sofa,
        to: rules::Placement {
            strip: Strip::Bottom(Nook::Users),
            anchor: Anchor {
                side: Side::Left,
                offset,
            },
            facing: sprite::Facing::Left,
        },
        at: Shown {
            item: Furniture::Sofa,
            facing: sprite::Facing::Left,
            boxed: false,
            strip: Some(Strip::Bottom(Nook::Users)),
            left,
            floor: 10,
            scrap: None,
        },
        cost: 1,
        tier: 0,
    };
    (osaka, terrain, repair(30, 31), vec![repair(40, 41)], rng)
}

/// Run `osaka` on `terrain` from `from` to `until` with `chances` as
/// `judge` makes them each tick (from the episode, if any); when `done`.
fn tick_pocket(
    osaka: &mut Osaka,
    terrain: &Terrain,
    rng: &mut super::Rng,
    from: u64,
    until: u64,
    mut judge: impl FnMut(&Osaka) -> osaka::Chances,
    mut done: impl FnMut(&Osaka) -> bool,
) -> Option<u64> {
    let mut now = from;
    while now < until {
        now += 50;
        let chances = judge(osaka);
        osaka.tick(now, None, terrain, &chances, rng);
        if done(osaka) {
            return Some(now);
        }
    }
    None
}

/// With the piece in her pocket and its move still right, but nowhere
/// she can get to to set it down, each setting off is a try: after
/// [`TRIES`](osaka) she lets it go (it's back where it stood), with a
/// glance at it there. Never before, however long she waits on a frame
/// that hasn't judged it.
#[test]
fn a_carry_she_cant_get_to_is_let_go_after_three_tries() {
    for graphics in [false, true] {
        let (mut osaka, terrain, repair, _, mut rng) = pocket_on_a_floor(graphics);
        osaka.lift(
            scenes::Lift {
                repair,
                trials: rules::Trials::default(),
                x: 10,
                y: 10,
                side: scenes::Side::Right,
            },
            0,
        );
        let home = (35, 10);
        let judged = |osaka: &Osaka| osaka::Chances {
            judged: osaka.episode().map(|ep| osaka::Judged {
                piece: ep.repair.piece,
                to: ep.repair.to,
                pocket: ep.pocket,
                holds: true,
                // Nowhere to stand: beside it or over it.
                spot: (!ep.pocket).then_some(((10, 10), scenes::Side::Right)),
                home: Some(home),
            }),
            ..osaka::Chances::default()
        };
        let lifted = tick_pocket(&mut osaka, &terrain, &mut rng, 0, 10_000, judged, |o| {
            o.carrying().is_some()
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never lifted it"));
        let tries = |osaka: &Osaka| {
            osaka
                .decisions
                .iter()
                .filter(|d| d.method == "can't get to it")
                .count()
        };
        let let_go = tick_pocket(
            &mut osaka,
            &terrain,
            &mut rng,
            lifted,
            lifted + 60_000,
            judged,
            |o| o.episode().is_none(),
        )
        .unwrap_or_else(|| panic!("graphics {graphics}: still carrying it"));
        assert_eq!(tries(&osaka), 3, "graphics {graphics}");
        assert_eq!(osaka.carrying(), None);
        assert!(
            let_go >= lifted + 3 * 2000,
            "{let_go}: a wait between tries"
        );
        assert_eq!(
            osaka.beats.last(),
            Some(&mind::Beat {
                loss: mind::Loss::Moved(Furniture::Sofa),
                toward: home
            }),
            "graphics {graphics}"
        );
    }
}

/// With the piece in her pocket, where it goes no longer right (text
/// came there), she sets it down another way the frame has for it: no
/// loss, no try spent, the piece in her pocket all along; with no other
/// way, she lets it go, with a glance at where it stood.
#[test]
fn a_carry_whose_spot_is_gone_goes_elsewhere_or_is_let_go() {
    for graphics in [false, true] {
        for other in [true, false] {
            let case = format!("graphics {graphics}, another way {other}");
            let (mut osaka, terrain, repair, others, mut rng) = pocket_on_a_floor(graphics);
            let first = repair.to;
            osaka.lift(
                scenes::Lift {
                    repair,
                    trials: rules::Trials::default(),
                    x: 10,
                    y: 10,
                    side: scenes::Side::Right,
                },
                0,
            );
            let home = (35, 10);
            // Lifted, the first spot is gone; the other holds (where
            // there is one).
            let others = if other { others } else { Vec::new() };
            let judged = |osaka: &Osaka| osaka::Chances {
                judged: osaka.episode().map(|ep| osaka::Judged {
                    piece: ep.repair.piece,
                    to: ep.repair.to,
                    pocket: ep.pocket,
                    holds: !ep.pocket || ep.repair.to != first,
                    spot: Some(((10, 10), scenes::Side::Right)),
                    home: Some(home),
                }),
                repairs: others.clone(),
                ..osaka::Chances::default()
            };
            let mut lifted = false;
            let ended = tick_pocket(&mut osaka, &terrain, &mut rng, 0, 60_000, judged, |o| {
                // In her pocket from the lift until it's set down (or let
                // go): never dropped and lifted again.
                if lifted && o.episode().is_some() {
                    assert_eq!(o.carrying(), Some(Furniture::Sofa), "{case}");
                }
                lifted |= o.carrying().is_some();
                o.episode().is_none_or(|ep| ep.set_down)
            })
            .unwrap_or_else(|| panic!("{case}: neither set down nor let go"));
            if other {
                let ep = osaka.episode().copied().unwrap_or_else(|| panic!("{case}"));
                assert!(ep.set_down, "{case}: set down at {ended}");
                assert_eq!(ep.repair.to, others[0].to, "{case}");
                assert_eq!(ep.tries, 0, "{case}");
                assert!(osaka.beats.is_empty(), "{case}: {:?}", osaka.beats);
            } else {
                assert_eq!(osaka.carrying(), None, "{case}");
                assert_eq!(
                    osaka.beats.last(),
                    Some(&mind::Beat {
                        loss: mind::Loss::Moved(Furniture::Sofa),
                        toward: home
                    }),
                    "{case}"
                );
            }
        }
    }
}

/// Set down in the first of two spots she means to try, she'd lift it
/// for the other; but that one's gone (text came there): she keeps it
/// where it is, as she would have chosen to ("There!"), and owes no
/// glance.
#[test]
fn every_other_spot_gone_she_keeps_it_where_it_is() {
    for graphics in [false, true] {
        let (mut osaka, terrain, repair, others, mut rng) = pocket_on_a_floor(graphics);
        // The dearer first (a whim's order), the cheapest after.
        let repair = rules::Repair {
            cost: others[0].cost + rules::TIE_CELLS,
            ..repair
        };
        let second = others[0];
        osaka.lift(
            scenes::Lift {
                repair,
                trials: rules::Trials::of(&[repair, second]),
                x: 10,
                y: 10,
                side: scenes::Side::Right,
            },
            0,
        );
        // Calm: the dearer spot she'd seldom keep it in.
        osaka.needs_mut().serve(super::brain::Need::Restless, 1.0);
        let judged = |osaka: &Osaka| osaka::Chances {
            judged: osaka.episode().map(|ep| osaka::Judged {
                piece: ep.repair.piece,
                to: ep.repair.to,
                pocket: ep.pocket,
                // Only the first spot (to lift it, and set it down there).
                holds: ep.tried == 0,
                spot: Some(((10, 10), scenes::Side::Right)),
                home: Some((35, 10)),
            }),
            ..osaka::Chances::default()
        };
        let mut now = 0;
        let mut set = false;
        let kept = loop {
            assert!(now < 60_000, "graphics {graphics}: still at it");
            now += 50;
            osaka.tick(now, None, &terrain, &judged(&osaka), &mut rng);
            if !set && osaka.episode().is_some_and(|e| e.set_down) {
                // The frame takes it.
                osaka.set_down_done(Furniture::Sofa, now);
                set = true;
            }
            if set && osaka.episode().is_none() {
                break now;
            }
        };
        assert_eq!(
            osaka.retried, 1,
            "graphics {graphics}: meant to try another"
        );
        assert_eq!(
            osaka.appearance(kept).2,
            Some(osaka::Bubble::Say("There!")),
            "graphics {graphics}"
        );
        assert_eq!(osaka.beats, [], "graphics {graphics}");
        assert_eq!(osaka.home_acts(), 1);
    }
}

/// Text comes up over the sofa as she bends to lift it, and stays: she
/// can't get to it, and after a few tries she lets the move go, with a
/// glance. Let go, it stays as it is this visit: she doesn't set about
/// it again once it's back, however keen on her home.
#[test]
fn a_piece_closeted_for_good_is_let_go() {
    for graphics in [false, true] {
        let mut ui = stage_ui();
        let (real, view) = real_frame(&mut ui, 100, 30);
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::Arrange);
        paint(&mut guest, &real, &view, 0);
        let lifting = carry_until(&mut guest, &real, &view, 0, 30_000, |guest, _| {
            visit_of(guest).osaka.lifting().is_some()
        })
        .unwrap_or_else(|| panic!("graphics {graphics}: never bent to it"));
        let sofa = prop_of(&guest, Furniture::Sofa);
        let at = visit_of(&guest)
            .shown
            .iter()
            .find(|s| s.item == Furniture::Sofa)
            .copied()
            .unwrap();
        let mut covered = view.clone();
        covered.protected.push(at.rect());
        let let_go = carry_until(
            &mut guest,
            &real,
            &covered,
            lifting,
            lifting + 60_000,
            |guest, _| visit_of(guest).osaka.episode().is_none(),
        )
        .unwrap_or_else(|| {
            panic!(
                "graphics {graphics}: still means to after a minute: {:?}",
                methods(&guest)
            )
        });
        assert_eq!(carried(&guest), None);
        assert_eq!(prop_of(&guest, Furniture::Sofa), sofa);
        assert!(
            visit_of(&guest)
                .osaka
                .beats
                .iter()
                .any(|b| b.loss == mind::Loss::Heading),
            "graphics {graphics}: a glance where she was heading"
        );
        let lifts = |guest: &Guest| {
            methods(guest)
                .iter()
                .filter(|&&m| m == "arrange/lift")
                .count()
        };
        let before = lifts(&guest);
        // However keen on her home she is.
        for from in (let_go..let_go + 120_000).step_by(10_000) {
            guest.press(stage::Want::Nesting);
            let _ = carry_until(&mut guest, &real, &view, from, from + 10_000, |_, _| false);
        }
        assert_eq!(lifts(&guest), before, "graphics {graphics}: at it again");
        assert_eq!(visit_of(&guest).osaka.episode(), None);
        assert_eq!(prop_of(&guest, Furniture::Sofa), sofa);
    }
}

/// Industrious, in [`wrong_home`] `home`, with nothing resized or
/// delivered: she feels what's wrong (sent to use her pieces while
/// something's unfelt and she's not busy about her home) and puts her
/// home right a piece at a time, and never moves a piece twice (a move
/// mends its rule and breaks none, so nothing she has set down needs
/// moving again). Home 5 can't be mended without breaking another rule:
/// she leaves it as it is.
fn moves_each_piece_once(home: usize) {
    use super::brain::Mood;
    let mendable = home != 5;
    for graphics in [false, true] {
        for seed in 0..2 {
            let until = if mendable { 900_000 } else { 300_000 };
            let (guest, moved) = keen_on(home, Mood::Industrious, graphics, seed, until);
            if mendable {
                assert!(
                    !moved.is_empty() && guest.broken().is_empty(),
                    "home {home}, graphics {graphics}, seed {seed}: moved {moved:?}, broken {}: {:?}",
                    guest.broken(),
                    methods(&guest)
                );
            } else {
                assert_eq!(moved, [], "home {home}, graphics {graphics}, seed {seed}");
                assert_eq!(
                    guest.broken(),
                    "apart(bed,TV)*",
                    "graphics {graphics}, seed {seed}"
                );
            }
        }
    }
}

/// In [`wrong_home`] `home`, in `mood`, with nothing resized or
/// delivered, keen on her home (nesting pressed) and sent to use her
/// pieces while something's unfelt and she's not busy about her home,
/// until all's right or `until`: her, and the pieces she moved, in
/// order, each once (checked as she goes; trying one in another spot is
/// moving it as the same one thing).
fn keen_on(
    home: usize,
    mood: super::brain::Mood,
    graphics: bool,
    seed: u64,
    until: u64,
) -> (Guest, Vec<Furniture>) {
    use super::room::{Anchor, Prop};
    let mut guest = Guest::new(seed);
    if graphics {
        guest.set_picker(kitty());
    }
    for (item, nook, side, offset, facing, settled) in wrong_home(home, 12) {
        assert!(guest.ledger.home.add(Prop {
            anchor: Some(Anchor { side, offset }),
            settled,
            ..Prop::new(item, nook, 0, facing)
        }));
    }
    let real = wordy_rooms(100, 30);
    let panes = nooks(100, 30);
    let view = IdleView {
        chat: panes[0].1,
        nooks: panes[1..].to_vec(),
        ..view(bottom_strip(100, 30))
    };
    let at = |guest: &Guest| -> Vec<(Furniture, room::Strip, Option<Anchor>, sprite::Facing)> {
        guest
            .ledger
            .home
            .props
            .iter()
            .map(|p| (p.item, p.strip, p.anchor, p.facing))
            .collect()
    };
    // (Only uses of what she owns: the stage would bring the rest.)
    let owns = |item| guest.ledger.home.owns(item);
    let uses: Vec<Scene> = [
        (Scene::Sleep, Furniture::Bed),
        (Scene::Watch, Furniture::Tv),
        (Scene::Lounge, Furniture::Sofa),
        (Scene::Snack, Furniture::Fridge),
        (Scene::Read, Furniture::Bookshelf),
    ]
    .into_iter()
    .filter(|&(_, item)| owns(item))
    .map(|(scene, _)| scene)
    .collect();
    let mut scenes = uses.into_iter().cycle();
    guest.cue(scenes.next().unwrap());
    paint(&mut guest, &real, &view, 0);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.set_mood(mood);
    guest.press(stage::Want::Nesting);
    let mut moved: Vec<Furniture> = Vec::new();
    let mut was = at(&guest);
    let mut now = 0;
    let mut next_cue = 60_000;
    while now < until && !guest.broken().is_empty() {
        let unfelt = guest
            .broken()
            .split(", ")
            .any(|b| !b.is_empty() && !b.ends_with('*'));
        if unfelt && now >= next_cue && visit_of(&guest).osaka.episode().is_none() {
            guest.cue(scenes.next().unwrap());
            guest.press(stage::Want::Nesting);
            next_cue = now + 60_000;
        }
        let acts = visit_of(&guest).osaka.home_acts();
        now = carry_until(&mut guest, &real, &view, now, now + 1, |_, _| true).unwrap_or(now + 1);
        let is = at(&guest);
        let new_act = visit_of(&guest).osaka.home_acts() > acts;
        for (a, b) in was.iter().zip(&is) {
            if a != b {
                // Trying it in another spot is the same one thing (no
                // new home act), and the piece she moved last.
                let trial = !new_act && moved.last() == Some(&a.0);
                assert!(
                    trial || !moved.contains(&a.0),
                    "home {home}, {mood:?}, graphics {graphics}, seed {seed}, {now}: {:?} moved twice: {a:?} → {b:?}",
                    a.0
                );
                if !trial {
                    moved.push(a.0);
                }
            }
        }
        assert_eq!(was.len(), is.len(), "nothing delivered: {is:?}");
        was = is;
    }
    (guest, moved)
}

/// Her lamp far from her bed, a TV she never settled in her bedroom,
/// her sofa turned from where the TV goes: she moves the lamp and the
/// TV once each, and all's right.
#[test]
fn no_piece_is_moved_twice() {
    moves_each_piece_once(3);
}

/// A fridge she never settled, in her bedroom away from its walls: one
/// move puts it against a wall where it makes a room.
#[test]
fn an_unsettled_fridge_is_moved_once() {
    moves_each_piece_once(4);
}

/// Her bed beside the TV she watches from her sofa, with her lamp by
/// it, her desk upstairs: feeling it, she still leaves it as it is (the
/// bed would go up to the desk, a bedroom then, but not from its lamp).
#[test]
fn what_she_cant_mend_she_leaves() {
    moves_each_piece_once(5);
}

/// Her sofa upstairs in her bedroom, her TV downstairs with her desk:
/// she takes the sofa down to the TV, and the study becomes her living
/// room (the TV can't come up: a bed and a TV make a den).
#[test]
fn the_sofa_joins_the_tv_in_her_study() {
    use super::brain::Mood;
    for graphics in [false, true] {
        for seed in 0..2 {
            let (guest, moved) = keen_on(7, Mood::Industrious, graphics, seed, 900_000);
            let case = format!("graphics {graphics}, seed {seed}");
            assert_eq!(moved, [Furniture::Sofa], "{case}: {:?}", methods(&guest));
            assert_eq!(guest.broken(), "", "{case}");
            let sofa = prop_of(&guest, Furniture::Sofa);
            assert_eq!(sofa.strip, room::Strip::Bottom(Nook::Playlist), "{case}");
        }
    }
}

/// A rule she can't mend doesn't keep her from one she can: her bed in
/// the living room, felt first (no move mends it), and her bookshelf
/// upstairs away from its walls, felt after: she puts the bookshelf
/// against a wall, and leaves the bed as it is.
#[test]
fn what_she_cant_mend_doesnt_stop_what_she_can() {
    use super::brain::Mood;
    for graphics in [false, true] {
        for seed in 0..2 {
            let (guest, moved) = keen_on(6, Mood::Industrious, graphics, seed, 600_000);
            let case = format!("graphics {graphics}, seed {seed}");
            let felt: Vec<String> = felt(&guest).iter().map(|g| g.label()).collect();
            assert_eq!(
                felt,
                ["apart(bed)", "wall(bookshelf)"],
                "{case}: what she felt, in order"
            );
            assert_eq!(moved, [Furniture::Bookshelf], "{case}");
            assert_eq!(guest.broken(), "apart(bed,TV)*", "{case}");
        }
    }
}

/// Her mood caps what she does about her home in a visit: in a home
/// with three things wrong with it (her lamp far from her bed, a TV she
/// never settled in her bedroom, her sofa turned from where the TV
/// goes), ordinary or dreamy she puts one right and leaves the rest for
/// ten minutes, however keen, though she has felt it; industrious, she
/// goes on to another (in [`keen_on`], nothing resized or delivered).
fn her_mood_caps_her_home_acts(mood: super::brain::Mood) {
    use super::brain::Mood;
    for graphics in [false, true] {
        for seed in 0..2 {
            // (Industrious, the second act can take her twelve minutes.)
            let until = if mood == Mood::Industrious {
                900_000
            } else {
                600_000
            };
            let (guest, moved) = keen_on(3, mood, graphics, seed, until);
            let acts = visit_of(&guest).osaka.home_acts();
            let broken = guest.broken();
            let case = format!(
                "{mood:?}, graphics {graphics}, seed {seed}: moved {moved:?}, broken {broken}"
            );
            if mood == Mood::Industrious {
                assert!(acts >= 2, "{case}: {acts} home acts");
            } else {
                assert_eq!(acts, 1, "{case}");
                assert!(
                    broken.split(", ").any(|b| b.ends_with('*')),
                    "{case}: nothing she felt left broken"
                );
            }
        }
    }
}

#[test]
fn ordinary_she_puts_one_thing_right() {
    her_mood_caps_her_home_acts(super::brain::Mood::Ordinary);
}

#[test]
fn dreamy_she_puts_one_thing_right() {
    her_mood_caps_her_home_acts(super::brain::Mood::Dreamy);
}

#[test]
fn industrious_she_goes_on_to_another() {
    her_mood_caps_her_home_acts(super::brain::Mood::Industrious);
}

/// A home with something wrong with it, on [`rooms`]' panes (her
/// bedroom upstairs in Users, her living room downstairs in Playlist):
/// 0, her sofa turned from the TV (`offset` along); 1, a TV she never
/// settled in her bedroom ([`two_rooms`]); 2, her lamp far from her bed;
/// 3, all of those at once; 4, a fridge she never settled, in her
/// bedroom away from its walls; 5, her bed in the living room, where she
/// watches the TV from her sofa, with her lamp beside it, and her desk
/// upstairs (no move mends that without breaking another rule: the bed
/// leaves its lamp, the TV her sofa); 6, that, and her bookshelf
/// upstairs away from its walls; 7, her sofa upstairs in her bedroom,
/// and her TV downstairs with her desk. Each piece anchored, as she'd
/// have left it.
fn wrong_home(
    home: usize,
    offset: u16,
) -> Vec<(Furniture, Nook, room::Side, u16, sprite::Facing, bool)> {
    use super::room::Side::{Left, Right};
    use sprite::Facing;
    let bed = (Furniture::Bed, Nook::Users, Left, 1, Facing::Right, true);
    match home {
        0 => vec![
            bed,
            (Furniture::Tv, Nook::Playlist, Left, 0, Facing::Right, true),
            (
                Furniture::Sofa,
                Nook::Playlist,
                Left,
                offset,
                Facing::Right,
                true,
            ),
        ],
        1 => vec![
            bed,
            (Furniture::Tv, Nook::Users, Right, 1, Facing::Left, false),
            (
                Furniture::Sofa,
                Nook::Playlist,
                Left,
                2,
                Facing::Right,
                true,
            ),
        ],
        2 => vec![
            bed,
            (Furniture::Lamp, Nook::Users, Right, 1, Facing::Left, true),
            (
                Furniture::Sofa,
                Nook::Playlist,
                Left,
                2,
                Facing::Right,
                true,
            ),
        ],
        4 => vec![
            bed,
            (
                Furniture::Fridge,
                Nook::Users,
                Left,
                20,
                Facing::Left,
                false,
            ),
            (
                Furniture::Sofa,
                Nook::Playlist,
                Left,
                2,
                Facing::Right,
                true,
            ),
        ],
        5 => vec![
            (Furniture::Tv, Nook::Playlist, Left, 0, Facing::Right, true),
            (
                Furniture::Sofa,
                Nook::Playlist,
                Left,
                offset,
                Facing::Left,
                true,
            ),
            (Furniture::Bed, Nook::Playlist, Right, 1, Facing::Left, true),
            (
                Furniture::Lamp,
                Nook::Playlist,
                Right,
                11,
                Facing::Left,
                true,
            ),
            (Furniture::Desk, Nook::Users, Right, 1, Facing::Left, true),
        ],
        6 => {
            let mut home = wrong_home(5, offset);
            home.push((
                Furniture::Bookshelf,
                Nook::Users,
                Left,
                12,
                Facing::Right,
                true,
            ));
            home
        }
        7 => vec![
            bed,
            (Furniture::Sofa, Nook::Users, Right, 2, Facing::Right, true),
            (Furniture::Tv, Nook::Playlist, Left, 0, Facing::Right, true),
            (
                Furniture::Desk,
                Nook::Playlist,
                Right,
                1,
                Facing::Left,
                true,
            ),
        ],
        _ => vec![
            bed,
            (Furniture::Lamp, Nook::Users, Right, 1, Facing::Left, true),
            (Furniture::Tv, Nook::Users, Right, 8, Facing::Left, false),
            (
                Furniture::Sofa,
                Nook::Playlist,
                Left,
                offset,
                Facing::Right,
                true,
            ),
        ],
    }
}

/// [`rooms`] at `w`×`h` with the chat full of text (as much as fits)
/// and a line at the top of each quiet pane.
fn wordy_rooms(w: u16, h: u16) -> Buffer {
    let mut real = rooms(w, h);
    let mut text: Vec<(u16, u16, String)> = (0..12)
        .map(|i| {
            (
                2 + i * 5 % 20,
                2 + i * 2,
                "so what did you think of it".to_owned(),
            )
        })
        .filter(|&(_, y, _)| y + 3 < h)
        .collect();
    for (_, pane) in &nooks(w, h)[1..] {
        text.push((pane.x + 2, pane.y + 1, "01 Frieren ep 12".to_owned()));
    }
    scatter(&mut real, &text, &[]);
    real
}

/// The rules of her home `home` breaks, laid out on `nooks`.
fn breaks(home: &room::Home, nooks: &[(Nook, Rect)]) -> Vec<rules::Grievance> {
    let mut home = home.clone();
    let laid = home.layout(nooks);
    rules::broken(&laid, &room::strips(nooks), &home)
        .into_iter()
        .map(|b| b.key)
        .collect()
}

/// What goes on around her in a promise run: the sizes the terminal
/// takes, each from when (the first from the start, at its size), chat
/// lines landing, and (a resident) the pane focused, from when.
struct Weather {
    resident: bool,
    sizes: Vec<(u64, (u16, u16))>,
    chats: Vec<u64>,
    focuses: Vec<(u64, Option<usize>)>,
}

impl Weather {
    /// The view at `size` with `focus` (resident only) and `mark`.
    fn view(&self, (w, h): (u16, u16), focus: Option<usize>, mark: ChatMark) -> IdleView {
        let panes = nooks(w, h);
        let view = if self.resident {
            resident_view(w, h, focus.map(|i| panes[i].1))
        } else {
            IdleView {
                chat: panes[0].1,
                nooks: panes[1..].to_vec(),
                ..view(bottom_strip(w, h))
            }
        };
        IdleView {
            chat_mark: mark,
            ..view
        }
    }

    /// The size the terminal has at `now` (from the size whose time has
    /// passed), and the next time it changes.
    fn size(&self, now: u64) -> ((u16, u16), Option<u64>) {
        let at = self
            .sizes
            .iter()
            .rposition(|&(from, _)| from < now)
            .unwrap_or(0);
        (
            self.sizes[at].1,
            self.sizes.get(at + 1).map(|&(from, _)| from),
        )
    }
}

/// Her run from `from` to `until` as the shell would, through
/// `weather`, each frame held to every promise a carry keeps
/// ([`promised_frame`]) and to `each` after it (with the piece a
/// set-down that frame committed; false ends the run there). Where the
/// run ended up: when, its frame, the view, and the chat's mark.
fn keep_promises(
    guest: &mut Guest,
    weather: &Weather,
    graphics: bool,
    mut mark: ChatMark,
    from: u64,
    until: u64,
    mut each: impl FnMut(&mut Guest, u64, Option<Furniture>) -> Result<bool, TestCaseError>,
) -> Result<(u64, Buffer, IdleView, ChatMark), TestCaseError> {
    let mut hidden = Hidden::default();
    let mut last = weather.size(from).0;
    let mut real = wordy_rooms(last.0, last.1);
    let mut view = weather.view(last, None, mark);
    let mut now = from;
    while now < until {
        // (No tick runs past a change of size.)
        let (_, change) = weather.size(now + 1);
        let bound = change.filter(|&c| c > now).unwrap_or(until).min(until);
        let step = guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 1000.min(bound - now));
        now += step;
        // The first frame at a new size moves pieces off strips gone
        // too small before it takes a set-down: judged as it was, the
        // piece set down might be on one of those.
        let (size, _) = weather.size(now);
        let fresh = size != last;
        if fresh {
            real = wordy_rooms(size.0, size.1);
            last = size;
        }
        if weather.chats.iter().any(|&c| c <= now && c > now - step) {
            mark.synced += 1;
        }
        let focus = weather
            .focuses
            .iter()
            .rev()
            .find(|(at, _)| weather.resident && *at <= now)
            .and_then(|&(_, pane)| pane);
        view = weather.view(size, focus, mark);
        let committed = promised_frame(guest, &real, &view, now, fresh, graphics, &mut hidden)?;
        if !each(guest, now, committed)? {
            break;
        }
    }
    Ok((now, real, view, mark))
}

/// One frame as the shell would at `now` (`fresh`: the first at a new
/// size), held to every promise a carry keeps: nothing she carries is
/// drawn; nothing she does touches what's protected or stays over text;
/// a home act comes only with a set-down; and a piece she sets down
/// mends the rule she moved it for (from broken, for a new home act;
/// tried in another spot, it keeps it mended) and breaks none that
/// held. The piece a set-down committed, if one did.
fn promised_frame(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    now: u64,
    fresh: bool,
    graphics: bool,
    hidden: &mut Hidden,
) -> Result<Option<Furniture>, TestCaseError> {
    guest.advance(now);
    // A set-down the frame is about to take, and her home as it stands
    // before it does.
    let (acts, pending) = {
        let visit = visit_of(guest);
        let key = visit.osaka.episode().map(|e| e.repair.key);
        (visit.osaka.home_acts(), visit.set_down.zip(key))
    };
    let before = guest.ledger.home.clone();
    let frame = paint(guest, real, view, now);
    carried_unseen(guest, &frame, real, view);
    let visit = visit_of(guest);
    assert_untouched_but_feet(&frame, real, &view.protected, feet(visit))?;
    // (A pane just focused rains out what of hers was in it.)
    if graphics && visit.fades.is_empty() {
        let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
        hidden.check(&frame, real, &layer, now)?;
    }
    let new_act = visit.osaka.home_acts() > acts;
    let after = guest.ledger.home.clone();
    // A set-down the frame took: the piece stands elsewhere. (Trying it
    // in another spot is no new home act.)
    let prop = |home: &room::Home, item| home.props.iter().find(|p| p.item == item).copied();
    let committed = pending.filter(|&((piece, _), _)| prop(&before, piece) != prop(&after, piece));
    if fresh {
        return Ok(None);
    }
    prop_assert!(
        !new_act || committed.is_some(),
        "{}: a home act with nothing set down",
        now
    );
    // She set a piece down: judged on this frame's home, with only that
    // piece as it was, and as she set it.
    let Some(((piece, _), key)) = committed else {
        return Ok(None);
    };
    let mut was = after.clone();
    was.props
        .retain(|p| before.props.iter().any(|b| b.item == p.item));
    let is = was.clone();
    if let (Some(p), Some(b)) = (
        was.props.iter_mut().find(|p| p.item == piece),
        before.props.iter().find(|b| b.item == piece),
    ) {
        *p = *b;
    }
    let (then, now_broken) = (breaks(&was, &view.nooks), breaks(&is, &view.nooks));
    // (Tried in another spot, it was right where it stood.)
    prop_assert!(
        !new_act || then.contains(&key),
        "{now}: {key:?} wasn't broken: {then:?}"
    );
    prop_assert!(
        !now_broken.contains(&key),
        "{now}: set down, {key:?} still broken"
    );
    for k in &now_broken {
        prop_assert!(
            then.contains(k),
            "{now}: setting {piece:?} down broke {k:?}"
        );
    }
    Ok(Some(piece))
}

/// Where in trying the sofa a trial run starts its weather.
#[derive(Clone, Copy, Debug)]
enum TrialStage {
    /// Set down in the first spot she tries, before she's made up her
    /// mind about it: sitting on it, then keeping it or not.
    Sitting,
    /// Lifted again for another spot: carrying it there and setting it
    /// down.
    Relifted,
}

impl TrialStage {
    fn reached(self, ep: &osaka::Episode) -> bool {
        match self {
            TrialStage::Sitting => ep.trying && ep.tried == 1,
            TrialStage::Relifted => ep.tried > 0 && ep.pocket,
        }
    }
}

/// [`sofa_to_try`] in `weather`'s first view, her calm, run to `stage`
/// of trying it: the first of `seed..` that gets there in ten minutes,
/// and then; how many spots she's set it down in so far. (Every seed
/// she doesn't try it in another spot for, she keeps it in the first.)
fn sofa_tried_to(
    graphics: bool,
    seed: u64,
    weather: &Weather,
    stage: TrialStage,
) -> (Guest, u64, usize) {
    let view = weather.view(weather.sizes[0].1, None, ChatMark::default());
    for seed in seed..seed + 16 {
        let (mut guest, real) = sofa_to_try_in(graphics, seed, &view);
        let mut spots = 0;
        let mut was = prop_of(&guest, Furniture::Sofa);
        let mut now = 0;
        while now < 600_000 {
            // (As the shell would, its ticks unclamped: quicker than
            // [`calm_tick`].)
            let State::Visiting(visit) = &mut guest.state else {
                panic!("visiting");
            };
            visit
                .osaka
                .needs_mut()
                .serve(super::brain::Need::Restless, 1.0);
            now += guest
                .next_tick(now)
                .map_or(100, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            if prop_of(&guest, Furniture::Sofa) != was {
                was = prop_of(&guest, Furniture::Sofa);
                spots += 1;
            }
            let osaka = &visit_of(&guest).osaka;
            if osaka.episode().is_some_and(|e| stage.reached(e)) {
                return (guest, now, spots);
            }
            if osaka.home_acts() > 0 && osaka.episode().is_none() {
                break;
            }
        }
    }
    panic!("graphics {graphics}: no seed from {seed} reached {stage:?}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(8)))]

    /// A lively chat (a line every four or five seconds) while she's out
    /// on text-dense floors never keeps her over text: where her image
    /// hides text she doesn't stop to look (only text coming up under her
    /// startles her there, and briefly), and no text stays hidden behind
    /// her for long; she goes on somewhere calm and watches it from there.
    #[test]
    fn a_lively_chat_never_keeps_her_over_text(
        seed in any::<u64>(),
        first in 0u64..6_000,
        gap in 4_000u64..5_000,
        resident in any::<bool>(),
    ) {
        for graphics in [false, true] {
            let mut guest = Guest::new(seed);
            if graphics {
                guest.set_picker(kitty());
            }
            let weather = Weather {
                resident,
                sizes: vec![(0, (100, 30))],
                chats: (first..40_000).step_by(gap as usize).collect(),
                focuses: Vec::new(),
            };
            let real = wordy_rooms(100, 30);
            guest.cue(Scene::Arrive);
            let _ = paint(&mut guest, &real, &weather.view((100, 30), None, ChatMark::default()), 0);
            let mut over: Option<((i32, i32), u64)> = None;
            let mut watched = false;
            keep_promises(
                &mut guest,
                &weather,
                graphics,
                ChatMark::default(),
                0,
                45_000,
                |guest, now, _| {
                    let visit = visit_of(guest);
                    let osaka = &visit.osaka;
                    let here = (osaka.x, osaka.y);
                    if osaka.looking() && !visit.terrain.restful(here.0, here.1) {
                        let since = match over {
                            Some((at, since)) if at == here => since,
                            _ => now,
                        };
                        over = Some((here, since));
                        prop_assert!(
                            now - since <= super::osaka::LOOK_MS / 2,
                            "graphics {}: looking over text at {:?} since {} (now {})",
                            graphics,
                            here,
                            since,
                            now
                        );
                    } else {
                        over = None;
                    }
                    watched |= osaka
                        .decisions
                        .iter()
                        .any(|d| d.method == "watching chat");
                    Ok(true)
                },
            )?;
            prop_assert!(watched, "graphics {}: she never watched the chat", graphics);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(8)))]

    /// Keen on her home from the start (nesting pressed), a home with
    /// something wrong with it, in any mood, through chat, resizes and
    /// (a resident) focus changes: she does no more about her home than
    /// her mood allows (lazy, nothing; a piece tried in a few spots is
    /// one thing); every piece she sets down mends the rule she moved it
    /// for (or, tried elsewhere, keeps it mended) and breaks none that
    /// held; nothing she carries is drawn; nothing she does touches
    /// what's protected or stays over text; and a visitor's goodbye
    /// lands on the real frame. (Two minutes rarely hold a second act:
    /// that no piece moves twice, and that her mood's cap above none
    /// holds, are `moves_each_piece_once`'s and
    /// `her_mood_caps_her_home_acts`'. Few of these visits try a piece
    /// in a second spot: `a_trial_keeps_every_promise` starts each in
    /// the middle of one.)
    #[test]
    fn the_carry_keeps_every_promise(
        seed in any::<u64>(),
        graphics in any::<bool>(),
        home in 0usize..8,
        offset in 8u16..20,
        mood in 0usize..4,
        cue in any::<bool>(),
        resident in any::<bool>(),
        sizes in proptest::collection::vec(
            proptest::sample::select(vec![(80u16, 24u16), (100, 30), (120, 36)]),
            0..3,
        ),
        chats in proptest::collection::vec(0u64..120_000, 0..4),
        focuses in proptest::collection::vec((0u64..120_000, proptest::option::of(0usize..3)), 0..4),
    ) {
        use super::brain::Mood;
        use super::room::{Anchor, Prop};
        let mut guest = Guest::new(seed);
        if graphics {
            guest.set_picker(kitty());
        }
        for (item, nook, side, offset, facing, settled) in wrong_home(home, offset) {
            let added = guest.ledger.home.add(Prop {
                anchor: Some(Anchor { side, offset }),
                settled,
                ..Prop::new(item, nook, 0, facing)
            });
            prop_assert!(added);
        }
        let sizes: Vec<(u16, u16)> = std::iter::once((100, 30)).chain(sizes).collect();
        let span = 120_000 / sizes.len() as u64;
        let mut focuses = focuses;
        focuses.sort_unstable();
        let weather = Weather {
            resident,
            sizes: sizes.iter().enumerate().map(|(i, &s)| (span * i as u64, s)).collect(),
            chats,
            focuses,
        };
        // What she'd use to feel what's wrong, or just her arrival.
        let scene = match (cue, home) {
            (false, _) => Scene::Arrive,
            (true, 0 | 7) => Scene::Lounge,
            (true, 1) => Scene::Watch,
            (true, 4) => Scene::Snack,
            (true, _) => Scene::Sleep,
        };
        let real = wordy_rooms(100, 30);
        guest.cue(scene);
        let _ = paint(&mut guest, &real, &weather.view(sizes[0], None, ChatMark::default()), 0);
        let mood = Mood::ALL[mood];
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.set_mood(mood);
        guest.press(stage::Want::Nesting);
        let (now, real, _, mark) = keep_promises(
            &mut guest,
            &weather,
            graphics,
            ChatMark::default(),
            0,
            120_000,
            |guest, _, _| {
                let acts = visit_of(guest).osaka.home_acts();
                prop_assert!(acts <= mood.home_acts(), "{:?}", mood);
                Ok(true)
            },
        )?;
        if mood == Mood::Lazy {
            let ways = methods(&guest);
            prop_assert!(!ways.iter().any(|m| m.starts_with("arrange/")), "{:?}", ways);
        }
        if !resident {
            let view = weather.view(*sizes.last().unwrap(), None, mark);
            guest.activity(now);
            let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
            prop_assert!(!guest.present());
            prop_assert!(end == real, "the rain restores the frame");
        }
    }

    /// [`sofa_to_try`], her calm, caught trying the sofa in a spot
    /// (sitting on it, before she's made up her mind) or lifted again for
    /// another, and then, within the moments it takes, chat, resizes and
    /// (a resident) focus changes: every promise a carry keeps holds
    /// ([`promised_frame`]); however many spots she sets it down in (three
    /// at most), it's one thing about her home; once she's kept it, it
    /// doesn't move again; and a visitor's goodbye lands on the real
    /// frame, the sofa where she last set it down.
    #[test]
    fn a_trial_keeps_every_promise(
        seed in 0u64..64,
        graphics in any::<bool>(),
        relifted in any::<bool>(),
        resident in any::<bool>(),
        sizes in proptest::collection::vec(
            (0u64..20_000, proptest::sample::select(vec![(80u16, 24u16), (100, 30), (120, 36)])),
            0..3,
        ),
        chats in proptest::collection::vec(0u64..20_000, 0..4),
        focuses in proptest::collection::vec((0u64..20_000, proptest::option::of(0usize..3)), 0..4),
    ) {
        let stage = if relifted { TrialStage::Relifted } else { TrialStage::Sitting };
        let mut sizes = sizes;
        sizes.sort_unstable();
        let mut focuses = focuses;
        focuses.sort_unstable();
        // Its weather as from the start, to run her there; then as from
        // when she got there.
        let mut weather = Weather {
            resident,
            sizes: std::iter::once((0, (100, 30))).chain(sizes).collect(),
            chats,
            focuses,
        };
        let (mut guest, start, mut spots) = sofa_tried_to(graphics, seed, &weather, stage);
        weather.sizes[1..].iter_mut().for_each(|(at, _)| *at += start + 1);
        weather.chats.iter_mut().for_each(|at| *at += start + 1);
        weather.focuses.iter_mut().for_each(|(at, _)| *at += start + 1);
        // Run on through the weather, and a while once she's kept it.
        let mut kept = None;
        let (end, real, view, _) = keep_promises(
            &mut guest,
            &weather,
            graphics,
            ChatMark::default(),
            start,
            start + 120_000,
            |guest, now, committed| {
                if committed == Some(Furniture::Sofa) {
                    prop_assert!(kept.is_none(), "{}: moved again once kept", now);
                    spots += 1;
                }
                let State::Visiting(visit) = &mut guest.state else {
                    panic!("visiting");
                };
                visit.osaka.needs_mut().serve(super::brain::Need::Restless, 1.0);
                prop_assert_eq!(visit.osaka.home_acts(), 1, "{}", now);
                if visit.osaka.episode().is_none() {
                    kept.get_or_insert(now);
                }
                Ok(kept.is_none_or(|at| now < (at + 15_000).max(start + 20_000)))
            },
        )?;
        prop_assert!((1..=3).contains(&spots), "spots {}", spots);
        prop_assert!(kept.is_some(), "{:?} still trying it two minutes on", stage);
        if !resident {
            let set = prop_of(&guest, Furniture::Sofa);
            guest.activity(end);
            let last = run(&mut guest, &real, &view, end, end + dissolve::DURATION_MS);
            prop_assert!(!guest.present());
            prop_assert!(last == real, "the rain restores the frame");
            prop_assert_eq!(prop_of(&guest, Furniture::Sofa), set);
        }
    }
}

/// Her, arrived in `weather`'s first view with a sofa, a TV, a bed and
/// a desk, `splice` cued, run (calm) until she's seen in the first use
/// of her own choosing it wraps (the cue waits through any it can't),
/// within three minutes: the guest, when, and the use's span (since,
/// the body's start and end, until); `None` if she started no use it
/// wraps. (The frame never changes, so it's painted once a second
/// until she's using something, which is all she needs to see it, and
/// keeps this cheap: painting every tick was most of the property's
/// time.)
fn spliced_use(
    graphics: bool,
    seed: u64,
    weather: &Weather,
    splice: script::SpliceId,
) -> Option<(Guest, u64, room::Use, [u64; 4])> {
    let view = weather.view(weather.sizes[0].1, None, ChatMark::default());
    let real = wordy_rooms(100, 30);
    let mut guest = Guest::new(seed);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    for item in [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Bed,
        Furniture::Desk,
    ] {
        guest.give(item);
        paint(&mut guest, &real, &view, 0);
    }
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.cue(Some(script::Cue::Splice(splice, None)));
    let mut now = 0;
    while now < 180_000 {
        let then = now;
        now += guest
            .next_tick(now)
            .map_or(100, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        guest.advance(now);
        if visit_of(&guest).osaka.use_span().is_some() || now / 1000 != then / 1000 {
            paint(&mut guest, &real, &view, now);
        }
        let osaka = &visit_of(&guest).osaka;
        if let Some((seat, since, until)) = osaka.use_span()
            && let Some(play) = osaka.plays()
            && play.before.or(play.after).is_some()
        {
            let span = [
                since,
                play.body_start(since),
                play.body_end(since, until),
                until,
            ];
            return Some((guest, now, seat.what, span));
        }
    }
    None
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(8)))]

    /// A use of her own choosing wrapped in a prelude or a coda, and
    /// chat, resizes and (a resident) focus changes landing inside it:
    /// she's credited the share of the body she did, so nothing at all
    /// for a use left in its prelude and the whole of it for one left in
    /// its coda; and every promise a frame keeps holds throughout, till
    /// a while after the splice ([`promised_frame`]: nothing she does
    /// touches what's protected or stays over text). Budgeted for the
    /// deep pass: about 4 s at 32 cases (most of it the warm-up to a
    /// use), so 256 stay well inside the gate's minute.
    #[test]
    fn weather_in_a_splice_credits_only_the_body(
        seed in 0u64..64,
        graphics in any::<bool>(),
        coda in any::<bool>(),
        resident in any::<bool>(),
        size in proptest::option::of(
            (0.0f64..1.0, proptest::sample::select(vec![(80u16, 24u16), (120, 36)])),
        ),
        chats in proptest::collection::vec(0.0f64..1.0, 0..3),
        focuses in proptest::collection::vec((0.0f64..1.0, proptest::option::of(0usize..3)), 0..3),
    ) {
        use script::SpliceId;
        let splice = if coda { SpliceId::TestSleep } else { SpliceId::TestSnack };
        let mut weather = Weather {
            resident,
            sizes: vec![(0, (100, 30))],
            chats: Vec::new(),
            focuses: Vec::new(),
        };
        let found = spliced_use(graphics, seed, &weather, splice);
        prop_assume!(found.is_some());
        let Some((mut guest, start, what, [since, body_start, body_end, until])) = found else {
            unreachable!();
        };
        // Inside the splice (from the frame she was seen starting it).
        let (from, to) = if coda { (body_end, until) } else { (start + 1, body_start) };
        prop_assume!(from < to);
        let inside = |f: f64| from + ((to - from) as f64 * f) as u64;
        weather.sizes.extend(size.map(|(f, s)| (inside(f), s)));
        weather.chats = chats.iter().map(|&f| inside(f)).collect();
        let mut focuses: Vec<(u64, Option<usize>)> =
            focuses.iter().map(|&(f, pane)| (inside(f), pane)).collect();
        focuses.sort_unstable();
        weather.focuses = focuses;
        keep_promises(
            &mut guest,
            &weather,
            graphics,
            ChatMark::default(),
            start,
            to + 5_000,
            |_, _, _| Ok(true),
        )?;
        let whole = body_end - body_start;
        let credited: Vec<(f64, u64)> = visit_of(&guest)
            .osaka
            .credited
            .iter()
            .filter(|&&(want, _, at)| want == brain::Want::Use(what) && at >= since)
            .map(|&(_, share, at)| (share, at))
            .collect();
        // The first since it began is this use's (left in its prelude,
        // she may choose the piece again, and that use is a new one).
        if let Some(&(share, at)) = credited.first() {
            let want = if at < body_start {
                0.0
            } else if at >= body_end {
                1.0
            } else {
                (at - body_start) as f64 / whole as f64
            };
            prop_assert_eq!(share, want, "left {:?} at {} of {:?}", what, at, [since, body_start, body_end, until]);
        }
    }
}

mod away;
mod census;
mod dash;
mod golden;
