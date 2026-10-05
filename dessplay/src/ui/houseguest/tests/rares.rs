//! Rarity and pity (phase 5b D6; step 7): her rare things play only on
//! a day they're open, each cued from the stage in the part of her day
//! it belongs to; seen means first shown (the script's first key playing,
//! not its planning), recorded in her ledger with her pity starting
//! again; and a game day is drawn once, carried over its visits, so it
//! has at most one new rare thing (unfed, each visit draws its own). Each
//! in both drawing modes where drawing matters.

use super::*;
use crate::ui::houseguest::osaka::Bubble;
use crate::ui::houseguest::rarity::{self, Rares};
use crate::ui::houseguest::routine::GameTime;
use crate::ui::houseguest::script::{self, ScriptId};

/// Her home on [`home_screen`]: a sofa and a fridge, and her bed.
const HOME: [(Furniture, Nook, u16); 3] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Fridge, Nook::Users, 800),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// The rare script playing (as her act's own script, or spliced round
/// it) in `guest`'s visit at `now`, if any.
fn rare_playing(guest: &Guest, now: u64) -> Option<ScriptId> {
    visit_of_opt(guest)?.osaka.rare_showing(now)
}

/// `guest` on [`home_screen`] at `at`, idle long enough for some pity
/// (500 idle minutes, the last new rare thing at 20).
fn idle_home_at(seed: u64, at: GameTime, graphics: bool) -> Guest {
    let mut guest = home_at(seed, at, &HOME, graphics);
    guest.ledger.idle_min = 500;
    guest.ledger.rare_at = 20;
    guest
}

/// Each rare thing, cued from the stage in the part of her day it
/// belongs to (the Dream at 23:00 on a school night, the melon bread in
/// the afternoon, the escalator in the evening, the scary story at 22:05
/// doing homework), plays, whatever's open today, in both drawing modes;
/// and as it first shows it's seen: in her ledger by its stable id, her
/// pity for Rare starting again from her idle minutes then. The Dream,
/// cued at night, begins on its first line, not under "Night-night..."
/// (the lamp key it starts on is the Dream's: she's long since said it).
/// That each is new only in its slots is the draw's (rarity.rs) and the
/// splice rows' (script.rs) to show: a cue plays whatever's open.
#[test]
fn each_rare_plays_in_its_slot_when_cued() {
    let (real, view) = home_screen();
    let cases = [
        (Scene::Dream, ScriptId::Dream, mon(23, 0)),
        (Scene::NoMelon, ScriptId::NoMelon, mon(16, 0)),
        (Scene::Escalator, ScriptId::Escalator, mon(19, 0)),
        (Scene::Scary, ScriptId::Scary, mon(22, 5)),
    ];
    for graphics in [false, true] {
        for (scene, id, at) in cases {
            let tag = format!("{id:?} graphics={graphics}");
            let mut guest = idle_home_at(3, at, graphics);
            guest.cue(scene);
            let mut now = 0;
            let mut played = None;
            while now < 90_000 {
                shell_step(&mut guest, &real, &view, &mut now, true);
                if rare_playing(&guest, now) == Some(id) {
                    played = Some(now);
                    break;
                }
            }
            let played = played.unwrap_or_else(|| panic!("{tag}: never played"));
            if id == ScriptId::Dream {
                let osaka = &visit_of(&guest).osaka;
                for t in [played, played + 1_000, played + 2_500] {
                    assert_eq!(
                        osaka.appearance(t).2,
                        Some(Bubble::Say(script::EVERYNYAN)),
                        "{tag}: at {t}, from {played}"
                    );
                }
            }
            // Recorded as her next tick takes it (the stage begins a
            // musing as it paints).
            shell_step(&mut guest, &real, &view, &mut now, true);
            let row = rarity::RARES.iter().find(|r| r.id == id).expect("a row");
            assert_eq!(guest.ledger.seen, [row.key], "{tag}");
            assert_eq!(guest.ledger.rare_at, guest.ledger.idle_min, "{tag}");
            assert!(guest.ledger.idle_min >= 500, "{tag}");
        }
    }
}

/// Seen means first shown: the melon bread planned after her snack (its
/// coda chosen as the snack began) isn't seen while she eats; it's seen
/// as its first key plays. A chat line that cuts the snack short before
/// it leaves it unseen, in her ledger and in her own count. What's new
/// today isn't seen for being drawn either. Both drawing modes.
#[test]
fn seen_only_as_its_first_key_plays() {
    let (real, view) = home_screen();
    let chatty = IdleView {
        chat_mark: ChatMark {
            synced: 1,
            newest: Some(42),
            synced_asks: false,
            irc: 0,
            irc_asks: false,
        },
        ..view.clone()
    };
    for graphics in [false, true] {
        for cut in [false, true] {
            let tag = format!("cut={cut} graphics={graphics}");
            let mut guest = idle_home_at(4, mon(16, 0), graphics);
            // Her pity certain: something new today, not seen for it.
            guest.ledger.idle_min = 2000;
            guest.cue(Scene::NoMelon);
            let mut now = 0;
            let mut planned = false;
            while now < 60_000 && !planned {
                shell_step(&mut guest, &real, &view, &mut now, true);
                planned = visit_of_opt(&guest)
                    .and_then(|v| v.osaka.plays())
                    .is_some_and(|p| {
                        p.after
                            .is_some_and(|s| s.splice.script() == ScriptId::NoMelon)
                    });
            }
            assert!(planned, "{tag}: never planned");
            let new = visit_of(&guest).osaka.rares().new_one();
            assert!(new.is_some(), "{tag}: her pity's certain");
            assert!(guest.ledger.seen.is_empty(), "{tag}: planned, not seen");
            assert_ne!(rare_playing(&guest, now), Some(ScriptId::NoMelon), "{tag}");
            let world = if cut { &chatty } else { &view };
            let until = now + 30_000;
            let mut shown = false;
            while now < until {
                shell_step(&mut guest, &real, world, &mut now, true);
                if rare_playing(&guest, now) == Some(ScriptId::NoMelon) {
                    shown = true;
                    assert_eq!(guest.ledger.seen, ["no-melon"], "{tag}: as it shows");
                }
            }
            assert_eq!(shown, !cut, "{tag}");
            if cut {
                assert!(guest.ledger.seen.is_empty(), "{tag}: cut short, unseen");
                assert!(visit_of(&guest).osaka.seen().is_empty(), "{tag}");
            }
            // Whatever was new, only what showed is seen.
            assert!(
                guest.ledger.seen.iter().all(|k| k == "no-melon"),
                "{tag}: {:?}",
                guest.ledger.seen
            );
        }
    }
}

/// Her visit, if she's visiting.
fn visit_of_opt(guest: &Guest) -> Option<&Visit> {
    match &guest.state {
        State::Visiting(visit) => Some(visit),
        _ => None,
    }
}

/// A game day is drawn once, her routine fed: a second visit the same
/// game day has the first's draw, whatever she has seen and however her
/// pity has grown since, so the day has at most one new rare thing; the
/// next game day draws its own. Unfed, each visit draws its own (from the
/// visit's seed), never anything of the evening or the night.
#[test]
fn a_day_is_drawn_once_over_its_visits() {
    let (real, view) = home_screen();
    let mut changed_days = 0;
    let mut unfed_differ = 0;
    for seed in 0..12 {
        let mut guest = idle_home_at(seed, mon(16, 0), false);
        guest.ledger.idle_min = 300;
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let first = visit_of(&guest).osaka.rares().clone();
        assert_eq!(
            guest.rares_today.as_ref().map(|(d, r)| (*d, r.clone())),
            Some((0, first.clone())),
            "seed {seed}"
        );
        // Seen since, pity grown: the same day's second visit, as drawn.
        guest.ledger.seen = vec!["escalator".into(), "no-melon".into()];
        guest.ledger.idle_min = 5000;
        guest.activity(now);
        now = until_visiting(&mut guest, &real, &view, now);
        assert_eq!(guest.day(now).map(|d| d.day), Some(0));
        assert_eq!(visit_of(&guest).osaka.rares(), &first, "seed {seed}");
        // The next game day: its own draw (pity certain, something new
        // if anything unseen fits; Tuesday 16:00).
        guest.activity(now);
        guest.ledger.clock = tue(16, 0).minutes();
        now = until_visiting(&mut guest, &real, &view, now);
        assert_eq!(guest.day(now).map(|d| d.day), Some(1));
        let next = visit_of(&guest).osaka.rares().clone();
        assert_eq!(
            next,
            Rares::draw(
                brain::day_seed(guest.ledger.master_seed, 1),
                &guest.ledger.seen_scripts(),
                rarity::Pity::of(
                    guest.ledger.idle_min,
                    guest.ledger.rare_at,
                    guest.ledger.legend_at
                ),
                rarity::visit_window(routine::Slot::Afternoon, routine::Slot::Evening),
            ),
            "seed {seed}"
        );
        assert_eq!(
            next.new_one(),
            Some(ScriptId::Scary),
            "seed {seed}: certain"
        );
        changed_days += usize::from(next != first);
        // Unfed: each visit its own draw, of what her afternoon can hold.
        let mut guest = idle_home_at(seed, mon(16, 0), false).unfed();
        guest.ledger.idle_min = 5000;
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let one = visit_of(&guest).osaka.rares().clone();
        assert!(guest.rares_today.is_none());
        guest.activity(now);
        now = until_visiting(&mut guest, &real, &view, now);
        let two = visit_of(&guest).osaka.rares().clone();
        let _ = now;
        for rares in [&one, &two] {
            assert!(
                matches!(
                    rares.new_one(),
                    Some(ScriptId::NoMelon | ScriptId::Escalator)
                ),
                "seed {seed}: {rares:?}"
            );
        }
        unfed_differ += usize::from(one != two);
    }
    assert!(changed_days > 0);
    assert!(unfed_differ > 0, "unfed visits draw their own");
}

/// A record whose pity counters are ahead of her idle minutes (garbled,
/// or written by a clock since set back) is no pity yet: her visit
/// begins, its draw made, without a panic, and nothing new without the
/// base roll.
#[test]
fn pity_ahead_of_her_idle_minutes_is_none_yet() {
    let (real, view) = home_screen();
    for seed in 0..8 {
        let mut guest = idle_home_at(seed, mon(16, 0), false);
        guest.ledger.rare_at = 10_000;
        guest.ledger.legend_at = u64::MAX;
        let now = until_visiting(&mut guest, &real, &view, 0);
        let rares = visit_of(&guest).osaka.rares().clone();
        let day = guest.day(now).expect("fed");
        assert_eq!(
            rares,
            Rares::draw(
                brain::day_seed(guest.ledger.master_seed, day.day),
                &[],
                rarity::Pity::default(),
                rarity::visit_window(routine::Slot::Afternoon, routine::Slot::Evening),
            ),
            "seed {seed}"
        );
    }
}

/// Moving out forgets what she has seen, and her pity, with the rest of
/// her record; and the day's draw goes with it.
#[test]
fn moving_out_forgets_what_shes_seen() {
    let (real, view) = home_screen();
    let mut guest = idle_home_at(2, mon(16, 0), false);
    guest.ledger.seen = vec!["dream".into()];
    let now = until_visiting(&mut guest, &real, &view, 0);
    assert!(guest.rares_today.is_some());
    guest.move_out(9);
    assert!(guest.ledger.seen.is_empty());
    assert_eq!(
        (
            guest.ledger.idle_min,
            guest.ledger.rare_at,
            guest.ledger.legend_at
        ),
        (0, 0, 0)
    );
    assert!(guest.rares_today.is_none());
    let _ = now;
}

/// The script her act plays as its own in `guest`'s visit, if any.
fn own_script(guest: &Guest) -> Option<ScriptId> {
    visit_of_opt(guest)?.osaka.plays().map(|p| p.own)
}

/// Once a night is the night's, not the visit's: the Dream comes 30
/// game minutes after she first slept (Monday 22:50, tucked in), a
/// visitor's key ends the visit soon after it, and she's tucked in
/// again that same night; it doesn't come again (the probe had a second
/// one 30 game minutes into her second visit). Both drawing modes.
#[test]
fn the_dream_comes_once_a_night_across_visits() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let tag = format!("graphics={graphics}");
        let mut guest = idle_home_at(5, mon(22, 50), graphics);
        // Her pity certain, and the Dream the one thing new tonight.
        guest.ledger.idle_min = 2000;
        guest.ledger.seen = vec!["no-melon".into(), "escalator".into()];
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        assert!(asleep(&guest), "{tag}: tucked in");
        assert_eq!(
            visit_of(&guest).osaka.rares().new_one(),
            Some(ScriptId::Dream),
            "{tag}"
        );
        let mut dreams = Vec::new();
        let mut was = own_script(&guest);
        let mut back = None;
        let until = now + 20 * 60_000;
        while now < until {
            shell_step(&mut guest, &real, &view, &mut now, true);
            let own = own_script(&guest);
            if own == Some(ScriptId::Dream) && was != own {
                dreams.push(now);
            }
            was = own;
            // Ten seconds after it, a visitor's key; she's back that
            // same night, tucked in again.
            if back.is_none() && dreams.first().is_some_and(|&d| now >= d + 10_000) {
                guest.activity(now);
                now = until_visiting(&mut guest, &real, &view, now);
                assert!(asleep(&guest), "{tag}: tucked in again");
                assert_eq!(guest.day(now).map(|d| d.day), Some(0), "{tag}");
                back = Some(now);
                was = own_script(&guest);
            }
        }
        assert!(back.is_some(), "{tag}: never dreamt, so never left");
        assert_eq!(dreams.len(), 1, "{tag}: {dreams:?}, back at {back:?}");
        assert_eq!(guest.ledger.seen, ["no-melon", "escalator", "dream"]);
    }
}

/// A day drawn as she wakes into it is the day's: tucked in on Monday
/// (its draw for the night and the morning), asleep through midnight,
/// she wakes on Tuesday and its draw is made then, for the whole day
/// (pity certain: the scary story new, the one unseen thing). The guest
/// keeps it, and her next visit that morning has it, not a draw of its
/// own for the morning (which would hold nothing new). The probe: the
/// guest dropping what she drew at her wake left the day's draw to the
/// next visit.
#[test]
fn a_day_drawn_as_she_wakes_carries_to_her_next_visit() {
    let (real, view) = home_screen();
    let mut guest = idle_home_at(6, mon(23, 50), false);
    guest.ledger.idle_min = 5000;
    guest.ledger.seen = vec!["dream".into(), "escalator".into(), "no-melon".into()];
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    assert!(asleep(&guest), "tucked in");
    assert_eq!(guest.rares_today.as_ref().map(|(d, _)| *d), Some(0));
    while asleep(&guest) {
        assert!(now < 100 * 60_000, "never woke");
        shell_step(&mut guest, &real, &view, &mut now, true);
    }
    assert_eq!(guest.day(now).map(|d| d.day), Some(1), "woke on Tuesday");
    let woke = visit_of(&guest).osaka.rares().clone();
    assert_eq!(woke.new_one(), Some(ScriptId::Scary), "{woke:?}");
    assert_eq!(guest.rares_today, Some((1, woke.clone())));
    guest.activity(now);
    now = until_visiting(&mut guest, &real, &view, now);
    assert_eq!(guest.day(now).map(|d| d.day), Some(1));
    assert_eq!(visit_of(&guest).osaka.rares(), &woke);
}
