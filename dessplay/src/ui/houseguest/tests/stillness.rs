//! Her stillness levers (phase 5c step 7) in her rooms as drawn, in
//! both modes: each turned on in its test (they ship neutral until the
//! band is tuned), on the stillness band's fed afternoon
//! ([`fed_afternoon`]). The levers' own rules are pinned beside them
//! (osaka.rs, mind.rs); these check they hold in a real room, its
//! terrain and what it offers read from the frame.

use super::census::{
    Room, at_afternoon, fed_afternoon, furnished_room, resident_room, stage_room, visit_from,
};
use super::*;
use crate::ui::houseguest::brain::{Mood, Want};
use crate::ui::houseguest::mind::{self, Bind, Ctx, Whims};
use crate::ui::houseguest::osaka::SETTLE_IN;
use crate::ui::houseguest::scenes::Job;
use crate::ui::houseguest::stillness::{ByMood, Stillness};

/// Her, fed an afternoon in `room` from `seed` in `mood`, with the
/// levers `still`.
fn fed_with(room: &Room, seed: u64, graphics: bool, mood: Mood, still: Stillness) -> Guest {
    let mut guest = fed_afternoon(room, seed, graphics, mood);
    match &mut guest.state {
        State::Visiting(visit) => visit.osaka.stillness = still,
        _ => panic!("visiting"),
    }
    guest
}

/// With settling always on, in a furnished home and as a resident, both
/// modes, over a few fed afternoons: she settles in, and each time it's
/// where she was, on calm floor, setting off for nothing.
#[test]
fn in_her_rooms_she_settles_in_where_she_is() {
    let still = Stillness {
        settle: ByMood::all(1.0),
        ..Stillness::STARTING
    };
    for room in [furnished_room(), resident_room()].map(at_afternoon) {
        for graphics in [false, true] {
            let at = format!("{} graphics={graphics}", room.name);
            let mut settled = 0;
            for (seed, mood) in [(0, Mood::Lazy), (1, Mood::Dreamy), (2, Mood::Ordinary)] {
                // Her before the step just taken: where, what she had
                // decided, how often she had set off.
                let mut before: Option<(i32, i32, usize, u32)> = None;
                let guest = fed_with(&room, seed, graphics, mood, still);
                visit_from(&room, guest, 8, |guest, now| {
                    let State::Visiting(visit) = &guest.state else {
                        return;
                    };
                    let osaka = &visit.osaka;
                    if let Some((x, y, decided, set_offs)) = before {
                        for decision in &osaka.decisions[decided..] {
                            if decision.method == SETTLE_IN {
                                settled += 1;
                                assert_eq!((osaka.x, osaka.y), (x, y), "{at} at {now}: moved");
                                assert_eq!(osaka.set_offs, set_offs, "{at} at {now}: set off");
                                assert!(
                                    visit.terrain.restful(x, y),
                                    "{at} at {now}: settled over text"
                                );
                            }
                        }
                    }
                    before = Some((osaka.x, osaka.y, osaka.decisions.len(), osaka.set_offs));
                });
            }
            println!("{at}: settled in {settled} times");
            assert!(settled > 0, "{at}: never settled in");
        }
    }
}

/// With three musings to a daydream, close together, in a furnished
/// home and as a resident, both modes: a daydream cued from the stage
/// (as she'd muse) that isn't a riddle says three musings in turn, each
/// one of her musings, and then only spaces out.
#[test]
fn in_her_rooms_a_daydream_muses_on() {
    let still = Stillness {
        musings: ByMood::all((3, 3)),
        musing_gap: (3_000, 3_000),
        linger: ByMood::all(2.0),
        ..Stillness::STARTING
    };
    for room in [furnished_room(), resident_room()].map(at_afternoon) {
        for graphics in [false, true] {
            let at = format!("{} graphics={graphics}", room.name);
            let mut sessions = 0;
            for seed in 0..6 {
                let mut guest = fed_with(&room, seed, graphics, Mood::Ordinary, still);
                let start = 1_000;
                guest.cue(Scene::Muse);
                paint(&mut guest, &room.real, &room.view, start);
                let State::Visiting(visit) = &guest.state else {
                    panic!("{at}: visiting");
                };
                if visit.osaka.act_name() != "SpaceOut" || visit.osaka.plays().is_some() {
                    continue;
                }
                let decided = visit.osaka.decisions.len();
                let mut musings: Vec<&'static str> = Vec::new();
                let mut now = start;
                while now < start + 12_000 {
                    let State::Visiting(visit) = &guest.state else {
                        panic!("{at}: visiting");
                    };
                    let osaka = &visit.osaka;
                    assert_eq!(
                        osaka.decisions.len(),
                        decided,
                        "{at} seed {seed}: one daydream"
                    );
                    if let (_, _, Some(osaka::Bubble::Say(text))) = osaka.appearance(now)
                        && musings.last() != Some(&text)
                    {
                        musings.push(text);
                    }
                    now += 250;
                    if guest.advance(now) {
                        paint(&mut guest, &room.real, &room.view, now);
                    }
                }
                assert!(
                    musings
                        .iter()
                        .all(|line| mind::MUSINGS.lines.contains(line)),
                    "{at} seed {seed}: {musings:?}"
                );
                assert!(musings.len() <= 3, "{at} seed {seed}: {musings:?}");
                if musings.len() == 3 {
                    sessions += 1;
                }
            }
            println!("{at}: {sessions} daydreams mused on");
            assert!(sessions > 0, "{at}: no daydream mused on");
        }
    }
}

/// With nearer spots drawing her, on the stage as drawn in each mode
/// (chat text on its floors, read from the frame): standing anywhere on
/// any floor, a line to pull and letters to swap are on her own floor
/// whenever it has any, and elsewhere only when it has none.
#[test]
fn on_the_stage_nearer_text_is_on_her_own_floor() {
    let room = at_afternoon(stage_room());
    for graphics in [false, true] {
        let guest = fed_with(&room, 0, graphics, Mood::Ordinary, Stillness::STARTING);
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting");
        };
        let (terrain, chances) = (&visit.terrain, &visit.chances);
        let mut floors_with = [0; 2];
        for (here, floor) in terrain.platforms.iter().enumerate() {
            for x in [floor.x0, (floor.x0 + floor.x1) / 2, floor.x1] {
                let ctx = Ctx {
                    x,
                    y: floor.y,
                    here,
                    links: Vec::new(),
                    terrain,
                    chances,
                    may_work: false,
                    owes: false,
                    episode: None,
                    just_set: None,
                    may_arrange: false,
                    near: true,
                };
                for (want, spots) in [
                    (
                        Want::Pull,
                        chances.pulls.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(),
                    ),
                    (
                        Want::Swap,
                        chances.swaps.iter().map(|s| (s.x, s.y)).collect(),
                    ),
                ] {
                    let mine = spots
                        .iter()
                        .any(|&(sx, sy)| terrain.platform_at(sx, sy) == Some(here));
                    floors_with[usize::from(mine)] += 1;
                    for w in 0..40 {
                        let spot = match mind::bind(&ctx, Whims(w), want) {
                            Some((_, Bind::Job(Job::Pull(p)))) => (p.x, p.y),
                            Some((_, Bind::Job(Job::Swap(s)))) => (s.x, s.y),
                            _ => continue,
                        };
                        assert_eq!(
                            terrain.platform_at(spot.0, spot.1) == Some(here),
                            mine,
                            "graphics={graphics}: {want:?} from ({x}, {}) to {spot:?}",
                            floor.y
                        );
                    }
                }
            }
        }
        println!("graphics={graphics}: floors without, with text: {floors_with:?}");
        assert!(
            floors_with.iter().all(|&n| n > 0),
            "graphics={graphics}: {floors_with:?}"
        );
    }
}
