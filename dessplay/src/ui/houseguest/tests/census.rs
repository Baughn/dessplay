//! The visit simulator: long visits in a few rooms over many seeds, in
//! ASCII, summarised — what she chose, where her time went, how her needs
//! moved, what she lost and said. It's how her needs and moods are tuned
//! before any statistics test is re-pinned. Ignored; run by hand:
//!
//! ```text
//! cargo test --release -p dessplay --lib visit_census -- --ignored --nocapture
//! ```

use super::*;
use crate::ui::houseguest::brain::{Mood, Want};
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
    let mut guest = Guest::new(seed);
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
    let mut out = Visit::default();
    let mut now = 0;
    let mut said: Option<&'static str> = None;
    while now < minutes * 60_000 {
        let step = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        if let State::Visiting(visit) = &guest.state {
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
    }
    if let State::Visiting(visit) = &guest.state {
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
/// furniture and less moving about than on an industrious one, and a
/// dreamy one spaces out more than an ordinary one.
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
        at(&lazy, "moving") < at(&busy, "moving"),
        "lazy {lazy:?}, industrious {busy:?}"
    );
    let (dreamy, ordinary) = (in_mood(Mood::Dreamy), in_mood(Mood::Ordinary));
    assert!(
        at(&dreamy, "spacing out") > at(&ordinary, "spacing out"),
        "dreamy {dreamy:?}, ordinary {ordinary:?}"
    );
}
