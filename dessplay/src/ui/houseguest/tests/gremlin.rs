//! Her gremlin (phase 6a, step 7; design D9): a figurine of small decor
//! who wants up high. On a floor he breaks `Rule::High`, which she feels
//! on sight only in his room, and only where a shelf stands or could go
//! up, and never while her shelf kit is coming (on order, boxed or going
//! up: the user's seventh round; one felt before, she lets be); she says
//! "It wants a shelf..." turned to face him; a felt rule
//! that wants a kit she doesn't own puts that kit first on the channel
//! (her wishlist, felt anew each visit); he's sold only where a shelf
//! stands or could go up; and a shelf, once up, is where a repair puts
//! him. Both modes.

use super::*;
use crate::ui::houseguest::brain::{self, Mood, Need, Needs};
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
        // Wished, so the channel pitches it for him (phase 6a step 8).
        assert_eq!(visit.chances.wished, Some(Furniture::ShelfKit), "{at}");
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
        assert_eq!(visit.chances.wished, None, "{at}");
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
        assert_eq!(visit.chances.wished, Some(Furniture::ShelfKit), "{at}");
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
/// on it, where he wants for nothing. From her order till her shelf is
/// finished she neither feels nor says it (the user's seventh round). Seeded (seed 3: bought Monday,
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
    // His lines this visit, as last seen.
    let mut lines = 0;
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
            let osaka = &visit.osaka;
            let now_lines = osaka
                .said_aloud()
                .iter()
                .filter(|&&(line, _)| line == LINE)
                .count();
            // Not a word of it, nor a thought, while her shelf is coming
            // (the user's seventh round).
            if her_shelf_coming(&guest.ledger) {
                let clock = guest.ledger.clock;
                assert!(
                    now_lines <= lines,
                    "{at}: said it with her shelf coming, at {clock} (bought {bought:?})"
                );
                assert!(
                    !osaka.felt().contains(&high()),
                    "{at}: felt it with her shelf coming, at {clock} (bought {bought:?})"
                );
            }
            said |= now_lines > 0;
            lines = now_lines;
        } else {
            lines = 0;
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

/// Whether a shelf of hers is coming (the user's seventh round, after
/// step 8: "It wants a shelf..." stops once the kit is ordered): her
/// shelf kit on order, in its box, or her shelf going up. The tests'
/// own reading of the rule.
fn her_shelf_coming(ledger: &Ledger) -> bool {
    ledger.ordered == Some(Furniture::ShelfKit)
        || ledger
            .home
            .props
            .iter()
            .any(|p| p.item == Furniture::ShelfKit)
        || ledger
            .home
            .structures
            .iter()
            .any(|&Structure::Shelf(shelf)| !shelf.finished())
}

/// In his room for an hour of a fed afternoon, from `(56, USERS_FLOOR)`,
/// industrious: whether she ever felt his wanting up, said so, or set
/// about moving him (`None`: never), how many paints she stood on his
/// floor, and when she first had a shelf going up, if she did.
fn an_hour_in_his_room(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
) -> (Option<u64>, usize, Option<u64>) {
    begun_at(
        guest,
        real,
        view,
        (56, USERS_FLOOR),
        Facing::Right,
        Mood::Industrious,
    );
    let mut on_floor = 0;
    let mut going_up = None;
    let felt_or_said = run_until(guest, real, view, 0, 600_000, |guest, now| {
        let visit = visit_of(guest);
        let osaka = &visit.osaka;
        on_floor += usize::from(on_users_floor((osaka.x, osaka.y)));
        if going_up.is_none() && !guest.ledger.home.structures.is_empty() {
            going_up = Some(now);
        }
        osaka.felt().contains(&high())
            || osaka.said_aloud().iter().any(|&(line, _)| line == LINE)
            || osaka.episode().is_some_and(|ep| ep.repair.key == high())
            || visit.repairs.iter().any(|r| r.key == high())
    });
    (felt_or_said, on_floor, going_up)
}

/// Her shelf kit on order (the user's seventh round: "It wants a
/// shelf..." stops once the kit is ordered), her gremlin on her floor is
/// no grievance, though a shelf could go up on the frame (the same home
/// and frame with no kit coming says it at once:
/// `no_gremlin_grievance_where_no_shelf_can_go`'s control): an hour in
/// his room, industrious, she never feels it, never says so and never
/// sets about moving him, and nothing asks whether a shelf could go up
/// (it's coming). Both modes.
#[test]
fn no_gremlin_grievance_with_her_kit_on_order() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(3, tue(14, 0), graphics);
        guest.ledger.ordered = Some(Furniture::ShelfKit);
        // Bought this visit: it comes on her next.
        guest.ledger.bought_on = guest.ledger.visits;
        let (felt_or_said, on_floor, _) = an_hour_in_his_room(&mut guest, &real, &view);
        assert_eq!(felt_or_said, None, "{at}: felt, said or set about");
        assert!(on_floor > 0, "{at}: she was in his room");
        assert_eq!(
            guest.ledger.ordered,
            Some(Furniture::ShelfKit),
            "{at}: still on order"
        );
        assert_eq!(
            visit_of(&guest).shelf_asks,
            0,
            "{at}: asked whether a shelf could go up"
        );
    }
}

/// Her shelf kit in its box (delivered, unopened: the user's seventh
/// round), her gremlin on her floor is no grievance: an hour in his
/// room, industrious, she never feels it, never says so and never sets
/// about moving him; and she opens the kit and puts its first half up as
/// ever (its seat never waited on his wanting). Both modes.
#[test]
fn no_gremlin_grievance_with_her_kit_boxed() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = wanting(3, tue(14, 0), graphics);
        assert!(guest.ledger.home.add(room::Prop::new(
            Furniture::ShelfKit,
            Nook::List,
            900,
            Facing::Right
        )));
        let (felt_or_said, on_floor, going_up) = an_hour_in_his_room(&mut guest, &real, &view);
        assert_eq!(felt_or_said, None, "{at}: felt, said or set about");
        assert!(on_floor > 0, "{at}: she was in his room");
        assert!(going_up.is_some(), "{at}: she never opened her kit");
    }
}

/// Felt and owed in his room with no kit coming (her hello still
/// showing, so not yet said), she buys her shelf kit (the user's seventh
/// round): the line goes unsaid, and she lets his wanting be: she no
/// longer feels it, would mend nothing for it, and wishes for nothing,
/// so nothing about him weighs on her while her shelf is coming. The
/// same visit with no purchase says it. Both modes.
#[test]
fn a_line_owed_as_she_buys_her_kit_goes_unsaid() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        for buy in [false, true] {
            let at = format!("graphics={graphics}, buy={buy}");
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
            if buy {
                visit_mut(&mut guest)
                    .osaka
                    .did(osaka::HomeEvent::Bought(Furniture::ShelfKit));
                paint(&mut guest, &real, &view, 100);
                assert_eq!(guest.ledger.ordered, Some(Furniture::ShelfKit), "{at}");
                let visit = visit_of(&guest);
                assert!(
                    !visit.osaka.felt().contains(&high()),
                    "{at}: still felt: {:?}",
                    visit.osaka.felt()
                );
                assert_eq!(visit.osaka.to_mend(&visit.broken), [], "{at}");
                assert_eq!(visit.wished, None, "{at}: still wished");
            }
            let said = run_until(&mut guest, &real, &view, 100, 60_000, |guest, _| {
                said_it(guest)
            });
            assert_eq!(said.is_some(), !buy, "{at}: said at {said:?}");
        }
    }
}

/// How she goes out of her sight, for his tiny life (phase 6a step 8).
#[derive(Clone, Copy, Debug)]
enum Going {
    /// To sleep for the night, her lamp going off: Monday's bedtime, a
    /// school night (game day 0).
    Asleep,
    /// Out by her door to school: Tuesday morning (game day 1).
    School,
    /// Out by her door to her part-time job: Saturday (game day 5).
    Work,
}

impl Going {
    /// Whether she has gone so at `now`, as the test sees it (not as the
    /// guest does: [`Osaka::out_of_sight`] is what's tested): asleep for
    /// the night with her lamp off; or out of sight, her visit going on
    /// behind her door (to work), or over (to school).
    fn gone(self, guest: &Guest, now: u64) -> bool {
        let State::Visiting(visit) = &guest.state else {
            return matches!(self, Self::School);
        };
        let osaka = &visit.osaka;
        match self {
            Self::Asleep => {
                osaka.sleeping()
                    && osaka.prop(now) == Some(crate::ui::houseguest::script::Prop::LampOff)
            }
            Self::School | Self::Work => osaka.hidden(now),
        }
    }

    /// When the home starts, and the game day she goes on.
    fn start(self) -> (routine::GameTime, u64) {
        match self {
            Self::Asleep => (mon(22, 20), 0),
            Self::School => (tue(8, 5), 1),
            Self::Work => (sat(11, 0), 5),
        }
    }
}

/// Where her gremlin stands, for his tiny life.
#[derive(Clone, Copy, Debug)]
enum Stands {
    /// On her finished shelf on Users' wall.
    Up,
    /// On Users' floor, under it.
    Floor,
    /// His record on her shelf's strip, her shelf only half up (shown on
    /// his room's floor: a record could say so).
    HalfUp,
    /// His record on her shelf, in his box (none could be put there, as
    /// her record is written; one read could say so).
    Boxed,
}

/// Her bed, her sofa (and, `fridge`, her fridge, for a midnight snack),
/// her shelf on Users' wall and her gremlin where `stands` says.
fn turning_home(
    seed: u64,
    start: routine::GameTime,
    stands: Stands,
    fridge: bool,
    graphics: bool,
) -> Guest {
    use Furniture::{Bed, Fridge, Gremlin, Sofa};
    let mut pieces = vec![(Bed, Nook::Playlist, 500), (Sofa, Nook::Users, 200)];
    if fridge {
        pieces.push((Fridge, Nook::Playlist, 100));
    }
    let mut guest = home_at(seed, start, &pieces, graphics);
    let shelf = match super::shelf::users_shelf() {
        Structure::Shelf(shelf) if matches!(stands, Stands::HalfUp) => {
            Structure::Shelf(Shelf { built: 1, ..shelf })
        }
        shelf => shelf,
    };
    guest.ledger.home.structures.push(shelf);
    let up = room::Prop {
        item: Gremlin,
        ..super::shelf::mantel_clock()
    };
    let gremlin = match stands {
        Stands::Up | Stands::HalfUp => up,
        Stands::Boxed => room::Prop {
            boxed: true.into(),
            ..up
        },
        Stands::Floor => room::Prop::new(Gremlin, Nook::Users, 420, Facing::Right),
    };
    assert!(guest.ledger.home.add(gremlin));
    assert_eq!(
        guest
            .ledger
            .home
            .props
            .iter()
            .any(|p| p.item == Gremlin && p.boxed()),
        matches!(stands, Stands::Boxed),
        "{stands:?}"
    );
    guest
}

/// Her gremlin's facing in `ledger`.
fn his_facing(ledger: &Ledger) -> Facing {
    ledger
        .home
        .props
        .iter()
        .find(|p| p.item == Furniture::Gremlin)
        .expect("her gremlin")
        .facing
}

/// Her gremlin's facing as her home shows him, while she's visiting.
fn shown_facing(guest: &Guest) -> Option<Facing> {
    match &guest.state {
        State::Visiting(visit) => visit
            .shown
            .iter()
            .find(|s| s.item == Furniture::Gremlin)
            .map(|s| s.facing),
        _ => None,
    }
}

/// Whether she's visiting and can't see her home at `now`.
fn out_of_sight(guest: &Guest, now: u64) -> bool {
    matches!(&guest.state, State::Visiting(visit) if visit.osaka.out_of_sight(now).is_some())
}

/// Sends her off to work by her door at `now` (her shift's gap drawn).
fn off_to_work(guest: &mut Guest, now: u64) {
    let visit = visit_mut(guest);
    let (terrain, chances) = (visit.terrain.clone(), visit.chances.clone());
    visit.osaka.go_to_work(&terrain, &chances, now, &mut Rng(1));
}

/// His tiny life (phase 6a step 8, the user's extra 1): up on her shelf,
/// her gremlin turns round now and then, never while she's awake and in
/// sight: drawn once a game day (one in [`brain::TURNS_ONE_IN`]), at the
/// first moment that day she can't see him: asleep for the night, her
/// lamp off (not as she settles, the lamp still on); out by her door to
/// school; out by her door to work, the visit going on behind it (a
/// sibling of school's: missed as first built). On a day the draw turns
/// him (the first home whose seed does), he's turned from that moment,
/// his record's facing written at once (handed out for saving that very
/// frame: nothing else of hers is unsaved then, every earlier frame's
/// taken), and she has it to notice; on one it doesn't, never. Once that
/// day: no more. Both modes.
#[test]
fn her_gremlin_turns_round_while_she_sleeps_or_is_out() {
    let (real, view) = rooms_frame(100, 30);
    for going in [Going::Asleep, Going::School, Going::Work] {
        let (start, day) = going.start();
        for turns in [true, false] {
            let seed = (0u64..)
                .find(|&s| brain::gremlin_turns(s, day) == turns)
                .expect("a home");
            for graphics in [false, true] {
                let at = format!("{going:?} turns {turns} graphics={graphics}");
                let mut guest = turning_home(seed, start, Stands::Up, false, graphics);
                let before = his_facing(&guest.ledger);
                let mut now = until_visiting(&mut guest, &real, &view, 0);
                assert_eq!(shown_facing(&guest), Some(before), "{at}: up on her shelf");
                if matches!(going, Going::Work) {
                    off_to_work(&mut guest, now);
                }
                // Awake and in sight: as he was.
                let mut awake = 0;
                while !going.gone(&guest, now) {
                    awake += 1;
                    assert!(now < 400_000, "{at}: never asleep or out");
                    assert!(!out_of_sight(&guest, now), "{at}: {now}: out of sight");
                    assert_eq!(his_facing(&guest.ledger), before, "{at}: {now}");
                    if let Some(shown) = shown_facing(&guest) {
                        assert_eq!(shown, before, "{at}: shown at {now}");
                    }
                    assert!(!guest.turned, "{at}: {now}");
                    // Taken for saving, as the shell takes it.
                    guest.ledger_to_save();
                    shell_step(&mut guest, &real, &view, &mut now, true);
                }
                // (To work by a door in space where she stands: soon.)
                let least = if matches!(going, Going::Work) { 3 } else { 20 };
                assert!(awake > least, "{at}: awake {awake} frames");
                let was = his_facing(&guest.ledger);
                assert_eq!(was != before, turns, "{at}: turned as she went");
                assert_eq!(guest.turned, turns, "{at}: hers to notice");
                assert_eq!(guest.gremlin_drawn, Some(day), "{at}");
                let saved = guest.ledger_to_save();
                if turns {
                    assert_eq!(
                        saved.as_ref().map(his_facing),
                        Some(was),
                        "{at}: written at once"
                    );
                }
                // Once that day: no more.
                for _ in 0..40 {
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    assert_eq!(his_facing(&guest.ledger), was, "{at}: {now}");
                    if !matches!(going, Going::School) {
                        assert_eq!(shown_facing(&guest), Some(was), "{at}: shown at {now}");
                    }
                }
            }
        }
    }
}

/// He turns only standing up on her finished shelf, out of his box
/// (phase 6a step 8: "a gremlin standing on a shelf"): on a day whose
/// draw would turn him, drawn as she falls asleep, a gremlin on her
/// floor, one whose record has him on her shelf while it's only half up
/// (shown on his room's floor), and one in his box on it (a record could
/// say so) stay as they were, nothing hers to notice. Both modes.
#[test]
fn he_turns_only_up_on_her_finished_shelf() {
    let (real, view) = rooms_frame(100, 30);
    let (start, day) = Going::Asleep.start();
    let seed = (0u64..)
        .find(|&s| brain::gremlin_turns(s, day))
        .expect("a home");
    for stands in [Stands::Floor, Stands::HalfUp, Stands::Boxed] {
        for graphics in [false, true] {
            let at = format!("{stands:?} graphics={graphics}");
            let mut guest = turning_home(seed, start, stands, false, graphics);
            let before = his_facing(&guest.ledger);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            while !out_of_sight(&guest, now) {
                assert!(now < 400_000, "{at}: never asleep");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            for _ in 0..10 {
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            assert_eq!(guest.gremlin_drawn, Some(day), "{at}: drawn");
            assert_eq!(his_facing(&guest.ledger), before, "{at}: turned");
            assert!(!guest.turned, "{at}: hers to notice");
        }
    }
}

/// His turning is drawn at the moments she goes, for the day that spell
/// began on (phase 6a step 8), never as midnight passes while she sleeps,
/// nor as she gets back into bed after her midnight snack. A home whose
/// draw keeps him still on Monday (game day 0) and turns him on Tuesday,
/// with a snack on Monday night: asleep from Monday's bedtime through
/// midnight, up for her snack in the small hours of Tuesday and back to
/// bed, he stays as he was (it's Monday's night; Tuesday's draw isn't
/// hers to make yet), and he turns as she goes out by her door to school
/// on Tuesday. Red as first built: the draw read "asleep" each tick, so
/// Tuesday's came at midnight, in her sleep, and none as she went out;
/// then, drawn on her falling asleep, Tuesday's came as she got back
/// into bed after her snack. Both modes.
#[test]
fn his_turning_is_drawn_as_she_goes_not_at_midnight() {
    let (real, view) = rooms_frame(100, 30);
    let seed = (0u64..)
        .find(|&s| {
            !brain::gremlin_turns(s, 0)
                && brain::gremlin_turns(s, 1)
                && brain::night_snack(s, 1).is_some()
        })
        .expect("a home");
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = turning_home(seed, mon(22, 20), Stands::Up, true, graphics);
        let before = his_facing(&guest.ledger);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let midnight = real_of(&guest, now, tue(0, 0));
        let (mut slept, mut past_midnight, mut snacked, mut back_to_bed) =
            (false, false, false, false);
        while !out_of_sight(&guest, now) || asleep(&guest) {
            assert!(now < midnight + 3 * 3_600_000, "{at}: never out to school");
            let up_tonight = slept && now > midnight && !asleep(&guest) && {
                let day = guest.day(now).expect("fed");
                day.slot == routine::Slot::Asleep
            };
            snacked |= up_tonight;
            back_to_bed |= snacked && asleep(&guest);
            slept |= asleep(&guest);
            past_midnight |= asleep(&guest) && now > midnight;
            assert_eq!(
                his_facing(&guest.ledger),
                before,
                "{at}: {now}: turned while she's in"
            );
            assert!(!guest.turned, "{at}: {now}");
            shell_step(&mut guest, &real, &view, &mut now, false);
        }
        assert!(slept && past_midnight, "{at}: asleep through midnight");
        assert!(
            snacked && back_to_bed,
            "{at}: up for her snack and back to bed"
        );
        assert_ne!(
            his_facing(&guest.ledger),
            before,
            "{at}: turned as she went out"
        );
        assert!(guest.turned, "{at}: hers to notice");
        assert_eq!(guest.gremlin_drawn, Some(1), "{at}");
    }
}

/// Once a game day, however often she goes (phase 6a step 8): a home
/// whose draw turns him on Tuesday; he turns as she goes out to school,
/// and coming home from school and asleep that night (her second going
/// that day), he stays as he was turned.
#[test]
fn he_turns_once_a_day_however_often_she_goes() {
    let (real, view) = rooms_frame(100, 30);
    let (start, day) = Going::School.start();
    let seed = (0u64..)
        .find(|&s| brain::gremlin_turns(s, day))
        .expect("a home");
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = turning_home(seed, start, Stands::Up, false, graphics);
        let before = his_facing(&guest.ledger);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        while !out_of_sight(&guest, now) {
            assert!(now < 400_000, "{at}: never out");
            shell_step(&mut guest, &real, &view, &mut now, false);
        }
        let turned = his_facing(&guest.ledger);
        assert_ne!(turned, before, "{at}: turned as she went to school");
        let bedtime = real_of(&guest, now, tue(23, 30));
        let (mut back, mut slept) = (false, false);
        while !slept {
            assert!(now < bedtime, "{at}: never home and asleep");
            shell_step(&mut guest, &real, &view, &mut now, false);
            back |= matches!(guest.state, State::Visiting(_)) && !out_of_sight(&guest, now);
            slept |= back && asleep(&guest) && out_of_sight(&guest, now);
            assert_eq!(his_facing(&guest.ledger), turned, "{at}: {now}");
        }
        for _ in 0..10 {
            shell_step(&mut guest, &real, &view, &mut now, false);
        }
        assert_eq!(his_facing(&guest.ledger), turned, "{at}: turned back");
        assert_eq!(guest.gremlin_drawn, Some(day), "{at}");
    }
}

/// Never as she says goodbye (she waves, in sight), nor with no routine
/// reaching her (unfed: a visit is no day of his), however a day's draw
/// would go (phase 6a step 8). Both modes.
#[test]
fn he_never_turns_at_a_goodbye_or_unfed() {
    let (real, view) = rooms_frame(100, 30);
    // Saturday evening: her job closed, not yet bedtime.
    let (start, day) = (sat(18, 0), 5);
    let seed = (0u64..)
        .find(|&s| brain::gremlin_turns(s, day))
        .expect("a home");
    for fed in [true, false] {
        for graphics in [false, true] {
            let at = format!("fed {fed} graphics={graphics}");
            let mut guest = turning_home(seed, start, Stands::Up, false, graphics);
            guest.set_feed_clock(fed);
            let before = his_facing(&guest.ledger);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            for _ in 0..200 {
                assert!(!out_of_sight(&guest, now), "{at}: {now}: out of sight");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            guest.activity(now);
            let end = now + dissolve::DURATION_MS + 5_000;
            while guest.present() {
                assert!(now < end, "{at}: never gone");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            assert_eq!(his_facing(&guest.ledger), before, "{at}: turned");
            assert!(!guest.turned, "{at}");
            assert_eq!(guest.gremlin_drawn, None, "{at}: drawn");
        }
    }
}
