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
use crate::ui::houseguest::graphics::Look;
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
/// alone, to or from a blink).
fn blink(was: &Shown, now: &Shown) -> bool {
    let ((pose, face, bubble), at, prop, dark) = *was;
    let ((pose2, face2, bubble2), at2, prop2, dark2) = *now;
    face != face2
        && (face == Face::Blink || face2 == Face::Blink)
        && (pose, bubble, at, prop, dark) == (pose2, bubble2, at2, prop2, dark2)
}

/// Whether going from `was` to `now` is only Chiyo-chichi's bob while
/// his hook plays (`hook`: what's on TV alone, from one of his frames to
/// the other).
fn hook_bob(was: &Shown, now: &Shown, hook: bool) -> bool {
    use crate::ui::houseguest::art::Channel;
    use crate::ui::houseguest::script::Prop;
    let ((pose, face, bubble), at, prop, dark) = *was;
    let ((pose2, face2, bubble2), at2, prop2, dark2) = *now;
    hook && (pose, face, bubble, at, dark) == (pose2, face2, bubble2, at2, dark2)
        && matches!(
            (prop, prop2),
            (
                Some(Prop::Tv(Channel::Shopping(_))),
                Some(Prop::Tv(Channel::Shopping(_)))
            )
        )
}

/// Whether going from `was` to `now` is only her slow blink, or
/// Chiyo-chichi's bob while his hook plays.
fn exempt(was: &Shown, now: &Shown, hook: bool) -> bool {
    blink(was, now) || hook_bob(was, now, hook)
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
/// what her script shows, the lamp dark), not drawn cells: a proxy for
/// what's drawn of her and what her script shows, which map from those
/// to images and glyphs (`each_prop_looks_distinct_in_each_mode`,
/// `every_piece_shows_what_her_script_shows_on_it`), so both modes check
/// the same states here. What is drawing alone is not in her model: the
/// film's fresh still once a minute (step 12b), her wall clock's dial
/// (each game quarter-hour) and her window's sky (dawn, day, dusk,
/// evening, night). The drawn variant
/// ([`no_long_act_flips_drawn_cells_faster_than_a_frame`]) holds the film
/// to the rule, and exempts the dial and the sky, each only its own
/// change ([`world_ticked`]).
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

/// One paint of her act, as drawn: when; what her model showed (as
/// [`Shown`]) and the key her script played; every cell of hers that
/// differs from the real frame, by where (a kitty image's cell as
/// "image", its id being random) and, in line art, each image's look
/// (what it shows: her pose, a piece, the TV's channel or the film's
/// still), sorted by how it prints; her box, her TV's footprint and every
/// shown piece's; and why a change there may be exempt: Chiyo-chichi's
/// hook playing, her looking up at the chat (or stirring at it), a still
/// of the film delivered at this paint, and what her wall clock's dial
/// and her window's sky read by her clock (`None` unfed: a plain face
/// and a day sky). Whether it was painted in line art.
#[derive(Clone)]
struct Drawn {
    t: u64,
    graphics: bool,
    shown: Shown,
    key: KeyRef,
    cells: std::collections::BTreeMap<(u16, u16), String>,
    looks: Vec<(String, Look)>,
    her: Option<Rect>,
    tv: Option<Rect>,
    pieces: Vec<Rect>,
    clocks: Vec<Rect>,
    windows: Vec<Rect>,
    hook: bool,
    chat: bool,
    delivered: bool,
    world: Option<(art::Dial, art::Sky)>,
}

impl Drawn {
    /// Its looks but those `skip` names, as they print.
    fn looks_but(&self, skip: impl Fn(&Look) -> bool) -> Vec<&str> {
        self.looks
            .iter()
            .filter(|(_, look)| !skip(look))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// What's on its TV: the looks of the TV switched on or holding the
    /// film's still.
    fn on_tv(&self) -> Vec<Look> {
        self.looks
            .iter()
            .map(|&(_, look)| look)
            .filter(on_tv)
            .collect()
    }
}

/// A look of her own (her pose, her wave).
fn hers(look: &Look) -> bool {
    matches!(look, Look::Pose(..) | Look::Wave(..))
}

/// A look of what's on her TV.
fn on_tv(look: &Look) -> bool {
    matches!(look, Look::Tv(_) | Look::Film(..))
}

/// The cells that differ between two paints.
fn changed(was: &Drawn, now: &Drawn) -> Vec<(u16, u16)> {
    let mut at: Vec<(u16, u16)> = was
        .cells
        .keys()
        .chain(now.cells.keys())
        .copied()
        .filter(|at| was.cells.get(at) != now.cells.get(at))
        .collect();
    at.sort_unstable();
    at.dedup();
    at
}

/// Whether `at` is in `rect`.
fn inside(rect: Option<Rect>, (x, y): (u16, u16)) -> bool {
    rect.is_some_and(|r| (r.x..r.right()).contains(&x) && (r.y..r.bottom()).contains(&y))
}

/// A look of the world's clock: her wall clock's dial, her window's sky.
fn world_clock(look: &Look) -> bool {
    use crate::ui::houseguest::art::PieceState;
    matches!(
        look,
        Look::Piece(Furniture::Clock, PieceState::Dial(_))
            | Look::Piece(Furniture::Window, PieceState::Sky(_))
    )
}

/// Whether going from `was` to `now`, as drawn, is the world's clock
/// stepping, and only that. Its trigger: a reading of her clock changed
/// between them (the dial's quarter-hour, or the sky: dawn, day, dusk,
/// evening, night), and in line art a look of a piece whose reading
/// changed went with it. Its scope: her model, her script's key and
/// every look but the dial's and the sky's the same, a dial or sky look
/// changing only if its reading did, and every changed cell in the
/// footprint of a wall clock or window whose reading changed, none in
/// her box. Not her doing, and slow and steady (a dial step each game
/// quarter-hour, the sky five times a game day): the user exempts it
/// (Round 8's open point; the periodic-motion principle).
fn world_ticked(was: &Drawn, now: &Drawn) -> bool {
    let (Some((dial, sky)), Some((dial2, sky2))) = (was.world, now.world) else {
        return false;
    };
    let items = [
        (Furniture::Clock, dial != dial2),
        (Furniture::Window, sky != sky2),
    ];
    if !items.iter().any(|&(_, ticked)| ticked) {
        return false;
    }
    let rects = |d: &Drawn, item: Furniture| match item {
        Furniture::Clock => d.clocks.clone(),
        _ => d.windows.clone(),
    };
    let in_ticked = |at: &(u16, u16)| {
        items.iter().any(|&(item, ticked)| {
            ticked
                && rects(was, item)
                    .into_iter()
                    .chain(rects(now, item))
                    .any(|r| inside(Some(r), *at))
        })
    };
    let in_her_box = |at: &(u16, u16)| inside(was.her, *at) || inside(now.her, *at);
    let looks_of = |d: &Drawn, item: Furniture| -> Vec<Look> {
        d.looks
            .iter()
            .map(|&(_, look)| look)
            .filter(|look| world_clock(look) && matches!(look, Look::Piece(it, _) if *it == item))
            .collect()
    };
    let redrawn = |item: Furniture| looks_of(was, item) != looks_of(now, item);
    was.shown == now.shown
        && was.key == now.key
        && was.looks_but(world_clock) == now.looks_but(world_clock)
        && items.iter().all(|&(item, ticked)| ticked || !redrawn(item))
        && (!now.graphics || items.iter().any(|&(item, ticked)| ticked && redrawn(item)))
        && changed(was, now)
            .iter()
            .all(|at| in_ticked(at) && !in_her_box(at))
}

/// Whether going from `was` to `now`, as drawn, is one of the stillness
/// rule's exemptions, and only that, nothing else drawn changing with it:
/// - her slow blink: her face alone in her model, the cells that change
///   in her box, every look not hers the same;
/// - Chiyo-chichi's bob through his hook: what's on TV alone in her
///   model, the cells that change in the TV's footprint, every look not
///   on the TV the same;
/// - a look up at the chat (or, dozing, a stir at it), on either side:
///   her pose, face, bubble or facing alone in her model (not where she
///   is, what her script shows or the lamp), every look not hers the
///   same, and no cell of a piece changing but where her box covers it;
/// - the film's fresh still, at a paint a still was delivered at: her
///   model the same, the TV's look going from the film's still (or the
///   programme it stands in for) to another still standing in for the
///   same programme, every other look the same, the cells that change in
///   the TV's footprint. That it comes once a minute is
///   `a_long_watch_takes_a_fresh_still_once_a_minute`'s to hold;
/// - the world's clock, her wall clock's dial or her window's sky
///   stepping ([`world_ticked`]): at a paint where her clock's reading
///   changed, her model, her key and every other look the same, the
///   cells that change in the footprint of the piece that stepped and
///   none in her box.
fn exempt_drawn(was: &Drawn, now: &Drawn) -> bool {
    use crate::ui::houseguest::art::Channel;
    let cells = changed(was, now);
    let in_her_box = |at: &(u16, u16)| inside(was.her, *at) || inside(now.her, *at);
    let in_tv = |at: &(u16, u16)| inside(was.tv, *at) || inside(now.tv, *at);
    let blinked = blink(&was.shown, &now.shown)
        && was.looks_but(hers) == now.looks_but(hers)
        && cells.iter().all(in_her_box);
    let bobbed = hook_bob(&was.shown, &now.shown, now.hook)
        && was.looks_but(on_tv) == now.looks_but(on_tv)
        && cells.iter().all(in_tv);
    let ((_, at, prop, dark), (_, at2, prop2, dark2)) = (was.shown, now.shown);
    let looked = (was.chat || now.chat)
        && was.shown != now.shown
        && ((at.0, at.1), prop, dark) == ((at2.0, at2.1), prop2, dark2)
        && was.looks_but(hers) == now.looks_but(hers)
        && cells
            .iter()
            .all(|at| in_her_box(at) || !now.pieces.iter().any(|&r| inside(Some(r), *at)));
    let swapped = now.delivered
        && was.shown == now.shown
        && was.looks_but(on_tv) == now.looks_but(on_tv)
        && cells.iter().all(in_tv)
        && match (&was.on_tv()[..], &now.on_tv()[..]) {
            ([Look::Film(a, card)], [Look::Film(b, card2)]) => a != b && card == card2,
            ([Look::Tv(Channel::Programme(card))], [Look::Film(_, card2)]) => card == card2,
            _ => false,
        };
    blinked || bobbed || looked || swapped || world_ticked(was, now)
}

/// The drawn half of the stillness rule (phase 5c D7, as the user
/// amended it for the film, Q2): no act of hers longer than 30 s flips
/// what's drawn of her faster than [`USE_FRAME_MS`] after its first
/// 10 s, but for exactly the five exemptions design.md names, each only
/// its own change with nothing else drawn alongside ([`exempt_drawn`]):
/// her slow blink, Chiyo-chichi's bob through his hook, a look up at the
/// chat (or, dozing, a stir at it: it comes when the line does, off her
/// breathing's frames, but it holds unchanged for a frame from its start,
/// checked at each), the film's fresh still once
/// a minute, and the world's clock (her wall clock's dial, her window's
/// sky); and, for now, a bob's key ending within a frame of its
/// last flip ([`Flips`]). On fed afternoons, with chat, in the home with
/// only a TV (she watches it most), the furnished home with the shopping
/// channel on at her first watch, the home with her window and the
/// resident, in each mood, in both modes, painted as a client painting
/// every 100 ms would (advancing her first, as the shell does before
/// each draw, and at each chat line), with the film's stills fed as the
/// shell feeds them ([`TvFeed`]: asked as she heads to watch, then once
/// a minute, each answered good 150 ms on). An act that takes her
/// somewhere is left out, as in [`no_long_act_flips_faster_than_a_frame`],
/// which checks her model's state, every 100 ms, quiet; this checks the
/// cells and images painted, so a change that is drawing alone (the
/// film's still) is held to the rule too. The home with her wall clock
/// runs apart
/// ([`no_long_act_flips_drawn_cells_faster_than_a_frame_by_her_clock`]).
/// Each room shows her watching for over 30 s with a fresh still
/// mid-watch in line art at least once, the shopping room the shopping
/// channel, and some act a look up at the chat, so each exemption is
/// tried.
///
/// [`USE_FRAME_MS`]: osaka::USE_FRAME_MS
/// [`TvFeed`]: crate::ui::tv_feed::TvFeed
#[test]
fn no_long_act_flips_drawn_cells_faster_than_a_frame() {
    let tv_only = Room {
        name: "home, TV only",
        owns: &[Furniture::Tv],
        ..furnished_room()
    };
    let shopping = Room {
        name: "home, shopping",
        ..furnished_room()
    };
    let windowed = Room {
        name: "home+window",
        owns: &super::census::WINDOWED_HOME,
        ..furnished_room()
    };
    drawn_stillness(&[tv_only, shopping, windowed, resident_room()].map(at_afternoon));
}

/// [`no_long_act_flips_drawn_cells_faster_than_a_frame`] in the home with
/// her wall clock as well as her window (the day census's home), where
/// her clock's dial steps each game quarter-hour (about 150 s at 6×) and
/// her window's sky at each change of the sky (dawn, day, dusk, evening,
/// night), whatever she's doing: of an afternoon (13:00, four dial
/// steps), and from 16:30, so the sky goes from day to dusk at 17:00.
/// The world's clock is exempt (the user, Round 8's open point: not her
/// doing, slow and steady, the periodic motion principle), only at a
/// paint where her clock's reading changed and scoped to its own change
/// ([`world_ticked`]): only the stepped piece's cells and look change.
/// At each paint it exempts, the same paint with one thing more changed
/// must not be exempt ([`world_ticked_guards`]). Before the exemption the
/// dial stepped 656 ms after her breathing's flip asleep ("home, clock
/// and window Lazy seed 0, an act from 151044 flipped at 299444 and
/// 300100", both modes). Each run sees a dial step in a long act, and
/// the run at dusk a sky step, so the exemption is tried.
#[test]
fn no_long_act_flips_drawn_cells_faster_than_a_frame_by_her_clock() {
    let clocked = Room {
        name: "home, clock and window",
        owns: &super::census::DAY_HOME,
        ..furnished_room()
    };
    let dusk = Room {
        name: "home, clock and window, to dusk",
        owns: &super::census::DAY_HOME,
        start: Some(routine::GameTime {
            day: 1,
            h: 16,
            m: 30,
        }),
        ..furnished_room()
    };
    let runs = drawn_stillness(&[at_afternoon(clocked), dusk]);
    for (at, [.., dial, sky, _]) in runs {
        assert!(dial > 0, "{at}: no dial step in a long act");
        if at.contains("to dusk") {
            assert!(sky > 0, "{at}: no sky step in a long act");
        }
    }
}

/// The drawn stillness check over `rooms`, each in both modes and every
/// mood (see [`no_long_act_flips_drawn_cells_faster_than_a_frame`]).
fn drawn_stillness(rooms: &[Room]) -> Vec<(String, [u32; 7])> {
    use crate::ui::houseguest::film::test_picture;
    use crate::ui::tv_feed::{Sent, TvAnswer, TvAsk, TvFeed};
    const MINUTES: u64 = 10;
    const PAINT_MS: u64 = 100;
    const FILM: dessplay_core::types::Ed2kHash = dessplay_core::types::Ed2kHash([0xC3; 16]);
    let moods = [
        (0, Mood::Lazy),
        (1, Mood::Ordinary),
        (2, Mood::Dreamy),
        (3, Mood::Industrious),
    ];
    // Per room and mode: watches over 30 s, those with a fresh still
    // after their first 10 s, shopping acts over 30 s, acts with a look
    // up at the chat after their first 10 s, and the world's clock
    // stepping in a checked act: her dial, and her window's sky; and her
    // stirs at the chat, dozing.
    let runs: Vec<(String, [u32; 7])> = std::thread::scope(|scope| {
        let mut runs = Vec::new();
        for room in rooms {
            for graphics in [false, true] {
                runs.push(scope.spawn(move || {
                    let mut seen = [0u32; 7];
                    let mut stirs = 0u32;
                    for (seed, mood) in moods {
                        let at = format!("{} {mood:?} seed {seed} graphics={graphics}", room.name);
                        let mut guest = fed_afternoon(room, seed, graphics, mood);
                        if room.name == "home, shopping" {
                            guest.shop();
                        }
                        if let Some(graphics) = guest.graphics.as_mut() {
                            graphics.take_looks();
                        }
                        let mut view = room.view.clone();
                        let mut feed = TvFeed::default();
                        let mut pending: Option<(u64, TvAsk)> = None;
                        let mut made = 0u32;
                        // The act painted so far: its start, whether a
                        // watch, whether the shopping channel, its paints.
                        let mut act: Option<(u64, bool, bool, Vec<Drawn>)> = None;
                        let mut check =
                            |(since, watch, shopping, paints): (u64, bool, bool, Vec<Drawn>)| {
                                let Some(last) = paints.last() else {
                                    return;
                                };
                                let (_, first_at, ..) = paints[0].shown;
                                let stays = paints.iter().all(|p| {
                                    (p.shown.1.0, p.shown.1.1) == (first_at.0, first_at.1)
                                });
                                if last.t - since <= 30_000 || !stays {
                                    return;
                                }
                                let mut flips = Flips::default();
                                let (mut swapped, mut looked) = (false, false);
                                for pair in paints.windows(2) {
                                    let (was, now) = (&pair[0], &pair[1]);
                                    let same = was.shown == now.shown
                                        && was.cells == now.cells
                                        && was.looks == now.looks;
                                    if same || now.t < since + 10_000 {
                                        continue;
                                    }
                                    // The world's clock first, so a step of it
                                    // credits no other exemption.
                                    if world_ticked(was, now) {
                                        world_ticked_guards(&at, was, now);
                                        seen[4] += u32::from(stepped(was, now, Furniture::Clock));
                                        seen[5] += u32::from(stepped(was, now, Furniture::Window));
                                        continue;
                                    }
                                    if exempt_drawn(was, now) {
                                        swapped |= now.delivered;
                                        looked |= was.chat || now.chat;
                                        continue;
                                    }
                                    if let Err(before) = flips.change(
                                        now.t,
                                        (&was.shown, was.key),
                                        (&now.shown, now.key),
                                    ) {
                                        panic!(
                                            "{at}: an act from {since} flipped at {before} and {}: \
                                             {:?} {:?} {:?} to {:?} {:?} {:?}; cells {:?}",
                                            now.t,
                                            was.shown,
                                            was.key,
                                            was.looks_but(|_| false),
                                            now.shown,
                                            now.key,
                                            now.looks_but(|_| false),
                                            changed(was, now)
                                                .iter()
                                                .map(|at| (
                                                    at,
                                                    was.cells.get(at),
                                                    now.cells.get(at)
                                                ))
                                                .collect::<Vec<_>>()
                                        );
                                    }
                                }
                                seen[0] += u32::from(watch);
                                seen[1] += u32::from(watch && swapped);
                                seen[2] += u32::from(shopping);
                                seen[3] += u32::from(looked);
                            };
                        // The stir showing: when it started, and how
                        // she looked then.
                        let mut stir: Option<(u64, script::Look)> = None;
                        let mut now = 0;
                        while now < MINUTES * 60_000 {
                            let tick = guest
                                .next_tick(now)
                                .map_or(1000, |d| d.as_millis() as u64)
                                .clamp(1, 1000);
                            let (step, line) = room.step_from(now, tick);
                            // A paint every 100 ms, and the answer at its time.
                            let grid = PAINT_MS - now % PAINT_MS;
                            let answer = pending.map_or(u64::MAX, |(t, _)| t - now);
                            let step = step.min(grid).min(answer);
                            let line = line && step == room.step_from(now, tick).0;
                            now += step;
                            if line {
                                view.chat_mark.synced += 1;
                                view.chat_mark.synced_asks =
                                    view.chat_mark.synced.is_multiple_of(2);
                            }
                            let mut delivered = false;
                            if let Some((t, ask)) = pending
                                && t == now
                            {
                                pending = None;
                                made += 1;
                                let still = test_picture(ask.file, made);
                                delivered =
                                    feed.deliver(&mut guest, ask, TvAnswer::Still(still), now);
                            }
                            let closed = feed.turn(&mut guest, Some(FILM), now, |ask| {
                                pending = Some((now + 150, ask));
                                Sent::Gone
                            });
                            assert!(!closed);
                            guest.advance(now);
                            let frame = paint(&mut guest, &room.real, &view, now);
                            let looks: Vec<(String, Look)> = match guest.graphics.as_mut() {
                                Some(graphics) => {
                                    let mut looks: Vec<(String, Look)> = graphics
                                        .take_looks()
                                        .into_iter()
                                        .map(|look| (format!("{look:?}"), look))
                                        .collect();
                                    looks.sort_by(|a, b| a.0.cmp(&b.0));
                                    looks
                                }
                                None => Vec::new(),
                            };
                            let State::Visiting(visit) = &guest.state else {
                                if let Some(done) = act.take() {
                                    check(done);
                                }
                                continue;
                            };
                            let osaka = &visit.osaka;
                            // A stir at the chat, dozing, holds for a
                            // frame from its start: it's exempt as a look
                            // up at the chat is (its start comes when the
                            // line does, off her breathing's frames), but it
                            // never comes and goes inside a frame. It starts
                            // as a line comes, lasts a frame, and at every
                            // paint of that frame shows as it began: her
                            // blink, its murmur, and her turn (what she's
                            // turned over from may change under it, as her
                            // key does: not the stir's).
                            let stirring = osaka.stirring_at_chat(now);
                            match stir {
                                None if stirring => {
                                    stirs += 1;
                                    assert!(line, "{at}: a stir at {now} comes with a line");
                                    assert!(
                                        osaka.stirring_at_chat(now + osaka::USE_FRAME_MS - 1),
                                        "{at}: a stir at {now} lasts a frame"
                                    );
                                    stir = Some((now, osaka.appearance(now)));
                                }
                                Some((from, (pose, face, bubble)))
                                    if now < from + osaka::USE_FRAME_MS =>
                                {
                                    let (pose2, face2, bubble2) = osaka.appearance(now);
                                    assert!(stirring, "{at}: a stir at {from} over by {now}");
                                    assert_eq!(
                                        (face2, bubble2),
                                        (face, bubble),
                                        "{at}: a stir at {from}, at {now}"
                                    );
                                    if osaka::turned_stirring(pose) {
                                        assert_eq!(pose2, pose, "{at}: a stir at {from}, at {now}");
                                    }
                                }
                                Some(_) if !stirring => stir = None,
                                _ => {}
                            }
                            let since = osaka.act_started();
                            if act.as_ref().is_some_and(|a| a.0 != since)
                                && let Some(done) = act.take()
                            {
                                check(done);
                            }
                            let span = osaka.use_span();
                            let watch = span.is_some_and(|(seat, ..)| seat.what == Use::Watch);
                            let shopping = osaka
                                .plays()
                                .is_some_and(|play| play.own == script::ScriptId::Shopping);
                            let hook_end = match (span, osaka.plays()) {
                                (Some((_, since, until)), Some(play))
                                    if play.own == script::ScriptId::Shopping =>
                                {
                                    let start = play.body_start(since);
                                    start + (play.body_end(since, until) - start) * 2 / 5
                                }
                                _ => 0,
                            };
                            let mut cells = std::collections::BTreeMap::new();
                            let width = usize::from(room.real.area.width);
                            for (i, (got, want)) in
                                frame.content.iter().zip(&room.real.content).enumerate()
                            {
                                if got != want {
                                    let cell = if got.symbol().contains('\x1b') {
                                        "image".to_owned()
                                    } else {
                                        format!("{got:?}")
                                    };
                                    cells.insert(((i % width) as u16, (i / width) as u16), cell);
                                }
                            }
                            let drawn = Drawn {
                                t: now,
                                graphics,
                                shown: (
                                    osaka.appearance(now),
                                    (osaka.x, osaka.y, osaka.facing),
                                    osaka.prop(now),
                                    osaka.dark(now),
                                ),
                                key: osaka.key_at(now),
                                cells,
                                looks,
                                her: crate::ui::houseguest::room::her_box(osaka.x, osaka.y),
                                tv: visit
                                    .shown
                                    .iter()
                                    .find(|piece| piece.item == Furniture::Tv)
                                    .map(|piece| piece.rect()),
                                pieces: visit.shown.iter().map(|piece| piece.rect()).collect(),
                                clocks: shown_rects(visit, Furniture::Clock),
                                windows: shown_rects(visit, Furniture::Window),
                                hook: now <= hook_end,
                                chat: osaka.looking_up_at_chat() || osaka.stirring_at_chat(now),
                                delivered,
                                world: guest
                                    .time_of_day(now)
                                    .map(|minute| (art::Dial::at(minute), art::Sky::at(minute))),
                            };
                            act.get_or_insert((since, watch, shopping, Vec::new()))
                                .3
                                .push(drawn);
                        }
                        if let Some(done) = act.take() {
                            check(done);
                        }
                    }
                    seen[6] = stirs;
                    (format!("{} graphics={graphics}", room.name), seen)
                }));
            }
        }
        runs.into_iter()
            .map(|run| run.join().expect("a run"))
            .collect()
    });
    for (at, [watched, swapped, shopped, looked, dial, sky, stirs]) in &runs {
        println!(
            "{at}: {watched} watches over 30 s ({swapped} with a fresh still), \
             {shopped} on the shopping channel, {looked} with a look up at the chat, \
             {dial} dial and {sky} sky steps in a long act, {stirs} stirs"
        );
    }
    assert!(
        runs.iter().any(|(_, seen)| seen[6] > 0),
        "no stir at the chat, dozing, so its frame isn't tried"
    );
    for (at, [watched, swapped, shopped, looked, ..]) in &runs {
        assert!(*watched > 0, "{at}: never watched for over 30 s");
        assert!(
            *looked > 0,
            "{at}: never looked up at the chat in a long act"
        );
        if at.ends_with("graphics=true") {
            assert!(*swapped > 0, "{at}: no fresh still mid-watch");
        }
        if at.starts_with("home, shopping ") {
            assert!(
                *shopped > 0,
                "{at}: never on the shopping channel for over 30 s"
            );
        }
    }
    runs
}

/// Whether `item` (her wall clock, her window) stepped going from `was`
/// to `now`: its reading by her clock changed (the dial's quarter-hour,
/// the sky), and it changed as drawn, a cell in its footprint (ASCII) or
/// its look (line art).
fn stepped(was: &Drawn, now: &Drawn, item: Furniture) -> bool {
    let (Some((dial, sky)), Some((dial2, sky2))) = (was.world, now.world) else {
        return false;
    };
    let read = match item {
        Furniture::Clock => dial != dial2,
        _ => sky != sky2,
    };
    let rects = |d: &Drawn| match item {
        Furniture::Clock => d.clocks.clone(),
        _ => d.windows.clone(),
    };
    let cells = changed(was, now)
        .iter()
        .any(|&at| rects(now).iter().any(|&r| inside(Some(r), at)));
    let of = |d: &Drawn| -> Vec<Look> {
        d.looks
            .iter()
            .map(|&(_, look)| look)
            .filter(|look| matches!(look, Look::Piece(it, _) if *it == item))
            .collect()
    };
    read && (cells || of(was) != of(now))
}

/// [`world_ticked`]'s guards, tried at every paint it holds at (`was` to
/// `now`): the same paint, but with one more thing changed, is never
/// exempt. No reading of her clock changed (a flicker in the clock's or
/// the window's footprint, off the quarter); a cell of her box changed;
/// a cell outside every footprint changed; her model changed (her lamp's
/// dark); another look changed (line art).
fn world_ticked_guards(at: &str, was: &Drawn, now: &Drawn) {
    let mark = "mutant".to_owned();
    let mut mutants: Vec<(&str, Drawn)> = Vec::new();
    let mut idle = now.clone();
    idle.world = was.world;
    mutants.push(("no reading of her clock changed", idle));
    if let Some(her) = now.her {
        let mut hers = now.clone();
        hers.cells.insert((her.x, her.y), mark.clone());
        mutants.push(("a cell of her box changed", hers));
    }
    let footprints: Vec<Rect> = [was, now]
        .into_iter()
        .flat_map(|d| {
            d.her
                .into_iter()
                .chain(d.tv)
                .chain(d.pieces.iter().copied())
        })
        .collect();
    if let Some(cell) = (0..100u16)
        .flat_map(|y| (0..300u16).map(move |x| (x, y)))
        .find(|&cell| !footprints.iter().any(|&r| inside(Some(r), cell)))
    {
        let mut elsewhere = now.clone();
        elsewhere.cells.insert(cell, mark.clone());
        mutants.push(("a cell outside every footprint changed", elsewhere));
    }
    let mut model = now.clone();
    model.shown.3 = !model.shown.3;
    mutants.push(("her model changed", model));
    if let Some(i) = now.looks.iter().position(|(_, look)| !world_clock(look)) {
        let mut look = now.clone();
        look.looks[i].0 = mark.clone();
        mutants.push(("another look changed", look));
    }
    for (what, mutant) in &mutants {
        assert!(
            !exempt_drawn(was, mutant),
            "{at}: at {} the world's clock exempts a paint where {what} as well",
            now.t
        );
    }
}

/// The footprints of the pieces `item` shown in `visit`.
fn shown_rects(visit: &Visit, item: Furniture) -> Vec<Rect> {
    visit
        .shown
        .iter()
        .filter(|piece| piece.item == item)
        .map(|piece| piece.rect())
        .collect()
}
