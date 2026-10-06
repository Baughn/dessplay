//! The visit simulator: long visits in a few rooms over many seeds, in
//! ASCII, summarised — what she chose, where her time went, how her needs
//! moved, what she lost and said, and what she did about her home (home
//! acts by mood, set-downs, trials, how soon felt rules were mended, the
//! rules broken at the end, what she bought). It's how her needs and
//! moods are tuned before any statistics test is re-pinned. Ignored; run
//! by hand:
//!
//! ```text
//! cargo test --release -p dessplay --lib visit_census -- --ignored --nocapture
//! ```
//!
//! The day census ([`day_census`]) runs her routine instead: a game week
//! from Monday 00:00 in each room, by the hour of her day. The fed
//! afternoon census ([`fed_afternoon_census`]) runs the stillness band's
//! own setup: each room fed on a Tuesday afternoon, in each mood forced.
//!
//! Moving, as the censuses count it (phase 5c, D1): moving in sight
//! (walking, climbing, falling, a door's seen beats, a pull's heave) as a
//! share of her time in sight (visiting, not asleep for the night, not
//! out of sight), from minute 3 ([`WARM_MS`]; the warm-up apart), and her
//! set-offs a minute in sight ([`Osaka::census_motion`],
//! [`Osaka::set_off_log`]).

use super::*;
use crate::ui::houseguest::brain::{Mood, Need, Want};
use crate::ui::houseguest::mind::Loss;
use crate::ui::houseguest::osaka::{Body, Motion};
use crate::ui::houseguest::routine::{self, GameTime, Slot};
use crate::ui::houseguest::script::{ANDAGI_COUNTS, Play, ScriptId, SpliceId};
use std::collections::BTreeMap;

/// The census's visits to each room: one a seed, from 0.
const SEEDS: u64 = 16;

/// The start of a visit the stillness band leaves out (phase 5c, B5):
/// her arrival's run of walks, at her arrival's needs, is a far bigger
/// share of a short census visit than of an evening. The censuses print
/// it apart.
pub(super) const WARM_MS: u64 = 3 * 60_000;

/// A room to visit: the frame and view at `now`, and whether a chat line
/// arrives on this step.
pub(super) struct Room {
    pub name: &'static str,
    pub real: Buffer,
    pub view: IdleView,
    /// Furniture she owns.
    pub owns: &'static [Furniture],
    /// A chat line every this often (ms), if at all.
    pub chat_every: Option<u64>,
    /// The frame and view once this many chat lines have arrived, where
    /// each one changes the screen (else the frame stays `real`).
    pub live: Option<fn(u64) -> (Buffer, IdleView)>,
    /// Her clock as [`day_census`] begins, her routine fed to her; `None`
    /// for the visit censuses, which run her unfed (5a's tables: her
    /// visit by its own hour, not her day's).
    pub start: Option<GameTime>,
}

/// The stage room: an evening's chat, a visitor, nothing owned.
pub(super) fn stage_room() -> Room {
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    Room {
        name: "stage",
        real,
        view,
        owns: &[],
        chat_every: Some(45_000),
        live: None,
        start: None,
    }
}

/// Her home: a living room and a bedroom, furnished, with a fridge (so
/// she snacks) and a lamp (so it goes dark as she sleeps). Each piece
/// stands where [`Guest::give`] puts it, wherever it fits, so the home
/// starts broken (the sofa not facing the TV, the bookshelf or fridge
/// off a wall, the lamp far from her bed and desk; the census's "broken
/// at the start" row counts it): its home rows measure her mending a
/// home she was handed, not keeping a tidy one.
pub(super) fn furnished_room() -> Room {
    let (real, view) = home_screen();
    Room {
        name: "home",
        real,
        view,
        owns: &[
            Furniture::Sofa,
            Furniture::Tv,
            Furniture::Bed,
            Furniture::Desk,
            Furniture::Bookshelf,
            Furniture::Fridge,
            Furniture::Lamp,
        ],
        chat_every: Some(90_000),
        live: None,
        start: None,
    }
}

/// A resident beside a chat, a sofa and TV owned. Its text all lies
/// above her reach: no floor gives her a line to pull or a letter to
/// swap, and none of her standing spots is unrestful in either drawing
/// mode, so her runs here are the same in ASCII and line art.
pub(super) fn resident_room() -> Room {
    let (w, h) = (100, 30);
    let mut real = rooms(w, h);
    let text: Vec<(u16, u16, String)> = (0..10)
        .map(|i| {
            (
                2 + i * 3 % 20,
                3 + i * 2,
                "so what did you think".to_owned(),
            )
        })
        .collect();
    scatter(&mut real, &text, &[]);
    Room {
        name: "resident",
        real,
        view: resident_view(w, h, None),
        owns: &[Furniture::Sofa, Furniture::Tv],
        chat_every: Some(60_000),
        live: None,
        start: None,
    }
}

/// A big live chat: the client's own layout at 200×50, its chat pane
/// full, and a line arriving every 45 seconds that scrolls the rest up
/// (each line its own, so the text under her keeps changing, as on a
/// real evening). The image census's hardest room: more text to pass
/// over, and none of it still.
pub(super) fn live_room() -> Room {
    let (real, view) = live_frame(0);
    Room {
        name: "live",
        real,
        view,
        owns: &[],
        chat_every: Some(45_000),
        live: Some(live_frame),
        start: None,
    }
}

/// [`live_room`]'s screen once `arrived` lines have come.
fn live_frame(arrived: u64) -> (Buffer, IdleView) {
    const WORDS: [&str; 16] = [
        "so", "what", "did", "you", "think", "of", "the", "ending", "osaka", "snacks", "episode",
        "next", "wait", "lol", "really", "same",
    ];
    let line = |i: u64| {
        (0..1 + i * 7 % 6)
            .map(|k| WORDS[((i * 5 + k * 3 + i / 16) % 16) as usize])
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut ui = crate::ui::houseguest::stage::chat_ui((arrived..arrived + 40).map(line));
    real_frame(&mut ui, 200, 50)
}

/// What one visit came to.
#[derive(Default)]
pub(super) struct Visit {
    /// Each choice she rolled, in order.
    pub choices: Vec<Want>,
    /// Milliseconds in each kind of act ("use:Sleep", "idle:LieBack"...).
    pub time: BTreeMap<String, u64>,
    /// The census group each kind of act counts in.
    pub groups: BTreeMap<String, &'static str>,
    /// Her needs every five minutes.
    pub needs: Vec<String>,
    /// Beats she was owed, by loss.
    pub beats: BTreeMap<String, usize>,
    /// Lines she said (bubble text), by line.
    pub said: BTreeMap<String, usize>,
    /// How her headings went.
    pub headings: BTreeMap<String, usize>,
    /// Her mood for the visit.
    pub mood: Option<Mood>,
    /// The things she did about her home (her mood's cap counts them).
    pub home_acts: u8,
    /// The pieces she set down that the frame took (a trial's every spot).
    pub set_downs: u32,
    /// The times she lifted a piece again, to try it in another spot.
    pub retried: u32,
    /// The moves she let go of with the piece in her pocket.
    pub dropped: usize,
    /// The rules of her home broken as the visit began.
    pub broken_at_start: Vec<String>,
    /// Each rule she felt: when, and when it was mended, if it was.
    pub felt: Vec<(String, u64, Option<u64>)>,
    /// The rules of her home broken at the visit's end (`*` felt).
    pub broken: Vec<String>,
    /// What she bought off the shopping channel, when, with her beauty
    /// need then and whether it was her most pressing.
    pub bought: Option<(Furniture, u64, f64, bool)>,
    /// Her beauty need at the visit's end.
    pub beauty: f64,
    /// How she'd put her home right at the visit's end, as last worked
    /// out (empty: no way, or nothing she'd mend).
    pub repairs: Vec<String>,
    /// Her nesting need at the visit's end.
    pub nesting: f64,
    /// The vignettes she played as a use's or a spacing-out's own
    /// script, by name: "shopping", "surf", "riddle", and "bedtime" (a
    /// sleep's own script, not a trial's, with a lamp shown: on a
    /// moment, then off).
    pub scripts: BTreeMap<String, usize>,
    /// The splices that wrapped her uses, by name and branch.
    pub splices: BTreeMap<String, usize>,
    /// The chat lines asking her something that she answered, playing
    /// on (the sata andagi's).
    pub answers: usize,
    /// The pooled lines she said, by pool.
    pub pooled: BTreeMap<String, usize>,
    /// Milliseconds in sight (visiting, not asleep for the night, not
    /// out of sight): in the warm-up ([`WARM_MS`]), and after it.
    pub sight: [u64; 2],
    /// Milliseconds moving in sight in the warm-up.
    pub warm_moving: u64,
    /// Milliseconds moving in sight after the warm-up, by what for and
    /// what her body did ([`Osaka::census_motion`]).
    pub motion: BTreeMap<(&'static str, Body), u64>,
    /// Milliseconds moving in sight after the warm-up to a job, by its
    /// kind and the want she went for.
    pub wants: BTreeMap<(&'static str, String), u64>,
    /// Milliseconds out of sight (away off the screen, between a door's
    /// ends), and asleep for the night.
    pub hidden: u64,
    pub asleep: u64,
    /// Her set-offs in the warm-up.
    pub warm_set_offs: usize,
    /// Her set-offs after the warm-up, by what for and how.
    pub set_offs: BTreeMap<(&'static str, Body), usize>,
    /// After the warm-up: the chat lines that stopped her ("look", or
    /// "passing" over text), by what she was moving for ("still" if she
    /// wasn't); and the set-offs that were her first after one, the same.
    pub chat_cuts: BTreeMap<String, usize>,
    pub restarts: BTreeMap<String, usize>,
    /// Milliseconds in sight after the warm-up she was still but busy:
    /// exercising, kicking her feet lying on her front, swapping letters,
    /// sneezing, picking up after a sneeze.
    pub busy: BTreeMap<&'static str, u64>,
    /// After the warm-up: the times she started exercising, and the
    /// bubbles that came up over her (each new one), in sight.
    pub exercise_starts: usize,
    pub bubbles: usize,
}

impl Visit {
    /// Milliseconds moving in sight after the warm-up.
    pub(super) fn moving(&self) -> u64 {
        self.motion.values().sum()
    }

    /// Her set-offs after the warm-up.
    pub(super) fn set_off_count(&self) -> usize {
        self.set_offs.values().sum()
    }
}

/// The census's label for what a chat line stopped her at: "look" or
/// "passing" (over text, on without a look), and what she was moving
/// for, or "still".
fn chat_label((passing, cut): (bool, Option<&'static str>)) -> String {
    format!(
        "{} {}",
        if passing { "passing" } else { "look" },
        cut.unwrap_or("still")
    )
}

/// What she's doing, as the census counts it.
fn doing(osaka: &Osaka, now: u64) -> String {
    if let Some((seat, ..)) = osaka.use_span() {
        let piece = if seat.makeshift() { "made" } else { "real" };
        return format!("use:{:?}:{piece}", seat.what);
    }
    let name = osaka.act_name();
    match name.as_str() {
        "Idle" => format!("idle:{:?}", osaka.appearance(now).0)
            .split('(')
            .next()
            .unwrap_or_default()
            .to_owned(),
        _ => name,
    }
}

/// A visit of `minutes` in `room` from `seed`, in `mood` if given (else
/// the one the visit draws).
pub(super) fn simulate(room: &Room, seed: u64, minutes: u64, mood: Option<Mood>) -> Visit {
    simulate_with(room, seed, minutes, mood, false, |_, _| {})
}

/// The census's names for the vignettes `play` plays: its own script,
/// if it's one (a sleep's own script is "bedtime" where a `lamp` shows:
/// on a moment, then off; trying the bed, off at once, or a sleep with
/// no lamp, is plain), and each splice round it, by branch.
fn named(play: Play, lamp: bool) -> Vec<(bool, String)> {
    let own = match play.own {
        ScriptId::Shopping => Some("shopping".to_owned()),
        ScriptId::Surf => Some("surf".to_owned()),
        ScriptId::Riddle => Some("riddle".to_owned()),
        ScriptId::Night => Some("night".to_owned()),
        ScriptId::DashLunch => Some("dash, lunch".to_owned()),
        ScriptId::DashForgot => Some("dash, forgot".to_owned()),
        ScriptId::Setsubun => Some("setsubun".to_owned()),
        ScriptId::FirstSunrise => Some("first sunrise".to_owned()),
        ScriptId::Dream => Some("dream".to_owned()),
        ScriptId::Escalator => Some("escalator".to_owned()),
        ScriptId::LookOut => Some("look out".to_owned()),
        ScriptId::ClockGlance => Some("clock glance".to_owned()),
        ScriptId::Sleep if play.branch == 0 && lamp => Some("bedtime".to_owned()),
        ScriptId::Sleep
        | ScriptId::Lounge
        | ScriptId::Nap
        | ScriptId::Homework
        | ScriptId::Watch
        | ScriptId::Read
        | ScriptId::Snack
        | ScriptId::Pet
        | ScriptId::Crumple
        | ScriptId::Unpack
        | ScriptId::Chopsticks
        | ScriptId::Andagi
        | ScriptId::NoMelon
        | ScriptId::Scary => None,
    };
    let splices = [play.before, play.after]
        .into_iter()
        .flatten()
        .map(|s| match s.splice {
            SpliceId::Chopsticks if s.branch == 0 => "chopsticks, clean".to_owned(),
            SpliceId::Chopsticks => "chopsticks, bad".to_owned(),
            SpliceId::Andagi => match ANDAGI_COUNTS.get(usize::from(s.branch)) {
                Some(count) => format!("sata andagi ×{count}"),
                None => format!("sata andagi, branch {}", s.branch),
            },
            SpliceId::NoMelon => "no melon".to_owned(),
            SpliceId::Scary => "scary story".to_owned(),
            SpliceId::TestSnack | SpliceId::TestSleep | SpliceId::TestBedtime => {
                format!("{:?}", s.splice)
            }
        });
    own.map(|own| (false, own))
        .into_iter()
        .chain(splices.map(|s| (true, s)))
        .collect()
}

/// Her arrival in `room` from `seed`, drawn in line art if `graphics`:
/// in `mood` if given, and given what she owns there, each where it
/// fits.
fn arrive_in(room: &Room, seed: u64, graphics: bool, mood: Option<Mood>) -> Guest {
    arrive_drawn(room, seed, mood, |guest| {
        if graphics {
            guest.set_picker(kitty());
        }
    })
}

/// [`arrive_in`], `draw` setting up how she's drawn before she comes.
fn arrive_drawn(
    room: &Room,
    seed: u64,
    mood: Option<Mood>,
    draw: impl FnOnce(&mut Guest),
) -> Guest {
    // Unfed: the censuses measure her by the hour of the visit, not of
    // her day (5a's tables; a census of her day starts her at a time:
    // see [`day_guest`]).
    assert!(
        room.start.is_none(),
        "{}: a visit census is unfed",
        room.name
    );
    let mut guest = Guest::new(seed).unfed();
    draw(&mut guest);
    guest.cue(Scene::Arrive);
    paint(&mut guest, &room.real, &room.view, 0);
    if let Some(mood) = mood {
        let State::Visiting(visit) = &mut guest.state else {
            panic!(
                "{} seed {seed}: not visiting after her arrival's paint, so {mood:?} can't be forced",
                room.name
            );
        };
        visit.osaka.set_mood(mood);
    }
    for &item in room.owns {
        guest.give(item);
        paint(&mut guest, &room.real, &room.view, 0);
    }
    guest
}

/// [`simulate`], drawn in line art if `graphics`, showing `watch` her at
/// every step (before it).
fn simulate_with(
    room: &Room,
    seed: u64,
    minutes: u64,
    mood: Option<Mood>,
    graphics: bool,
    mut watch: impl FnMut(&Osaka, u64),
) -> Visit {
    let guest = arrive_in(room, seed, graphics, mood);
    let (visit, _) = visit_from(room, guest, minutes, |guest, now| {
        if let State::Visiting(visit) = &guest.state {
            watch(&visit.osaka, now);
        }
    });
    visit
}

/// The visit `guest`, just arrived in `room`, goes on to have for
/// `minutes`, showing `watch` her at every step (before it); and her at
/// its end.
pub(super) fn visit_from(
    room: &Room,
    mut guest: Guest,
    minutes: u64,
    mut watch: impl FnMut(&Guest, u64),
) -> (Visit, Guest) {
    let (mut real, mut view) = (room.real.clone(), room.view.clone());
    let mut arrived = 0;
    // Unfed, the shopping channel is on at her first watch, as on any
    // visit it's due: what she buys, and when, shows whether decor comes
    // before the furniture she lacks. A fed census keeps its room as set
    // (see [`fed_afternoon`]).
    let unfed = room.start.is_none();
    if unfed {
        guest.shop();
    }
    let mut out = Visit::default();
    if let State::Visiting(visit) = &guest.state {
        out.broken_at_start = visit.broken.iter().map(|b| b.key.label()).collect();
    }
    let mut now = 0;
    let mut said: Option<&'static str> = None;
    let mut playing: Option<u64> = None;
    let mut answered: Option<u64> = None;
    let mut tally = Tally::default();
    while now < minutes * 60_000 {
        let step = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        watch(&guest, now);
        if let State::Visiting(visit) = &guest.state {
            let doing = doing(&visit.osaka, now);
            out.groups.insert(doing.clone(), visit.osaka.census_group());
            *out.time.entry(doing.clone()).or_default() += step;
            let speech = match visit.osaka.appearance(now).2 {
                Some(osaka::Bubble::Say(text)) => Some(text),
                _ => None,
            };
            if speech != said
                && let Some(text) = speech
            {
                *out.said.entry(text.to_owned()).or_default() += 1;
            }
            said = speech;
            let plays = visit.osaka.plays_since();
            if let Some((since, play)) = plays
                && playing != Some(since)
            {
                let lamp = visit
                    .shown
                    .iter()
                    .any(|s| s.item == Furniture::Lamp && !s.boxed);
                for (splice, name) in named(play, lamp) {
                    let row = if splice {
                        &mut out.splices
                    } else {
                        &mut out.scripts
                    };
                    *row.entry(name).or_default() += 1;
                }
            }
            playing = plays.map(|(since, _)| since);
            let answer = visit.osaka.answered_until();
            if answer.is_some() && answer != answered {
                out.answers += 1;
            }
            answered = answer;
            tally.step(&visit.osaka, &doing, now, step, &mut out);
        } else {
            // A new visit's logs start afresh.
            tally = Tally::default();
        }
        if (now + step) / 300_000 != now / 300_000
            && let State::Visiting(visit) = &guest.state
        {
            out.needs.push(visit.osaka.needs().summary());
        }
        now += step;
        let mut changed = false;
        if room
            .chat_every
            .is_some_and(|every| now / every != (now - step) / every)
        {
            if let Some(live) = room.live {
                arrived += 1;
                let mark = view.chat_mark;
                (real, view) = live(arrived);
                view.chat_mark = mark;
                // The client draws the line as it comes.
                changed = true;
            }
            // Every other line asks her something.
            view.chat_mark.synced += 1;
            view.chat_mark.synced_asks = view.chat_mark.synced.is_multiple_of(2);
        }
        if guest.advance(now) || changed {
            paint(&mut guest, &real, &view, now);
        }
        if let State::Visiting(visit) = &guest.state {
            // Her logs as this step left them, while she's still here to
            // read (only what the step that ends a visit logs is lost).
            tally.logs(&visit.osaka, &mut out);
            if unfed {
                home_census(&guest, visit, now, &mut out);
            }
        }
    }
    if let State::Visiting(visit) = &guest.state {
        tally.logs(&visit.osaka, &mut out);
        out.mood = Some(visit.osaka.mood());
        out.home_acts = visit.osaka.home_acts();
        out.set_downs = visit.osaka.set_downs;
        out.retried = visit.osaka.retried;
        out.dropped = visit
            .osaka
            .beats
            .iter()
            .filter(|b| matches!(b.loss, Loss::Moved(_)))
            .count();
        let felt = visit.osaka.felt();
        out.broken = visit
            .broken
            .iter()
            .map(|b| {
                let star = if felt.contains(&b.key) { "*" } else { "" };
                format!("{}{star}", b.key.label())
            })
            .collect();
        out.beauty = visit.osaka.needs().get(Need::Beauty);
        out.nesting = visit.osaka.needs().get(Need::Nesting);
        out.repairs = visit.repairs.iter().map(|r| r.label()).collect();
        out.choices = visit
            .osaka
            .decisions
            .iter()
            .filter_map(|d| d.want)
            .collect();
        for h in &visit.osaka.headings {
            *out.headings.entry(h.clone()).or_default() += 1;
        }
        for beat in &visit.osaka.beats {
            *out.beats.entry(format!("{:?}", beat.loss)).or_default() += 1;
        }
        for (pool, ..) in visit.osaka.said_lines() {
            *out.pooled.entry(format!("{pool:?}")).or_default() += 1;
        }
    }
    (out, guest)
}

/// Her moving as a census visit tallies it into a [`Visit`], step by
/// step: where it has got to in her logs, and how she was at the last
/// step in sight.
#[derive(Default)]
struct Tally {
    /// Her set-offs and chat cuts tallied so far.
    set_offs: usize,
    chat_cuts: usize,
    /// Her bubble and census group at the last step in sight.
    bubble: Option<osaka::Bubble>,
    group: &'static str,
}

impl Tally {
    /// Tally the step of `step` ms from `now`, `doing` what the census
    /// calls it.
    fn step(&mut self, osaka: &Osaka, doing: &str, now: u64, step: u64, out: &mut Visit) {
        self.logs(osaka, out);
        let warm = now < WARM_MS;
        // Out of sight, or asleep: whatever she's at when she's back in
        // sight is a new onset.
        if osaka.sleeping() {
            out.asleep += step;
            (self.bubble, self.group) = (None, "");
            return;
        }
        if osaka.hidden(now) {
            out.hidden += step;
            (self.bubble, self.group) = (None, "");
            return;
        }
        out.sight[usize::from(!warm)] += step;
        let bubble = osaka.appearance(now).2;
        let group = osaka.census_group();
        if !warm {
            if bubble.is_some() && bubble != self.bubble {
                out.bubbles += 1;
            }
            if group == "exercise" && self.group != "exercise" {
                out.exercise_starts += 1;
            }
        }
        self.bubble = bubble;
        self.group = group;
        match osaka.census_motion(now) {
            Some(_) if warm => out.warm_moving += step,
            Some(Motion {
                purpose,
                body,
                want,
            }) => {
                *out.motion.entry((purpose, body)).or_default() += step;
                if purpose.starts_with("to ") {
                    let want = want.map_or("?".to_owned(), |w| format!("{w:?}"));
                    *out.wants.entry((purpose, want)).or_default() += step;
                }
            }
            None if warm => {}
            None => {
                let busy = match doing {
                    _ if group == "exercise" => Some("exercise"),
                    "idle:LieFront" => Some("lie front"),
                    "Swap" => Some("swap"),
                    "Sneeze" => Some("sneeze"),
                    "PutBack" => Some("put back"),
                    _ => None,
                };
                if let Some(busy) = busy {
                    *out.busy.entry(busy).or_default() += step;
                }
            }
        }
    }

    /// Tally what her set-off and chat-cut logs have gained.
    fn logs(&mut self, osaka: &Osaka, out: &mut Visit) {
        for set_off in osaka.set_off_log.iter().skip(self.set_offs) {
            if set_off.at < WARM_MS {
                out.warm_set_offs += 1;
                continue;
            }
            *out.set_offs
                .entry((set_off.purpose, set_off.body))
                .or_default() += 1;
            if let Some(after) = set_off.after_chat {
                *out.restarts.entry(chat_label(after)).or_default() += 1;
            }
        }
        self.set_offs = osaka.set_off_log.len();
        for &(at, passing, cut) in osaka.chat_cuts.iter().skip(self.chat_cuts) {
            if at >= WARM_MS {
                *out.chat_cuts.entry(chat_label((passing, cut))).or_default() += 1;
            }
        }
        self.chat_cuts = osaka.chat_cuts.len();
    }
}

/// The census of her home, after the step at `now`: rules she has just
/// felt, felt rules now mended, and what she has just bought.
fn home_census(guest: &Guest, visit: &super::super::Visit, now: u64, out: &mut Visit) {
    for key in visit.osaka.felt() {
        let label = key.label();
        if !out.felt.iter().any(|(l, ..)| *l == label) {
            out.felt.push((label, now, None));
        }
    }
    for (label, _, mended) in &mut out.felt {
        if mended.is_none() && !visit.broken.iter().any(|b| b.key.label() == *label) {
            *mended = Some(now);
        }
    }
    if out.bought.is_none()
        && let Some(item) = guest.ledger.ordered
    {
        let needs = visit.osaka.needs();
        out.bought = Some((
            item,
            now,
            needs.get(Need::Beauty),
            needs.pressing(Need::Beauty),
        ));
    }
}

/// The census group a kind of act counts in, as `groups` recorded it
/// (see [`Osaka::census_group`]: every act is in one).
fn group(groups: &BTreeMap<String, &'static str>, doing: &str) -> &'static str {
    groups.get(doing).copied().unwrap_or("unrecorded")
}

/// The share of `part` in `whole`, as a percentage.
fn pct(part: u64, whole: u64) -> f64 {
    100.0 * part as f64 / whole.max(1) as f64
}

/// The mean and the (sample) standard deviation of `values`.
fn mean_sd(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n.max(1.0);
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean, var.sqrt())
}

/// One census cell's moving, as its last table has it.
struct MotionRow {
    /// Moving in sight after the warm-up, % of the time in sight then,
    /// pooled over its visits; σ of the pooled share of a set of
    /// [`MotionRow::set`] visits, over its disjoint sets (`None` with
    /// fewer than two); and the least and most a visit had.
    share: f64,
    share_sd: Option<f64>,
    least: f64,
    most: f64,
    /// Set-offs a minute in sight after the warm-up, pooled, and σ of a
    /// set's as for the share.
    rate: f64,
    rate_sd: Option<f64>,
    /// The visits a set has, and how many sets.
    set: usize,
    sets: usize,
    /// The warm-up's moving share and set-offs a minute in sight.
    warm_share: f64,
    warm_rate: f64,
}

/// Prints the moving of `visits` (moving in sight, with the warm-up
/// apart; by what for and by what her body did; to jobs, by the want;
/// set-offs a minute in sight, by what for, doors apart; what chat lines
/// stopped her, and her set-offs after them; still but busy; exercise
/// begun and bubbles a minute; time out of sight and asleep), taking
/// sets of `set` visits in order for the spread of its means; returns
/// its row.
fn motion_summary(visits: &[Visit], set: usize) -> MotionRow {
    let sight: u64 = visits.iter().map(|v| v.sight[1]).sum();
    let minutes = sight as f64 / 60_000.0;
    let per_min = |n: usize| n as f64 / minutes.max(1e-9);
    let moving: u64 = visits.iter().map(Visit::moving).sum();
    let set_offs: usize = visits.iter().map(Visit::set_off_count).sum();
    let shares: Vec<f64> = visits.iter().map(|v| pct(v.moving(), v.sight[1])).collect();
    let pooled = |of: &[Visit]| {
        let sight: u64 = of.iter().map(|v| v.sight[1]).sum();
        (
            pct(of.iter().map(Visit::moving).sum(), sight),
            of.iter().map(Visit::set_off_count).sum::<usize>() as f64
                / (sight as f64 / 60_000.0).max(1e-9),
        )
    };
    let sets: Vec<(f64, f64)> = visits.chunks_exact(set.max(1)).map(pooled).collect();
    let spread = |values: Vec<f64>| (values.len() >= 2).then(|| mean_sd(&values).1);
    let warm_sight: u64 = visits.iter().map(|v| v.sight[0]).sum();
    let row = MotionRow {
        share: pct(moving, sight),
        share_sd: spread(sets.iter().map(|s| s.0).collect()),
        least: shares.iter().copied().fold(f64::INFINITY, f64::min),
        most: shares.iter().copied().fold(0.0, f64::max),
        rate: per_min(set_offs),
        rate_sd: spread(sets.iter().map(|s| s.1).collect()),
        set,
        sets: sets.len(),
        warm_share: pct(visits.iter().map(|v| v.warm_moving).sum(), warm_sight),
        warm_rate: visits.iter().map(|v| v.warm_set_offs).sum::<usize>() as f64
            / (warm_sight as f64 / 60_000.0).max(1e-9),
    };
    let sd = |sd: Option<f64>| sd.map_or("-".to_owned(), |sd| format!("{sd:.2}"));
    let (_, seed_sd) = mean_sd(&shares);
    eprintln!(
        "  moving in sight (from minute {}): {:.1}% (σ of a {set}-visit mean {} over {} sets; a visit {:.1}–{:.1}, σ {seed_sd:.1}); set-offs {:.2} a minute (σ {}); warm-up {:.1}%, {:.2} a minute",
        WARM_MS / 60_000,
        row.share,
        sd(row.share_sd),
        row.sets,
        row.least,
        row.most,
        row.rate,
        sd(row.rate_sd),
        row.warm_share,
        row.warm_rate,
    );
    let mut purposes: BTreeMap<&str, u64> = BTreeMap::new();
    let mut bodies: BTreeMap<Body, u64> = BTreeMap::new();
    let mut wants: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    let mut offs: BTreeMap<&str, usize> = BTreeMap::new();
    let mut doors = 0;
    let mut cuts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut restarts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut busy: BTreeMap<&str, u64> = BTreeMap::new();
    for v in visits {
        for (&(purpose, body), &ms) in &v.motion {
            *purposes.entry(purpose).or_default() += ms;
            *bodies.entry(body).or_default() += ms;
        }
        for ((purpose, want), &ms) in &v.wants {
            *wants.entry((purpose, want.as_str())).or_default() += ms;
        }
        for (&(purpose, body), &n) in &v.set_offs {
            *offs.entry(purpose).or_default() += n;
            if body == Body::Door {
                doors += n;
            }
        }
        for (k, &n) in &v.chat_cuts {
            *cuts.entry(k.as_str()).or_default() += n;
        }
        for (k, &n) in &v.restarts {
            *restarts.entry(k.as_str()).or_default() += n;
        }
        for (&k, &ms) in &v.busy {
            *busy.entry(k).or_default() += ms;
        }
    }
    let by_ms = |rows: Vec<(String, u64)>| {
        let mut rows = rows;
        rows.sort_by_key(|(_, ms)| std::cmp::Reverse(*ms));
        rows.iter()
            .map(|(k, ms)| format!("{k} {:.1}", pct(*ms, sight)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    eprintln!(
        "  moving by what for (% in sight): {}",
        by_ms(purposes.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    );
    eprintln!(
        "  moving by body (% in sight): {}",
        by_ms(
            Body::ALL
                .iter()
                .filter_map(|b| bodies.get(b).map(|ms| (b.label().to_owned(), *ms)))
                .collect()
        )
    );
    eprintln!(
        "  to jobs, by want (% in sight): {}",
        by_ms(
            wants
                .iter()
                .map(|((p, w), ms)| (format!("{p}/{w}"), *ms))
                .collect()
        )
    );
    let mut offs: Vec<(&str, usize)> = offs.into_iter().collect();
    offs.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!(
        "  set-offs a minute in sight by what for: {}; doors {:.2}",
        offs.iter()
            .map(|(k, n)| format!("{k} {:.2}", per_min(*n)))
            .collect::<Vec<_>>()
            .join(", "),
        per_min(doors)
    );
    let counts = |rows: &BTreeMap<&str, usize>| {
        rows.iter()
            .map(|(k, n)| format!("{k} {:.2}", per_min(*n)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    eprintln!(
        "  chat lines that stopped her, a minute in sight: {}; her set-offs right after one: {}",
        counts(&cuts),
        counts(&restarts)
    );
    eprintln!(
        "  still but busy (% in sight): {}; exercise begun {:.2} a minute; bubbles {:.2} a minute",
        by_ms(busy.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
        per_min(visits.iter().map(|v| v.exercise_starts).sum()),
        per_min(visits.iter().map(|v| v.bubbles).sum()),
    );
    let mut time: BTreeMap<&str, u64> = BTreeMap::new();
    for v in visits {
        for (k, &ms) in &v.time {
            *time.entry(k.as_str()).or_default() += ms;
        }
    }
    let whole: u64 = time.values().sum();
    let mut time: Vec<(&str, u64)> = time.into_iter().collect();
    time.sort_by_key(|(_, ms)| std::cmp::Reverse(*ms));
    eprintln!(
        "  where her time went (% of the visit, warm-up too): {}",
        time.iter()
            .filter(|(_, ms)| pct(*ms, whole) >= 1.0)
            .map(|(k, ms)| format!("{k} {:.1}", pct(*ms, whole)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let all: u64 = visits
        .iter()
        .map(|v| v.sight[0] + v.sight[1] + v.hidden + v.asleep)
        .sum();
    eprintln!(
        "  of the visit: out of sight {:.1}%, asleep {:.1}%",
        pct(visits.iter().map(|v| v.hidden).sum(), all),
        pct(visits.iter().map(|v| v.asleep).sum(), all)
    );
    row
}

/// Prints the vignettes `visits` played: scripts and splices by name (a
/// visit's mean, and how many visits had one), the chat lines she
/// answered playing on, and her pooled lines by pool.
fn vignette_summary(visits: &[Visit]) {
    let n = visits.len().max(1) as f64;
    let rows = |of: fn(&Visit) -> &BTreeMap<String, usize>| {
        let mut total: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for visit in visits {
            for (k, &v) in of(visit) {
                let row = total.entry(k).or_default();
                row.0 += v;
                row.1 += usize::from(v > 0);
            }
        }
        let rows: Vec<String> = total
            .iter()
            .map(|(k, (sum, had))| format!("{k} {:.2} ({had} visits)", *sum as f64 / n))
            .collect();
        if rows.is_empty() {
            "none".to_owned()
        } else {
            rows.join(", ")
        }
    };
    eprintln!("  scripts a visit: {}", rows(|v| &v.scripts));
    eprintln!("  splices a visit: {}", rows(|v| &v.splices));
    eprintln!(
        "  andagi answers: {} (in {} visits)",
        visits.iter().map(|v| v.answers).sum::<usize>(),
        visits.iter().filter(|v| v.answers > 0).count()
    );
    eprintln!("  pooled lines a visit: {}", rows(|v| &v.pooled));
}

/// Prints what `visits` did about her home: home acts by mood, set-downs,
/// trials and dropped carries, how soon felt rules were mended, the rules
/// still broken at the end, what she bought, and her beauty need.
fn home_summary(visits: &[Visit]) {
    let mut by_mood: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for visit in visits {
        by_mood
            .entry(visit.mood.map_or("?".to_owned(), |m| format!("{m:?}")))
            .or_default()
            .push(visit.home_acts);
    }
    eprintln!(
        "  home acts a visit by mood: {}",
        by_mood
            .iter()
            .map(|(mood, acts)| {
                let mut hist = [0usize; 4];
                for &a in acts {
                    hist[usize::from(a).min(3)] += 1;
                }
                let mean = acts.iter().map(|&a| f64::from(a)).sum::<f64>() / acts.len() as f64;
                format!("{mood} {mean:.2} (of {}: 0/1/2/3 = {hist:?})", acts.len())
            })
            .collect::<Vec<_>>()
            .join(", ")
    );
    let sum = |f: fn(&Visit) -> u64| visits.iter().map(f).sum::<u64>();
    eprintln!(
        "  set down {}, spots tried again {}, carries dropped {}",
        sum(|v| u64::from(v.set_downs)),
        sum(|v| u64::from(v.retried)),
        sum(|v| v.dropped as u64)
    );
    let felt: Vec<&(String, u64, Option<u64>)> = visits.iter().flat_map(|v| &v.felt).collect();
    let mut took: Vec<u64> = felt
        .iter()
        .filter_map(|(_, at, mended)| mended.map(|m| (m - at) / 1000))
        .collect();
    took.sort_unstable();
    let at = |q: f64| took.get(((took.len() as f64 - 1.0) * q).round() as usize);
    let mut first: Vec<u64> = felt.iter().map(|(_, at, _)| at / 1000).collect();
    first.sort_unstable();
    eprintln!(
        "  felt {} (first felt median {:?} s), mended {} (felt -> mended median {:?} / p90 {:?} / max {:?} s)",
        felt.len(),
        first.get(first.len() / 2),
        took.len(),
        at(0.5),
        at(0.9),
        took.last()
    );
    let mut at_start: BTreeMap<&str, usize> = BTreeMap::new();
    for label in visits.iter().flat_map(|v| &v.broken_at_start) {
        *at_start.entry(label).or_default() += 1;
    }
    eprintln!(
        "  broken at the start: {} visits of {} with none; {at_start:?}",
        visits
            .iter()
            .filter(|v| v.broken_at_start.is_empty())
            .count(),
        visits.len()
    );
    let mut broken: BTreeMap<&str, usize> = BTreeMap::new();
    for label in visits.iter().flat_map(|v| &v.broken) {
        *broken.entry(label).or_default() += 1;
    }
    eprintln!(
        "  broken at the end: {} visits of {} with none; {broken:?}",
        visits.iter().filter(|v| v.broken.is_empty()).count(),
        visits.len()
    );
    // Keen on her home at the end, a felt rule broken, and no way to
    // mend it worked out (none, or her mood's cap reached).
    eprintln!(
        "  keen at the end with no way to mend: {} (nesting at the end: median {:.2})",
        visits
            .iter()
            .filter(|v| v.nesting > 0.5
                && v.repairs.is_empty()
                && v.broken.iter().any(|b| b.ends_with('*')))
            .count(),
        {
            let mut n: Vec<f64> = visits.iter().map(|v| v.nesting).collect();
            n.sort_by(f64::total_cmp);
            n[n.len() / 2]
        }
    );
    let bought: Vec<String> = visits
        .iter()
        .filter_map(|v| v.bought)
        .map(|(item, at, beauty, pressing)| {
            let why = if pressing { ", most pressing" } else { "" };
            format!("{item:?} at {} min (beauty {beauty:.2}{why})", at / 60_000)
        })
        .collect();
    eprintln!("  bought: {}", bought.join("; "));
    let mut beauty: Vec<f64> = visits.iter().map(|v| v.beauty).collect();
    beauty.sort_by(f64::total_cmp);
    eprintln!(
        "  beauty at the end: min {:.2}, median {:.2}, max {:.2}",
        beauty[0],
        beauty[beauty.len() / 2],
        beauty[beauty.len() - 1]
    );
}

/// The visit census: [`SEEDS`] unfed visits of 30 minutes to each room
/// (5a's tables), in a drawn mood, or each forced with `CENSUS_MOODS`;
/// in ASCII, or as `CENSUS_MODES` says ("line", "both"); at the rooms'
/// chat cadence, or as `CENSUS_CHAT` says ("quiet", "both"). Each cell
/// prints her choices and her time, then her moving
/// ([`motion_summary`]); a line a cell at the end.
#[test]
#[ignore = "the visit simulator: run by hand in release with --nocapture"]
fn visit_census() {
    const MINUTES: u64 = 30;
    // With CENSUS_MOODS set, each room in each mood (forced).
    let moods: Vec<Option<Mood>> = if std::env::var_os("CENSUS_MOODS").is_some() {
        Mood::ALL.into_iter().map(Some).collect()
    } else {
        vec![None]
    };
    let modes = census_modes();
    let chats = census_chats();
    let mut table = Vec::new();
    let rooms = [stage_room(), furnished_room(), resident_room()];
    let mut cells = Vec::new();
    for room in &rooms {
        for &mood in &moods {
            for &quiet in &chats {
                for &graphics in &modes {
                    cells.push((room, mood, quiet, graphics));
                }
            }
        }
    }
    for (room, mood, quiet, graphics) in cells {
        let room = &with_chat(room, quiet);
        let started = std::time::Instant::now();
        let visits: Vec<Visit> = std::thread::scope(|scope| {
            let runs: Vec<_> = (0..SEEDS)
                .map(|seed| {
                    scope.spawn(move || {
                        simulate_with(room, seed, MINUTES, mood, graphics, |_, _| {})
                    })
                })
                .collect();
            runs.into_iter()
                .map(|run| run.join().expect("a visit"))
                .collect()
        });
        let cpu = started.elapsed();
        let mut choices: BTreeMap<String, usize> = BTreeMap::new();
        let mut time: BTreeMap<String, u64> = BTreeMap::new();
        let mut kinds: BTreeMap<String, &'static str> = BTreeMap::new();
        let mut beats: BTreeMap<String, usize> = BTreeMap::new();
        let mut said: BTreeMap<String, usize> = BTreeMap::new();
        let mut headings: BTreeMap<String, usize> = BTreeMap::new();
        let mut dominant = 0.0f64;
        let mut longest = 0;
        for visit in &visits {
            for want in &visit.choices {
                *choices.entry(format!("{want:?}")).or_default() += 1;
            }
            for (k, v) in &visit.time {
                *time.entry(k.clone()).or_default() += v;
            }
            kinds.extend(visit.groups.iter().map(|(k, &g)| (k.clone(), g)));
            for (k, v) in &visit.beats {
                *beats.entry(k.clone()).or_default() += v;
            }
            for (k, v) in &visit.headings {
                *headings.entry(k.clone()).or_default() += v;
            }
            for (k, v) in &visit.said {
                *said.entry(k.clone()).or_default() += v;
            }
            let most = visit
                .choices
                .iter()
                .map(|w| visit.choices.iter().filter(|c| *c == w).count())
                .max()
                .unwrap_or(0);
            dominant = dominant.max(most as f64 / visit.choices.len().max(1) as f64);
            let mut run = 0;
            for (i, w) in visit.choices.iter().enumerate() {
                run = if i > 0 && visit.choices[i - 1] == *w {
                    run + 1
                } else {
                    1
                };
                longest = longest.max(run);
            }
        }
        let n: usize = choices.values().sum();
        let total: u64 = time.values().sum();
        let mut groups: BTreeMap<&str, u64> = BTreeMap::new();
        for (k, v) in &time {
            *groups.entry(group(&kinds, k)).or_default() += v;
        }
        let cell = format!(
            "{} {} {} {}",
            room.name,
            mood.map_or("(drawn)".to_owned(), |m| format!("{m:?}")),
            chat_name(room),
            mode_name(graphics)
        );
        eprintln!(
            "\n== {cell} ({SEEDS} visits × {MINUTES} min, {:.1} s): {} choices ({:.0} a visit)",
            cpu.as_secs_f64(),
            n,
            n as f64 / SEEDS as f64
        );
        let mut by: Vec<_> = choices.into_iter().collect();
        by.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        eprintln!(
            "  chose: {}",
            by.iter()
                .map(|(k, v)| format!("{k} {:.1}%", pct(*v as u64, n as u64)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        eprintln!(
            "  groups: {}",
            groups
                .iter()
                .map(|(k, v)| format!("{k} {:.1}%", pct(*v, total)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut by: Vec<_> = time.into_iter().collect();
        by.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        eprintln!(
            "  time: {}",
            by.iter()
                .filter(|(_, v)| pct(*v, total) >= 0.5)
                .map(|(k, v)| format!("{k} {:.1}%", pct(*v, total)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        eprintln!(
            "  most of a visit one want took: {:.0}%; longest run of one want: {longest}",
            dominant * 100.0
        );
        eprintln!("  headings: {headings:?}");
        eprintln!("  beats: {beats:?}");
        eprintln!("  said: {said:?}");
        vignette_summary(&visits);
        home_summary(&visits);
        for (i, needs) in visits[0].needs.iter().enumerate() {
            eprintln!("  seed 0 at {:>2} min: {needs}", (i + 1) * 5);
        }
        table.push((cell, motion_summary(&visits, 4)));
    }
    motion_table("visit census (unfed)", &table);
}

/// The drawing modes a census runs in: ASCII, unless `CENSUS_MODES` says
/// "line" (line art) or "both".
fn census_modes() -> Vec<bool> {
    match std::env::var("CENSUS_MODES").as_deref() {
        Ok("both") => vec![false, true],
        Ok("line") => vec![true],
        _ => vec![false],
    }
}

/// The chat a census runs at: its rooms' cadence, unless `CENSUS_CHAT`
/// says "quiet" (none) or "both" (quiet first).
fn census_chats() -> Vec<bool> {
    match std::env::var("CENSUS_CHAT").as_deref() {
        Ok("both") => vec![true, false],
        Ok("quiet") => vec![true],
        _ => vec![false],
    }
}

/// `room` quiet, or with a chat line at its census cadence.
pub(super) fn with_chat(room: &Room, quiet: bool) -> Room {
    Room {
        real: room.real.clone(),
        view: room.view.clone(),
        chat_every: if quiet { None } else { room.chat_every },
        ..*room
    }
}

/// The census's name for `room`'s chat: "quiet", or its cadence.
pub(super) fn chat_name(room: &Room) -> String {
    room.chat_every.map_or("quiet".to_owned(), |every| {
        format!("chat/{}s", every / 1000)
    })
}

/// The census's name for a drawing mode.
fn mode_name(graphics: bool) -> &'static str {
    if graphics { "line art" } else { "ascii" }
}

/// Prints the cells of a census, a line each: moving in sight (from the
/// warm-up's end), its spread, and set-offs a minute in sight.
fn motion_table(name: &str, rows: &[(String, MotionRow)]) {
    let sd = |sd: Option<f64>| sd.map_or("-".to_owned(), |sd| format!("{sd:.2}"));
    eprintln!(
        "\n== {name}: moving % in sight from minute {} (σ of a set's mean; a visit's least–most) | set-offs a minute in sight (σ) | warm-up % and set-offs a minute",
        WARM_MS / 60_000
    );
    for (cell, row) in rows {
        eprintln!(
            "  {cell:<40} {:>5.1} (σ {} by {}×{}; {:.1}–{:.1}) | {:.2} (σ {}) | {:.1}, {:.2}",
            row.share,
            sd(row.share_sd),
            row.sets,
            row.set,
            row.least,
            row.most,
            row.rate,
            sd(row.rate_sd),
            row.warm_share,
            row.warm_rate
        );
    }
}

/// Her images over long visits: each room in each mood (forced), drawn
/// in line art at 10×20-pixel cells (a common Ghostty cell) with a frame
/// cache that never drops an image, measuring at 20, 40, 60 and 120
/// minutes the distinct images encoded so far (every one the terminal
/// has been sent), her working set (the smallest cache that would never
/// have dropped one she showed again), the images a cache of each size
/// in [`LIMITS`] would have encoded again, and what the images hold on
/// the client. It's how the frame cache's limit is sized. Ignored; run
/// by hand:
///
/// ```text
/// cargo test --release -p dessplay --lib image_census -- --ignored --nocapture
/// ```
#[test]
#[ignore = "the image census: run by hand in release with --nocapture"]
fn image_census() {
    const MARKS: [u64; 4] = [20, 40, 60, 120];
    /// Cache sizes weighed.
    const LIMITS: [usize; 4] = [256, 512, 1024, 2048];
    /// At a mark: distinct images, working set, cached bytes, and the
    /// images each of [`LIMITS`] would have encoded again.
    type Sample = (usize, usize, usize, [usize; LIMITS.len()]);
    let measure = |room: &Room, seed: u64, mood: Mood| -> Vec<Sample> {
        let guest = arrive_drawn(room, seed, Some(mood), |guest| {
            guest.set_picker(kitty_cells(10, 20));
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.set_limit(usize::MAX);
                graphics.measure();
            }
        });
        let sample = |guest: &Guest| {
            let graphics = guest.graphics.as_ref().expect("line art");
            let counts = graphics.counts();
            assert_eq!(counts.evicted, 0, "the cache never drops one here");
            let reuses = graphics.reuses();
            (
                counts.encoded,
                reuses.iter().max().map_or(0, |&d| d + 1),
                graphics.cached_bytes(),
                LIMITS.map(|limit| reuses.iter().filter(|&&d| d >= limit).count()),
            )
        };
        let mut marks = Vec::new();
        let last = MARKS[MARKS.len() - 1];
        let (_, guest) = visit_from(room, guest, last, |guest, now| {
            if let Some(&mark) = MARKS.get(marks.len())
                && mark < last
                && now >= mark * 60_000
            {
                marks.push(sample(guest));
            }
        });
        marks.push(sample(&guest));
        marks
    };
    let max = |v: &[usize]| v.iter().copied().max().unwrap_or(0);
    let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len().max(1) as f64;
    for room in [stage_room(), furnished_room(), resident_room(), live_room()] {
        let mut most: Vec<Sample> = vec![(0, 0, 0, [0; LIMITS.len()]); MARKS.len()];
        let mut per_image = (0usize, 0usize);
        for mood in Mood::ALL {
            let room = &room;
            let runs: Vec<Vec<Sample>> = std::thread::scope(|scope| {
                let runs: Vec<_> = (0..SEEDS)
                    .map(|seed| scope.spawn(move || measure(room, seed, mood)))
                    .collect();
                runs.into_iter()
                    .map(|run| run.join().expect("a visit"))
                    .collect()
            });
            eprintln!("\n== {} {mood:?} ({SEEDS} visits)", room.name);
            for (i, minutes) in MARKS.iter().enumerate() {
                let at: Vec<Sample> = runs.iter().map(|run| run[i]).collect();
                let distinct: Vec<usize> = at.iter().map(|s| s.0).collect();
                let working: Vec<usize> = at.iter().map(|s| s.1).collect();
                let bytes: usize = at.iter().map(|s| s.2).sum();
                let again: Vec<usize> = (0..LIMITS.len())
                    .map(|l| at.iter().map(|s| s.3[l]).max().unwrap_or(0))
                    .collect();
                let images: usize = distinct.iter().sum();
                eprintln!(
                    "  {minutes:>3} min: distinct mean {:.0} max {}; working set mean {:.0} max {}; encoded again at most {again:?} by {LIMITS:?}; {:.1} KB an image",
                    mean(&distinct),
                    max(&distinct),
                    mean(&working),
                    max(&working),
                    bytes as f64 / images.max(1) as f64 / 1024.0,
                );
                let most = &mut most[i];
                most.0 = most.0.max(max(&distinct));
                most.1 = most.1.max(max(&working));
                most.2 = most.2.max(at.iter().map(|s| s.2).max().unwrap_or(0));
                for (m, a) in most.3.iter_mut().zip(&again) {
                    *m = (*m).max(*a);
                }
                if i == MARKS.len() - 1 {
                    per_image.0 += bytes;
                    per_image.1 += images;
                }
            }
        }
        eprintln!(
            "\n== {}: {:.1} KB an image at 10×20-pixel cells",
            room.name,
            per_image.0 as f64 / per_image.1.max(1) as f64 / 1024.0
        );
        for (minutes, (distinct, working, bytes, again)) in MARKS.iter().zip(&most) {
            eprintln!(
                "  {minutes:>3} min: most distinct {distinct}, largest working set {working}, most held {:.1} MB, most encoded again {again:?} by {LIMITS:?}",
                *bytes as f64 / 1024.0 / 1024.0
            );
        }
    }
}

// ---- Her day ----

/// Monday 00:00, the day census's start: her second Monday (her clock
/// started at 16:00 on the first).
const MONDAY: GameTime = GameTime { day: 7, h: 0, m: 0 };

/// A game week, in real millis at her clock's speed.
const WEEK_MS: u64 = 7 * 24 * 60 * 60_000 / CLOCK_SPEED;

/// Her home for the day census: the census home's pieces, with her wall
/// clock and her window (so she can glance at the one and look out of
/// the other).
const DAY_HOME: [Furniture; 9] = [
    Furniture::Sofa,
    Furniture::Tv,
    Furniture::Bed,
    Furniture::Desk,
    Furniture::Bookshelf,
    Furniture::Fridge,
    Furniture::Lamp,
    Furniture::Clock,
    Furniture::Window,
];

/// The day census's rooms: the stage (nothing owned), her home with a
/// clock and a window, and the resident's (a sofa and a TV, her wall
/// clock not yet sent: it comes in the week), each from [`MONDAY`].
fn day_rooms() -> [Room; 3] {
    [
        Room {
            start: Some(MONDAY),
            ..stage_room()
        },
        Room {
            owns: &DAY_HOME,
            start: Some(MONDAY),
            ..furnished_room()
        },
        Room {
            start: Some(MONDAY),
            ..resident_room()
        },
    ]
}

/// Her in `room` from `seed` as its day census begins, the real date
/// `date`: a home she has visited once, her clock at `room.start`, her
/// pieces each where the visit census puts them (given on an unfed
/// arrival, then carried over), her wall clock sent if she owns one;
/// absent, and coming as the idle gate opens.
fn day_guest(room: &Room, seed: u64, date: Option<chrono::NaiveDate>) -> Guest {
    let start = room.start.expect("a day census room starts at a time");
    let unfed = Room {
        start: None,
        real: room.real.clone(),
        view: room.view.clone(),
        ..*room
    };
    let given = arrive_in(&unfed, seed, false, None);
    let State::Visiting(visit) = &given.state else {
        panic!(
            "{} seed {seed}: never arrived to be given her pieces",
            room.name
        );
    };
    for item in room.owns {
        assert!(
            visit.shown.iter().any(|s| s.item == *item && !s.boxed),
            "{} seed {seed}: {item:?} not shown: {:?}",
            room.name,
            visit.shown
        );
    }
    let mut ledger = Ledger::new_at(seed, start);
    ledger.home = given.ledger.home.clone();
    ledger.clock_sent = ledger.home.owns(Furniture::Clock);
    let mut guest = Guest::restore(ledger);
    guest.set_date(date);
    guest
}

/// Tuesday 13:00, the fed afternoon's start: school is done (Monday's
/// would be before her clock started), and the Afternoon slot runs to
/// 18:00, so fifteen real minutes (90 game minutes) keep her in it, and
/// in her day's mood (no wake between).
const AFTERNOON: GameTime = GameTime {
    day: 1,
    h: 13,
    m: 0,
};

/// A visit far ahead: the stage's TV is held back on order to it (her
/// first TV is ordered for her on her second visit otherwise; the
/// shopping channel's due is this plus [`SHOP_EVERY`], so no overflow).
const HELD_BACK: u64 = 1_000;

/// The fed afternoon's rooms (phase 5c, B4): the visit census's, from
/// [`AFTERNOON`], each as set (the stage with nothing, its TV held back).
fn afternoon_rooms() -> [Room; 3] {
    [stage_room(), furnished_room(), resident_room()].map(at_afternoon)
}

/// `room` fed from [`AFTERNOON`].
pub(super) fn at_afternoon(room: Room) -> Room {
    Room {
        start: Some(AFTERNOON),
        ..room
    }
}

/// Her in `room` from `seed`, drawn in line art if `graphics`, fed her
/// routine at `room.start` (an afternoon), arrived and visiting, in
/// `mood` (forced after her arrival's paint): the stillness band's setup
/// (phase 5c, B4). Her pieces stand where an unfed arrival in the same
/// mode puts them, each shown out of its box; her wall clock is already
/// sent (no parcel comes); a room with no TV holds its first back on
/// order ([`HELD_BACK`]); no shopping. The real date is none.
pub(super) fn fed_afternoon(room: &Room, seed: u64, graphics: bool, mood: Mood) -> Guest {
    let start = room.start.expect("a fed room starts at a time");
    let unfed = Room {
        start: None,
        real: room.real.clone(),
        view: room.view.clone(),
        ..*room
    };
    let given = arrive_in(&unfed, seed, graphics, None);
    let mut ledger = Ledger::new_at(seed, start);
    ledger.home = given.ledger.home.clone();
    ledger.clock_sent = true;
    if !ledger.home.owns(Furniture::Tv) {
        ledger.ordered = Some(Furniture::Tv);
        ledger.bought_on = HELD_BACK;
    }
    let mut guest = Guest::restore(ledger);
    if graphics {
        guest.set_picker(kitty());
    }
    let at = format!("{} seed {seed} graphics={graphics}", room.name);
    assert_eq!(guest.graphics.is_some(), graphics, "{at}: drawn so");
    guest.cue(Scene::Arrive);
    paint(&mut guest, &room.real, &room.view, 0);
    let State::Visiting(visit) = &mut guest.state else {
        panic!("{at}: not visiting after her arrival's paint");
    };
    visit.osaka.set_mood(mood);
    for item in room.owns {
        assert!(
            visit.shown.iter().any(|s| s.item == *item && !s.boxed),
            "{at}: {item:?} not shown: {:?}",
            visit.shown
        );
    }
    guest
}

/// A fed afternoon of `minutes` in `room` from `seed` ([`fed_afternoon`]),
/// checked as it goes: she stays visiting, the same visit (her decisions
/// only grow), in `mood`; and at its end her room is as it was set: the
/// same pieces, none boxed, nothing on order but what was held back.
/// (Where they stand may change: putting her home right is hers to do.)
pub(super) fn afternoon(
    room: &Room,
    seed: u64,
    graphics: bool,
    mood: Mood,
    minutes: u64,
) -> (Visit, Guest) {
    let guest = fed_afternoon(room, seed, graphics, mood);
    let pieces = |guest: &Guest| {
        let mut pieces: Vec<(Furniture, bool)> = guest
            .ledger
            .home
            .props
            .iter()
            .map(|p| (p.item, p.boxed))
            .collect();
        pieces.sort_by_key(|&(item, boxed)| (format!("{item:?}"), boxed));
        pieces
    };
    let (props, ordered) = (pieces(&guest), guest.ledger.ordered);
    let at = format!("{} {mood:?} seed {seed} graphics={graphics}", room.name);
    let mut decided = 0;
    let (visit, guest) = visit_from(room, guest, minutes, |guest, now| {
        let State::Visiting(visit) = &guest.state else {
            panic!("{at} at {now}: left her visit");
        };
        assert_eq!(visit.osaka.mood(), mood, "{at} at {now}: her mood");
        let decisions = visit.osaka.decisions.len();
        assert!(decisions >= decided, "{at} at {now}: a new visit");
        decided = decisions;
    });
    assert_eq!(pieces(&guest), props, "{at}: her pieces changed");
    assert_eq!(guest.ledger.ordered, ordered, "{at}: an order");
    (visit, guest)
}

/// Her moving on fed afternoons (phase 5c, D1 and B4/B5): each census
/// room fed at Tuesday 13:00 ([`fed_afternoon`]: as set, no shopping, her
/// mood forced and checked at every step), in each mood, quiet and at
/// the room's chat cadence, in ASCII and line art, for fifteen minutes
/// (`CENSUS_MINUTES`), at `CENSUS_SETS` (20) disjoint sets of
/// `CENSUS_SET_SEEDS` (4) seeds.
/// Prints each cell's moving in sight (from minute 3, the warm-up apart)
/// with the σ of a set's mean, by what for and how; set-offs a minute in
/// sight; what chat lines stopped her; still-but-busy time, exercise
/// begun and bubbles a minute; and what a sim-minute cost to run, then a
/// line a cell. It's the stillness band's own setup, read before the
/// band's thresholds are set. Ignored; run by hand:
///
/// ```text
/// cargo test --release -p dessplay --lib fed_afternoon_census -- --ignored --nocapture
/// ```
#[test]
#[ignore = "the fed afternoon census: run by hand in release with --nocapture"]
fn fed_afternoon_census() {
    let knob = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    // `CENSUS_MINUTES` sets the visit's length: 15 for baseline.md's
    // tables; the stillness band's `BASELINE` and `TUNED` rows are read
    // at the band's own length (`band::BAND_MINUTES`), since a shorter
    // visit reads busier (her arrival spills past the warm-up).
    let minutes = knob("CENSUS_MINUTES", 15);
    let (sets, set) = (knob("CENSUS_SETS", 20), knob("CENSUS_SET_SEEDS", 4));
    let modes = match std::env::var("CENSUS_MODES").as_deref() {
        Ok("ascii") => vec![false],
        Ok("line") => vec![true],
        _ => vec![false, true],
    };
    // Visits run at once: a hardware thread each, unless
    // `CENSUS_THREADS` says fewer. The cost column is each visit's own
    // wall time, so it's inflated under contention (about 2× at 32
    // threads here): the cost table is read with few at once (see
    // phase5c/baseline.md).
    let threads = knob(
        "CENSUS_THREADS",
        std::thread::available_parallelism().map_or(1, |n| n.get() as u64),
    )
    .max(1) as usize;
    let mut table = Vec::new();
    for room in afternoon_rooms() {
        for mood in Mood::ALL {
            for quiet in [true, false] {
                for &graphics in &modes {
                    let room = &with_chat(&room, quiet);
                    let started = std::time::Instant::now();
                    let seeds: Vec<u64> = (0..sets * set).collect();
                    let runs: Vec<(Visit, f64)> = seeds
                        .chunks(threads)
                        .flat_map(|chunk| {
                            std::thread::scope(|scope| {
                                let runs: Vec<_> = chunk
                                    .iter()
                                    .map(|&seed| {
                                        scope.spawn(move || {
                                            let started = std::time::Instant::now();
                                            let (visit, _) =
                                                afternoon(room, seed, graphics, mood, minutes);
                                            (visit, started.elapsed().as_secs_f64())
                                        })
                                    })
                                    .collect();
                                runs.into_iter()
                                    .map(|run| run.join().expect("an afternoon"))
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect();
                    let cpu: f64 = runs.iter().map(|(_, s)| s).sum();
                    let visits: Vec<Visit> = runs.into_iter().map(|(v, _)| v).collect();
                    let cell = format!(
                        "{} {mood:?} {} {}",
                        room.name,
                        chat_name(room),
                        mode_name(graphics)
                    );
                    eprintln!(
                        "\n== fed afternoon: {cell} ({} visits × {minutes} min; {:.1} s; {:.1} ms a sim-minute)",
                        visits.len(),
                        started.elapsed().as_secs_f64(),
                        1000.0 * cpu / (visits.len() as u64 * minutes) as f64
                    );
                    table.push((cell, motion_summary(&visits, set as usize)));
                }
            }
        }
    }
    motion_table("fed afternoon", &table);
}

/// Where her time goes, as the day census counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Where {
    /// Visiting, awake and in sight.
    Present,
    /// On a dash home from school, in sight.
    Dash,
    /// Asleep for the night.
    Asleep,
    /// Visiting but out of sight (at work, through a door).
    Hidden,
    /// Out by her routine (at school): her empty home shown, or nothing.
    Away,
    /// Not here otherwise (coming, going, waiting for the idle gate).
    Gone,
}

impl Where {
    const ALL: [Where; 6] = [
        Where::Present,
        Where::Dash,
        Where::Asleep,
        Where::Hidden,
        Where::Away,
        Where::Gone,
    ];

    /// Its column heading.
    fn label(self) -> &'static str {
        match self {
            Where::Present => "here",
            Where::Dash => "dash",
            Where::Asleep => "asleep",
            Where::Hidden => "hidden",
            Where::Away => "away",
            Where::Gone => "gone",
        }
    }

    /// Where `guest` is at `now`.
    fn of(guest: &Guest, now: u64) -> Where {
        match &guest.state {
            State::Visiting(visit) if visit.osaka.sleeping() => Where::Asleep,
            State::Visiting(visit) if visit.osaka.hidden(now) => Where::Hidden,
            State::Visiting(visit) if visit.kind == Kind::Dash => Where::Dash,
            State::Visiting(_) | State::Leaving(_) => Where::Present,
            State::Away(_) => Where::Away,
            State::Absent | State::Arriving(_) if guest.out.is_some() => Where::Away,
            State::Absent | State::Arriving(_) => Where::Gone,
        }
    }
}

/// A stretch of her day (an hour of it, or a slot), over the census's
/// runs.
#[derive(Clone, Default)]
struct Stretch {
    /// Real millis by where she was.
    at: BTreeMap<Where, u64>,
    /// Real millis awake and in sight, by census group.
    groups: BTreeMap<&'static str, u64>,
    /// What happened, by name: comings and goings, vignettes, the
    /// calendar, rare things first seen.
    events: BTreeMap<String, usize>,
    /// Real millis awake and in sight moving, by what for
    /// ([`Osaka::census_purpose`]).
    moving: BTreeMap<&'static str, u64>,
}

impl Stretch {
    /// The share of its time she was `at`, as a percentage.
    fn share(&self, at: Where) -> f64 {
        pct(
            self.at.get(&at).copied().unwrap_or(0),
            self.at.values().sum(),
        )
    }

    /// Its line: where she was (%), the groups awake and in sight (% of
    /// that time), and what happened (counts over all runs).
    fn line(&self) -> String {
        let at: Vec<String> = Where::ALL
            .iter()
            .map(|&w| format!("{:>5.1}", self.share(w)))
            .collect();
        let awake: u64 = self.groups.values().sum();
        let mut groups: Vec<(&&str, &u64)> = self.groups.iter().collect();
        groups.sort_by_key(|(_, ms)| std::cmp::Reverse(**ms));
        let groups: Vec<String> = groups
            .iter()
            .take(4)
            .map(|(g, ms)| format!("{g} {:.0}", pct(**ms, awake)))
            .collect();
        let events: Vec<String> = self
            .events
            .iter()
            .map(|(e, n)| format!("{e} {n}"))
            .collect();
        format!(
            "{} | {} | {}",
            at.join(" "),
            groups.join(", "),
            events.join(", ")
        )
    }

    fn add(&mut self, other: &Stretch) {
        for (k, v) in &other.at {
            *self.at.entry(*k).or_default() += v;
        }
        for (k, v) in &other.groups {
            *self.groups.entry(k).or_default() += v;
        }
        for (k, v) in &other.events {
            *self.events.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.moving {
            *self.moving.entry(k).or_default() += v;
        }
    }

    /// Its moving by what for, as % of its time awake and in sight.
    fn purposes(&self) -> String {
        let awake: u64 = self.groups.values().sum();
        let mut rows: Vec<(&&str, &u64)> = self.moving.iter().collect();
        rows.sort_by_key(|(_, ms)| std::cmp::Reverse(**ms));
        rows.iter()
            .map(|(k, ms)| format!("{k} {:.1}", pct(**ms, awake)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Her week in one mood (her day's), here, awake and in sight on a visit
/// that isn't a dash home: the band's quantity, fed.
#[derive(Clone, Default)]
struct MoodWeek {
    /// Real millis here, awake and in sight.
    here: u64,
    /// Real millis of that moving, by what for.
    moving: BTreeMap<&'static str, u64>,
    /// Her set-offs (awake, not on a dash).
    set_offs: usize,
}

impl MoodWeek {
    fn add(&mut self, other: &MoodWeek) {
        self.here += other.here;
        for (k, v) in &other.moving {
            *self.moving.entry(k).or_default() += v;
        }
        self.set_offs += other.set_offs;
    }
}

/// What one census week came to.
#[derive(Default)]
struct Week {
    /// By hour of the day, school days then days off.
    hours: [[Stretch; 24]; 2],
    /// By slot, school days then days off.
    slots: [BTreeMap<String, Stretch>; 2],
    /// Each game day's rare draw, as her first visit that day had it.
    rares: Vec<String>,
    /// What happened once, when (game day and time): the calendar played,
    /// a rare thing first seen, her wall clock delivered.
    once: Vec<String>,
    /// By her mood that day.
    moods: BTreeMap<String, MoodWeek>,
}

/// How `guest` is, for her comings and goings: her state, how she's
/// coming, and what kind of visit it is.
fn phase(guest: &Guest) -> String {
    match &guest.state {
        State::Absent => "absent".to_owned(),
        State::Arriving(how) => format!("arriving {how:?}"),
        State::Visiting(visit) => format!("visiting {:?}", visit.kind),
        State::Leaving(_) => "leaving".to_owned(),
        State::Away(_) => "away".to_owned(),
    }
}

/// The coming or going between `was` and `now` ([`phase`]s), if one.
fn moved(was: &str, now: &str, out: bool) -> Option<&'static str> {
    if was == now {
        return None;
    }
    if now.starts_with("arriving Idle") {
        Some("arrive")
    } else if now.starts_with("arriving Return") {
        Some("return")
    } else if now == "visiting Dash" {
        Some("dash in")
    } else if now == "leaving" {
        Some("goodbye")
    } else if was == "visiting Normal" && !now.starts_with("visiting") && out {
        Some("out to school")
    } else {
        None
    }
}

/// "Tue 08:15" for the game time `day`.
fn when(day: &routine::DayTime) -> String {
    format!(
        "{:?} {:02}:{:02}",
        day.weekday,
        day.minute / 60,
        day.minute % 60
    )
}

/// A game week of her in `room` from `seed`, the real date `date`, as the
/// shell would run it: a tick when she asks (at least every second), the
/// date set before it, a paint when it says the screen changed, a chat
/// line as the room has them, nobody at the keys.
fn live_week(room: &Room, seed: u64, date: Option<chrono::NaiveDate>) -> Week {
    let mut guest = day_guest(room, seed, date);
    let (real, mut view) = (room.real.clone(), room.view.clone());
    // Her idle gate as the tests have it: a census of her day, not of
    // the client's idle delay.
    view.delay = Some(DELAY);
    let mut week = Week::default();
    let mut now = 0;
    paint(&mut guest, &real, &view, now);
    let mut was = phase(&guest);
    let mut playing: Option<u64> = None;
    let (mut seen, mut calendar, mut clock_sent) = (
        guest.ledger.seen.len(),
        guest.ledger.calendar_on,
        guest.ledger.clock_sent,
    );
    let mut drawn: Option<u64> = None;
    let mut set_offs = 0;
    while now < WEEK_MS {
        let day = guest.day(now).expect("fed, and met");
        let off = usize::from(!day.school_day);
        let hour = usize::from(day.minute / 60);
        let slot = format!("{:?}", day.slot);
        let step = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        let at = Where::of(&guest, now);
        let mut spent = Stretch::default();
        spent.at.insert(at, step);
        if let (Where::Present | Where::Dash, State::Visiting(visit)) = (at, &guest.state) {
            spent.groups.insert(visit.osaka.census_group(), step);
            let motion = visit.osaka.census_motion(now);
            if let Some(motion) = motion {
                spent.moving.insert(motion.purpose, step);
            }
            if at == Where::Present {
                let mood = week
                    .moods
                    .entry(format!("{:?}", visit.osaka.mood()))
                    .or_default();
                mood.here += step;
                if let Some(motion) = motion {
                    *mood.moving.entry(motion.purpose).or_default() += step;
                }
            }
        }
        // Her set-offs, by her mood, awake and not on a dash (counted
        // as her log gains them; a new visit's log starts afresh).
        match &guest.state {
            State::Visiting(visit) => {
                let log = &visit.osaka.set_off_log;
                if visit.kind != Kind::Dash && !visit.osaka.sleeping() {
                    week.moods
                        .entry(format!("{:?}", visit.osaka.mood()))
                        .or_default()
                        .set_offs += log.len().saturating_sub(set_offs);
                }
                set_offs = log.len();
            }
            _ => set_offs = 0,
        }
        week.hours[off][hour].add(&spent);
        week.slots[off].entry(slot.clone()).or_default().add(&spent);
        now += step;
        if room
            .chat_every
            .is_some_and(|every| now / every != (now - step) / every)
        {
            // Every other line asks her something.
            view.chat_mark.synced += 1;
            view.chat_mark.synced_asks = view.chat_mark.synced.is_multiple_of(2);
        }
        guest.set_date(date);
        let changed = guest.advance(now);
        // Her comings and goings, as the tick left her and as the paint
        // did (an arrival lasts only until the paint).
        let mut events: Vec<String> = Vec::new();
        let mut note = |guest: &Guest, was: &mut String| {
            let is = phase(guest);
            if let Some(event) = moved(was, &is, guest.out.is_some()) {
                events.push(event.to_owned());
            }
            *was = is;
        };
        note(&guest, &mut was);
        if changed {
            paint(&mut guest, &real, &view, now);
        }
        note(&guest, &mut was);
        if let State::Visiting(visit) = &guest.state {
            let plays = visit.osaka.plays_since();
            if let Some((since, play)) = plays
                && playing != Some(since)
            {
                let lamp = visit
                    .shown
                    .iter()
                    .any(|s| s.item == Furniture::Lamp && !s.boxed);
                for (_, name) in named(play, lamp) {
                    events.push(if play.own == ScriptId::ClockGlance {
                        match play.branch {
                            0 => "clock glance, bed".to_owned(),
                            1 => "clock glance, school".to_owned(),
                            _ => "clock glance, hour".to_owned(),
                        }
                    } else {
                        name
                    });
                }
            }
            playing = plays.map(|(since, _)| since);
            if let Some((day, rares)) = visit.osaka.rares_today()
                && drawn != Some(day)
            {
                drawn = Some(day);
                week.rares.push(format!(
                    "day {day}: open {:?}, new {:?}",
                    rares.open(),
                    rares.new_one()
                ));
            }
        } else {
            playing = None;
        }
        let day = guest.day(now).expect("fed, and met");
        let mut once = |what: String| week.once.push(format!("{} {what}", when(&day)));
        if guest.ledger.seen.len() > seen {
            for key in &guest.ledger.seen[seen..] {
                events.push("first seen".to_owned());
                once(format!("first seen: {key}"));
            }
            seen = guest.ledger.seen.len();
        }
        if guest.ledger.calendar_on != calendar {
            calendar = guest.ledger.calendar_on;
            events.push("calendar".to_owned());
            once(format!("calendar played: {calendar:?}"));
        }
        if guest.ledger.clock_sent != clock_sent {
            clock_sent = guest.ledger.clock_sent;
            events.push("wall clock delivered".to_owned());
            once("her wall clock delivered".to_owned());
        }
        let off = usize::from(!day.school_day);
        let hour = usize::from(day.minute / 60);
        let slot = format!("{:?}", day.slot);
        for event in events {
            *week.hours[off][hour]
                .events
                .entry(event.clone())
                .or_default() += 1;
            *week.slots[off]
                .entry(slot.clone())
                .or_default()
                .events
                .entry(event)
                .or_default() += 1;
        }
    }
    week
}

/// Her days: a game week from Monday 00:00 (her routine fed), in each
/// room at a few seeds and on two real dates (Oct 31, which owes her
/// calendar's greeting, and none, which owes nothing and has no
/// vacation), as the shell runs her with nobody at the keys. Reports, by
/// the hour of her day, school days and days off apart: where her time
/// went (here, on a dash home, asleep, out of sight in a visit, away at
/// school, or gone), her census groups while here and awake, and what
/// happened (arrivals, going out to school, coming home, dash-ins,
/// goodbyes, her vignettes, clock glances by kind, looking out of her
/// window, the calendar, rare things first seen); then the same by slot,
/// and each run's rare draws and one-off events. About a minute in
/// release. Ignored; run by hand:
///
/// ```text
/// cargo test --release -p dessplay --lib day_census -- --ignored --nocapture
/// ```
#[test]
#[ignore = "the day census: run by hand in release with --nocapture"]
fn day_census() {
    const DAY_SEEDS: [u64; 3] = [0, 1, 2];
    let dates = [date(2026, 10, 31), None];
    for room in day_rooms() {
        let started = std::time::Instant::now();
        let room = &room;
        let runs: Vec<((u64, Option<chrono::NaiveDate>), Week)> = std::thread::scope(|scope| {
            let runs: Vec<_> = DAY_SEEDS
                .iter()
                .flat_map(|&seed| dates.iter().map(move |&date| (seed, date)))
                .map(|(seed, date)| {
                    (
                        (seed, date),
                        scope.spawn(move || live_week(room, seed, date)),
                    )
                })
                .collect();
            runs.into_iter()
                .map(|(at, run)| (at, run.join().expect("a week")))
                .collect()
        });
        let mut hours: [[Stretch; 24]; 2] = Default::default();
        let mut slots: [BTreeMap<String, Stretch>; 2] = Default::default();
        let mut whole = Stretch::default();
        for (_, week) in &runs {
            for (kind, row) in week.hours.iter().enumerate() {
                for (hour, stretch) in row.iter().enumerate() {
                    hours[kind][hour].add(stretch);
                    whole.add(stretch);
                }
            }
            for (kind, row) in week.slots.iter().enumerate() {
                for (slot, stretch) in row {
                    slots[kind].entry(slot.clone()).or_default().add(stretch);
                }
            }
        }
        eprintln!(
            "\n== day census: {} ({} runs: seeds {DAY_SEEDS:?} × dates {dates:?}; a game week from Monday 00:00 each; {:.0} s)",
            room.name,
            runs.len(),
            started.elapsed().as_secs_f64()
        );
        let heading = Where::ALL
            .iter()
            .map(|w| format!("{:>5}", w.label()))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("  the week: {heading} (%)\n            {}", whole.line());
        for (kind, name) in [(0, "school days"), (1, "days off")] {
            eprintln!(
                "  {name}, by game hour: {heading} (%) | groups here and awake (%) | events (all runs)"
            );
            for (hour, stretch) in hours[kind].iter().enumerate() {
                if stretch.at.is_empty() {
                    continue;
                }
                eprintln!("    {hour:02}  {}", stretch.line());
            }
        }
        for (kind, name) in [(0, "school days"), (1, "days off")] {
            eprintln!("  {name}, by slot:");
            for slot in Slot::ALL {
                if let Some(stretch) = slots[kind].get(&format!("{slot:?}")) {
                    eprintln!("    {:<9} {}", format!("{slot:?}"), stretch.line());
                    eprintln!("              moving by what for: {}", stretch.purposes());
                }
            }
        }
        // The band's quantity, fed: by her day's mood, here, awake and in
        // sight, not on a dash home (the Asleep and Away slots' lines
        // are a dash's or her way to bed or out, and no band reading).
        let mut moods: BTreeMap<String, MoodWeek> = BTreeMap::new();
        for (_, week) in &runs {
            for (mood, row) in &week.moods {
                moods.entry(mood.clone()).or_default().add(row);
            }
        }
        eprintln!(
            "  the week by her mood (here, awake, in sight, not on a dash): mood | minutes | moving % | set-offs a minute | moving by what for (%)"
        );
        for (mood, row) in &moods {
            let minutes = row.here as f64 / 60_000.0;
            let mut purposes: Vec<(&&str, &u64)> = row.moving.iter().collect();
            purposes.sort_by_key(|(_, ms)| std::cmp::Reverse(**ms));
            eprintln!(
                "    {mood:<11} {minutes:>6.0} | {:>5.1} | {:.2} | {}",
                pct(row.moving.values().sum(), row.here),
                row.set_offs as f64 / minutes.max(1e-9),
                purposes
                    .iter()
                    .map(|(k, ms)| format!("{k} {:.1}", pct(**ms, row.here)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        for ((seed, date), week) in &runs {
            eprintln!("  seed {seed}, date {date:?}:");
            eprintln!("    rares: {}", week.rares.join("; "));
            eprintln!("    once: {}", week.once.join("; "));
        }
    }
}

/// The fed afternoon's tallies add up, in both drawing modes, on the
/// stage (doors, text, chat) and in the home (a TV, so a wall clock would
/// come if it weren't already sent): her time is all in sight (the
/// warm-up apart), out of sight or asleep, step by step; out of sight shows on the
/// stage (over two seeds: whether one seed's four minutes take her out of
/// sight is its luck; seed 0's line art stopped doing so with the 5 s
/// watch); the warm-up's set-offs are her log's before minute 3 (her
/// arrival's among them) and the rest are the table's; the chat lines
/// that stopped her after the warm-up are the table's; moving is part of
/// her time in sight. And [`afternoon`]'s own checks hold: the forced
/// mood, the one visit, her room as set (no parcel, no order).
#[test]
fn the_fed_afternoon_tallies_add_up() {
    const MINUTES: u64 = 4;
    for graphics in [false, true] {
        let mut hidden = 0;
        for room in [stage_room(), furnished_room()] {
            let room = Room {
                start: Some(AFTERNOON),
                ..room
            };
            for seed in 0..2 {
                let at = format!("{} seed {seed} graphics={graphics}", room.name);
                let (visit, guest) = afternoon(&room, seed, graphics, Mood::Ordinary, MINUTES);
                let State::Visiting(her) = &guest.state else {
                    panic!("{at}: visiting");
                };
                let log = &her.osaka.set_off_log;
                // The run's steps, the last overrunning its end by under
                // a second.
                let all = visit.sight[0] + visit.sight[1] + visit.hidden + visit.asleep;
                assert!(
                    (MINUTES * 60_000..MINUTES * 60_000 + 1000).contains(&all),
                    "{at}: her time {all}"
                );
                // The warm-up's steps, by when each began.
                assert!(
                    visit.sight[0] < WARM_MS + 1000 && visit.sight[0] + visit.hidden >= WARM_MS,
                    "{at}: the warm-up's time in sight {}",
                    visit.sight[0]
                );
                assert!(visit.moving() <= visit.sight[1], "{at}: moving");
                let warm = log.iter().filter(|s| s.at < WARM_MS).count();
                assert!(warm >= 1, "{at}: her arrival is a set-off: {log:?}");
                assert_eq!(visit.warm_set_offs, warm, "{at}: the warm-up's set-offs");
                assert_eq!(
                    visit.set_off_count(),
                    log.len() - warm,
                    "{at}: the set-offs after it"
                );
                assert_eq!(
                    visit.chat_cuts.values().sum::<usize>(),
                    her.osaka
                        .chat_cuts
                        .iter()
                        .filter(|&&(at, ..)| at >= WARM_MS)
                        .count(),
                    "{at}: the chat lines that stopped her"
                );
                if room.name == "stage" {
                    hidden += visit.hidden;
                }
            }
        }
        assert!(hidden > 0, "graphics={graphics}: never out of sight");
    }
}

/// A trip she sets off on runs its course when nothing interrupts her:
/// each hop she lands from carries on without choosing anew, so in a
/// quiet home she never lets one go for something else.
#[test]
fn an_uninterrupted_trip_runs_its_course() {
    let room = Room {
        chat_every: None,
        ..furnished_room()
    };
    let mut set_off = 0;
    for seed in 0..6 {
        let visit = simulate(&room, seed, 10, None);
        set_off += visit.headings.get("set off").copied().unwrap_or(0);
        assert_eq!(
            visit.headings.get("let go: Other"),
            None,
            "seed {seed}: {:?}",
            visit.headings
        );
    }
    assert!(set_off >= 8, "only {set_off} trips");
}

/// The census's home shows every piece she owns as each of its visits
/// begins, in both drawing modes (one in the closet would read as never
/// used: no snacks without the fridge, no bedtime lamp without the
/// lamp).
#[test]
fn the_census_home_shows_every_piece() {
    let room = furnished_room();
    for graphics in [false, true] {
        for seed in 0..SEEDS {
            let guest = arrive_in(&room, seed, graphics, None);
            let State::Visiting(visit) = &guest.state else {
                panic!("seed {seed} graphics={graphics}: visiting");
            };
            for item in room.owns {
                assert!(
                    visit.shown.iter().any(|s| s.item == *item && !s.boxed),
                    "seed {seed} graphics={graphics}: {item:?} not shown: {:?}",
                    visit.shown
                );
            }
        }
    }
}

/// Milliseconds of `visits` in each census group.
fn grouped(visits: &[Visit]) -> BTreeMap<&'static str, u64> {
    let mut groups = BTreeMap::new();
    for visit in visits {
        for (k, v) in &visit.time {
            *groups.entry(group(&visit.groups, k)).or_default() += v;
        }
    }
    groups
}

/// At home she rests on her furniture, not the floor: far more of her
/// time is on her things than sitting or lying on the floor.
#[test]
fn at_home_her_furniture_beats_the_floor() {
    let room = furnished_room();
    let visits: Vec<Visit> = (0..4)
        .map(|seed| simulate(&room, seed, 10, Some(Mood::Ordinary)))
        .collect();
    let groups = grouped(&visits);
    let at = |g: &str| groups.get(g).copied().unwrap_or(0);
    assert!(at("furniture") > 10 * at("floor rest").max(1), "{groups:?}");
}

/// Her mood shows: on a lazy visit she spends more of her time on her
/// furniture and less of it exercising than on an industrious one, and
/// a dreamy one spaces out more than an ordinary one. (Time spent moving
/// about is no measure: in this room it's within a few percent either
/// way, seed set to seed set.)
#[test]
fn her_mood_shows() {
    let room = resident_room();
    let in_mood = |mood: Mood| {
        let visits: Vec<Visit> = (0..4)
            .map(|seed| simulate(&room, seed, 15, Some(mood)))
            .collect();
        grouped(&visits)
    };
    let (lazy, busy) = (in_mood(Mood::Lazy), in_mood(Mood::Industrious));
    let at = |g: &BTreeMap<&str, u64>, k: &str| g.get(k).copied().unwrap_or(0);
    assert!(
        at(&lazy, "furniture") > at(&busy, "furniture"),
        "lazy {lazy:?}, industrious {busy:?}"
    );
    assert!(
        at(&lazy, "exercise") < at(&busy, "exercise"),
        "lazy {lazy:?}, industrious {busy:?}"
    );
    let (dreamy, ordinary) = (in_mood(Mood::Dreamy), in_mood(Mood::Ordinary));
    assert!(
        at(&dreamy, "spacing out") > at(&ordinary, "spacing out"),
        "dreamy {dreamy:?}, ordinary {ordinary:?}"
    );
}

/// Every pooled line she shows is one she drew from its pool: the
/// [`Lines`](crate::ui::houseguest::mind::Lines) store has it, from that
/// pool, said no longer ago than it shows (spoken, or a riddle's key), so
/// its cooldown counts it and none is said around the store (as the door
/// once said "Where was I?"). And every line she speaks from a pool shows
/// (none is drawn, then spoken over before it could show, cooling for
/// nothing). Two seeds in each of the three rooms, twelve minutes each;
/// musings are rare there, so then a dozen cued on the stage, where some
/// are riddles: every question shown is answered after it, and then she's
/// pleased with it. All in ASCII and in line art.
#[test]
fn every_pooled_line_shown_was_drawn_from_its_pool() {
    use crate::ui::houseguest::mind::{PoolId, RIDDLES};
    let pooled = mind::all_lines();
    // How long a pooled line shows: a riddle's half for its key, any
    // other as it's said aloud.
    let shows = |pool: PoolId, line: &str| {
        if pool != PoolId::Riddle {
            osaka::speech_ms(line)
        } else if mind::riddle_of(line).is_some() {
            script::RIDDLE_ASKED_MS
        } else {
            script::RIDDLE_ANSWERED_MS - script::RIDDLE_ASKED_MS
        }
    };
    /// One run's watch: what showed when, and what she had said by the
    /// last step watched.
    #[derive(Default)]
    struct Watched {
        shown: Vec<(Option<osaka::Bubble>, u64)>,
        said: Vec<(PoolId, &'static str, u64)>,
        last: u64,
    }
    let watch = |seen: &mut Watched, osaka: &Osaka, now: u64| {
        seen.shown.push((osaka.appearance(now).2, now));
        seen.said = osaka.said_lines().to_vec();
        seen.last = now;
    };
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut check = |seen: &Watched, run: &str, cued: bool| {
        for &(bubble, now) in &seen.shown {
            let Some(osaka::Bubble::Say(text)) = bubble else {
                continue;
            };
            let Some(&(pool, _)) = pooled.iter().find(|&&(_, line)| line == text) else {
                continue;
            };
            let kind = match pool {
                PoolId::Riddle if mind::riddle_of(text).is_some() => "question",
                PoolId::Riddle => "answer",
                PoolId::Door => "door",
                PoolId::Musing => "musing",
                _ => "other",
            };
            *kinds.entry(kind).or_default() += 1;
            assert!(
                seen.said.iter().any(|&(said, line, when)| said == pool
                    && line == text
                    && (when..when + shows(pool, line)).contains(&now)),
                "{run} at {now}: {text:?} ({pool:?}) shown, never drawn: {:?}",
                seen.said
            );
        }
        // Every line spoken from a pool showed (a riddle's halves are
        // keys, checked below).
        for &(pool, line, when) in &seen.said {
            if pool == PoolId::Riddle || when > seen.last {
                continue;
            }
            assert!(
                seen.shown
                    .iter()
                    .any(|&(bubble, now)| bubble == Some(osaka::Bubble::Say(line))
                        && (when..when + shows(pool, line)).contains(&now)),
                "{run}: {line:?} ({pool:?}) said at {when}, never shown"
            );
        }
        // A riddle's answer follows its own question; cued (nothing to
        // interrupt her), every riddle asked is answered, then she's
        // pleased with it, and none is asked over another.
        let mut pending: Option<(usize, bool)> = None;
        for &(bubble, now) in &seen.shown {
            match bubble {
                Some(osaka::Bubble::Say(text)) => {
                    if let Some(i) = mind::riddle_of(text) {
                        assert!(
                            !cued || pending.is_none_or(|p| p == (i, false)),
                            "{run} at {now}: {text:?} asked over {pending:?}"
                        );
                        pending = Some((i, false));
                    } else if let Some(i) = RIDDLES.iter().position(|&(_, a)| a == text) {
                        assert!(
                            matches!(pending, Some((p, _)) if p == i),
                            "{run} at {now}: {text:?} answers {pending:?}"
                        );
                        pending = Some((i, true));
                    }
                }
                Some(osaka::Bubble::Hehe) if matches!(pending, Some((_, true))) => {
                    *kinds.entry("hehe").or_default() += 1;
                    pending = None;
                }
                _ => {}
            }
        }
        assert!(
            !cued || pending.is_none(),
            "{run}: a riddle left unfinished: {pending:?}"
        );
    };
    for graphics in [false, true] {
        for room in [stage_room(), furnished_room(), resident_room()] {
            for seed in [3, 11] {
                let run = format!("{}/{seed}/{graphics}", room.name);
                let mut seen = Watched::default();
                simulate_with(&room, seed, 12, None, graphics, |osaka, now| {
                    watch(&mut seen, osaka, now)
                });
                check(&seen, &run, false);
            }
        }
        let room = stage_room();
        for seed in 0..12 {
            let mut guest = Guest::new(seed);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::Muse);
            let mut now = 0;
            paint(&mut guest, &room.real, &room.view, now);
            let mut seen = Watched::default();
            while now < osaka::SPACE_OUT_MS.0 {
                if let State::Visiting(visit) = &guest.state {
                    watch(&mut seen, &visit.osaka, now);
                }
                now += guest
                    .next_tick(now)
                    .map_or(100, |d| d.as_millis() as u64)
                    .clamp(1, 100);
                if guest.advance(now) {
                    paint(&mut guest, &room.real, &room.view, now);
                }
            }
            check(&seen, &format!("muse/{seed}/{graphics}"), true);
        }
    }
    for kind in ["door", "musing", "question", "answer", "hehe"] {
        assert!(kinds.contains_key(kind), "no {kind} shown: {kinds:?}");
    }
}

/// The census counts her vignettes as she plays them: on quiet visits
/// home (no chat, so nothing cuts a coda short or is answered), in both
/// drawing modes, the shopping channel plays on each (it's on at her
/// first watch), bedtime with her lamp on some, and some uses are
/// spliced, the chopsticks among them; and on a visit cued one, a sata
/// andagi (rare enough to play on none of the others). Each splice's
/// count agrees with what she was heard to say: "Sata andagi." as many
/// times as the andagis' branches name it, "Hold 'em by the ends!" once
/// a bad split; a riddle's question and answer are both pooled lines;
/// and with no question asked, nothing is answered.
#[test]
fn the_census_counts_her_vignettes() {
    use script::{HOLD_EM, SATA_ANDAGI};
    let room = Room {
        chat_every: None,
        ..furnished_room()
    };
    for graphics in [false, true] {
        let mut scripts: BTreeMap<String, usize> = BTreeMap::new();
        let mut splices: BTreeMap<String, usize> = BTreeMap::new();
        let visits = (0..3)
            .map(|seed| (seed, None))
            .chain([(3, Some(Scene::Andagi))]);
        for (seed, cue) in visits {
            let at = format!("seed {seed} {cue:?} graphics={graphics}");
            let mut guest = arrive_in(&room, seed, graphics, None);
            if let Some(scene) = cue {
                guest.cue(scene);
            }
            let (visit, _) = visit_from(&room, guest, 15, |_, _| {});
            let get = |row: &BTreeMap<String, usize>, k: &str| row.get(k).copied().unwrap_or(0);
            assert_eq!(
                get(&visit.scripts, "shopping"),
                1,
                "{at}: {:?}",
                visit.scripts
            );
            let named: usize = ANDAGI_COUNTS
                .iter()
                .map(|&n| n as usize * get(&visit.splices, &format!("sata andagi ×{n}")))
                .sum();
            assert_eq!(
                get(&visit.said, SATA_ANDAGI),
                named,
                "{at}: {:?}",
                visit.splices
            );
            assert_eq!(
                get(&visit.said, HOLD_EM),
                get(&visit.splices, "chopsticks, bad"),
                "{at}: {:?}",
                visit.splices
            );
            assert_eq!(
                get(&visit.pooled, "Riddle"),
                2 * get(&visit.scripts, "riddle"),
                "{at}: {:?} {:?}",
                visit.pooled,
                visit.scripts
            );
            assert_eq!(visit.answers, 0, "{at}");
            if cue.is_some() {
                assert!(
                    visit
                        .splices
                        .iter()
                        .any(|(k, &v)| k.starts_with("sata andagi") && v > 0),
                    "{at}: {:?}",
                    visit.splices
                );
            }
            for (k, v) in visit.scripts {
                *scripts.entry(k).or_default() += v;
            }
            for (k, v) in visit.splices {
                *splices.entry(k).or_default() += v;
            }
        }
        let had = |row: &BTreeMap<String, usize>, start: &str| {
            row.iter().any(|(k, &v)| k.starts_with(start) && v > 0)
        };
        assert!(had(&scripts, "bedtime"), "graphics={graphics}: {scripts:?}");
        assert!(
            had(&splices, "sata andagi"),
            "graphics={graphics}: {splices:?}"
        );
        assert!(
            had(&splices, "chopsticks"),
            "graphics={graphics}: {splices:?}"
        );
    }
}
