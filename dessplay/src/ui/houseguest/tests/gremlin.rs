//! Her gremlin (phase 6a, step 7; design D9): a figurine of small decor
//! who wants up high. On a floor he breaks `Rule::High`, which she feels
//! on sight only in his room, and only where a shelf stands or could go
//! up; she says "It wants a shelf..." turned to face him; a felt rule
//! that wants a kit she doesn't own puts that kit first on the channel
//! (her wishlist, felt anew each visit); he's sold only where a shelf
//! stands or could go up; and a shelf, once up, is where a repair puts
//! him. Both modes.

use super::*;
use crate::ui::houseguest::brain::{Mood, Need, Needs};
use crate::ui::houseguest::sprite::Facing;
use crate::ui::houseguest::structures::{LOW_ABOVE, LOW_RISE, Shelf, Structure};

/// [`rooms`] at 100×30: Users is `50, 0, 50, 13`, its floor row 12 (11
/// rows inside: a high shelf fits over it, rise 7 with her height over
/// the run); List `0, 0, 50, 27` and the Playlist `50, 13, 50, 14`, both
/// floors on row 26.
const USERS_FLOOR: i32 = 12;
const LOWER_FLOOR: i32 = 26;

/// What she says, felt on sight, for his rule.
const LINE: &str = "It wants a shelf...";

/// The row of the table that wants her gremlin up high.
fn high_row() -> usize {
    rules::RULES
        .iter()
        .position(|r| r.rule == rules::Rule::High(Furniture::Gremlin))
        .expect("a row for her gremlin")
}

/// Her gremlin's grievance.
fn high() -> rules::Grievance {
    rules::Grievance {
        row: high_row(),
        piece: Furniture::Gremlin,
    }
}

/// Her TV, her sofa and her window on Users' floor, her gremlin among
/// them; the rest of what the channel sells before him on the other
/// floors but her bookshelf (furniture) and her poster (decor), so what
/// the channel sells first shows: the bookshelf, or the poster with her
/// room feeling bare, unless she wishes for her kit. The channel due.
fn wanting(seed: u64, at: routine::GameTime, graphics: bool) -> Guest {
    use Furniture::*;
    let pieces = [
        (Tv, Nook::Users, 150),
        (Gremlin, Nook::Users, 420),
        (Sofa, Nook::Users, 600),
        (Window, Nook::Users, 900),
        (Bed, Nook::Playlist, 100),
        (Desk, Nook::Playlist, 500),
        (Plant, Nook::Playlist, 900),
        (Lamp, Nook::List, 50),
        (Fridge, Nook::List, 550),
        (CatBed, Nook::List, 750),
    ];
    let mut guest = home_at(seed, at, &pieces, graphics);
    guest.ledger.visits = 9;
    guest.ledger.bought_on = 0;
    guest
}

fn visit_of(guest: &Guest) -> &Visit {
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    visit
}

fn visit_mut(guest: &mut Guest) -> &mut Visit {
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit
}

/// A visit begun with her standing at `at`, facing `facing`, in `mood`,
/// and painted once.
fn begun_at(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    at: (i32, i32),
    facing: Facing,
    mood: Mood,
) {
    visiting_at(guest, real, view, at);
    let visit = visit_mut(guest);
    visit.osaka.facing = facing;
    visit.osaka.set_mood(mood);
    paint(guest, real, view, 0);
}

/// Her gremlin as her home shows him.
fn gremlin_shown(guest: &Guest) -> room::Shown {
    *visit_of(guest)
        .shown
        .iter()
        .find(|s| s.item == Furniture::Gremlin)
        .expect("her gremlin shown")
}

/// Whether `(x, y)` is on Users' floor.
fn on_users_floor((x, y): (i32, i32)) -> bool {
    y == USERS_FLOOR && (50..100).contains(&x)
}

/// A felt High puts her shelf kit first on the channel (`wish`, then
/// `advert`): ahead of the furniture she lacks and of the decor her bare
/// room would have her buy, while a shelf could go up (else the next
/// piece as ever); her felt rules read in the order felt, one wanting no
/// kit passed over; nothing wished with nothing felt, with the rule no
/// longer broken, or with her kit hers (in its box, or up); and up on her
/// shelf, he breaks no rule.
#[test]
fn a_felt_high_rule_puts_the_kit_first() {
    use Furniture::{Bookshelf, Gremlin, Poster, ShelfKit};
    let fun = Needs::with(&[(Need::Beauty, 0.5), (Need::Fun, 0.9)]);
    let bare = Needs::with(&[(Need::Beauty, 0.9), (Need::Fun, 0.5)]);
    let room = || true;
    let none = || false;
    let guest = wanting(1, tue(14, 0), false);
    let mut ledger = guest.ledger.clone();
    let nooks = nooks(100, 30);
    let judged = |ledger: &Ledger| {
        let mut home = ledger.home.clone();
        let laid = home.laid_out(&nooks);
        rules::broken(&laid, &ledger.home, &door::Keep::default())
    };
    let broken = judged(&ledger);
    assert!(
        broken.iter().any(|b| b.key == high()),
        "her gremlin on a floor: {broken:?}"
    );
    // Felt: his kit first, ahead of the furniture she lacks, whatever
    // her mood, and ahead of the decor her bare room would have her buy.
    let felt = [high()];
    let wished = wish(&felt, &broken, &ledger.home);
    assert_eq!(wished, Some(ShelfKit));
    assert_eq!(advert(&ledger, false, &fun, None, &room), Some(Bookshelf));
    assert_eq!(advert(&ledger, false, &bare, None, &room), Some(Poster));
    for needs in [&fun, &bare] {
        assert_eq!(advert(&ledger, false, needs, wished, &room), Some(ShelfKit));
    }
    // Where no shelf could go up, the next piece as ever.
    assert_eq!(advert(&ledger, false, &fun, wished, &none), Some(Bookshelf));
    // In the order she felt them: a felt rule that wants no kit comes to
    // none (her sofa turned from her TV), and the next is read.
    let faces = rules::Grievance {
        row: rules::FACES_ROW,
        piece: Furniture::Sofa,
    };
    let mut also = broken.clone();
    also.push(rules::Broken {
        row: rules::FACES_ROW,
        pieces: vec![Furniture::Sofa],
        involved: vec![Furniture::Sofa],
        key: faces,
    });
    assert_eq!(wish(&[faces, high()], &also, &ledger.home), Some(ShelfKit));
    assert_eq!(wish(&[faces], &also, &ledger.home), None);
    // Nothing felt, nothing wished.
    assert_eq!(wish(&[], &broken, &ledger.home), None);
    // Felt, but no longer broken (judged on what's broken now).
    assert_eq!(wish(&felt, &[], &ledger.home), None);
    // Felt and broken, but her kit is hers: in its box, or up.
    assert!(
        ledger
            .home
            .add(room::Prop::new(ShelfKit, Nook::List, 900, Facing::Right))
    );
    assert_eq!(wish(&felt, &broken, &ledger.home), None, "in its box");
    ledger.home.props.retain(|p| p.item != ShelfKit);
    ledger.home.structures.push(super::shelf::users_shelf());
    assert_eq!(wish(&felt, &broken, &ledger.home), None, "up");
    // Up on it, he wants for nothing.
    ledger.home.props.retain(|p| p.item != Gremlin);
    assert!(ledger.home.add(room::Prop {
        strip: room::Strip::Shelf(Nook::Users),
        anchor: Some(room::Anchor {
            side: room::Side::Left,
            offset: 4,
        }),
        ..room::Prop::new(Gremlin, Nook::Users, 0, Facing::Right)
    }));
    assert!(
        !judged(&ledger).iter().any(|b| b.key == high()),
        "on her shelf: {:?}",
        judged(&ledger)
    );
}

/// Her wishlist is per visit, as what she has felt is: a High felt on
/// one visit puts her kit first on the channel then; on the next she
/// wishes for nothing until she has felt it again (in his room), and the
/// channel sells what it would (her bookshelf). Both modes.
#[test]
fn a_rule_felt_last_visit_is_no_wish_this_one() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(2, tue(14, 0), graphics);
        let users = (56, USERS_FLOOR);
        begun_at(
            &mut guest,
            &real,
            &view,
            users,
            Facing::Right,
            Mood::Ordinary,
        );
        paint(&mut guest, &real, &view, 100);
        let visit = visit_of(&guest);
        assert!(
            visit.osaka.felt().contains(&high()),
            "{at}: felt in his room"
        );
        assert_eq!(visit.chances.advert, Some(Furniture::ShelfKit), "{at}");
        // The next visit, on List's floor: nothing felt, nothing wished.
        begun_at(
            &mut guest,
            &real,
            &view,
            (10, LOWER_FLOOR),
            Facing::Right,
            Mood::Ordinary,
        );
        paint(&mut guest, &real, &view, 100);
        let visit = visit_of(&guest);
        assert!(
            visit.osaka.felt().is_empty(),
            "{at}: {:?}",
            visit.osaka.felt()
        );
        assert_eq!(
            visit.chances.advert,
            Some(Furniture::Bookshelf),
            "{at}: no wish this visit"
        );
        // And the one after, in his room again: felt anew, wished anew.
        begun_at(
            &mut guest,
            &real,
            &view,
            users,
            Facing::Right,
            Mood::Ordinary,
        );
        paint(&mut guest, &real, &view, 100);
        let visit = visit_of(&guest);
        assert!(visit.osaka.felt().contains(&high()), "{at}");
        assert_eq!(visit.chances.advert, Some(Furniture::ShelfKit), "{at}");
    }
}

/// Everything the channel sells before her gremlin, on [`rooms`]'
/// floors, and the channel due: he's next where a shelf could go up.
fn all_but_the_gremlin(seed: u64, graphics: bool) -> Guest {
    let mut guest = super::kit::all_but_the_kit(seed, graphics);
    guest
        .ledger
        .home
        .props
        .retain(|p| p.item != Furniture::Gremlin);
    guest
}

/// The channel sells her gremlin only where a shelf stands or could go
/// up on the frame (so the story he starts can finish): not on the
/// bundled layout at 80×24 (no pane tall enough), nor where every row a
/// shelf could take is full of text; on the same frame clear, it does;
/// and with a shelf of hers up or going up (half up), even one the frame
/// can't hold, on any frame.
#[test]
fn no_gremlin_is_sold_where_no_shelf_can_go() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        // `shelf`: her Users shelf with that many stages up.
        let advert_on = |real: &Buffer, view: &IdleView, shelf: Option<u8>| {
            let mut guest = all_but_the_gremlin(1, graphics);
            if let Some(built) = shelf {
                guest
                    .ledger
                    .home
                    .props
                    .retain(|p| p.item != Furniture::ShelfKit);
                let Structure::Shelf(up) = super::shelf::users_shelf();
                guest
                    .ledger
                    .home
                    .structures
                    .push(Structure::Shelf(Shelf { built, ..up }));
            }
            super::kit::arrived(&mut guest, real, view);
            visit_of(&guest).chances.advert
        };
        let (real, view) = rooms_frame(100, 30);
        assert_eq!(
            advert_on(&real, &view, None),
            Some(Furniture::Gremlin),
            "{at}: on a frame that holds a shelf"
        );
        let (small, small_view) = real_frame(&mut stage_ui(), 80, 24);
        assert!(
            room::pane_strips(&small_view.nooks)
                .iter()
                .all(|p| p.raw.rows < LOW_RISE + LOW_ABOVE),
            "{at}: no pane takes a shelf at 80x24"
        );
        assert_eq!(advert_on(&small, &small_view, None), None, "{at}: 80x24");
        let full = crowded(&real, &view.nooks);
        assert_eq!(advert_on(&full, &view, None), None, "{at}: full of text");
        // Her shelf up, or going up (half up: she's at it): he's sold
        // wherever she watches.
        for built in [1, 2] {
            assert_eq!(
                advert_on(&small, &small_view, Some(built)),
                Some(Furniture::Gremlin),
                "{at}: her shelf {built} up, at 80x24"
            );
            assert_eq!(
                advert_on(&full, &view, Some(built)),
                Some(Furniture::Gremlin),
                "{at}: her shelf {built} up, full of text"
            );
        }
    }
}

/// Where no shelf could go up (every row of every pane above her head
/// full of text) and none stands, her gremlin on her floor is no
/// grievance: in his room an hour of a fed afternoon, industrious, she
/// never feels it, never says so, and never sets about moving him; on
/// the same frame clear she feels it at once and says so. Both modes.
#[test]
fn no_gremlin_grievance_where_no_shelf_can_go() {
    let (real, view) = rooms_frame(100, 30);
    let full = crowded(&real, &view.nooks);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        // The control: the frame clear, a shelf could go up.
        let mut guest = wanting(3, tue(14, 0), graphics);
        begun_at(
            &mut guest,
            &real,
            &view,
            (56, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        let said = run_until(&mut guest, &real, &view, 0, 60_000, |guest, _| {
            visit_of(guest)
                .osaka
                .said_aloud()
                .iter()
                .any(|&(line, _)| line == LINE)
        });
        assert!(said.is_some(), "{at}: the control never said it");
        // Full of text: never.
        let mut guest = wanting(3, tue(14, 0), graphics);
        assert!(
            guest
                .ledger
                .home
                .shelf_spot(
                    &full,
                    plan_of(&view, &full),
                    &structures::KeepOff {
                        protected: &view.protected,
                        layer: &[],
                        made: &[],
                    },
                    None
                )
                .is_none(),
            "{at}: no shelf could go up"
        );
        begun_at(
            &mut guest,
            &full,
            &view,
            (56, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        let mut on_floor = 0;
        let mut paints = 0;
        let felt_or_said = run_until(&mut guest, &full, &view, 0, 600_000, |guest, _| {
            let visit = visit_of(guest);
            let osaka = &visit.osaka;
            on_floor += usize::from(on_users_floor((osaka.x, osaka.y)));
            paints += 1;
            osaka.felt().contains(&high())
                || osaka.said_aloud().iter().any(|&(line, _)| line == LINE)
                || osaka.episode().is_some_and(|ep| ep.repair.key == high())
                || visit.repairs.iter().any(|r| r.key == high())
        });
        assert_eq!(felt_or_said, None, "{at}: felt, said or set about");
        assert!(on_floor > 0, "{at}: she was in his room");
        assert!(
            !visit_of(&guest).osaka.felt().contains(&high()),
            "{at}: never felt"
        );
        // Unfelt in his room, whether a shelf could go up is asked again
        // only as what it reads changes (the text she moves), not each
        // paint she stands there.
        let asks = visit_of(&guest).shelf_asks;
        assert!(
            asks * 10 < on_floor,
            "{at}: asked {asks} times in {on_floor} paints in his room ({paints} in all)"
        );
    }
}

/// She feels her gremlin wants up only in his room (standing on its
/// floor), and says so there, turned to face him: begun on List's floor
/// she doesn't feel it while she's out of his room; begun on Users' floor
/// facing away from him, she says "It wants a shelf..." standing on that
/// floor, facing him. Industrious (she'd put it right). Both modes.
#[test]
fn the_grievance_is_said_in_its_room_facing_it() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        // Out of his room: felt only once she has stood in it.
        let mut guest = wanting(4, tue(14, 0), graphics);
        begun_at(
            &mut guest,
            &real,
            &view,
            (10, LOWER_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        let mut been = false;
        let mut frames = 0;
        let _ = run_until(&mut guest, &real, &view, 0, 120_000, |guest, _| {
            let osaka = &visit_of(guest).osaka;
            been |= on_users_floor((osaka.x, osaka.y));
            frames += 1;
            assert!(
                been || !osaka.felt().contains(&high()),
                "{at}: felt out of his room at ({}, {})",
                osaka.x,
                osaka.y
            );
            false
        });
        assert!(frames > 100, "{at}");
        // And felt once she has: she went up to his room within the two
        // minutes (seed 4), so the half above isn't vacuous.
        assert!(been, "{at}: never in his room");
        assert!(
            visit_of(&guest).osaka.felt().contains(&high()),
            "{at}: in his room, never felt"
        );
        // In his room, facing away: said facing him.
        let mut guest = wanting(4, tue(14, 0), graphics);
        let her = (90, USERS_FLOOR);
        begun_at(
            &mut guest,
            &real,
            &view,
            her,
            Facing::Right,
            Mood::Industrious,
        );
        let gremlin = gremlin_shown(&guest);
        let middle = gremlin.left + i32::from(gremlin.size().0) / 2;
        assert!(middle < her.0, "{at}: he's to her left, she faces right");
        let said = run_until(&mut guest, &real, &view, 0, 60_000, |guest, _| {
            visit_of(guest)
                .osaka
                .said_aloud()
                .iter()
                .any(|&(line, _)| line == LINE)
        });
        assert!(said.is_some(), "{at}: never said");
        let osaka = &visit_of(&guest).osaka;
        assert!(
            on_users_floor((osaka.x, osaka.y)),
            "{at}: said at ({}, {})",
            osaka.x,
            osaka.y
        );
        let toward = if middle < osaka.x {
            Facing::Left
        } else {
            Facing::Right
        };
        assert_eq!(osaka.facing, toward, "{at}: facing him as she says it");
    }
}

/// The story, end to end over game days on [`rooms`] at 100×30: her
/// gremlin on her floor (her bookshelf and her poster not yet bought, so
/// the channel would sell them first), she feels he wants up, says so,
/// and the channel sells her shelf kit before anything else; it comes,
/// she opens it, puts it up a stage a game day, and a repair puts him up
/// on it, where he wants for nothing. Seeded (seed 3: bought Monday,
/// the first half up Tuesday, finished and him up on it first thing on
/// Wednesday, 1.6 game days; most seeds go so, some a day later, as her
/// mood has her mend). One test a mode, so they run side by side.
fn wants_a_shelf_and_gets_one(graphics: bool) {
    const DAYS: u64 = 4;
    let (real, view) = rooms_frame(100, 30);
    let at = format!("graphics={graphics}");
    let mut guest = wanting(3, mon(16, 0), graphics);
    let until = guest.ledger.clock + DAYS * 24 * 60;
    let mut now = 0;
    let mut bought = Vec::new();
    let mut said = false;
    let mut built = 0;
    paint(&mut guest, &real, &view, now);
    let up = loop {
        assert!(
            guest.ledger.clock < until,
            "{at}: not up in {DAYS} game days: bought {bought:?}, said {said}, built {built}"
        );
        shell_step(&mut guest, &real, &view, &mut now, false);
        if let Some(item) = guest.ledger.ordered
            && bought.last() != Some(&item)
        {
            bought.push(item);
        }
        if let State::Visiting(visit) = &guest.state {
            said |= visit
                .osaka
                .said_aloud()
                .iter()
                .any(|&(line, _)| line == LINE);
        }
        let home = &guest.ledger.home;
        built = home
            .structures
            .first()
            .map_or(0, |&Structure::Shelf(Shelf { built, .. })| built);
        let gremlin = home.props.iter().find(|p| p.item == Furniture::Gremlin);
        if let Some(g) = gremlin.filter(|g| !g.strip.is_floor()) {
            break g.strip;
        }
    };
    assert_eq!(
        bought.first(),
        Some(&Furniture::ShelfKit),
        "{at}: {bought:?}"
    );
    assert!(said, "{at}: she said he wants a shelf");
    assert_eq!(built, 2, "{at}: up on a finished shelf");
    let Structure::Shelf(shelf) = guest.ledger.home.structures[0];
    assert_eq!(up, room::Strip::Shelf(shelf.nook), "{at}");
}

/// [`wants_a_shelf_and_gets_one`] in ASCII.
#[test]
fn the_gremlin_wants_a_shelf_and_gets_one() {
    wants_a_shelf_and_gets_one(false);
}

/// [`wants_a_shelf_and_gets_one`] in line art.
#[test]
fn the_gremlin_wants_a_shelf_and_gets_one_in_line_art() {
    wants_a_shelf_and_gets_one(true);
}

/// With her shelf finished and shown, his wanting up is felt in his
/// room, said, and mended: begun on Users' floor with her Users shelf up
/// (he came after it), she says "It wants a shelf..." and a repair sets
/// him on it. Both modes.
#[test]
fn she_feels_and_mends_it_with_her_shelf_up() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(3, tue(14, 0), graphics);
        guest
            .ledger
            .home
            .structures
            .push(super::shelf::users_shelf());
        begun_at(
            &mut guest,
            &real,
            &view,
            (56, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        assert_eq!(guest.shelves_shown, [Nook::Users], "{at}: her shelf shows");
        let said = run_until(&mut guest, &real, &view, 0, 60_000, |guest, _| {
            visit_of(guest)
                .osaka
                .said_aloud()
                .iter()
                .any(|&(line, _)| line == LINE)
        });
        assert!(said.is_some(), "{at}: never said");
        let up = run_until(
            &mut guest,
            &real,
            &view,
            said.unwrap(),
            600_000,
            |guest, _| {
                guest
                    .ledger
                    .home
                    .props
                    .iter()
                    .any(|p| p.item == Furniture::Gremlin && !p.strip.is_floor())
            },
        );
        assert!(up.is_some(), "{at}: never up");
        let gremlin = guest
            .ledger
            .home
            .props
            .iter()
            .find(|p| p.item == Furniture::Gremlin)
            .expect("her gremlin");
        assert_eq!(gremlin.strip, room::Strip::Shelf(Nook::Users), "{at}");
    }
}

/// Her shelf finished but hidden for good (text over its row), his
/// wanting up is no grievance: nothing could answer it (no repair finds
/// a place on a shelf the frame doesn't show, and her kit is hers, so
/// nothing's wished). In his room an hour of a fed afternoon,
/// industrious, she never feels it, never says so, and never sets about
/// moving him. Both modes.
#[test]
fn no_gremlin_grievance_with_her_shelf_hidden() {
    let (real, view) = rooms_frame(100, 30);
    let full = crowded(&real, &view.nooks);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(3, tue(14, 0), graphics);
        guest
            .ledger
            .home
            .structures
            .push(super::shelf::users_shelf());
        begun_at(
            &mut guest,
            &full,
            &view,
            (56, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        assert!(
            guest.shelves_shown.is_empty(),
            "{at}: her shelf hidden: {:?}",
            guest.shelves_shown
        );
        let mut on_floor = 0;
        let felt_or_said = run_until(&mut guest, &full, &view, 0, 600_000, |guest, _| {
            let visit = visit_of(guest);
            let osaka = &visit.osaka;
            on_floor += usize::from(on_users_floor((osaka.x, osaka.y)));
            osaka.felt().contains(&high())
                || osaka.said_aloud().iter().any(|&(line, _)| line == LINE)
                || osaka.episode().is_some_and(|ep| ep.repair.key == high())
                || visit.repairs.iter().any(|r| r.key == high())
        });
        assert_eq!(felt_or_said, None, "{at}: felt, said or set about");
        assert!(on_floor > 0, "{at}: she was in his room");
    }
}

/// Whether she has said his line.
fn said_it(guest: &Guest) -> bool {
    visit_of(guest)
        .osaka
        .said_aloud()
        .iter()
        .any(|&(line, _)| line == LINE)
}

/// Which way she'd face to look at her gremlin as the frame shows him
/// now, from where she stands.
fn toward_him(guest: &Guest) -> Facing {
    let gremlin = gremlin_shown(guest);
    let middle = gremlin.left + i32::from(gremlin.size().0) / 2;
    if middle < visit_of(guest).osaka.x {
        Facing::Left
    } else {
        Facing::Right
    }
}

/// Felt in his room but not yet said (her hello still showing), she's
/// put out of it: she says nothing of him while she's out of his room,
/// and says it once she's back on his floor, turned to face him. Both
/// modes.
#[test]
fn a_line_felt_in_his_room_waits_for_her_back_in_it() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(4, tue(14, 0), graphics);
        begun_at(
            &mut guest,
            &real,
            &view,
            (90, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        assert!(
            visit_of(&guest).osaka.felt().contains(&high()),
            "{at}: felt"
        );
        assert!(!said_it(&guest), "{at}: not said yet");
        visit_mut(&mut guest).osaka.place(10, LOWER_FLOOR, 100);
        paint(&mut guest, &real, &view, 100);
        let said = run_until(&mut guest, &real, &view, 100, 30_000, |guest, _| {
            said_it(guest)
        });
        if said.is_some() {
            // She walked back on her own: said there.
            let osaka = &visit_of(&guest).osaka;
            assert!(
                on_users_floor((osaka.x, osaka.y)),
                "{at}: said out of his room at ({}, {})",
                osaka.x,
                osaka.y
            );
            continue;
        }
        let back = 30_100;
        visit_mut(&mut guest).osaka.place(90, USERS_FLOOR, back);
        paint(&mut guest, &real, &view, back);
        let said = run_until(&mut guest, &real, &view, back, back + 30_000, |guest, _| {
            said_it(guest)
        });
        assert!(said.is_some(), "{at}: never said back in his room");
        let osaka = &visit_of(&guest).osaka;
        assert!(on_users_floor((osaka.x, osaka.y)), "{at}");
        assert_eq!(osaka.facing, toward_him(&guest), "{at}: facing him");
    }
}

/// Felt in his room but not yet said (her hello still showing), the
/// frame resizes (Users' floor a row lower and his room wider, so he
/// stands elsewhere): she says it on his room's floor as the frame has
/// it now, turned to face him where he stands now. Both modes.
#[test]
fn a_line_felt_in_his_room_is_said_where_a_resize_puts_it() {
    let (real, view) = rooms_frame(100, 30);
    let (big, big_view) = rooms_frame(110, 32);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(4, tue(14, 0), graphics);
        begun_at(
            &mut guest,
            &real,
            &view,
            (90, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        assert!(
            visit_of(&guest).osaka.felt().contains(&high()),
            "{at}: felt"
        );
        assert!(!said_it(&guest), "{at}: not said yet");
        let was = gremlin_shown(&guest).left;
        paint(&mut guest, &big, &big_view, 100);
        assert_ne!(gremlin_shown(&guest).left, was, "{at}: he moved with it");
        let said = run_until(&mut guest, &big, &big_view, 100, 60_000, |guest, _| {
            said_it(guest)
        });
        assert!(said.is_some(), "{at}: never said");
        let osaka = &visit_of(&guest).osaka;
        // Users at 110×32: `55, 0, 55, 14`, its floor row 13.
        assert!(
            osaka.y == USERS_FLOOR + 1 && (55..110).contains(&osaka.x),
            "{at}: said at ({}, {})",
            osaka.x,
            osaka.y
        );
        assert_eq!(osaka.facing, toward_him(&guest), "{at}: facing him");
    }
}

/// A High felt where no shelf is finished (a shelf could go up: the
/// frame clear, his kit not yet hers) is no repair, and costs her no
/// slot of her offers (D9): industrious, she'd mend it (`to_mend` names
/// it), but no repair is found for it, `Arrange` binds to nothing, and
/// every want binds as it would were she not minded to mend anything.
/// Both modes, over 64 whims.
#[test]
fn a_felt_high_with_no_shelf_takes_none_of_her_offers() {
    use crate::ui::houseguest::brain::Want;
    use crate::ui::houseguest::mind::{self, Ctx, Whims};
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(3, tue(14, 0), graphics);
        begun_at(
            &mut guest,
            &real,
            &view,
            (56, USERS_FLOOR),
            Facing::Right,
            Mood::Industrious,
        );
        paint(&mut guest, &real, &view, 100);
        paint(&mut guest, &real, &view, 1200);
        let visit = visit_of(&guest);
        let osaka = &visit.osaka;
        assert!(
            guest.ledger.home.structures.is_empty(),
            "{at}: no shelf of hers"
        );
        assert!(
            osaka.to_mend(&visit.chances.broken).contains(&high()),
            "{at}: felt, and she'd mend it: {:?}",
            osaka.felt()
        );
        assert_eq!(visit.chances.repairs, [], "{at}: no repair for it");
        let terrain = &visit.terrain;
        let here = terrain.platform_at(osaka.x, osaka.y).expect("on his floor");
        let ctx = |may_arrange| Ctx {
            x: osaka.x,
            y: osaka.y,
            here,
            links: terrain
                .links
                .iter()
                .filter(|l| l.from == here)
                .copied()
                .collect(),
            terrain,
            chances: &visit.chances,
            may_work: false,
            owes: false,
            episode: None,
            just_set: None,
            may_arrange,
            put_up_waits: false,
            near: false,
        };
        let (minded, not) = (ctx(true), ctx(false));
        for whims in (0..64).map(Whims) {
            assert!(
                mind::bind(&minded, whims, Want::Arrange).is_none(),
                "{at}: Arrange bound"
            );
            for want in Want::ALL {
                assert_eq!(
                    format!("{:?}", mind::bind(&minded, whims, want)),
                    format!("{:?}", mind::bind(&not, whims, want)),
                    "{at}: {want:?}"
                );
            }
        }
    }
}
