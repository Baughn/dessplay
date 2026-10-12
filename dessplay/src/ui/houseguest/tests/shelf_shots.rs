//! Her shelf and her gremlin in play, for the user's eye (phase 6a,
//! step 9): real game frames in line art at 9×19 px cells, written as
//! PNGs by the ignored [`shelf_shots`] (the door batch's `door_shots` is
//! the model; README.md beside the shots lists them).

use super::away::{Shot, contact_sheet_round, film};
use super::kit::{all_but_the_kit, arrived, shelf_spot_of};
use super::*;
use crate::ui::houseguest::brain::{Mood, Want};
use crate::ui::houseguest::room::Use;
use crate::ui::houseguest::structures::{SHELF_STAGES, SHELF_WIDTH, Shelf, Structure};

/// The user's terminal with an evening's content (`fuller_ui`).
fn users_terminal() -> (Buffer, IdleView) {
    real_frame(&mut super::super::stage::fuller_ui(), 191, 46)
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

/// Her shelf as her record has it (one in all, in 6a).
fn shelf_of(guest: &Guest) -> Option<Shelf> {
    match guest.ledger.home.structures[..] {
        [Structure::Shelf(shelf)] => Some(shelf),
        _ => None,
    }
}

/// What she shows saying this frame, if anything.
fn bubble_of(guest: &Guest, now: u64) -> String {
    match &guest.state {
        State::Visiting(visit) => visit
            .osaka
            .appearance(now)
            .2
            .map(|b| format!(" \"{}\"", b.text()))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// What a frame of her kit, her shelf going up and her gremlin going up
/// on it shows, for [`film`]: a parcel through her flap, her unpacking
/// (each line its own shot), each of her acts about her shelf (reaching
/// up, the hop, her pleased beat on its floor), her line for him, and
/// her carrying a piece up and setting it down (small decor only);
/// `None` for the rest (her walks, her other moves).
fn shelf_label(view: &IdleView, guest: &Guest, now: u64) -> Option<String> {
    let visit = match &guest.state {
        State::Visiting(visit) => visit,
        _ => return None,
    };
    let osaka = &visit.osaka;
    let said = bubble_of(guest, now);
    let built = shelf_of(guest).map_or(0, |s| s.built);
    if visit.flap.is_some() {
        return Some("a parcel through her flap".into());
    }
    if let Some((seat, ..)) = osaka.use_span() {
        return match seat.what {
            Use::Unpack => Some(format!("unpacking {:?}{said}", seat.item)),
            Use::GazeUp => Some(format!("gazing up at {:?}{said}", seat.item)),
            _ => None,
        };
    }
    // On her shelf's floor (her pleased beat there is the shelf's, not a
    // pull's elsewhere).
    let under = shelf_of(guest).is_some_and(|shelf| {
        view.nooks
            .iter()
            .any(|&(n, pane)| n == shelf.nook && osaka.y == i32::from(pane.bottom()) - 1)
    });
    let up_there = guest.ledger.home.props.iter().any(|p| !p.strip.is_floor());
    let decor = osaka.carrying().filter(|item| item.spec().shelf);
    let act = osaka.act_name();
    match act.as_str() {
        "PutUp" => Some(format!("putting up stage {}{said}", built + 1)),
        "Hop" if !guest.shelves_shown.is_empty() && up_there => {
            Some("the hop (a piece set up there)".into())
        }
        // From the first frame that shows the stage up (the frame that
        // takes it was written before it was up).
        "Hop" if shows_built(view, guest, visit) => Some(format!("the hop (stage {built} up)")),
        "Admire" if built > 0 && under => Some(format!("pleased{said} (stage {built})")),
        "SetDown" if decor.is_some() => Some(format!("setting {:?} down{said}", decor?)),
        _ if decor.is_some() => Some(format!("carrying {:?}{said}", decor?)),
        _ if said.contains("It wants a shelf") => Some(format!("{act}{said}")),
        _ => None,
    }
}

/// Whether the frame just painted shows every stage of her shelf her
/// record has up: each cell of its built run kept in what her home
/// painted (the frame that takes a stage was written before the stage
/// was up, so it shows one stage fewer).
fn shows_built(view: &IdleView, guest: &Guest, visit: &Visit) -> bool {
    let Some(shelf) = shelf_of(guest) else {
        return false;
    };
    let Some(run) = view
        .nooks
        .iter()
        .find(|(n, _)| *n == shelf.nook)
        .and_then(|&(_, pane)| shelf.run(room::pane_raw(pane)))
    else {
        return false;
    };
    let cells = shelf.built_on(run);
    !cells.is_empty()
        && cells.iter().all(|&(x, glyph)| {
            visit
                .painted
                .iter()
                .any(|c| !c.burst && (i32::from(c.x), i32::from(c.y), c.glyph) == (x, run.y, glyph))
        })
}

/// Every shot carried her images (she, or a piece of hers, is in each):
/// none is the shooting itself broken.
fn all_drawn(shots: &[Shot], what: &str) {
    assert!(!shots.is_empty(), "{what}: no shots");
    for shot in shots {
        assert!(
            shot.images > 0,
            "{what}: {:?} carried none of her images",
            shot.label
        );
    }
}

/// Labels of `shots`, for an assertion's message.
fn labels(shots: &[Shot]) -> Vec<&str> {
    shots.iter().map(|s| s.label.as_str()).collect()
}

/// Her kit, her shelf, her gremlin up, and her gaze, in line art, for
/// the user's eye (phase 6a step 9):
/// `HOUSEGUEST_SHELF_SHOTS=$PWD/docs/proposals/2026-10-02-houseguest-mind/phase6/shots
/// cargo test --release -p dessplay --lib shelf_shots -- --ignored --nocapture`
/// (about two minutes: past nextest's default kill). Writes (README.md there lists them):
/// - `kit.png`: her kit through her flap, unpacked ("Which way's up?"),
///   the first stage put up ("Up it goes..."), the hop, her pleased beat,
///   and the half shelf;
/// - `stage-2.png`: the second stage, its hop (on the first frame the
///   whole run shows) and her beat, finished; then her gremlin's line
///   ("It wants a shelf...") as her finished shelf shows;
/// - `carry-up.png`: her carrying him up onto it, the hop and her beat;
/// - `gaze-up.png`: her gremlin before and after he turned round, her
///   gaze up noticing it ("?", "Did you move...?"), and a gaze up with a
///   line of hers to him;
/// - `high-191x46.png`: a high shelf on the user's terminal, her plant
///   and gremlin on it, the whole screen;
/// - `low-shelf.png`: a low shelf on the census home's 9-row panes, her
///   gremlin on it and her poster beside it, the whole screen;
/// - `his-parcel.png`: her gremlin delivered, his parcel at rest,
///   unpacked, and him out of it.
///
/// Line art only (ASCII is dev/test-only, never reviewed as art).
/// Without the variable set it writes nothing and passes, so
/// `--run-ignored all` sweeps stay green.
#[test]
#[ignore = "writes PNGs for review: set HOUSEGUEST_SHELF_SHOTS"]
#[allow(clippy::too_many_lines)]
fn shelf_shots() {
    let Ok(dir) = std::env::var("HOUSEGUEST_SHELF_SHOTS") else {
        eprintln!("shelf_shots: HOUSEGUEST_SHELF_SHOTS unset; nothing written");
        return;
    };
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("creating {dir}: {e}"));
    let save = |image: &image::RgbaImage, name: &str| {
        let path = format!("{dir}/{name}");
        image
            .save(&path)
            .unwrap_or_else(|e| panic!("saving {path}: {e}"));
        eprintln!("shelf_shots: wrote {path}");
    };
    let cell = (9, 19);
    // Round her, 44 × 15 cells: her floor second from the bottom, so a
    // high shelf over it (7 rows up) and what stands on it show; a
    // parcel's slide round her door.
    let sheet = |shots: &[Shot], across: usize| {
        contact_sheet_round(shots, cell, across, (44, 15), |s| {
            if s.label.contains("flap") {
                s.door.map(|d| d.spot())
            } else {
                s.her.map(|(x, y)| (x + 2, y))
            }
        })
    };
    let (real, view) = users_terminal();

    // Her kit: delivered, unpacked, its first stage up.
    let mut guest = all_but_the_kit(4, true);
    guest.ledger.ordered = Some(Furniture::ShelfKit);
    arrived(&mut guest, &real, &view);
    let mut shots = Vec::new();
    let mut up_at = None;
    let now = film(
        &mut guest,
        (&real, &view),
        (0, 15 * 60_000),
        &mut shots,
        |g, t| shelf_label(&view, g, t),
        |g, t| {
            if shelf_of(g).is_some_and(|s| s.built == 1) {
                up_at.get_or_insert(t);
            }
            up_at.is_some_and(|up| t > up + 2_500)
        },
    );
    let mut half = Vec::new();
    film(
        &mut guest,
        (&real, &view),
        (now, 0),
        &mut half,
        |_, _| Some("the half shelf".into()),
        |_, _| true,
    );
    shots.extend(half);
    all_drawn(&shots, "her kit");
    let seen = labels(&shots);
    assert!(
        seen.iter().any(|l| l.contains("Which way's up?"))
            && seen.iter().any(|l| l.contains("Up it goes..."))
            && seen.contains(&"the hop (stage 1 up)")
            && seen.iter().any(|l| l.starts_with("pleased")),
        "her kit: {seen:?}"
    );
    let shelf = shelf_of(&guest).expect("her shelf");
    assert!(shelf.high(), "a high shelf at 191×46: {shelf:?}");
    save(&sheet(&shots, 4), "kit.png");

    // The second stage (its day's pace forgotten, offered alone).
    guest.ledger.paced = None;
    visit_mut(&mut guest).osaka.offer_only = Some((Want::PutUp, "put-up/stage"));
    let mut shots = Vec::new();
    let mut up_at = None;
    let now = film(
        &mut guest,
        (&real, &view),
        (now, 5 * 60_000),
        &mut shots,
        |g, t| shelf_label(&view, g, t),
        |g, t| {
            if shelf_of(g).is_some_and(|s| s.built == SHELF_STAGES) {
                up_at.get_or_insert(t);
            }
            up_at.is_some_and(|up| t > up + 2_500)
        },
    );
    all_drawn(&shots, "the second stage");
    assert!(
        shelf_of(&guest).is_some_and(|s| s.finished()),
        "finished: {:?}",
        labels(&shots)
    );
    let seen = labels(&shots);
    assert!(
        seen.contains(&"the hop (stage 2 up)")
            && seen.iter().any(|l| l.contains("It wants a shelf...")),
        "the second stage: {seen:?}"
    );
    save(&sheet(&shots, 4), "stage-2.png");

    // Her gremlin, on Users' floor: his line, and up he goes.
    let visit = visit_mut(&mut guest);
    visit.osaka.offer_only = None;
    visit.osaka.set_mood(Mood::Industrious);
    let mut shots = Vec::new();
    let mut up_at = None;
    film(
        &mut guest,
        (&real, &view),
        (now, 20 * 60_000),
        &mut shots,
        |g, t| shelf_label(&view, g, t),
        |g, t| {
            let up = g
                .ledger
                .home
                .props
                .iter()
                .any(|p| p.item == Furniture::Gremlin && !p.strip.is_floor());
            if up {
                up_at.get_or_insert(t);
            }
            up_at.is_some_and(|up| t > up + 2_500)
        },
    );
    all_drawn(&shots, "his carry up");
    assert!(up_at.is_some(), "never up: {:?}", labels(&shots));
    save(&sheet(&shots, 4), "carry-up.png");

    // His tiny life: her gremlin up on her shelf, on a Saturday whose
    // draw turns him; before, turned, and her gazes up at him.
    let seed = (0u64..)
        .find(|&s| brain::gremlin_turns(s, 5))
        .expect("a home");
    let mut guest = home_at(
        seed,
        sat(14, 0),
        &[(Furniture::Sofa, Nook::Users, 300)],
        true,
    );
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    let frame = paint(&mut guest, &real, &view, now);
    let spot = shelf_spot_of(&guest, &frame, &view).expect("a shelf goes up");
    let shelf = Shelf {
        built: SHELF_STAGES,
        ..spot
    };
    guest.ledger.home.structures.push(Structure::Shelf(shelf));
    assert!(guest.ledger.home.add(room::Prop {
        strip: room::Strip::Shelf(shelf.nook),
        anchor: Some(room::Anchor {
            side: room::Side::Left,
            offset: 4,
        }),
        ..room::Prop::new(Furniture::Gremlin, shelf.nook, 0, sprite::Facing::Right)
    }));
    let pane = view
        .nooks
        .iter()
        .find(|(n, _)| *n == shelf.nook)
        .expect("its pane")
        .1;
    let run = shelf.run(room::pane_raw(pane)).expect("it hangs");
    let floor = i32::from(pane.bottom()) - 1;
    now += 250;
    guest.advance(now);
    visit_mut(&mut guest)
        .osaka
        .place(run.from + SHELF_WIDTH / 2 - 6, floor, now);
    let mut shots = Vec::new();
    now = film(
        &mut guest,
        (&real, &view),
        (now, 0),
        &mut shots,
        |_, _| Some("him up, before".into()),
        |_, _| true,
    );
    assert!(guest.gremlin_turns(5), "turned by the day's draw");
    now = film(
        &mut guest,
        (&real, &view),
        (now + 50, 0),
        &mut shots,
        |_, _| Some("him turned round".into()),
        |_, _| true,
    );
    // Her hello said first (it would show over the gaze's opening).
    now = film(
        &mut guest,
        (&real, &view),
        (now, 8_000),
        &mut Vec::new(),
        |_, _| None,
        |_, _| false,
    );
    // Offered alone, each time she's free to choose.
    visit_mut(&mut guest).osaka.offer_only = Some((Want::Use(Use::GazeUp), "use/real"));
    let mut gazes = 0;
    let mut gazing = None;
    let mut lined = false;
    let pool = |g: &Guest, t: u64| {
        matches!(
            visit_of(g).osaka.appearance(t).2,
            Some(osaka::Bubble::Say(line)) if mind::SHELF.lines.contains(&line)
        )
    };
    film(
        &mut guest,
        (&real, &view),
        (now, 30 * 60_000),
        &mut shots,
        |g, t| {
            let span = visit_of(g)
                .osaka
                .use_span()
                .filter(|(seat, ..)| seat.what == Use::GazeUp);
            match span {
                Some((_, since, _)) => {
                    if gazing != Some(since) {
                        gazing = Some(since);
                        gazes += 1;
                    }
                    Some(format!("gaze up {gazes}{}", bubble_of(g, t)))
                }
                None => {
                    gazing = None;
                    None
                }
            }
        },
        |g, t| {
            lined |= pool(g, t);
            lined && visit_of(g).osaka.use_span().is_none()
        },
    );
    all_drawn(&shots, "his gaze up");
    let seen = labels(&shots);
    assert!(
        seen.iter().any(|l| l.contains("Did you move...?"))
            && seen
                .iter()
                .any(|l| mind::SHELF.lines.iter().any(|line| l.contains(line))),
        "his gaze up: {seen:?}"
    );
    save(&sheet(&shots, 4), "gaze-up.png");

    // The whole screen: a high shelf at 191×46 (the census's room).
    let room = super::census::wide_shelf_room();
    let mut guest = super::census::arrive_in(&room, 0, true, None);
    let mut full = Vec::new();
    film(
        &mut guest,
        (&room.real, &room.view),
        (0, 0),
        &mut full,
        |_, _| Some("high".into()),
        |_, _| true,
    );
    all_drawn(&full, "the high shelf");
    save(&full[0].image, "high-191x46.png");
    // And a low one on the census home's 9-row panes.
    let room = super::census::low_shelf_room();
    let mut guest = super::census::arrive_in(&room, 0, true, None);
    let mut full = Vec::new();
    film(
        &mut guest,
        (&room.real, &room.view),
        (0, 0),
        &mut full,
        |_, _| Some("low".into()),
        |_, _| true,
    );
    all_drawn(&full, "the low shelf");
    save(&full[0].image, "low-shelf.png");

    // His parcel: her gremlin ordered, her shelf up.
    let mut guest = all_but_the_kit(4, true);
    guest
        .ledger
        .home
        .props
        .retain(|p| p.item != Furniture::Gremlin);
    guest.ledger.ordered = Some(Furniture::Gremlin);
    guest.ledger.home.structures.push(Structure::Shelf(Shelf {
        built: SHELF_STAGES,
        ..shelf_spot_of(&guest, &real, &view).expect("a shelf goes up")
    }));
    arrived(&mut guest, &real, &view);
    let mut shots = Vec::new();
    let mut out_at = None;
    // Where his parcel stands (its middle, its floor): the sheet's crop.
    let parcel_at = std::cell::Cell::new(None);
    let now = film(
        &mut guest,
        (&real, &view),
        (0, 15 * 60_000),
        &mut shots,
        |g, t| {
            let visit = visit_of(g);
            let parcel = visit
                .shown
                .iter()
                .find(|s| s.item == Furniture::Gremlin && s.boxed());
            if let Some(p) = parcel {
                parcel_at.set(Some((p.left + 1, p.floor)));
            }
            match parcel {
                Some(_) if visit.flap.is_some() => Some("his parcel through her flap".into()),
                Some(_) => shelf_label(&view, g, t).or(Some("his parcel at rest".into())),
                None => shelf_label(&view, g, t),
            }
        },
        |g, t| {
            let out = visit_of(g)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Gremlin && !s.boxed());
            if out {
                out_at.get_or_insert(t);
            }
            out_at.is_some_and(|o| t > o + 2_000)
        },
    );
    film(
        &mut guest,
        (&real, &view),
        (now, 0),
        &mut shots,
        |_, _| Some("him out of his box".into()),
        |_, _| true,
    );
    all_drawn(&shots, "his parcel");
    assert!(
        labels(&shots).contains(&"his parcel at rest") && out_at.is_some(),
        "his parcel: {:?}",
        labels(&shots)
    );
    let at = parcel_at.get().expect("his parcel shown");
    let parcel_sheet = contact_sheet_round(&shots, cell, 3, (44, 15), |_| Some(at));
    save(&parcel_sheet, "his-parcel.png");
}
