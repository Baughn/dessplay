//! Her window, hung low (phase 5c D6): it may hang behind her sofa, which
//! is drawn in front of it, and behind nothing else. Both drawing modes,
//! under quiet panes and text-dense ones.

use super::*;

/// Her sofa against the Users pane's left wall, her window hung with its
/// corner behind it (its left two columns over the sofa's right end), at
/// 160 thousandths along, facing right: a home she has visited once, at
/// `start` on seed `seed`.
fn sofa_and_window_at(seed: u64, start: routine::GameTime, graphics: bool) -> Guest {
    // On the 50-wide Users pane the strip runs 1..49: the sofa (9 wide)
    // at 0 stands on columns 1..10; the window (4 wide) at 160 hangs
    // on 8..12.
    let mut guest = home_at(seed, start, &[(Furniture::Sofa, Nook::Users, 0)], graphics);
    assert!(guest.ledger.home.add(room::Prop::new(
        Furniture::Window,
        Nook::Users,
        160,
        sprite::Facing::Right
    )));
    guest
}

/// [`sofa_and_window_at`] of a Tuesday afternoon.
fn sofa_and_window(graphics: bool) -> Guest {
    sofa_and_window_at(3, tue(14, 0), graphics)
}

/// The piece a layer's look draws, if it's one.
fn piece_of(look: &Look) -> Option<Furniture> {
    match *look {
        Look::Prop(item, _) | Look::Parcel(item, _) | Look::Piece(item, _) => Some(item),
        Look::Tv(_) => Some(Furniture::Tv),
        _ => None,
    }
}

/// That in `looks` (the layers of what was painted, in order) every
/// layer of her window comes before any of her sofa's, and each shows.
fn window_first(looks: &[Look], at: &str) {
    let of = |item: Furniture| -> Vec<usize> {
        looks
            .iter()
            .enumerate()
            .filter(|(_, l)| piece_of(l) == Some(item))
            .map(|(i, _)| i)
            .collect()
    };
    let (window, sofa) = (of(Furniture::Window), of(Furniture::Sofa));
    assert!(!window.is_empty() && !sofa.is_empty(), "{at}: {looks:?}");
    assert!(
        window.iter().max() < sofa.iter().min(),
        "{at}: the window behind the sofa: {looks:?}"
    );
}

/// Her window behind her sofa is drawn behind it (phase 5c D6): in ASCII
/// the sofa's glyphs win where both would draw, the window's show where
/// the sofa draws nothing (or isn't); in line art the two are one image
/// (two would cut each other out), the window's layer under the sofa's
/// (painted first).
#[test]
fn her_window_behind_her_sofa_is_drawn_behind_it() {
    for (name, (real, view)) in [("quiet", home_screen()), ("busy", busy_home_screen())] {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = sofa_and_window(graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            // Her well away from both (she'd join their image).
            if let State::Visiting(visit) = &mut guest.state {
                visit.osaka.x = 40;
            }
            if let Some(graphics) = &mut guest.graphics {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &real, &view, now);
            if let Some(graphics) = &mut guest.graphics {
                window_first(&graphics.take_looks(), &at);
            }
            let shown = &visit_of(&guest).shown;
            let get = |item: Furniture| {
                *shown
                    .iter()
                    .find(|s| s.item == item)
                    .unwrap_or_else(|| panic!("{at}: {item:?} shown"))
            };
            let (window, sofa) = (get(Furniture::Window), get(Furniture::Sofa));
            assert!(
                window.rect().intersects(sofa.rect()),
                "{at}: {window:?} {sofa:?}"
            );
            if graphics {
                // One image: every placeholder cell over either is the
                // same image's (its id is the cell's colour).
                let union = window.rect().union(sofa.rect());
                let ids: std::collections::BTreeSet<String> = (union.y..union.bottom())
                    .flat_map(|y| (union.x..union.right()).map(move |x| (x, y)))
                    .filter(|&(x, y)| frame[(x, y)].symbol().contains('\u{10EEEE}'))
                    .map(|(x, y)| format!("{:?}", frame[(x, y)].fg))
                    .collect();
                assert_eq!(ids.len(), 1, "{at}: {ids:?}");
                // Every cell of either is the image's (its row's run of
                // placeholders begins in its first cell; the rest skip).
                let image = |x: u16, y: u16| super::cells::untouchable(&frame[(x, y)]);
                let covered = (union.y..union.bottom())
                    .all(|y| (union.x..union.right()).all(|x| image(x, y)));
                assert!(
                    covered,
                    "{at}: the window's and the sofa's cells are the image's"
                );
            } else {
                let sofa_glyph = |x: i32, y: i32| {
                    sofa.cells()
                        .find(|&(cx, cy, _)| (cx, cy) == (x, y))
                        .and_then(|(.., g)| g)
                };
                for (x, y, glyph) in window.cells() {
                    let drawn = frame[(x as u16, y as u16)].symbol().chars().next();
                    match sofa_glyph(x, y) {
                        Some(g) => assert_eq!(drawn, Some(g), "{at}: ({x}, {y}) the sofa's"),
                        // The window's own glyph, or its sky over it.
                        None if glyph.is_some() => assert!(
                            drawn.is_some_and(|c| c != ' '),
                            "{at}: ({x}, {y}) the window's"
                        ),
                        None => {}
                    }
                }
            }
        }
    }
}

/// In line art, text between her window and her sofa where neither
/// stands (under the window's free end, on the row above the floor) is
/// no image's to cover: the two are drawn each alone, and the text stays.
/// In ASCII neither draws there.
#[test]
fn text_beside_her_window_and_sofa_stays() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut real, view) = home_screen();
        let mut guest = sofa_and_window(graphics);
        let now = until_visiting(&mut guest, &real, &view, 0);
        if let State::Visiting(visit) = &mut guest.state {
            visit.osaka.x = 40;
        }
        paint(&mut guest, &real, &view, now);
        let shown = &visit_of(&guest).shown;
        let window = *shown
            .iter()
            .find(|s| s.item == Furniture::Window)
            .unwrap_or_else(|| panic!("{at}: shown"));
        let (x, y) = (window.rect().right() - 1, window.rect().bottom());
        real.set_string(x, y, "q", Style::new());
        let frame = paint(&mut guest, &real, &view, now + 1);
        let shown = &visit_of(&guest).shown;
        assert!(
            shown.iter().any(|s| s.item == Furniture::Window)
                && shown.iter().any(|s| s.item == Furniture::Sofa),
            "{at}: both still shown: {shown:?}"
        );
        // Shown as it is, not an image's cell (an image's run of
        // placeholders skips the cells after its first).
        assert!(
            frame[(x, y)].symbol() == "q" && !super::cells::untouchable(&frame[(x, y)]),
            "{at}: the text at ({x}, {y}): {:?}",
            frame[(x, y)]
        );
    }
}

/// Her own image, standing in front of her window and her sofa, keeps
/// them back to front (phase 5c D6): the window's layers before the
/// sofa's. Line art (in ASCII she's drawn over them, glyph by glyph).
#[test]
fn standing_at_her_sofa_her_window_is_behind_it() {
    let (real, view) = home_screen();
    let mut guest = sofa_and_window(true);
    let now = until_visiting(&mut guest, &real, &view, 0);
    let sofa = *visit_of(&guest)
        .shown
        .iter()
        .find(|s| s.item == Furniture::Sofa)
        .expect("shown");
    if let State::Visiting(visit) = &mut guest.state {
        visit.osaka.x = 10;
        visit.osaka.y = sofa.floor;
    }
    if let Some(graphics) = &mut guest.graphics {
        graphics.take_looks();
    }
    paint(&mut guest, &real, &view, now);
    let looks = guest.graphics.as_mut().expect("line art").take_looks();
    assert!(
        looks.iter().any(|l| matches!(l, Look::Pose(..))),
        "she's drawn: {looks:?}"
    );
    window_first(&looks, "her image");
}

/// Her closed door, standing in front of her window and her sofa while
/// she's out at school, keeps them back to front in its image: the
/// window's layers before the sofa's. Line art.
#[test]
fn her_door_at_her_sofa_keeps_her_window_behind_it() {
    let (real, view) = home_screen();
    let mut guest = sofa_and_window_at(4, tue(8, 10), true);
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    let sofa = *visit_of(&guest)
        .shown
        .iter()
        .find(|s| s.item == Furniture::Sofa)
        .expect("shown");
    paint(&mut guest, &real, &view, now);
    let from = now;
    while !matches!(guest.state, State::Away(_)) {
        assert!(now < from + 60 * 60_000, "she never went out");
        shell_step(&mut guest, &real, &view, &mut now, true);
    }
    shell_step(&mut guest, &real, &view, &mut now, true);
    let out = guest.out.as_mut().expect("out");
    out.door = Some(DoorAt {
        x: 10,
        y: sofa.floor,
        facing: sprite::Facing::Right,
    });
    if let Some(graphics) = &mut guest.graphics {
        graphics.take_looks();
    }
    paint(&mut guest, &real, &view, now);
    assert_eq!(
        guest.closed_door().map(|d| (d.x, d.y)),
        Some((10, sofa.floor)),
        "her door stands there"
    );
    let looks = guest.graphics.as_mut().expect("line art").take_looks();
    assert!(
        looks.iter().any(|l| matches!(l, Look::Door(_))),
        "her door is drawn: {looks:?}"
    );
    window_first(&looks, "her door's image");
}

/// What she painted over what, where her sofa covers her window's corner
/// (the dissolve rains each cell back to what's under it): one cell each,
/// the piece in front's (in ASCII, the glyph the frame shows), over what
/// the screen has there, not over the window's glyph. Both modes.
#[test]
fn where_her_sofa_covers_her_window_what_she_painted_is_one_cell_each() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = home_screen();
        let mut guest = sofa_and_window(graphics);
        let now = until_visiting(&mut guest, &real, &view, 0);
        if let State::Visiting(visit) = &mut guest.state {
            visit.osaka.x = 40;
        }
        let frame = paint(&mut guest, &real, &view, now);
        let visit = visit_of(&guest);
        let get = |item: Furniture| *visit.shown.iter().find(|s| s.item == item).expect("shown");
        let both = get(Furniture::Window)
            .rect()
            .union(get(Furniture::Sofa).rect());
        let mut seen = std::collections::BTreeSet::new();
        for cell in visit
            .painted
            .iter()
            .filter(|c| both.contains((c.x, c.y).into()))
        {
            assert!(
                seen.insert((cell.x, cell.y)),
                "{at}: ({}, {}) twice",
                cell.x,
                cell.y
            );
            assert_eq!(
                Some(&cell.under),
                real.cell((cell.x, cell.y)),
                "{at}: under ({}, {})",
                cell.x,
                cell.y
            );
            if !graphics {
                assert_eq!(
                    frame[(cell.x, cell.y)].symbol().chars().next(),
                    Some(cell.glyph),
                    "{at}: ({}, {})",
                    cell.x,
                    cell.y
                );
            }
        }
        assert!(!seen.is_empty(), "{at}");
    }
}

/// A saved home whose window hung at the old height (four rows up, over
/// her TV, or wholly over her sofa), loaded and visited: the window shows
/// hung low (its bottom row two above the floor), clear of the TV, or
/// with only its corner behind the sofa; drawn (in ASCII its glyphs where
/// the sofa draws none, in line art an image's cells); and on the quiet
/// screen she can lean on its sill (a look-out spot she'd stand on).
/// Both modes, quiet and text-dense screens.
#[test]
fn an_old_window_loads_hung_low_drawn_and_in_reach() {
    let record = |under: &str| {
        format!(
            concat!(
                r#"{{"version":1,"master_seed":3,"visits":9,"rooms":[["Living","Users"]],"#,
                r#""props":[{{"item":"{under}","at":0,"facing":"Right","boxed":false}},"#,
                r#"{{"item":"Window","at":20,"facing":"Right","boxed":false}}],"#,
                r#""anchors":[{{"item":"{under}","strip":{{"Bottom":"Users"}},"anchor":{{"side":"Left","offset":0}}}},"#,
                r#"{{"item":"Window","strip":{{"Bottom":"Users"}},"anchor":{{"side":"Left","offset":1}}}}]}}"#
            ),
            under = under
        )
    };
    for (screen, (real, view)) in [("quiet", home_screen()), ("busy", busy_home_screen())] {
        for under in [Furniture::Tv, Furniture::Sofa] {
            for graphics in [false, true] {
                let at = format!("{screen} {under:?} graphics={graphics}");
                let mut ledger =
                    Ledger::from_json(&record(&format!("{under:?}"))).expect("it reads");
                ledger.clock_sent = true;
                let mut guest = Guest::restore(ledger);
                guest.set_date(date(2026, 6, 17));
                if graphics {
                    guest.set_picker(kitty());
                }
                let now = until_visiting(&mut guest, &real, &view, 0);
                if let State::Visiting(visit) = &mut guest.state {
                    visit.osaka.x = 40;
                }
                let frame = paint(&mut guest, &real, &view, now);
                let visit = visit_of(&guest);
                let get = |item: Furniture| {
                    *visit
                        .shown
                        .iter()
                        .find(|s| s.item == item)
                        .unwrap_or_else(|| panic!("{at}: {item:?} shown: {:?}", visit.shown))
                };
                let (window, below) = (get(Furniture::Window), get(under));
                assert_eq!(
                    i32::from(window.rect().bottom()),
                    window.floor - 1,
                    "{at}: hung low"
                );
                if window.rect().intersects(below.cover()) {
                    assert!(
                        under == Furniture::Sofa && window.may_overlap(&below),
                        "{at}: {window:?} over {below:?}"
                    );
                }
                let below_glyph = |x: i32, y: i32| {
                    below
                        .cells()
                        .find(|&(cx, cy, _)| (cx, cy) == (x, y))
                        .and_then(|(.., g)| g)
                };
                for (x, y, glyph) in window.cells() {
                    let cell = &frame[(x as u16, y as u16)];
                    if graphics {
                        assert!(super::cells::untouchable(cell), "{at}: ({x}, {y}) drawn");
                    } else if glyph.is_some() && below_glyph(x, y).is_none() {
                        assert!(cell.symbol() != " ", "{at}: ({x}, {y}) drawn");
                    }
                }
                if screen == "quiet" {
                    let seats = spots_for(&window, &visit.shown, &visit.terrain, false);
                    assert!(
                        seats.iter().any(|s| s.what == room::Use::LookOut),
                        "{at}: in reach: {seats:?}"
                    );
                }
            }
        }
    }
}

/// Nothing is offered to make where it would meet a real piece of hers
/// (phase 5c D6: her window hangs low enough to meet a piece made
/// beneath it, which would fall apart as it was made, see `tend_made`):
/// a line pulled from in front of her window makes nothing there, while
/// the same line pulled from bare floor makes a piece. Both modes.
#[test]
fn nothing_is_made_under_her_window() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = home_screen();
        let mut guest = home_at(3, tue(14, 0), &[], graphics);
        assert!(guest.ledger.home.add(room::Prop::new(
            Furniture::Window,
            Nook::Users,
            500,
            sprite::Facing::Right
        )));
        let now = until_visiting(&mut guest, &real, &view, 0);
        if let State::Visiting(visit) = &mut guest.state {
            visit.osaka.x = 80;
        }
        paint(&mut guest, &real, &view, now);
        let visit = visit_of(&guest);
        let window = *visit
            .shown
            .iter()
            .find(|s| s.item == Furniture::Window)
            .unwrap_or_else(|| panic!("{at}: shown"));
        // The pane's last line ("Tab Next pane | Enter Send"), as though
        // she pulled it standing at `x` on the window's floor.
        let row = real.area.height - 2;
        let pull = |x: i32| scenes::Pull {
            x,
            y: window.floor,
            row,
            side: scenes::Side::Right,
            cells: (0..26).collect(),
            glyphs: "Tab Next pane | Enter Send".to_owned(),
            gap: 0,
        };
        let bare = builds(&real, visit, &[pull(40)], &[]);
        assert!(!bare.is_empty(), "{at}: a piece from bare floor");
        let under = builds(&real, visit, &[pull(window.left + 1)], &[]);
        assert!(
            under.is_empty(),
            "{at}: made under {window:?}: {:?}",
            under.iter().map(|b| b.piece).collect::<Vec<_>>()
        );
    }
}
