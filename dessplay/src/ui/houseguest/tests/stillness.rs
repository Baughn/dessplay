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

/// Where the key of her act's script playing at a moment is, if any
/// (for telling a change of key apart in a failure, and the world's
/// clock's scope: [`world_ticked`]).
type KeyRef = Option<script::KeyPlace>;

/// The stillness rule's count over an act's changes after its first
/// 10 s: no change within a frame ([`osaka::USE_FRAME_MS`]) of the last.
/// No allowance: a bob's key ending, or starting, keeps a frame clear of
/// its flips (Round 8, the user; phase 5c's tail, T2), as every other
/// change does. An exempt change is never a flip itself, but one the
/// rule counts on from ([`Exempt::counts`]) is the last change the next
/// one waits a frame from (phase 5c's tail, T4).
#[derive(Default)]
struct Flips {
    /// The last change's time.
    last: Option<u64>,
}

impl Flips {
    /// A change at `t`: the last change's time if that was within a
    /// frame.
    fn see(&mut self, t: u64) -> Result<(), u64> {
        if let Some(before) = self.last
            && t - before < osaka::USE_FRAME_MS
        {
            return Err(before);
        }
        self.last = Some(t);
        Ok(())
    }

    /// An exempt change at `t` the rule counts on from: no flip, but the
    /// next change waits a frame from it.
    fn counts_from(&mut self, t: u64) {
        self.last = Some(t);
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

/// Which of the exemptions going from `was` to `now` is: only her slow
/// blink, or Chiyo-chichi's bob while his hook plays (`Some(false)`), or
/// both at once (`Some(true)`: the exemptions combine, her face and
/// what's on TV each exempt).
fn exempt(was: &Shown, now: &Shown, hook: bool) -> Option<bool> {
    let ((pose, _, bubble), at, prop, dark) = *was;
    let blinked = ((pose, now.0.1, bubble), at, prop, dark);
    if blink(was, now) || hook_bob(was, now, hook) {
        Some(false)
    } else {
        (blink(was, &blinked) && hook_bob(&blinked, now, hook)).then_some(true)
    }
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
/// hook (the first two fifths of the shopping channel's body). A bob's
/// key ending (her homework nodding off at its share of the use) or
/// starting keeps a frame clear of its flips as any change does (no
/// allowance since phase 5c's tail, T2). Quiet: a
/// chat line's look up (`!`, then `?`) is the other exemption design.md
/// names, so it isn't tried here. Each room shows her watching for over
/// 30 s at least once in each mode, so the TV is tried, and the shopping
/// room the shopping channel, so the hook's exemption is.
///
/// Its exemptions are steady periodic motion, so neither counts on
/// ([`Exempt::counts`]): her next change is judged from the last that
/// wasn't exempt. They combine: her blink and the hook's bob at one
/// sample are both exempt (the shopping room also runs lazy seed 2 and
/// ordinary seed 5, where T2's probe found her blink beginning on the
/// sample the hook's bob flipped, and a flip counted).
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
    // And an industrious afternoon at her desk in the furnished homes
    // (seed 0): her homework's writing ends at its share of the use,
    // off its bob's frames, where the rule once had an allowance.
    let moods = [
        (0, Mood::Lazy),
        (1, Mood::Ordinary),
        (2, Mood::Dreamy),
        (3, Mood::Industrious),
        (0, Mood::Industrious),
    ];
    let runs: Vec<(String, u32, u32, u32)> = std::thread::scope(|scope| {
        let mut runs = Vec::new();
        for room in &rooms {
            for graphics in [false, true] {
                runs.push(scope.spawn(move || {
                    let (mut watched, mut shopped, mut combined) = (0, 0, 0);
                    // And where her blink once began on the sample the
                    // hook's bob flipped (T2's probe), the two exempt at
                    // once.
                    let blink_on_hook: &[(u64, Mood)] = if room.name == "home, shopping" {
                        &[(2, Mood::Lazy), (5, Mood::Ordinary)]
                    } else {
                        &[]
                    };
                    for (seed, mood) in moods.into_iter().chain(blink_on_hook.iter().copied()) {
                        let at = format!("{} {mood:?} seed {seed} graphics={graphics}", room.name);
                        let mut guest = fed_afternoon(room, seed, graphics, mood);
                        if room.name == "home, shopping" {
                            guest.shop();
                        }
                        let mut act: Option<Sampled> = None;
                        // Whether a checked act was a watch, and the
                        // shopping channel.
                        let mut check = |act: &Sampled| -> (bool, bool) {
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
                                if was == now || *t < since + 10_000 {
                                    continue;
                                }
                                if let Some(both) = exempt(was, now, *hook) {
                                    combined += u32::from(both);
                                    continue;
                                }
                                if let Err(before) = flips.see(*t) {
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
                        combined,
                    )
                }));
            }
        }
        runs.into_iter()
            .map(|run| run.join().expect("a run"))
            .collect()
    });
    for (at, watched, shopped, combined) in &runs {
        println!(
            "{at}: {watched} watches over 30 s, {shopped} on the shopping channel, \
             {combined} samples of her blink with the hook's bob"
        );
    }
    for (at, watched, shopped, combined) in runs {
        assert!(watched > 0, "{at}: never watched for over 30 s");
        if at.starts_with("home, shopping ") {
            assert!(
                shopped > 0,
                "{at}: never on the shopping channel for over 30 s"
            );
            // The combined clause, and only it, exempted a sample, so it
            // is tried (a behaviour change moving the coincidence away
            // would leave it untried unseen).
            assert!(
                combined > 0,
                "{at}: her blink never came with the hook's bob"
            );
        }
    }
}

/// One paint of her act, as drawn: when; what her model showed (as
/// [`Shown`]) and the key her script played; every cell of hers that
/// differs from the real frame, by where (a kitty image's cell as
/// "image", its id being random) and, in line art, each image's look
/// (what it shows: her pose, a piece, the TV's channel or the film's
/// still), sorted by how it prints; her box, her glyphs' cells (ASCII:
/// the cells her sprite paints; none in line art, where she's an image),
/// her TV's footprint and every shown piece's; and why a change there may
/// be exempt: Chiyo-chichi's
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
    glyphs: Vec<(u16, u16)>,
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

/// Which of the stillness rule's exemptions (design.md's five) a change
/// is, as [`exempt_drawn`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exempt {
    /// Her slow blink.
    Blink,
    /// Chiyo-chichi's bob through his hook.
    Hook,
    /// A look up at the chat, or (dozing) a stir at it.
    Look,
    /// The film's fresh still.
    Swap,
    /// The world's clock alone: her wall clock's dial, her window's sky.
    World,
}

impl Exempt {
    /// Whether the rule counts on from it ([`Flips::counts_from`]): an
    /// exempt change doesn't hide a flip of hers right after it. A look
    /// up at the chat, or a stir at it, is her reaction and a change the
    /// eye catches, so what she does next waits a frame from its last
    /// change, as from any change of hers. Her slow blink and the hook's
    /// bob are steady periodic motion, the background the eye filters out
    /// (the user, Round 8), and the film's still and the world's clock
    /// aren't hers and come on their own time whatever she does (the
    /// world's clock is exempt for just that, T1): none of them sets the
    /// count, or her breathing would have to wait on her blinks. That is
    /// design.md's rule as written ("her blink, the hook's bob, the
    /// film's still and the world's clock count nothing"), not a
    /// measured need for each: counting the world's clock fails (her
    /// breathing 744 ms after a dial step), counting blinks fails (they
    /// come off her bob's grid by design), and counting the film's still
    /// fails no run today, but nothing keeps her bob off the still's
    /// moment (it comes as the player answers), so it would fail by
    /// chance, on a principle that isn't hers to keep.
    fn counts(self) -> bool {
        self == Self::Look
    }
}

/// Whether two paints show the same: her model, the cells and the looks
/// (as [`judge`] skips a paint where nothing changed: a key boundary that
/// shows nothing new is no change, at a step of the world's clock too).
fn unchanged(was: &Drawn, now: &Drawn) -> bool {
    was.shown == now.shown && was.cells == now.cells && was.looks == now.looks
}

/// `now` with the world's clock's own step taken back, so what else
/// changed with it is judged by the other exemptions (phase 5c's tail,
/// T4: the exemptions combine). At a paint where a reading of her clock
/// changed (the dial's quarter-hour, or the sky: dawn, day, dusk,
/// evening, night), the changed cells in the footprint of each wall
/// clock or window whose reading changed, and its dial or sky looks,
/// are put back as `was` had them, and the reading too; `None` where no
/// reading changed. A cell in her box is the piece's only in ASCII,
/// where her glyphs are known, at a paint where her model and key didn't
/// change, and off her glyphs (the sky round her as she leans at the
/// sill); in line art she's one image over it, and with her model or key
/// changed her box is hers. A dial or sky look of a piece whose reading
/// didn't change stays, as does every other cell and look.
fn world_stripped(was: &Drawn, now: &Drawn) -> Option<Drawn> {
    let (Some((dial, sky)), Some((dial2, sky2))) = (was.world, now.world) else {
        return None;
    };
    let items = [
        (Furniture::Clock, dial != dial2),
        (Furniture::Window, sky != sky2),
    ];
    if !items.iter().any(|&(_, ticked)| ticked) {
        return None;
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
    let still = !now.graphics && was.shown == now.shown && was.key == now.key;
    let hers = |at: &(u16, u16)| was.glyphs.contains(at) || now.glyphs.contains(at);
    let theirs = |at: &(u16, u16)| in_ticked(at) && (!in_her_box(at) || (still && !hers(at)));
    let mut rest = now.clone();
    rest.world = was.world;
    for at in changed(was, now) {
        if theirs(&at) {
            match was.cells.get(&at) {
                Some(cell) => rest.cells.insert(at, cell.clone()),
                None => rest.cells.remove(&at),
            };
        }
    }
    let of = |look: &Look, item: Furniture| {
        world_clock(look) && matches!(look, Look::Piece(it, _) if *it == item)
    };
    for (item, _) in items.into_iter().filter(|&(_, ticked)| ticked) {
        rest.looks.retain(|(_, look)| !of(look, item));
        rest.looks
            .extend(was.looks.iter().filter(|(_, look)| of(look, item)).cloned());
    }
    rest.looks.sort_by(|a, b| a.0.cmp(&b.0));
    Some(rest)
}

/// Whether going from `was` to `now`, as drawn, is the world's clock
/// stepping, and only that ([`world_stripped`] takes all of it back):
/// not her doing, and slow and steady (a dial step each game
/// quarter-hour, the sky five times a game day), the user exempts it
/// (Round 8's open point; the periodic-motion principle).
fn world_ticked(was: &Drawn, now: &Drawn) -> bool {
    exempt_drawn(was, now) == Some(Exempt::World)
}

/// Which of the stillness rule's exemptions going from `was` to `now`,
/// as drawn, is, each only its own change, nothing else drawn changing
/// with it (`None`: no exemption's):
/// - the world's clock, her wall clock's dial or her window's sky
///   stepping, first: its own cells and looks are taken back
///   ([`world_stripped`]), and whatever else changed with it is judged
///   by the others (it is the world's alone if nothing else did);
/// - her slow blink: her face alone in her model, the cells that change
///   in her box, every look not hers the same;
/// - Chiyo-chichi's bob through his hook: what's on TV alone in her
///   model, the cells that change in the TV's footprint, every look not
///   on the TV the same;
/// - a look up at the chat (or, dozing, a stir at it), on either side:
///   her face, bubble or facing alone in her model, her pose only into
///   or out of a stir's turn with its murmur ([`looks_own_pose`]; not
///   her bob's flip or her act's key under the look, nor where she is,
///   what her script shows or the lamp), every look not hers the same,
///   and no cell of a piece changing but where her box covers it;
/// - the film's fresh still, at a paint a still was delivered at: her
///   model the same, the TV's look going from the film's still (or the
///   programme it stands in for) to another still standing in for the
///   same programme, every other look the same, the cells that change in
///   the TV's footprint. That it comes once a minute is
///   `a_long_watch_takes_a_fresh_still_once_a_minute`'s to hold.
fn exempt_drawn(was: &Drawn, now: &Drawn) -> Option<Exempt> {
    match world_stripped(was, now) {
        Some(rest) if unchanged(was, &rest) => Some(Exempt::World),
        Some(rest) => exempt_hers(was, &rest),
        None => exempt_hers(was, now),
    }
}

/// [`exempt_drawn`] but for the world's clock: her slow blink, the hook's
/// bob, a look up at the chat (or a stir) and the film's fresh still;
/// and her blink with the hook's bob at once, each its own change (the
/// exemptions combine: her face, cells and looks going first, then the
/// TV's).
///
/// Its limit: those are the only pairs that combine here (the world's
/// clock with any one of these, above it). A look up's change or the
/// film's still on the same paint as her blink or the hook's bob, or a
/// stir's murmur beginning on the paint her bob flips, is judged as a
/// change of hers ([`Flips::see`]): it fails only if something counted
/// came within the frame before it, which no run does today; the paint
/// grid (100 ms) makes such a coincidence rare, and a future seed that
/// finds one should widen the pairs here, not loosen the rule.
fn exempt_hers(was: &Drawn, now: &Drawn) -> Option<Exempt> {
    exempt_one(was, now).or_else(|| {
        let mut blinked = was.clone();
        blinked.shown.0.1 = now.shown.0.1;
        let in_her_box = |at: &(u16, u16)| inside(was.her, *at) || inside(now.her, *at);
        for at in changed(was, now).into_iter().filter(in_her_box) {
            match now.cells.get(&at) {
                Some(cell) => blinked.cells.insert(at, cell.clone()),
                None => blinked.cells.remove(&at),
            };
        }
        blinked.looks.retain(|(_, look)| !hers(look));
        blinked
            .looks
            .extend(now.looks.iter().filter(|(_, look)| hers(look)).cloned());
        blinked.looks.sort_by(|a, b| a.0.cmp(&b.0));
        (exempt_one(was, &blinked) == Some(Exempt::Blink)
            && exempt_one(&blinked, now) == Some(Exempt::Hook))
        .then_some(Exempt::Blink)
    })
}

/// One exemption alone: her slow blink, the hook's bob, a look up at the
/// chat (or a stir), the film's fresh still.
fn exempt_one(was: &Drawn, now: &Drawn) -> Option<Exempt> {
    use crate::ui::houseguest::art::Channel;
    let cells = changed(was, now);
    let in_her_box = |at: &(u16, u16)| inside(was.her, *at) || inside(now.her, *at);
    let in_tv = |at: &(u16, u16)| inside(was.tv, *at) || inside(now.tv, *at);
    if blink(&was.shown, &now.shown)
        && was.looks_but(hers) == now.looks_but(hers)
        && cells.iter().all(in_her_box)
    {
        return Some(Exempt::Blink);
    }
    if hook_bob(&was.shown, &now.shown, now.hook)
        && was.looks_but(on_tv) == now.looks_but(on_tv)
        && cells.iter().all(in_tv)
    {
        return Some(Exempt::Hook);
    }
    let (((pose, _, bubble), at, prop, dark), ((pose2, _, bubble2), at2, prop2, dark2)) =
        (was.shown, now.shown);
    if (was.chat || now.chat)
        && was.shown != now.shown
        && ((at.0, at.1), prop, dark) == ((at2.0, at2.1), prop2, dark2)
        && looks_own_pose(pose, bubble, pose2, bubble2)
        && was.looks_but(hers) == now.looks_but(hers)
        && cells
            .iter()
            .all(|at| in_her_box(at) || !now.pieces.iter().any(|&r| inside(Some(r), *at)))
    {
        return Some(Exempt::Look);
    }
    let swapped = now.delivered
        && was.shown == now.shown
        && was.looks_but(on_tv) == now.looks_but(on_tv)
        && cells.iter().all(in_tv)
        && match (&was.on_tv()[..], &now.on_tv()[..]) {
            ([Look::Film(a, card)], [Look::Film(b, card2)]) => a != b && card == card2,
            ([Look::Tv(Channel::Programme(card))], [Look::Film(_, card2)]) => card == card2,
            _ => false,
        };
    swapped.then_some(Exempt::Swap)
}

/// Whether her pose going from `pose` to `pose2` (her bubble from
/// `bubble` to `bubble2`) is a look up's or a stir's own: a look up keeps
/// her act's pose (her face, bubble and facing are its change), and a
/// stir turns her over as its murmur starts and back as it ends, so her
/// pose changes only into or out of the stir's turn, with her bubble.
/// Anything else of her pose under a look or a stir (her bob's flip, her
/// act's key) is her act's change, judged as any ([`Flips::see`]).
fn looks_own_pose(
    pose: Pose,
    bubble: Option<osaka::Bubble>,
    pose2: Pose,
    bubble2: Option<osaka::Bubble>,
) -> bool {
    pose == pose2
        || bubble != bubble2 && (osaka::turned_stirring(pose) || osaka::turned_stirring(pose2))
}

/// What a checked act showed of the exemptions: a fresh still mid-watch,
/// a look up at the chat, and the world's clock stepping (her dial, her
/// window's sky).
#[derive(Default)]
struct Judged {
    swapped: bool,
    looked: bool,
    dial: u32,
    sky: u32,
}

/// The drawn stillness rule over an act's paints from `since`, after its
/// first 10 s: every change exempt ([`exempt_drawn`]) or a frame from the
/// last change the rule counts ([`Flips`]: every change that isn't
/// exempt, and the exempt changes [`Exempt::counts`] names). `Err`: the
/// first two changes within a frame, and what changed at the second.
fn judge(paints: &[Drawn], since: u64) -> Result<Judged, (u64, u64, String)> {
    let mut flips = Flips::default();
    let mut judged = Judged::default();
    for pair in paints.windows(2) {
        let (was, now) = (&pair[0], &pair[1]);
        let same = was.shown == now.shown && was.cells == now.cells && was.looks == now.looks;
        if same || now.t < since + 10_000 {
            continue;
        }
        if world_stripped(was, now).is_some() {
            judged.dial += u32::from(stepped(was, now, Furniture::Clock));
            judged.sky += u32::from(stepped(was, now, Furniture::Window));
        }
        match exempt_drawn(was, now) {
            Some(kind) => {
                judged.swapped |= kind == Exempt::Swap;
                judged.looked |= kind == Exempt::Look;
                if kind.counts() {
                    flips.counts_from(now.t);
                }
            }
            None => {
                if let Err(before) = flips.see(now.t) {
                    let what = format!(
                        "{:?} {:?} {:?} to {:?} {:?} {:?}; cells {:?}",
                        was.shown,
                        was.key,
                        was.looks_but(|_| false),
                        now.shown,
                        now.key,
                        now.looks_but(|_| false),
                        changed(was, now)
                            .iter()
                            .map(|at| (at, was.cells.get(at), now.cells.get(at)))
                            .collect::<Vec<_>>()
                    );
                    return Err((before, now.t, what));
                }
            }
        }
    }
    Ok(judged)
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
/// sky); a bob's key ending or starting gets no allowance ([`Flips`]).
///
/// Precisely (phase 5c's tail, T4), over each act's paints in turn
/// ([`judge`]): the world's clock's own step is taken back first and
/// what else changed with it is judged by the others, so exemptions
/// combine (her blink as the dial steps is exempt; [`world_stripped`],
/// whose cells in her box are the piece's in ASCII off her glyphs where
/// her model and key didn't change); a change that is no exemption's
/// fails if it comes within a frame of the last change the rule counts;
/// that is every change that isn't exempt and every change of a look up
/// at the chat or a stir at it, which, though exempt, are hers and catch
/// the eye, so her next change waits a frame from them; her blink and the
/// hook's bob (steady periodic motion) and the film's still and the
/// world's clock (not hers, on their own time) count nothing
/// ([`Exempt::counts`]). Each act's real paints carry the rule's own
/// mutants ([`mutants`]): a change of hers a paint after a counted
/// change fails, at that change. Before her bob held a frame from a look
/// up's or a stir's changes, the count failed on a stir by day ("an act
/// from 151044 flipped at 181400 and 181844": the stir's end and her
/// breathing's next flip, 444 ms apart).
/// On fed afternoons, with chat, in the home with
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

/// A parcel due while she watches TV keeps her still (door batch T12,
/// Open choice 3): it waits until she's walking or an act of hers has just
/// begun (its slide through her door's flap, five changes in 650 ms,
/// would break the drawn stillness rule), so her watch, held over 40 s
/// with the parcel due 15 s into it, passes the drawn rule ([`judge`]),
/// and the parcel comes after it (not vacuous). Both modes. Red before
/// step 9: it came mid-watch.
#[test]
fn a_parcel_keeps_her_still() {
    let room = at_afternoon(Room {
        name: "home, TV only",
        owns: &[Furniture::Tv],
        ..furnished_room()
    });
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = fed_afternoon(&room, 0, graphics, Mood::Lazy);
        guest.cue(Scene::Watch);
        let mut now = 0;
        let (mut watch, mut ended, mut came, mut sent) = (None, None, None, false);
        let mut paints = Vec::new();
        while came.is_none() || ended.is_none() {
            assert!(
                now < 600_000,
                "{at}: the parcel never came, or her watch never ended"
            );
            now += 100;
            guest.advance(now);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &room.real, &room.view, now);
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
                panic!("{at}: visiting throughout");
            };
            let osaka = &visit.osaka;
            came = came.or(visit.flap.map(|(_, since)| since));
            let watching = osaka
                .use_span()
                .is_some_and(|(seat, ..)| seat.what == Use::Watch);
            let since = osaka.act_started();
            match watch {
                None if watching => watch = Some(since),
                Some(from) if ended.is_none() && since != from => ended = Some(now),
                _ => {}
            }
            let Some(from) = watch.filter(|_| ended.is_none()) else {
                continue;
            };
            paints.push(drawn_of(
                &guest,
                &room.real,
                &frame,
                looks,
                (now, graphics),
                false,
                false,
            ));
            if !sent && now >= from + 15_000 {
                sent = true;
                guest.send_parcel();
            }
        }
        let from = watch.expect("she watched");
        let end = ended.expect("her watch ended");
        assert!(
            end > from + 40_000,
            "{at}: the precondition: a watch over 40 s"
        );
        assert!(sent, "{at}: sent mid-watch");
        if let Err((before, t, what)) = judge(&paints, from) {
            panic!("{at}: her watch from {from} flipped at {before} and {t}: {what}");
        }
        let came = came.expect("it came");
        assert!(came >= end, "{at}: it came at {came}, mid-watch (to {end})");
    }
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
/// must not be exempt ([`world_ticked_guards`]), her blink with it must be
/// (the exemptions combine) and, in ASCII at a sky step, with her box put
/// over the window, the sky's cells off her glyphs are the sky's but a
/// glyph of hers is hers ([`mutants`]). Before the exemption the
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
    for (at, seen) in &runs {
        assert!(seen.dial > 0, "{at}: no dial step in a long act");
        // The exemptions combining, tried at each paint the world's
        // clock exempts.
        assert!(seen.mutants[2] > 0, "{at}: no step to combine with");
        if at.contains("to dusk") {
            assert!(seen.sky > 0, "{at}: no sky step in a long act");
            // The sky's cells in her box, in ASCII.
            if at.ends_with("graphics=false") {
                assert!(seen.mutants[4] > 0, "{at}: no sky step in ASCII to try");
            }
        }
    }
}

/// What a paint of `guest` drew over `real` (`frame`, with its `looks` in
/// line art) at `now`, as the drawn stillness rule judges it: her, her
/// pieces, and every cell changed. `hook`: Chiyo-chichi's bob showing;
/// `delivered`: a fresh still for her film came with this paint.
#[allow(clippy::too_many_arguments)]
fn drawn_of(
    guest: &Guest,
    real: &Buffer,
    frame: &Buffer,
    looks: Vec<(String, Look)>,
    (now, graphics): (u64, bool),
    hook: bool,
    delivered: bool,
) -> Drawn {
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let osaka = &visit.osaka;
    let mut cells = std::collections::BTreeMap::new();
    let width = usize::from(real.area.width);
    for (i, (got, want)) in frame.content.iter().zip(&real.content).enumerate() {
        if got != want {
            let cell = if got.symbol().contains('\x1b') {
                "image".to_owned()
            } else {
                format!("{got:?}")
            };
            cells.insert(((i % width) as u16, (i / width) as u16), cell);
        }
    }
    Drawn {
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
        glyphs: if graphics {
            Vec::new()
        } else {
            let (sprite, _) = osaka.picture(now);
            sprite
                .iter()
                .filter_map(|c| {
                    Some((
                        u16::try_from(osaka.x + c.dx).ok()?,
                        u16::try_from(osaka.y + c.dy).ok()?,
                    ))
                })
                .collect()
        },
        tv: visit
            .shown
            .iter()
            .find(|piece| piece.item == Furniture::Tv)
            .map(|piece| piece.rect()),
        pieces: visit.shown.iter().map(|piece| piece.rect()).collect(),
        clocks: shown_rects(visit, Furniture::Clock),
        windows: shown_rects(visit, Furniture::Window),
        hook,
        chat: osaka.looking_up_at_chat() || osaka.stirring_at_chat(now),
        delivered,
        world: guest
            .time_of_day(now)
            .map(|minute| (art::Dial::at(minute), art::Sky::at(minute))),
    }
}

/// The drawn stillness check over `rooms`, each in both modes and every
/// mood (see [`no_long_act_flips_drawn_cells_faster_than_a_frame`]).
fn drawn_stillness(rooms: &[Room]) -> Vec<(String, Seen)> {
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
    let runs: Vec<(String, Seen)> = std::thread::scope(|scope| {
        let mut runs = Vec::new();
        for room in rooms {
            for graphics in [false, true] {
                runs.push(scope.spawn(move || {
                    let mut seen = Seen::default();
                    let mut stirs = 0u32;
                    // And in the shopping room, a run that sees a fresh
                    // still mid-watch (since the door batch's step 2 moved
                    // her stage gifts, none of the four moods' runs there
                    // does in line art: of 32 tried, only this one).
                    let fresh_still: &[(u64, Mood)] = if room.name == "home, shopping" {
                        &[(0, Mood::Ordinary)]
                    } else {
                        &[]
                    };
                    for (seed, mood) in moods.into_iter().chain(fresh_still.iter().copied()) {
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
                                let judged = judge(&paints, since).unwrap_or_else(|(before, t, what)| {
                                    panic!("{at}: an act from {since} flipped at {before} and {t}: {what}")
                                });
                                mutants(&at, &paints, since, &mut seen.mutants);
                                seen.watched += u32::from(watch);
                                seen.swapped += u32::from(watch && judged.swapped);
                                seen.shopped += u32::from(shopping);
                                seen.looked += u32::from(judged.looked);
                                seen.dial += judged.dial;
                                seen.sky += judged.sky;
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
                            let drawn = drawn_of(
                                &guest,
                                &room.real,
                                &frame,
                                looks,
                                (now, graphics),
                                now <= hook_end,
                                delivered,
                            );
                            act.get_or_insert((since, watch, shopping, Vec::new()))
                                .3
                                .push(drawn);
                        }
                        if let Some(done) = act.take() {
                            check(done);
                        }
                    }
                    seen.stirs = stirs;
                    (format!("{} graphics={graphics}", room.name), seen)
                }));
            }
        }
        runs.into_iter()
            .map(|run| run.join().expect("a run"))
            .collect()
    });
    for (at, seen) in &runs {
        let Seen {
            watched,
            swapped,
            shopped,
            looked,
            dial,
            sky,
            stirs,
            mutants,
        } = seen;
        println!(
            "{at}: {watched} watches over 30 s ({swapped} with a fresh still), \
             {shopped} on the shopping channel, {looked} with a look up at the chat, \
             {dial} dial and {sky} sky steps in a long act, {stirs} stirs; \
             mutants tried {mutants:?}"
        );
    }
    assert!(
        runs.iter().any(|(_, seen)| seen.stirs > 0),
        "no stir at the chat, dozing, so its frame isn't tried"
    );
    for (at, seen) in &runs {
        assert!(seen.watched > 0, "{at}: never watched for over 30 s");
        assert!(
            seen.looked > 0,
            "{at}: never looked up at the chat in a long act"
        );
        // An exempt change doesn't hide a flip after it, nor does a flip.
        assert!(seen.mutants[0] > 0, "{at}: no flip to follow");
        assert!(seen.mutants[1] > 0, "{at}: no look's change to follow");
        assert!(seen.mutants[6] > 0, "{at}: no look's change to pose under");
        if at.ends_with("graphics=true") {
            assert!(seen.swapped > 0, "{at}: no fresh still mid-watch");
        }
        if at.starts_with("home, shopping ") {
            assert!(
                seen.shopped > 0,
                "{at}: never on the shopping channel for over 30 s"
            );
            assert!(seen.mutants[5] > 0, "{at}: no hook's bob to blink with");
        }
    }
    runs
}

/// What a room's runs in one mode showed: watches over 30 s, those with a
/// fresh still after their first 10 s, shopping acts over 30 s, acts with
/// a look up at the chat after their first 10 s, the world's clock
/// stepping in a checked act (her dial, her window's sky), her stirs at
/// the chat, dozing, and each of [`mutants`] tried.
#[derive(Default)]
struct Seen {
    watched: u32,
    swapped: u32,
    shopped: u32,
    looked: u32,
    dial: u32,
    sky: u32,
    stirs: u32,
    mutants: [u32; 7],
}

/// The rule's mutants over an act's real paints (phase 5c's tail, T4), so
/// the test is shown to catch what it should; `tried` counts each:
/// 0. a change of hers (a mark in a cell of her box) a paint after a
///    change that isn't exempt, with nothing else between, is a flip
///    within a frame of it (once an act);
/// 1. the same a paint after a look up's or a stir's change (exempt, but
///    counted on from: [`Exempt::counts`]), its start, a step or its end,
///    the look still on or over, is a flip within a frame of that (once
///    an act);
///
/// and at each paint the world's clock alone exempts, beside
/// [`world_ticked_guards`]:
/// 2. her blink with it is exempt, the exemptions combining (her face
///    to or from a blink, a cell of her glyphs or her look changed);
/// 3. that with a cell outside every footprint changed as well is not
///    (but under a look up, whose own change may take one in);
/// 4. in ASCII, where her window's sky stepped and with her box put over
///    the window: the sky's cells in her box, off her glyphs, are the
///    sky's (exempt); with one of them her glyph, it's hers (not);
///
/// and at the first paint in an act the hook's bob alone exempts:
/// 5. her blink with it is exempt, and with a cell outside every
///    footprint as well is not;
///
/// and at the first look up's or stir's change in an act with the look
/// still on a paint later:
/// 6. her pose changed at that paint (her bob flipping under the look,
///    say: not the look's own change, [`looks_own_pose`]) is a flip
///    within a frame of the look's change.
fn mutants(at: &str, paints: &[Drawn], since: u64, tried: &mut [u32; 7]) {
    let mark = || "mutant".to_owned();
    let quiet = |was: &Drawn, now: &Drawn| {
        was.shown == now.shown
            && was.cells == now.cells
            && was.looks == now.looks
            && was.world == now.world
    };
    let mut done = [false; 2];
    let mut posed = false;
    for k in 1..paints.len().saturating_sub(1) {
        if done == [true; 2] && posed {
            break;
        }
        let (was, now, next) = (&paints[k - 1], &paints[k], &paints[k + 1]);
        let same = was.shown == now.shown && was.cells == now.cells && was.looks == now.looks;
        if now.t < since + 10_000 || same {
            continue;
        }
        let kind = exempt_drawn(was, now);
        let which = match kind {
            None => 0,
            Some(Exempt::Look) => 1,
            Some(_) => continue,
        };
        let Some(her) = now.her else {
            continue;
        };
        if !quiet(now, next) || next.t - now.t >= osaka::USE_FRAME_MS {
            continue;
        }
        if which == 1 && next.chat && !posed {
            // Her pose changed under the look, as drawn: her look (line
            // art) or a cell of her box (ASCII) with it.
            let mut moved = next.clone();
            let ((pose, ..), ..) = moved.shown;
            moved.shown.0.0 = if pose == Pose::Sit {
                Pose::Stand
            } else {
                Pose::Sit
            };
            match moved.looks.iter().position(|(_, look)| hers(look)) {
                Some(i) => {
                    moved.looks[i].0 = mark();
                    moved.looks.sort_by(|a, b| a.0.cmp(&b.0));
                }
                None => {
                    moved.cells.insert((her.x, her.y), mark());
                }
            }
            let mutant = [was.clone(), now.clone(), moved];
            assert_eq!(
                judge(&mutant, since)
                    .err()
                    .map(|(before, t, _)| (before, t)),
                Some((now.t, next.t)),
                "{at}: her pose changed at {} under the look a paint after {:?} at {}",
                next.t,
                kind,
                now.t
            );
            posed = true;
            tried[6] += 1;
        }
        if done[which] {
            continue;
        }
        // The act judged as far as this passed, so the rule's count runs
        // from this change: these three paints are the rest of it.
        let mut marked = next.clone();
        marked.cells.insert((her.x, her.y), mark());
        let mutant = [was.clone(), now.clone(), marked];
        assert_eq!(
            judge(&mutant, since)
                .err()
                .map(|(before, t, _)| (before, t)),
            Some((now.t, next.t)),
            "{at}: a change of hers at {} a paint after {:?} at {}",
            next.t,
            kind,
            now.t
        );
        done[which] = true;
        tried[which] += 1;
    }
    for pair in paints.windows(2) {
        let (was, now) = (&pair[0], &pair[1]);
        if now.t < since + 10_000 || !now.hook || exempt_drawn(was, now) != Some(Exempt::Hook) {
            continue;
        }
        let Some(blinked) = with_blink(was, now) else {
            continue;
        };
        assert_eq!(
            exempt_drawn(was, &blinked),
            Some(Exempt::Blink),
            "{at}: at {} her blink with the hook's bob",
            now.t
        );
        if let Some(cell) = outside_every_footprint(was, now) {
            let mut elsewhere = blinked;
            elsewhere.cells.insert(cell, "mutant".to_owned());
            assert_eq!(
                exempt_drawn(was, &elsewhere),
                None,
                "{at}: at {} her blink, the hook's bob and a cell elsewhere",
                now.t
            );
            tried[5] += 1;
            // Once an act.
            break;
        }
    }
    for pair in paints.windows(2) {
        let (was, now) = (&pair[0], &pair[1]);
        let same = was.shown == now.shown && was.cells == now.cells && was.looks == now.looks;
        if now.t < since + 10_000 || was.world == now.world || same || !world_ticked(was, now) {
            continue;
        }
        world_ticked_guards(at, was, now);
        // Her blink with it.
        if let Some(mut blinked) = with_blink(was, now) {
            assert_eq!(
                exempt_drawn(was, &blinked),
                Some(Exempt::Blink),
                "{at}: at {} her blink with the world's clock",
                now.t
            );
            tried[2] += 1;
            // (Not under a look up, whose own change may take in a cell
            // outside every piece.)
            if let Some(cell) = outside_every_footprint(was, now).filter(|_| !was.chat && !now.chat)
            {
                blinked.cells.insert(cell, mark());
                assert_eq!(
                    exempt_drawn(was, &blinked),
                    None,
                    "{at}: at {} her blink, the world's clock and a cell elsewhere",
                    now.t
                );
                tried[3] += 1;
            }
        }
        // The sky's cells in her box, in ASCII.
        // The window her box is put over's (below), the first.
        let sky: Vec<(u16, u16)> = now
            .windows
            .first()
            .map(|&w| {
                changed(was, now)
                    .into_iter()
                    .filter(|&c| inside(Some(w), c))
                    .collect()
            })
            .unwrap_or_default();
        if now.graphics || !stepped(was, now, Furniture::Window) || sky.is_empty() {
            continue;
        }
        let over = |glyphs: Vec<(u16, u16)>| {
            let (mut was, mut now) = (was.clone(), now.clone());
            for paint in [&mut was, &mut now] {
                paint.her = paint.windows.first().copied();
                paint.glyphs.clone_from(&glyphs);
            }
            exempt_drawn(&was, &now)
        };
        assert_eq!(
            over(Vec::new()),
            Some(Exempt::World),
            "{at}: at {} the sky's cells in her box, off her glyphs",
            now.t
        );
        assert_eq!(
            over(vec![sky[0]]),
            None,
            "{at}: at {} one of her glyphs changed with the sky",
            now.t
        );
        tried[4] += 1;
    }
}

/// `now` with her slow blink as well, coming or going since `was`: her
/// face to or from a blink, and one of her glyphs (ASCII) or her look
/// (line art) changed. `None` if she isn't drawn.
fn with_blink(was: &Drawn, now: &Drawn) -> Option<Drawn> {
    let mut blinked = now.clone();
    blinked.shown.0.1 = if was.shown.0.1 == Face::Blink {
        Face::Vacant
    } else {
        Face::Blink
    };
    if let Some(&cell) = now.glyphs.first() {
        blinked.cells.insert(cell, "mutant".to_owned());
    } else {
        let i = now.looks.iter().position(|(_, look)| hers(look))?;
        blinked.looks[i].0 = "mutant".to_owned();
        blinked.looks.sort_by(|a, b| a.0.cmp(&b.0));
    }
    Some(blinked)
}

/// A cell outside every footprint (her box, her TV, every piece) of
/// `was` and `now`, if the screen has one.
fn outside_every_footprint(was: &Drawn, now: &Drawn) -> Option<(u16, u16)> {
    let footprints: Vec<Rect> = [was, now]
        .into_iter()
        .flat_map(|d| {
            d.her
                .into_iter()
                .chain(d.tv)
                .chain(d.pieces.iter().copied())
        })
        .collect();
    (0..100u16)
        .flat_map(|y| (0..300u16).map(move |x| (x, y)))
        .find(|&cell| !footprints.iter().any(|&r| inside(Some(r), cell)))
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
/// the window's footprint, off the quarter); a cell of hers changed (one
/// of her glyphs in ASCII, any cell of her box in line art); a cell
/// outside every footprint changed; her model changed (her lamp's
/// dark); another look changed (line art).
fn world_ticked_guards(at: &str, was: &Drawn, now: &Drawn) {
    let mark = "mutant".to_owned();
    let mut mutants: Vec<(&str, Drawn)> = Vec::new();
    let mut idle = now.clone();
    idle.world = was.world;
    mutants.push(("no reading of her clock changed", idle));
    // In ASCII one of her glyphs (the sky's cells round her in her box
    // are the sky's: [`world_stripped`]); in line art any cell of her box.
    let her_cell = now
        .glyphs
        .first()
        .copied()
        .or_else(|| now.her.map(|her| (her.x, her.y)));
    if let Some(cell) = her_cell {
        let mut hers = now.clone();
        hers.cells.insert(cell, mark.clone());
        mutants.push(("a cell of hers changed", hers));
    }
    if let Some(cell) = outside_every_footprint(was, now) {
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
            exempt_drawn(was, mutant).is_none(),
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
