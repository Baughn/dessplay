//! The credit class (phase 5c D0b): whatever she chooses that serves a
//! need eases it, by some path. Statically, every want by every method
//! has its path ([`Osaka::credit_path`]); in a room where the method
//! binds, chosen by it and run to its end, it eases her
//! ([`Osaka::served`], which every credit passes through). What she is
//! drawn as doesn't bear on what eases her, so these run in ASCII (but
//! for the pit, whose floor has no way off it only in line art).

use super::*;
use crate::ui::houseguest::brain::Want;
use crate::ui::houseguest::mind;
use crate::ui::houseguest::osaka::{Activity, Bucket, CreditPath, Served, Via};
use crate::ui::houseguest::room::Use;
use crate::ui::houseguest::scrap::MAKES;

/// Every want by every one of its methods has a way to ease what it
/// serves, and only a want that serves nothing has none.
#[test]
fn every_want_that_serves_has_a_credit_path() {
    for want in Want::ALL {
        for method in mind::methods(want) {
            let path = Osaka::credit_path(want, method.name)
                .unwrap_or_else(|| panic!("{want:?} by {}: no credit path", method.name));
            assert_eq!(
                path == CreditPath::Nothing,
                want.def().serves.is_empty(),
                "{want:?} by {}: {path:?}",
                method.name
            );
        }
    }
}

/// A room to drive her choices in, as a test set it up.
struct Room {
    guest: Guest,
    real: Buffer,
    view: IdleView,
    now: u64,
    /// She went through a door, or off the screen's edge (as driven).
    doored: bool,
    went_out: bool,
}

impl Room {
    fn osaka(&self) -> &Osaka {
        &visit_of(&self.guest).osaka
    }

    fn osaka_mut(&mut self) -> &mut Osaka {
        match &mut self.guest.state {
            State::Visiting(visit) => &mut visit.osaka,
            _ => panic!("visiting"),
        }
    }
}

/// The stage room (100×30, chat text to pull and tear), her clock on a
/// Saturday at 10:00 (her part-time job open), arrived, with `pieces`
/// given.
fn stage_room(seed: u64, pieces: &[Furniture]) -> Room {
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = on_saturday(seed);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    for &item in pieces {
        guest.give(item);
        paint(&mut guest, &real, &view, 0);
        assert!(guest.ledger.home.owns(item), "seed {seed}: {item:?}");
    }
    Room {
        guest,
        real,
        view,
        now: 0,
        doored: false,
        went_out: false,
    }
}

/// [`home_screen`]'s one floor across the screen (out at either edge),
/// on a Saturday at 10:00, with her sofa.
fn edge_room(seed: u64) -> Room {
    let (real, view) = home_screen();
    let mut guest = on_saturday(seed);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    guest.give(Furniture::Sofa);
    paint(&mut guest, &real, &view, 0);
    Room {
        guest,
        real,
        view,
        now: 0,
        doored: false,
        went_out: false,
    }
}

/// Down in [`pit_screen`]'s pit, on a Saturday at 10:00, with her sofa
/// next door: in line art (in ASCII she can climb its wide text), so
/// there's no way off its floor but a door.
fn pit_room(seed: u64) -> Room {
    let (real, view, floor) = pit_screen();
    let mut guest = on_saturday(seed);
    guest.set_picker(kitty());
    visiting_at(&mut guest, &real, &view, (20, i32::from(floor)));
    guest.give(Furniture::Sofa);
    paint(&mut guest, &real, &view, 0);
    Room {
        guest,
        real,
        view,
        now: 0,
        doored: false,
        went_out: false,
    }
}

/// The piece whose real seat is for `what` (none for a parcel, which
/// comes boxed).
fn piece_for(what: Use) -> Option<Furniture> {
    Furniture::ALL
        .into_iter()
        .find(|item| item.spec().uses.contains(&what))
}

/// What a run of one choice came to: the index in her decisions of the
/// choice, and of the next decision she chose among offers after it
/// (where it had run its course).
struct Ran {
    chosen: usize,
    end: usize,
}

/// One step of her visit (at most 100 ms on), noting whether she went
/// through a door or off the screen.
fn step(room: &mut Room) -> Result<(), String> {
    room.now += room
        .guest
        .next_tick(room.now)
        .map_or(100, |d| d.as_millis() as u64)
        .clamp(1, 100);
    room.guest.advance(room.now);
    paint(&mut room.guest, &room.real, &room.view, room.now);
    let State::Visiting(visit) = &room.guest.state else {
        return Err(format!("not visiting at {}", room.now));
    };
    room.doored |= visit.osaka.door(room.now).is_some();
    room.went_out |= !(0..i32::from(room.real.area.width)).contains(&visit.osaka.x);
    Ok(())
}

/// Offered only `want` by `method`, run until she chooses it and then
/// until she next chooses among offers (at most `bound` ms on).
fn drive(room: &mut Room, want: Want, method: &'static str, bound: u64) -> Result<Ran, String> {
    let from = room.osaka().decisions.len();
    room.osaka_mut().offer_only = Some((want, method));
    let until = room.now + bound;
    let mut chosen = None;
    while room.now < until {
        step(room)?;
        let decisions = &room.osaka().decisions;
        if chosen.is_none() {
            chosen = (from..decisions.len())
                .find(|&i| decisions[i].method == method && decisions[i].want == Some(want));
        }
        if let Some(chosen) = chosen
            && let Some(end) =
                (chosen + 1..decisions.len()).find(|&i| decisions[i].bucket == Bucket::Normal)
        {
            return Ok(Ran { chosen, end });
        }
    }
    let methods: Vec<_> = room.osaka().decisions[from..]
        .iter()
        .map(|d| d.method)
        .collect();
    Err(match chosen {
        None => format!("never chosen in {bound} ms: {methods:?}"),
        Some(_) => format!("never ran its course in {bound} ms: {methods:?}"),
    })
}

/// What eased her for the choice `ran` (of `want`): as she set off
/// (`set_off`: the choice's own decision), or after it, up to and with
/// the decision after it where what she finished is credited, and never
/// past her next choice of the same want (whose own credit isn't this
/// one's).
fn eased(room: &Room, ran: &Ran, want: Want, set_off: bool) -> Vec<Served> {
    let decisions = &room.osaka().decisions;
    let again = (ran.chosen + 1..decisions.len())
        .find(|&i| decisions[i].want == Some(want))
        .unwrap_or(usize::MAX);
    let window = if set_off {
        ran.chosen..=ran.chosen
    } else {
        ran.chosen + 1..=ran.end.min(again)
    };
    room.osaka()
        .served
        .iter()
        .filter(|s| window.contains(&s.decision))
        .copied()
        .collect()
}

/// Whether `eased` has `want` eased by a share above nothing, the first
/// time, by `via`.
fn eased_by(eased: &[Served], want: Want, via: Via) -> bool {
    eased
        .iter()
        .find(|s| s.want == want)
        .is_some_and(|s| s.share > 0.0 && s.via == via)
}

/// Chosen by `method` and run to its end, `want` eased her by a share
/// above nothing, the way its credit path says.
fn check(room: &mut Room, want: Want, method: &'static str, bound: u64) -> Result<Ran, String> {
    let path = Osaka::credit_path(want, method).ok_or("no credit path")?;
    let CreditPath::By(via) = path else {
        return Err(format!("{path:?} isn't checked by a run of its own"));
    };
    let ran = drive(room, want, method, bound)?;
    let eased = eased(room, &ran, want, via == Via::SetOff);
    if eased_by(&eased, want, via) {
        Ok(ran)
    } else {
        let methods: Vec<_> = room.osaka().decisions[ran.chosen..=ran.end]
            .iter()
            .map(|d| d.method)
            .collect();
        Err(format!("not eased ({path:?}): {eased:?}, by {methods:?}"))
    }
}

/// Every want that serves, chosen by each of its methods in a room where
/// it binds and run to its end, eases her by a share of it. Each case
/// is named for its want and method; the methods a choice never reaches
/// on its own are driven where they come (arranging's carry and use-it,
/// in its lift's run) or have the path of one that is (a heap or piece
/// she made is used as any: `use/finish-my-heap` and `use/mine` bind
/// only once her leftover reflex has given up on it).
#[test]
fn what_she_chooses_eases_what_it_serves() {
    const MINUTE: u64 = 60_000;
    let mut failed = Vec::new();
    let mut ran = std::collections::BTreeSet::new();
    let mut note = |want: Want, method: &'static str, result: Result<Ran, String>| {
        ran.insert((format!("{want:?}"), method));
        if let Err(e) = result {
            failed.push(format!("{want:?} by {method}: {e}"));
        }
    };
    // On the spot, and walking about.
    for method in ["space-out", "space-out/muse"] {
        let mut room = stage_room(1, &[]);
        note(
            Want::SpaceOut,
            method,
            check(&mut room, Want::SpaceOut, method, 3 * MINUTE),
        );
    }
    // (Those she only settles into are credited as they end: osaka.rs's
    // settling tests.)
    // (The stage room has no desk and no bookshelf: her homework and a
    // book on the floor bind there.)
    for what in Activity::ALL.into_iter().filter(|a| a.chosen()) {
        let want = Want::Idle(what);
        for method in mind::methods(want) {
            let mut room = stage_room(2, &[]);
            note(
                want,
                method.name,
                check(&mut room, want, method.name, 5 * MINUTE),
            );
        }
    }
    let mut room = stage_room(3, &[]);
    note(
        Want::Walk,
        "walk/along",
        check(&mut room, Want::Walk, "walk/along", 3 * MINUTE),
    );
    let mut room = stage_room(3, &[]);
    note(
        Want::Travel,
        "travel/link",
        check(&mut room, Want::Travel, "travel/link", 3 * MINUTE),
    );
    let mut room = pit_room(3);
    note(
        Want::Travel,
        "travel/door",
        check(&mut room, Want::Travel, "travel/door", 3 * MINUTE),
    );
    // Text to tidy and play with.
    for (want, method) in [(Want::Pull, "pull"), (Want::Swap, "swap")] {
        let mut room = stage_room(4, &[]);
        note(want, method, check(&mut room, want, method, 5 * MINUTE));
    }
    // Her part-time job: out at a screen edge and back in, or through her
    // door (each its own way home).
    for (room, how) in [(edge_room(5), "edge"), (pit_room(5), "door")] {
        let mut room = room;
        let result = check(&mut room, Want::Work, "work", 10 * MINUTE);
        let (doored, went_out) = (room.doored, room.went_out);
        let result = result.and_then(|ran| {
            if (doored, went_out) == (how == "door", how == "edge") {
                Ok(ran)
            } else {
                Err(format!("not by the {how}: door {doored}, out {went_out}"))
            }
        });
        note(Want::Work, "work", result);
    }
    // Her things: the real one; one she makes (no real one of its kind,
    // or making one is a whim), then one she has made.
    let mut unmade = Vec::new();
    for what in Use::ALL {
        let want = Want::Use(what);
        if mind::methods(want).is_empty() {
            continue;
        }
        let piece = piece_for(what);
        let mut room = stage_room(6, &Vec::from_iter(piece));
        match what {
            Use::Unpack => room.guest.send_parcel(),
            Use::Pet => room.guest.cat_now = true,
            _ => {}
        }
        paint(&mut room.guest, &room.real, &room.view, 0);
        note(
            want,
            "use/real",
            check(&mut room, want, "use/real", 5 * MINUTE),
        );
        // The real pieces for it but those she could make, if any text
        // here makes one she'd use for it.
        let pieces: Vec<Furniture> = piece.filter(|p| !MAKES.contains(p)).into_iter().collect();
        let mut room = stage_room(7, &pieces);
        let makes = |room: &Room| {
            visit_of(&room.guest)
                .chances
                .builds
                .iter()
                .any(|b| b.then == what)
        };
        if !makes(&room) {
            unmade.push(what);
            continue;
        }
        let made = check(&mut room, want, "use/make", 10 * MINUTE);
        let made_ok = made.is_ok();
        note(want, "use/make", made);
        if made_ok {
            note(
                want,
                "use/made",
                check(&mut room, want, "use/made", 5 * MINUTE),
            );
        } else {
            note(want, "use/made", Err("nothing made to use".into()));
        }
    }
    // Arranging: her sofa turned from her TV, felt as she lounges, and her
    // home on her mind; lifted, carried, set down, and sat back down to.
    {
        use super::super::room::Side;
        use sprite::Facing;
        let (mut guest, real, view) = rule_home(
            &[
                (Furniture::Tv, Side::Left, 0, Facing::Right, true),
                (Furniture::Sofa, Side::Left, 12, Facing::Right, true),
            ],
            false,
            1,
        );
        guest.cue(Scene::Lounge);
        paint(&mut guest, &real, &view, 0);
        let felt_at = run_until(&mut guest, &real, &view, 0, 30_000, |guest, _| {
            !felt(guest).is_empty()
        })
        .expect("never felt it");
        guest.press(stage::Want::Nesting);
        let mut room = Room {
            guest,
            real,
            view,
            now: felt_at,
            doored: false,
            went_out: false,
        };
        let lifted = check(&mut room, Want::Arrange, "arrange/lift", 10 * MINUTE);
        // Carried, and set down: arranging eases her as the frame takes
        // the piece, after she carried it. Sat back down to: eased as the
        // use it is, as she leaves it.
        let decisions = &room.osaka().decisions;
        let steps: Vec<&str> = decisions.iter().map(|d| d.method).collect();
        let at = |step: &str| decisions.iter().position(|d| d.method == step);
        let served = &room.osaka().served;
        for step in ["arrange/carry", "arrange/use-it"] {
            let result = match (&lifted, at(step)) {
                (Err(e), _) => Err(format!("its lift failed: {e}")),
                (Ok(_), None) => Err(format!("never came to it: {steps:?}")),
                (Ok(_), Some(i)) => {
                    let next = (i + 1..decisions.len())
                        .find(|&j| decisions[j].bucket == Bucket::Normal)
                        .unwrap_or(usize::MAX);
                    let after: Vec<Served> = served
                        .iter()
                        .filter(|s| s.decision > i && s.decision <= next)
                        .copied()
                        .collect();
                    let ok = match Osaka::credit_path(Want::Arrange, step) {
                        Some(CreditPath::AsUse) => after.iter().any(|s| {
                            matches!(s.want, Want::Use(_)) && s.share > 0.0 && s.via == Via::Share
                        }),
                        Some(CreditPath::By(via)) => eased_by(&after, Want::Arrange, via),
                        path => panic!("{step}: {path:?}"),
                    };
                    if ok {
                        Ok(Ran {
                            chosen: i,
                            end: next,
                        })
                    } else {
                        Err(format!("not eased: {after:?}"))
                    }
                }
            };
            note(Want::Arrange, step, result);
        }
        note(Want::Arrange, "arrange/lift", lifted);
    }
    // What no makeshift piece offers here: a made sofa offers lounging and
    // napping, and watching only where it would face her TV (none of the
    // stage room's text is on her TV's floor, so that is driven nowhere:
    // it's credited as any use of a made piece); a made bed, sleep; a
    // paper desk, homework.
    assert_eq!(
        unmade,
        [
            Use::Watch,
            Use::Unpack,
            Use::Read,
            Use::Snack,
            Use::Pet,
            Use::LookOut
        ]
    );
    // Every want that serves, by every method, ran (or is one of the two
    // a made piece's use stands for, or one no made piece offers).
    for want in Want::ALL {
        for method in mind::methods(want) {
            let path = Osaka::credit_path(want, method.name);
            let stands_for = matches!(method.name, "use/finish-my-heap" | "use/mine");
            if path == Some(CreditPath::Nothing) || stands_for {
                continue;
            }
            let unmakeable = matches!(want, Want::Use(what) if unmade.contains(&what));
            if unmakeable && matches!(method.name, "use/make" | "use/made") {
                continue;
            }
            assert!(
                ran.contains(&(format!("{want:?}"), method.name)),
                "{want:?} by {} never ran",
                method.name
            );
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// Off to work at the screen's edge (offered nothing else), stepped
/// until she has been out of sight `away_ms`: where she stood as she
/// chose it.
fn out_at_work(room: &mut Room, away_ms: u64) -> (i32, i32) {
    room.osaka_mut().offer_only = Some((Want::Work, "work"));
    let mut stood = None;
    let mut away_since = None;
    while room.now < 10 * 60_000 {
        step(room).expect("visiting");
        let osaka = room.osaka();
        if stood.is_none()
            && osaka
                .decisions
                .last()
                .is_some_and(|d| d.want == Some(Want::Work))
        {
            stood = Some((osaka.x, osaka.y));
        }
        if room.went_out && osaka.hidden(room.now) {
            let since = *away_since.get_or_insert(room.now);
            if room.now >= since + away_ms {
                return stood.expect("chose work");
            }
        }
    }
    panic!("never out at work: {:?}", room.osaka().decisions);
}

/// Her shift cut short by an errand (the accordion nudged while she's
/// out at work: "Work can wait"): she comes for it through a door, and
/// the shift eases her restlessness by the share of it she worked, some
/// but not all of it.
#[test]
fn a_shift_cut_short_eases_her_by_its_share() {
    let mut room = edge_room(5);
    let spot = out_at_work(&mut room, 20_000);
    let before = room.osaka().served.len();
    let now = room.now;
    match &mut room.guest.state {
        State::Visiting(visit) => visit.osaka.errand(spot, &visit.terrain, now),
        _ => panic!("visiting"),
    }
    let eased = &room.osaka().served[before..];
    assert!(
        eased
            .iter()
            .any(|s| s.want == Want::Work && s.share > 0.0 && s.share < 1.0 && s.via == Via::Shift),
        "{eased:?}"
    );
}

/// Her third way home from work: back in sight, walking in, and her
/// walk cut short (a chat line: she stops to look). Her next decision
/// has her home from work, and her shift eases her restlessness as a
/// whole one, as off the edge or through her door.
#[test]
fn a_shift_whose_walk_home_is_cut_short_eases_her() {
    let mut room = edge_room(5);
    out_at_work(&mut room, 0);
    while room.osaka().hidden(room.now) {
        assert!(room.now < 10 * 60_000, "never back");
        step(&mut room).expect("visiting");
    }
    let (from, before) = (room.osaka().decisions.len(), room.osaka().served.len());
    let now = room.now;
    match &mut room.guest.state {
        State::Visiting(visit) => visit.osaka.look(now, 0, false, &visit.terrain),
        _ => panic!("visiting"),
    }
    let bound = room.now + 60_000;
    while !room.osaka().decisions[from..]
        .iter()
        .any(|d| d.method == "work/home")
    {
        assert!(room.now < bound, "{:?}", &room.osaka().decisions[from..]);
        step(&mut room).expect("visiting");
    }
    let eased: Vec<(Want, f64, Via)> = room.osaka().served[before..]
        .iter()
        .map(|s| (s.want, s.share, s.via))
        .collect();
    assert_eq!(eased, [(Want::Work, 1.0, Via::Shift)]);
}
