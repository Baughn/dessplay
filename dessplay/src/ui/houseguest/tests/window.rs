//! Her window, hung low (phase 5c D6): it may hang behind her sofa, which
//! is drawn in front of it, and behind nothing else. Both drawing modes,
//! under quiet panes and text-dense ones.

use super::*;

/// Her sofa against her door's space at the Users pane's left wall, her
/// window hung with its corner behind it (its left two columns over the
/// sofa's right end), at 300 thousandths along, facing right: a home she
/// has visited once, at `start` on seed `seed`.
fn sofa_and_window_at(seed: u64, start: routine::GameTime, graphics: bool) -> Guest {
    // On the 50-wide Users pane the strip runs 1..49, her door's space
    // (Users' left wall is the screen's edge) on 1..6: the sofa (9 wide)
    // at 0 stands on columns 7..15; the window (4 wide) at 300 hangs on
    // 14..17.
    let mut guest = home_at(seed, start, &[(Furniture::Sofa, Nook::Users, 0)], graphics);
    assert!(guest.ledger.home.add(room::Prop::new(
        Furniture::Window,
        Nook::Users,
        300,
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
/// her TV, or wholly over her sofa, both standing past her door's space
/// at Users' left wall, on 7..), loaded and visited: the window shows
/// hung low (its bottom row two above the floor), clear of the TV, or
/// with only its corner behind the sofa; drawn (in ASCII its glyphs where
/// the sofa draws none, in line art an image's cells); and on the quiet
/// screen she can lean on its sill (a look-out spot she'd stand on).
/// Both modes, quiet and text-dense screens.
#[test]
fn an_old_window_loads_hung_low_drawn_and_in_reach() {
    let record = |under: &str, offset: u16| {
        format!(
            concat!(
                r#"{{"version":1,"master_seed":3,"visits":9,"rooms":[["Living","Users"]],"#,
                r#""props":[{{"item":"{under}","at":0,"facing":"Right","boxed":false}},"#,
                r#"{{"item":"Window","at":20,"facing":"Right","boxed":false}}],"#,
                r#""anchors":[{{"item":"{under}","strip":{{"Bottom":"Users"}},"anchor":{{"side":"Left","offset":0}}}},"#,
                r#"{{"item":"Window","strip":{{"Bottom":"Users"}},"anchor":{{"side":"Left","offset":{offset}}}}}]}}"#
            ),
            under = under,
            offset = offset,
        )
    };
    // At 1, as older records have it: over the TV, and now in her door's
    // space on Users' left (the screen's edge); at 7, past the space.
    for offset in [1, 7] {
        for (screen, (real, view)) in [("quiet", home_screen()), ("busy", busy_home_screen())] {
            for under in [Furniture::Tv, Furniture::Sofa] {
                for graphics in [false, true] {
                    let at = format!("{screen} {under:?} at {offset} graphics={graphics}");
                    let mut ledger = Ledger::from_json(&record(&format!("{under:?}"), offset))
                        .expect("it reads");
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
                    // Her door's space: kept, the window hangs clear of it
                    // (shifted aside); yielding, the strip is laid out as
                    // with no door.
                    let home = &guest.ledger.home;
                    let door = home.door.expect("her door's wall chosen");
                    let plan = home
                        .extents(&view.nooks)
                        .into_iter()
                        .find(|p| p.strip == door.strip)
                        .expect("its strip");
                    match plan.space {
                        Some(space) if space.kept => assert!(
                            !window.rect().intersects(space.rect),
                            "{at}: {window:?} in {space:?}"
                        ),
                        _ => assert_eq!(
                            room::Home {
                                door: None,
                                ..home.clone()
                            }
                            .layout(&view.nooks),
                            home.clone().layout(&view.nooks),
                            "{at}: the yield"
                        ),
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
}

/// A record from before her door (no `door` key), loaded on [`rooms`] at
/// 100×30, where Users' right wall (column 99) is the screen's edge: her
/// fridge anchored against that wall, and her lamp with only a share of
/// the way along (an older record's, never yet stood: anchor none, `at`
/// 700). After her first frame her door's wall is Users' right, the
/// fridge stands flush with her door's space (its right edge at `w - 7`,
/// its anchor as it was), and the lamp is pinned where its share puts it
/// between the walls (the raw strip: shares are always of it), laid from
/// there against the floor beside the space; nothing in the closet, the
/// fridge still against a wall, and the record round-trips with the
/// door. Both modes.
#[test]
fn an_older_record_keeps_its_fridge_by_her_door() {
    let record = concat!(
        r#"{"version":1,"master_seed":7,"visits":3,"rooms":[],"#,
        r#""props":[{"item":"Fridge","at":1000,"facing":"Left","boxed":false},"#,
        r#"{"item":"Lamp","at":700,"facing":"Left","boxed":false},"#,
        r#"{"item":"Tv","at":500,"facing":"Left","boxed":false}],"#,
        r#""anchors":[{"item":"Fridge","strip":{"Bottom":"Users"},"anchor":{"side":"Right","offset":0}},"#,
        r#"{"item":"Lamp","strip":{"Bottom":"Users"}},"#,
        r#"{"item":"Tv","strip":{"Bottom":"Playlist"},"anchor":{"side":"Left","offset":3}}],"#,
        r#""bought_on":3,"clock_sent":true}"#
    );
    let (real, view) = rooms_frame(100, 30);
    let users = view.nooks[1].1;
    assert_eq!(users, Rect::new(50, 0, 50, 13));
    let w = i32::from(users.right()) - 1;
    // The raw strip, between Users' walls, and the floor beside the space.
    let raw = room::Extent {
        from: 51,
        to: 99,
        floor: 12,
        rows: 11,
    };
    let floor = room::Extent { to: w - 6, ..raw };
    let lamp_cols = Furniture::Lamp.spec().footprint.0;
    let (lamp_anchor, _) = raw.pin(51 + (48 - i32::from(lamp_cols)) * 700 / 1000, lamp_cols);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let ledger = Ledger::from_json(record).expect("it reads");
        assert_eq!(ledger.home.door, None);
        let mut guest = Guest::restore(ledger);
        guest.set_date(date(2026, 6, 17));
        if graphics {
            guest.set_picker(kitty());
        }
        let now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let home = &guest.ledger.home;
        assert_eq!(
            home.door,
            Some(room::DoorWall {
                strip: room::Strip::Bottom(Nook::Users),
                side: room::Side::Right,
            }),
            "{at}"
        );
        let prop = |item: Furniture| *home.props.iter().find(|p| p.item == item).unwrap();
        assert_eq!(
            prop(Furniture::Fridge).anchor,
            Some(room::Anchor {
                side: room::Side::Right,
                offset: 0,
            }),
            "{at}: its anchor as it was"
        );
        assert_eq!(prop(Furniture::Lamp).anchor, Some(lamp_anchor), "{at}");
        assert_eq!(prop(Furniture::Lamp).at, 700, "{at}: its share kept");
        let fridge = shown_piece(&guest, Furniture::Fridge).expect("shown");
        assert_eq!(
            fridge.left + i32::from(fridge.size().0) - 1,
            w - 7,
            "{at}: flush with the space"
        );
        let lamp = shown_piece(&guest, Furniture::Lamp).expect("shown");
        assert_eq!(lamp.left, floor.left(lamp_anchor, lamp_cols), "{at}");
        assert!(
            shown_piece(&guest, Furniture::Tv).is_some(),
            "{at}: nothing in the closet"
        );
        assert!(
            !guest.broken().contains("wall(fridge)"),
            "{at}: {}",
            guest.broken()
        );
        let back = Ledger::from_json(&guest.ledger.to_json()).expect("it reads");
        assert_eq!(back.home.door, guest.ledger.home.door, "{at}");
    }
}

/// An older record whose pieces on the strip of her door's wall pack only
/// between the walls (never anchored, her bed, desk, sofa, TV, bookshelf,
/// fridge and lamp: 44 columns of the 48, the space's 6 too many): her
/// door's space yields, every piece is laid on the raw strip where its
/// share puts it, nothing goes to the closet and nothing moves off its
/// strip. Both modes.
#[test]
fn an_older_record_too_full_for_her_door_lays_out_between_the_walls() {
    let items = [
        ("Bed", 0),
        ("Desk", 150),
        ("Sofa", 300),
        ("Tv", 450),
        ("Bookshelf", 600),
        ("Fridge", 800),
        ("Lamp", 1000),
    ];
    let props: Vec<String> = items
        .iter()
        .map(|(item, at)| format!(r#"{{"item":"{item}","at":{at},"facing":"Left","boxed":false}}"#))
        .collect();
    let anchors: Vec<String> = items
        .iter()
        .map(|(item, _)| format!(r#"{{"item":"{item}","strip":{{"Bottom":"Users"}}}}"#))
        .collect();
    let record = format!(
        r#"{{"version":1,"master_seed":7,"visits":3,"rooms":[],"props":[{}],"anchors":[{}],"bought_on":3,"clock_sent":true}}"#,
        props.join(","),
        anchors.join(",")
    );
    let (real, view) = rooms_frame(100, 30);
    let wide: u16 = items
        .iter()
        .map(|(item, _)| {
            serde_json::from_value::<Furniture>(serde_json::json!(item))
                .unwrap()
                .spec()
                .footprint
                .0
        })
        .sum();
    assert!((43..=48).contains(&wide), "{wide}");
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::restore(Ledger::from_json(&record).expect("it reads"));
        guest.set_date(date(2026, 6, 17));
        if graphics {
            guest.set_picker(kitty());
        }
        let now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let home = &guest.ledger.home;
        assert_eq!(
            home.door,
            Some(room::DoorWall {
                strip: room::Strip::Bottom(Nook::Users),
                side: room::Side::Right,
            }),
            "{at}"
        );
        let plan = home.extents(&view.nooks)[1];
        let space = plan.space.expect("a space");
        assert!(!space.kept, "{at}: it yields");
        assert_eq!(plan.floor, plan.raw, "{at}");
        for prop in &home.props {
            assert_eq!(prop.strip, room::Strip::Bottom(Nook::Users), "{at}: moved");
        }
        let laid = home.clone().layout(&view.nooks);
        assert_eq!(laid.len(), items.len(), "{at}: {laid:?}");
        // Each anchored where its share of the raw strip puts it, and laid
        // out as with no door at all.
        for prop in &home.props {
            let cols = prop.item.spec().footprint.0;
            let share = plan.raw.pin(plan.raw.share(prop.at, cols), cols).0;
            assert_eq!(prop.anchor, Some(share), "{at}: {prop:?}");
        }
        let doorless = room::Home {
            door: None,
            ..home.clone()
        }
        .layout(&view.nooks);
        assert_eq!(laid, doorless, "{at}");
        for item in laid.iter().map(|s| s.item) {
            assert!(
                shown_piece(&guest, item).is_some(),
                "{at}: {item:?} closeted"
            );
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
