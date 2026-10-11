//! Her shelf kit (phase 6a, step 5b; design D7b): sold last, and only
//! where a shelf could go up on the frame; delivered through her door's
//! flap like any parcel; never out of its box, whatever the path; opened
//! only while a shelf could go up, and then up on her wall as her shelf
//! (`built: 0`: putting it up is step 5c). Both modes.

use super::*;
use crate::ui::houseguest::osaka::HomeEvent;
use crate::ui::houseguest::sprite::Facing;
use crate::ui::houseguest::structures::{
    Drawn, KeepOff, LOW_ABOVE, LOW_RISE, SHELF_STAGES, SHELF_WIDTH, Shelf, Structure,
};

/// Everything the channel sold before her kit (her TV out of its box
/// among it) on [`rooms`]' floors, and the channel due: her kit is next.
fn all_but_the_kit(seed: u64, graphics: bool) -> Guest {
    use Furniture::*;
    let pieces = [
        (Tv, Nook::Users, 150),
        (Sofa, Nook::Users, 600),
        (Window, Nook::Users, 900),
        (Bed, Nook::Playlist, 100),
        (Desk, Nook::Playlist, 500),
        (Plant, Nook::Playlist, 900),
        (Lamp, Nook::List, 50),
        (Bookshelf, Nook::List, 300),
        (Fridge, Nook::List, 550),
        (CatBed, Nook::List, 750),
        (Poster, Nook::List, 950),
    ];
    let mut guest = home_at(seed, mon(16, 0), &pieces, graphics);
    for item in PRE_SHELF_CATALOGUE {
        assert!(guest.ledger.home.owns(item), "{item:?}");
    }
    guest.ledger.visits = 9;
    guest.ledger.bought_on = 0;
    guest
}

/// Her visit begun on `real`, the frame painted once.
fn arrived(guest: &mut Guest, real: &Buffer, view: &IdleView) -> Buffer {
    guest.cue(Scene::Arrive);
    let frame = paint(guest, real, view, 0);
    assert!(matches!(guest.state, State::Visiting(_)), "visiting");
    frame
}

fn visit_of(guest: &Guest) -> &Visit {
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    visit
}

/// Where her shelf would go up on `frame` for `guest`'s home as the
/// paint reads it (what her visit keeps off it as it stood), the pane
/// her kit is in first.
fn shelf_spot_of(guest: &Guest, frame: &Buffer, view: &IdleView) -> Option<Shelf> {
    let (layer, made) = match &guest.state {
        State::Visiting(visit) => visit.shelf_keep_off(),
        _ => (Vec::new(), Vec::new()),
    };
    let keep_off = KeepOff {
        protected: &view.protected,
        layer: &layer,
        made: &made,
    };
    let home = &guest.ledger.home;
    let kit_at = home
        .props
        .iter()
        .find(|p| p.item.is_kit())
        .map(|p| p.strip.nook());
    home.shelf_spot(frame, plan_of(view, frame), &keep_off, kit_at)
}

/// The channel sells her kit only on a frame where a shelf could go up
/// on it: not on the bundled layout at 80×24 (no pane tall enough), nor
/// where every row of her panes a shelf could take is full of text; on
/// the same frame clear, it does.
#[test]
fn no_kit_is_sold_where_no_pane_takes_a_shelf() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let advert_on = |real: &Buffer, view: &IdleView| {
            let mut guest = all_but_the_kit(1, graphics);
            arrived(&mut guest, real, view);
            let spot = shelf_spot_of(&guest, real, view);
            (visit_of(&guest).chances.advert, spot)
        };
        let (real, view) = rooms_frame(100, 30);
        let (advert, spot) = advert_on(&real, &view);
        assert!(spot.is_some(), "{at}: the control holds a shelf");
        assert_eq!(
            advert,
            Some(Furniture::ShelfKit),
            "{at}: on a frame that holds one"
        );
        // Geometry: the bundled layout at 80×24 has no pane 9 rows tall.
        let (small, small_view) = real_frame(&mut stage_ui(), 80, 24);
        assert!(
            room::pane_strips(&small_view.nooks)
                .iter()
                .all(|p| p.raw.rows < LOW_RISE + LOW_ABOVE),
            "{at}: no pane takes a shelf at 80x24"
        );
        assert_eq!(advert_on(&small, &small_view), (None, None), "{at}: 80x24");
        // Text: every row a shelf could take is full.
        let full = crowded(&real, &view.nooks);
        assert_eq!(advert_on(&full, &view), (None, None), "{at}: full of text");
    }
}

/// Her kit is never out of its box, by any path: as set or as read from
/// her record; her unpacking it (`Home::unbox`); the doorstep's probe of
/// the piece out of its box (every look it takes at the kit is at its
/// box); the stage's gift of it (her shelf, finished, where one goes,
/// never the kit); and her opening it (the event): it goes up as her
/// shelf, `built: 0`, where [`room::Home::shelf_spot`] puts it, and the
/// kit is gone.
#[test]
fn a_kit_is_never_unboxed() {
    use Furniture::ShelfKit;
    let kit = room::Prop {
        boxed: false.into(),
        ..room::Prop::new(ShelfKit, Nook::Users, 500, Facing::Right)
    };
    assert!(kit.boxed(), "as set");
    let ledger = Ledger::from_json(
        r#"{"version":1,"master_seed":1,"visits":3,"rooms":[["Living","Users"]],"props":[{"item":"ShelfKit","at":500,"facing":"Right","boxed":false}]}"#,
    )
    .expect("reads");
    let read: Vec<_> = ledger
        .home
        .props
        .iter()
        .map(|p| (p.item, p.boxed()))
        .collect();
    assert_eq!(read, [(ShelfKit, true)], "as read");
    assert_eq!(
        Ledger::from_json(&ledger.to_json()),
        Ok(ledger.clone()),
        "written back as read"
    );
    let mut home = room::Home::default();
    assert!(home.add(kit));
    assert_eq!(
        home.props,
        [room::Prop {
            boxed: true.into(),
            ..kit
        }],
        "added as its accessor reads it"
    );
    assert!(!home.unbox(ShelfKit), "unboxed");
    assert!(home.props[0].boxed());
    assert_eq!(home.props.len(), 1);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = rooms_frame(100, 30);
        // The doorstep's probe.
        let mut home = room::Home::default();
        assert!(home.add(room::Prop::new(
            Furniture::Tv,
            Nook::Users,
            0,
            Facing::Right
        )));
        let plan = plan_of(&view, &real);
        let (shown, _) = home.frame(Drawn::bare(&real), plan, &|_, _| false);
        let looks = std::cell::RefCell::new(Vec::new());
        let seats = |piece: &room::Shown, _: &[room::Shown], _: &room::Home| {
            if piece.item == ShelfKit {
                looks.borrow_mut().push(piece.boxed());
            }
            vec![piece.seat(room::Use::Unpack, 0)]
        };
        let (prop, _) = home
            .doorstep(
                Drawn::bare(&real),
                plan,
                &shown,
                &|_, _| false,
                &seats,
                ShelfKit,
            )
            .expect("delivered");
        assert!(prop.boxed(), "{at}: delivered boxed");
        let looks = looks.into_inner();
        assert!(
            !looks.is_empty() && looks.iter().all(|&b| b),
            "{at}: {looks:?}"
        );
        // The stage's gift: her shelf, finished, never the kit (her kit
        // in its box, if she has it, is that shelf).
        let mut guest = home_at(
            1,
            mon(16, 0),
            &[
                (Furniture::Tv, Nook::Users, 100),
                (ShelfKit, Nook::Playlist, 300),
            ],
            graphics,
        );
        arrived(&mut guest, &real, &view);
        guest.give(ShelfKit);
        paint(&mut guest, &real, &view, 1);
        let note = guest.cue_note().cloned();
        assert!(
            note.as_ref()
                .is_some_and(|n| n.as_ref().is_ok_and(|n| n.contains("shelf"))),
            "{at}: {note:?}"
        );
        let home = &guest.ledger.home;
        assert!(
            !home.props.iter().any(|p| p.item == ShelfKit),
            "{at}: {home:?}"
        );
        let built: Vec<u8> = home
            .structures
            .iter()
            .map(|&Structure::Shelf(s)| s.built)
            .collect();
        assert_eq!(built, [SHELF_STAGES], "{at}: the stage's shelf");
        assert!(home.owns(ShelfKit), "{at}");
        // Her opening it: up as her shelf, where one goes, the kit gone.
        let mut guest = home_at(
            1,
            mon(16, 0),
            &[
                (Furniture::Tv, Nook::Users, 100),
                (ShelfKit, Nook::Playlist, 300),
            ],
            graphics,
        );
        arrived(&mut guest, &real, &view);
        let spot = shelf_spot_of(&guest, &real, &view).expect("a shelf goes up here");
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.did(HomeEvent::Unpacked(ShelfKit));
        paint(&mut guest, &real, &view, 100);
        let home = &guest.ledger.home;
        assert!(
            !home.props.iter().any(|p| p.item == ShelfKit),
            "{at}: {home:?}"
        );
        assert_eq!(
            home.structures,
            [Structure::Shelf(Shelf { built: 0, ..spot })],
            "{at}"
        );
        assert_eq!(spot.nook, Nook::Playlist, "{at}: where she opened it");
        assert!(home.owns(ShelfKit), "{at}");
        assert!(!guest.ledger.home.unbox(ShelfKit), "{at}");
    }
}

/// A kit delivered where no shelf could go up stays in its box: no seat
/// to open it from is offered, so she never does, over ten minutes; her
/// opening it then all the same (the event) leaves it boxed, nothing up.
/// Once the frame holds a shelf, she opens it, and it goes up.
#[test]
fn a_kit_with_nowhere_to_go_stays_boxed() {
    use Furniture::ShelfKit;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = rooms_frame(100, 30);
        let full = crowded(&real, &view.nooks);
        let mut guest = all_but_the_kit(3, graphics);
        guest.ledger.ordered = Some(ShelfKit);
        arrived(&mut guest, &full, &view);
        let mut now = 0;
        let kit_seat = |guest: &Guest| {
            visit_of(guest)
                .chances
                .seats
                .iter()
                .any(|s| s.item == ShelfKit && s.what == room::Use::Unpack)
        };
        let boxed = |guest: &Guest| {
            guest
                .ledger
                .home
                .props
                .iter()
                .any(|p| p.item == ShelfKit && p.boxed())
        };
        let mut delivered = false;
        while now < 10 * 60_000 {
            shell_step(&mut guest, &full, &view, &mut now, true);
            let State::Visiting(_) = &guest.state else {
                panic!("{at}: gone at {now}");
            };
            assert!(guest.ledger.home.structures.is_empty(), "{at}: up at {now}");
            delivered |= boxed(&guest);
            if delivered {
                assert!(boxed(&guest), "{at}: opened at {now}");
                assert!(!kit_seat(&guest), "{at}: a seat to open it from at {now}");
            }
        }
        assert!(delivered, "{at}: never delivered");
        assert_eq!(shelf_spot_of(&guest, &full, &view), None, "{at}");
        // Opened all the same: the box stays shut.
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.did(HomeEvent::Unpacked(ShelfKit));
        shell_step(&mut guest, &full, &view, &mut now, true);
        assert!(boxed(&guest), "{at}: opened with nowhere to go");
        assert!(guest.ledger.home.structures.is_empty(), "{at}");
        // The frame clears: she has a seat to open it from, opens it, and
        // it goes up.
        let until = now + 10 * 60_000;
        let mut seated = false;
        while now < until && boxed(&guest) {
            shell_step(&mut guest, &real, &view, &mut now, true);
            if matches!(guest.state, State::Visiting(_)) {
                seated |= kit_seat(&guest);
            }
        }
        assert!(seated, "{at}: no seat to open it from");
        assert!(!boxed(&guest), "{at}: never opened");
        let built: Vec<u8> = guest
            .ledger
            .home
            .structures
            .iter()
            .map(|&Structure::Shelf(s)| s.built)
            .collect();
        assert_eq!(built, [0], "{at}: up, nothing built yet");
    }
}

/// [`rooms_frame`]'s `real` at 100×30 with one place for `guest`'s
/// shelf to go up: every pane full of text above her head ([`crowded`])
/// but the Users pane, full only above a low shelf's rows, and on its
/// row only its run and margin left blank. That shelf, asserted where
/// [`room::Home::shelf_spot`] puts it; its run's row and middle.
fn one_shelf_spot(guest: &Guest, real: &Buffer, view: &IdleView) -> (Buffer, (u16, u16)) {
    let users = (view.nooks.iter())
        .position(|&(nook, _)| nook == Nook::Users)
        .expect("Users");
    let others = Crowd {
        panes: !(1 << users),
        low: false,
    };
    let band = crowded_by(real, &view.nooks, others);
    let mut one = crowded_by(
        &band,
        &view.nooks,
        Crowd {
            panes: 1 << users,
            low: true,
        },
    );
    let spot = shelf_spot_of(guest, &one, view).expect("a low shelf in Users");
    assert_eq!((spot.nook, spot.rise), (Nook::Users, LOW_RISE));
    let rect = view.nooks[users].1;
    let run = spot.run(room::pane_raw(rect)).expect("it hangs");
    let kept = run.kept();
    let y = u16::try_from(run.y).expect("on screen");
    for x in rect.x + 1..rect.right() - 1 {
        if !(kept.x..kept.right()).contains(&x) {
            one[(x, y)].set_symbol("x");
        }
    }
    assert_eq!(shelf_spot_of(guest, &one, view), Some(spot), "that place");
    let middle = u16::try_from(run.from + SHELF_WIDTH / 2).expect("on screen");
    (one, (middle, y))
}

/// Her visit's text layer with a glyph of the Users pane's top row (full
/// of text) moved to `to`.
fn moved_onto(guest: &mut Guest, frame: &Buffer, view: &IdleView, to: (u16, u16)) {
    let rect = (view.nooks.iter())
        .find(|&&(nook, _)| nook == Nook::Users)
        .expect("Users")
        .1;
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    assert!(
        visit.layer.take(frame, &[], (rect.x + 1, rect.y + 1), to),
        "a glyph moved to {to:?}"
    );
}

/// What her visit keeps off counts for whether a shelf could go up (the
/// paint asks with it, as the write and her kit's raise do: phase 6a
/// D4, D7b). On a frame with one place for a shelf, a glyph of hers moved
/// onto its run leaves none: her kit in its box has no seat to open it
/// from (else she'd open it, and it couldn't go up), and the channel,
/// due with her kit next, doesn't sell it; with the glyph gone, both
/// come back. Both modes.
#[test]
fn her_own_text_on_the_one_place_for_a_shelf_keeps_her_kit_waiting() {
    use Furniture::ShelfKit;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = rooms_frame(100, 30);
        let kit_seat = |guest: &Guest| {
            visit_of(guest)
                .chances
                .seats
                .iter()
                .any(|s| s.item == ShelfKit && s.what == room::Use::Unpack)
        };
        let advert = |guest: &Guest| visit_of(guest).chances.advert;
        // Her kit in its box.
        let mut guest = all_but_the_kit(5, graphics);
        assert!(
            guest
                .ledger
                .home
                .add(room::Prop::new(ShelfKit, Nook::Users, 400, Facing::Right))
        );
        let (one, on_run) = one_shelf_spot(&guest, &real, &view);
        arrived(&mut guest, &one, &view);
        assert!(kit_seat(&guest), "{at}: a seat to open it from");
        moved_onto(&mut guest, &one, &view, on_run);
        paint(&mut guest, &one, &view, 1);
        assert_eq!(shelf_spot_of(&guest, &one, &view), None, "{at}: none now");
        assert!(!kit_seat(&guest), "{at}: a seat with her text on its place");
        if let State::Visiting(visit) = &mut guest.state {
            visit.layer = layer::TextLayer::default();
        }
        paint(&mut guest, &one, &view, 2);
        assert!(kit_seat(&guest), "{at}: no seat with her text gone");
        // The channel, her kit next.
        let mut guest = all_but_the_kit(5, graphics);
        let (one, on_run) = one_shelf_spot(&guest, &real, &view);
        arrived(&mut guest, &one, &view);
        assert_eq!(advert(&guest), Some(ShelfKit), "{at}: on sale");
        moved_onto(&mut guest, &one, &view, on_run);
        paint(&mut guest, &one, &view, 1);
        assert_eq!(
            advert(&guest),
            None,
            "{at}: sold with her text on its place"
        );
        if let State::Visiting(visit) = &mut guest.state {
            visit.layer = layer::TextLayer::default();
        }
        paint(&mut guest, &one, &view, 2);
        assert_eq!(advert(&guest), Some(ShelfKit), "{at}: her text gone");
    }
}

/// Her kit opened (her tick records it) and her visit over before the
/// next paint puts it up: it stays in its box, nothing up (phase 6a
/// D7b); on a later visit she opens it again, and it goes up. Both
/// modes.
#[test]
fn her_kit_opened_as_she_leaves_waits_for_her_next_visit() {
    use Furniture::ShelfKit;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = rooms_frame(100, 30);
        let mut guest = all_but_the_kit(6, graphics);
        assert!(
            guest
                .ledger
                .home
                .add(room::Prop::new(ShelfKit, Nook::Users, 400, Facing::Right))
        );
        arrived(&mut guest, &real, &view);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.did(HomeEvent::Unpacked(ShelfKit));
        guest.advance(1);
        assert_eq!(
            visit_of(&guest).raise,
            Some(room::Kit::Shelf),
            "{at}: opened, for the next paint"
        );
        guest.activity(1);
        assert!(!matches!(guest.state, State::Visiting(_)), "{at}: gone");
        let boxed = |guest: &Guest| {
            (guest.ledger.home.props.iter()).any(|p| p.item == ShelfKit && p.boxed())
        };
        let mut now = 1;
        while guest.present() {
            assert!(now < 60_000, "{at}: never gone");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        assert!(boxed(&guest), "{at}: opened as she left");
        assert!(guest.ledger.home.structures.is_empty(), "{at}");
        // Her next visit: she opens it again, and it goes up.
        guest.cue(Scene::Arrive);
        now += 1;
        paint(&mut guest, &real, &view, now);
        assert!(boxed(&guest), "{at}: opened on arriving");
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.did(HomeEvent::Unpacked(ShelfKit));
        now += 1;
        paint(&mut guest, &real, &view, now);
        assert!(!boxed(&guest), "{at}: never opened");
        let built: Vec<u8> = (guest.ledger.home.structures.iter())
            .map(|&Structure::Shelf(s)| s.built)
            .collect();
        assert_eq!(built, [0], "{at}: up, nothing built yet");
    }
}

/// An order for what she has already is no order (as her record reads
/// it): her kit on order, the stage puts up her shelf (taking the kit's
/// place), or a piece on order, the stage gives it her; at the next
/// paint nothing's on order, and the channel sells again. Both modes.
#[test]
fn an_order_for_what_she_has_is_dropped() {
    use Furniture::{Bed, ShelfKit};
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = rooms_frame(100, 30);
        for (ordered, given) in [(ShelfKit, None), (Bed, Some(Bed))] {
            let at = format!("{at} {ordered:?}");
            let mut guest = all_but_the_kit(7, graphics);
            guest.ledger.home.props.retain(|p| p.item != Bed);
            guest.ledger.ordered = Some(ordered);
            // Not due to come this visit (bought on it).
            guest.ledger.bought_on = guest.ledger.visits + 1;
            match given {
                None => guest.cue(Scene::Shelf),
                Some(item) => guest.give(item),
            }
            arrived(&mut guest, &real, &view);
            assert!(guest.ledger.home.owns(ordered), "{at}: hers");
            paint(&mut guest, &real, &view, 1);
            assert_eq!(guest.ledger.ordered, None, "{at}: still on order");
            guest.shop();
            paint(&mut guest, &real, &view, 2);
            let expected = if given.is_some() { ShelfKit } else { Bed };
            assert_eq!(
                visit_of(&guest).chances.advert,
                Some(expected),
                "{at}: the channel off"
            );
        }
    }
}

/// The stage's gift of her kit says how it went: where her shelf went
/// up, or that none goes up on the frame (the bundled layout at 80×24:
/// no pane tall enough), her kit then still hers in its box. Both modes.
#[test]
fn the_stage_says_where_her_shelf_went_or_why_not() {
    use Furniture::ShelfKit;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (small, small_view) = real_frame(&mut stage_ui(), 80, 24);
        let mut guest = all_but_the_kit(8, graphics);
        assert!(
            guest
                .ledger
                .home
                .add(room::Prop::new(ShelfKit, Nook::Users, 400, Facing::Right))
        );
        arrived(&mut guest, &small, &small_view);
        guest.give(ShelfKit);
        paint(&mut guest, &small, &small_view, 1);
        assert_eq!(
            guest.cue_note(),
            Some(&Err("no shelf goes up on this frame".to_owned())),
            "{at}"
        );
        assert!(guest.ledger.home.structures.is_empty(), "{at}");
        assert!(
            (guest.ledger.home.props.iter()).any(|p| p.item == ShelfKit && p.boxed()),
            "{at}: her kit"
        );
    }
}
