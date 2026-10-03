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

use super::*;
use crate::ui::houseguest::brain::{Mood, Need, Want};
use crate::ui::houseguest::mind::Loss;
use std::collections::BTreeMap;

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
    }
}

/// Her home: a living room and a bedroom, furnished.
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
        ],
        chat_every: Some(90_000),
    }
}

/// A resident beside a chat with text in reach, a sofa and TV owned.
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
    }
}

/// What one visit came to.
#[derive(Default)]
pub(super) struct Visit {
    /// Each choice she rolled, in order.
    pub choices: Vec<Want>,
    /// Milliseconds in each kind of act ("use:Sleep", "idle:LieBack"...).
    pub time: BTreeMap<String, u64>,
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
    let mut guest = Guest::new(seed);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrive);
    let mut view = room.view.clone();
    paint(&mut guest, &room.real, &view, 0);
    if let (Some(mood), State::Visiting(visit)) = (mood, &mut guest.state) {
        visit.osaka.set_mood(mood);
    }
    for &item in room.owns {
        guest.give(item);
        paint(&mut guest, &room.real, &view, 0);
    }
    // The shopping channel is on at her first watch, as on any visit
    // it's due: what she buys, and when, shows whether decor comes
    // before the furniture she lacks.
    guest.shop();
    let mut out = Visit::default();
    let mut now = 0;
    let mut said: Option<&'static str> = None;
    while now < minutes * 60_000 {
        let step = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        if let State::Visiting(visit) = &guest.state {
            watch(&visit.osaka, now);
            *out.time.entry(doing(&visit.osaka, now)).or_default() += step;
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
        }
        if (now + step) / 300_000 != now / 300_000
            && let State::Visiting(visit) = &guest.state
        {
            out.needs.push(visit.osaka.needs().summary());
        }
        now += step;
        if room
            .chat_every
            .is_some_and(|every| now / every != (now - step) / every)
        {
            view.chat_mark.synced += 1;
        }
        if guest.advance(now) {
            paint(&mut guest, &room.real, &view, now);
        }
        if let State::Visiting(visit) = &guest.state {
            home_census(&guest, visit, now, &mut out);
        }
    }
    if let State::Visiting(visit) = &guest.state {
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
    }
    out
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

/// What a census act counts as.
pub(super) fn group(doing: &str) -> &'static str {
    match doing {
        d if d.starts_with("use:") => "furniture",
        "idle:Sit" | "idle:LieBack" | "idle:LieFront" => "floor rest",
        "SpaceOut" | "idle:Gaze" => "spacing out",
        "Walk" | "Climb" | "Clamber" | "Out" | "Away" | "Door" | "Fall" | "Peer" | "Dazed" => {
            "moving"
        }
        "Pull" | "Swap" | "Giggle" | "Innocent" | "Tear" | "Sneeze" | "PutBack" | "Admire" => {
            "mischief"
        }
        "Lift" | "SetDown" => "home",
        d if d.starts_with("idle:") => "exercise",
        _ => "standing",
    }
}

/// The share of `part` in `whole`, as a percentage.
fn pct(part: u64, whole: u64) -> f64 {
    100.0 * part as f64 / whole.max(1) as f64
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

#[test]
#[ignore = "the visit simulator: run by hand in release with --nocapture"]
fn visit_census() {
    const SEEDS: u64 = 16;
    const MINUTES: u64 = 30;
    // With CENSUS_MOODS set, each room in each mood (forced).
    let moods: Vec<Option<Mood>> = if std::env::var_os("CENSUS_MOODS").is_some() {
        Mood::ALL.into_iter().map(Some).collect()
    } else {
        vec![None]
    };
    let rooms = [stage_room(), furnished_room(), resident_room()];
    for (room, mood) in rooms
        .iter()
        .flat_map(|room| moods.iter().map(move |&mood| (room, mood)))
    {
        let visits: Vec<Visit> = (0..SEEDS)
            .map(|seed| simulate(room, seed, MINUTES, mood))
            .collect();
        let mut choices: BTreeMap<String, usize> = BTreeMap::new();
        let mut time: BTreeMap<String, u64> = BTreeMap::new();
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
            *groups.entry(group(k)).or_default() += v;
        }
        eprintln!(
            "\n== {} {} ({SEEDS} visits × {MINUTES} min): {} choices ({:.0} a visit)",
            room.name,
            mood.map_or("(drawn)".to_owned(), |m| format!("{m:?}")),
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
        home_summary(&visits);
        for (i, needs) in visits[0].needs.iter().enumerate() {
            eprintln!("  seed 0 at {:>2} min: {needs}", (i + 1) * 5);
        }
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

/// Milliseconds of `visits` in each census group.
fn grouped(visits: &[Visit]) -> BTreeMap<&'static str, u64> {
    let mut groups = BTreeMap::new();
    for visit in visits {
        for (k, v) in &visit.time {
            *groups.entry(group(k)).or_default() += v;
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
