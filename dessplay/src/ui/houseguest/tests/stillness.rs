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
use crate::ui::houseguest::room::Use;
use crate::ui::houseguest::scenes::Job;
use crate::ui::houseguest::sprite::{Face, Facing, Pose};
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

/// What a client painting at `now` would show of her act: how she looks
/// (pose, face, bubble), where and which way, what her script shows on
/// her furniture, and her lamp dark for the night.
type Shown = (
    (Pose, Face, Option<osaka::Bubble>),
    (i32, i32, Facing),
    Option<crate::ui::houseguest::script::Prop>,
    bool,
);

/// An act sampled: its start, whether it's a watch, whether it's the
/// shopping channel, and its samples (when, what shows, whether
/// Chiyo-chichi's hook plays, the key of her script playing).
type Sampled = (u64, bool, bool, Vec<(u64, Shown, bool, KeyRef)>);

/// Where the key of her act's script playing at a moment is, if any: a
/// bob's flips, and its key ending, are told apart by it.
type KeyRef = Option<script::KeyPlace>;

/// Whether going from `was` to `now`, each with the key playing then, is
/// a bob's own flip (her writing, her breathing asleep, a page turning):
/// in the one key, which bobs, her pose alone changed.
fn bob_flip(was: (&Shown, KeyRef), now: (&Shown, KeyRef)) -> bool {
    let (Some(a), Some(b)) = (was.1, now.1) else {
        return false;
    };
    let ((pose, face, bubble), at, prop, dark) = *was.0;
    let ((pose2, face2, bubble2), at2, prop2, dark2) = *now.0;
    a == b
        && a.bobs
        && pose != pose2
        && (face, bubble, at, prop, dark) == (face2, bubble2, at2, prop2, dark2)
}

/// The key that bobbed, if going from `was` to `now` ends it (another key
/// playing now).
fn bob_ends(was: KeyRef, now: KeyRef) -> KeyRef {
    was.filter(|key| key.bobs && now != was)
}

/// The stillness rule's count over an act's changes after its first
/// 10 s: no change within a frame ([`osaka::USE_FRAME_MS`]) of the last,
/// but one: for now, a bob's key ending may come within a frame of that
/// same bob's last flip (her homework nodding off: its key ends at a
/// share of the use, off the bob's frame grid). That allowance is
/// provisional (phase 5c step 12c's review: whether she should hold the
/// bob's last frame through the part of a period before its key ends,
/// instead, is open); any other change near a bob's flip still counts.
#[derive(Default)]
struct Flips {
    /// The last change's time, and the key it flipped if a bob's flip.
    last: Option<(u64, KeyRef)>,
}

impl Flips {
    /// A change at `t`: `flipped`, the key if it's a bob's own flip;
    /// `ends`, the key that bobbed if it ends that key. The last change's
    /// time if that was within a frame (and not its bob's flip as the bob
    /// ends).
    fn see(&mut self, t: u64, flipped: KeyRef, ends: KeyRef) -> Result<(), u64> {
        if let Some((before, last_flip)) = self.last
            && t - before < osaka::USE_FRAME_MS
            && (ends.is_none() || last_flip != ends)
        {
            return Err(before);
        }
        self.last = Some((t, flipped));
        Ok(())
    }

    /// [`Flips::see`] going from `was` to `now` at `t`.
    fn change(&mut self, t: u64, was: (&Shown, KeyRef), now: (&Shown, KeyRef)) -> Result<(), u64> {
        let flipped = if bob_flip(was, now) { now.1 } else { None };
        self.see(t, flipped, bob_ends(was.1, now.1))
    }
}

/// Whether going from `was` to `now` is only her slow blink (her face
/// alone, to or from a blink), or Chiyo-chichi's bob while his hook
/// plays (`hook`: what's on TV alone, from one of his frames to the
/// other).
fn exempt(was: &Shown, now: &Shown, hook: bool) -> bool {
    use crate::ui::houseguest::art::Channel;
    use crate::ui::houseguest::script::Prop;
    let ((pose, face, bubble), at, prop, dark) = *was;
    let ((pose2, face2, bubble2), at2, prop2, dark2) = *now;
    let blink = face != face2
        && (face == Face::Blink || face2 == Face::Blink)
        && (pose, bubble, at, prop, dark) == (pose2, bubble2, at2, prop2, dark2);
    let bob = hook
        && (pose, face, bubble, at, dark) == (pose2, face2, bubble2, at2, dark2)
        && matches!(
            (prop, prop2),
            (
                Some(Prop::Tv(Channel::Shopping(_))),
                Some(Prop::Tv(Channel::Shopping(_)))
            )
        );
    blink || bob
}

/// No act of hers longer than 30 s flips faster than [`USE_FRAME_MS`]
/// after its first 10 s (phase 5c D7), but those that take her
/// somewhere (a walk; the band counts what moves her; a turn where she
/// stands is checked): on quiet fed afternoons in her rooms with a TV
/// (the furnished home, the home with only a TV, the resident, the home
/// with her window, whose sill session is her longest still act, and the
/// furnished home with the shopping channel on at her first watch), in
/// each mood, in both modes, what a client painting at any moment would
/// show of her act, sampled every 100 ms between the steps the shell
/// takes, never changes twice within a frame once the act's first 10 s
/// are over: her slow blink apart, and Chiyo-chichi's bob through his
/// hook (the first two fifths of the shopping channel's body); and, for
/// now, a bob's key ending within a frame of that bob's last flip (her
/// homework nodding off at its share of the use: provisional, see
/// [`Flips`]). Quiet: a
/// chat line's look up (`!`, then `?`) is the other exemption design.md
/// names, so it isn't tried here. Each room shows her watching for over
/// 30 s at least once in each mode, so the TV is tried, and the shopping
/// room the shopping channel, so the hook's exemption is.
///
/// What it compares is her model's (how she looks, where and which way,
/// what her script shows, the lamp dark), not drawn cells: a proxy, sound
/// while drawing is a pure map from those to images and glyphs
/// (`each_prop_looks_distinct_in_each_mode`,
/// `every_piece_shows_what_her_script_shows_on_it`), so both modes check
/// the same states here. A change that is drawing alone (step 12b's
/// once-a-minute fresh still of the film) has its own variant comparing
/// drawn images (`a_long_watch_takes_a_fresh_still_once_a_minute`).
///
/// [`USE_FRAME_MS`]: osaka::USE_FRAME_MS
#[test]
fn no_long_act_flips_faster_than_a_frame() {
    const MINUTES: u64 = 10;
    const SAMPLE_MS: u64 = 100;
    let tv_only = Room {
        name: "home, TV only",
        owns: &[Furniture::Tv],
        ..furnished_room()
    };
    let windowed = Room {
        name: "home+window",
        owns: &super::census::WINDOWED_HOME,
        ..furnished_room()
    };
    let shopping = Room {
        name: "home, shopping",
        ..furnished_room()
    };
    let rooms = [
        furnished_room(),
        tv_only,
        resident_room(),
        windowed,
        shopping,
    ]
    .map(at_afternoon)
    .map(|room| super::census::with_chat(&room, true));
    let moods = [
        (0, Mood::Lazy),
        (1, Mood::Ordinary),
        (2, Mood::Dreamy),
        (3, Mood::Industrious),
    ];
    let runs: Vec<(String, u32, u32)> = std::thread::scope(|scope| {
        let mut runs = Vec::new();
        for room in &rooms {
            for graphics in [false, true] {
                runs.push(scope.spawn(move || {
                    let (mut watched, mut shopped) = (0, 0);
                    for (seed, mood) in moods {
                        let at = format!("{} {mood:?} seed {seed} graphics={graphics}", room.name);
                        let mut guest = fed_afternoon(room, seed, graphics, mood);
                        if room.name == "home, shopping" {
                            guest.shop();
                        }
                        let mut act: Option<Sampled> = None;
                        // Whether a checked act was a watch, and the
                        // shopping channel.
                        let check = |act: &Sampled| -> (bool, bool) {
                            let (since, watch, shopping, samples) = act;
                            let Some(&(last, ..)) = samples.last() else {
                                return (false, false);
                            };
                            // Over 30 s, where she is: an act that takes
                            // her somewhere (a walk the screen's width is
                            // 31 s) moves her as the band counts it. A
                            // turn where she stands is checked.
                            let (_, (_, first_at, ..), ..) = samples[0];
                            let stays = samples.iter().all(|(_, (_, at, ..), ..)| {
                                (at.0, at.1) == (first_at.0, first_at.1)
                            });
                            if last - since <= 30_000 || !stays {
                                return (false, false);
                            }
                            let mut flips = Flips::default();
                            for pair in samples.windows(2) {
                                let ((_, was, _, was_key), (t, now, hook, key)) =
                                    (&pair[0], &pair[1]);
                                if was == now || *t < since + 10_000 || exempt(was, now, *hook) {
                                    continue;
                                }
                                if let Err(before) = flips.change(*t, (was, *was_key), (now, *key))
                                {
                                    panic!(
                                        "{at}: an act from {since} flipped at {before} and {t}: \
                                         {was:?} {was_key:?} to {now:?} {key:?}"
                                    );
                                }
                            }
                            (*watch, *shopping)
                        };
                        let mut tally = |(watch, shopping): (bool, bool)| {
                            watched += u32::from(watch);
                            shopped += u32::from(shopping);
                        };
                        let mut now = 0;
                        while now < MINUTES * 60_000 {
                            let step = guest
                                .next_tick(now)
                                .map_or(1000, |d| d.as_millis() as u64)
                                .clamp(1, 1000);
                            if let State::Visiting(visit) = &guest.state {
                                let osaka = &visit.osaka;
                                let since = osaka.act_started();
                                if act.as_ref().is_some_and(|a| a.0 != since)
                                    && let Some(done) = act.take()
                                {
                                    tally(check(&done));
                                }
                                let span = osaka.use_span();
                                let watch = span.is_some_and(|(seat, ..)| seat.what == Use::Watch);
                                let shopping = osaka
                                    .plays()
                                    .is_some_and(|play| play.own == script::ScriptId::Shopping);
                                let samples =
                                    &mut act.get_or_insert((since, watch, shopping, Vec::new())).3;
                                let hook_end = match (span, osaka.plays()) {
                                    (Some((_, since, until)), Some(play))
                                        if play.own == script::ScriptId::Shopping =>
                                    {
                                        let start = play.body_start(since);
                                        start + (play.body_end(since, until) - start) * 2 / 5
                                    }
                                    _ => 0,
                                };
                                let mut t = now.next_multiple_of(SAMPLE_MS);
                                while t < now + step {
                                    let shown = (
                                        osaka.appearance(t),
                                        (osaka.x, osaka.y, osaka.facing),
                                        osaka.prop(t),
                                        osaka.dark(t),
                                    );
                                    samples.push((t, shown, t <= hook_end, osaka.key_at(t)));
                                    t += SAMPLE_MS;
                                }
                            }
                            now += step;
                            if guest.advance(now) {
                                paint(&mut guest, &room.real, &room.view, now);
                            }
                        }
                        if let Some(done) = act.take() {
                            tally(check(&done));
                        }
                    }
                    (
                        format!("{} graphics={graphics}", room.name),
                        watched,
                        shopped,
                    )
                }));
            }
        }
        runs.into_iter()
            .map(|run| run.join().expect("a run"))
            .collect()
    });
    for (at, watched, shopped) in &runs {
        println!("{at}: {watched} watches over 30 s, {shopped} on the shopping channel");
    }
    for (at, watched, shopped) in runs {
        assert!(watched > 0, "{at}: never watched for over 30 s");
        if at.starts_with("home, shopping ") {
            assert!(
                shopped > 0,
                "{at}: never on the shopping channel for over 30 s"
            );
        }
    }
}
