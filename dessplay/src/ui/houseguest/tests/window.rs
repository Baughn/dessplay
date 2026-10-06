//! Her window, hung low (phase 5c D6): it may hang behind her sofa, which
//! is drawn in front of it, and behind nothing else. Both drawing modes,
//! under quiet panes and text-dense ones.

use super::*;

/// Her sofa against the Users pane's left wall, her window hung partly
/// behind it (its left three columns over the sofa's right end), at
/// `at` thousandths along, facing right: a home she has visited once.
fn sofa_and_window(graphics: bool) -> Guest {
    // On the 50-wide Users pane the strip runs 1..49: the sofa (9 wide)
    // at 0 stands on columns 1..10; the window (4 wide) at 137 hangs
    // on 7..11.
    let mut guest = home_at(
        3,
        tue(14, 0),
        &[(Furniture::Sofa, Nook::Users, 0)],
        graphics,
    );
    assert!(guest.ledger.home.add(room::Prop::new(
        Furniture::Window,
        Nook::Users,
        137,
        sprite::Facing::Right
    )));
    guest
}

/// Her window behind her sofa is drawn behind it (phase 5c D6): in ASCII
/// the sofa's glyphs win where both would draw, the window's show where
/// the sofa draws nothing (or isn't); in line art the two are one image
/// (two would cut each other out), the window's layer under the sofa's.
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
            let frame = paint(&mut guest, &real, &view, now);
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
