//! The film on her TV (phase 5c D7, step 12b): a still of what's
//! playing, fed as the shell feeds it ([`TvFeed`]), shown in place of
//! her programme in line art. Only her drawing changes: what she does,
//! and every ASCII frame, are the same with or without it.

use super::census::{Room, at_afternoon, fed_afternoon, furnished_room};
use super::*;
use crate::ui::houseguest::brain::Mood;
use crate::ui::houseguest::film::{FilmId, test_picture};
use crate::ui::tv_feed::{Sent, TvAnswer, TvAsk, TvFeed};
use dessplay_core::types::Ed2kHash;
use std::collections::HashMap;

const FILM: Ed2kHash = Ed2kHash([0xA7; 16]);
const OTHER: Ed2kHash = Ed2kHash([0xB8; 16]);

/// Her home with only a TV (she watches it a lot), on a fed afternoon.
fn tv_home() -> Room {
    at_afternoon(Room {
        name: "home, TV only",
        owns: &[Furniture::Tv],
        ..furnished_room()
    })
}

/// What the films do over a run: the session's answers in turn (what
/// each is, and how long it takes), and things arriving at their own
/// times whatever she wants (a still, a clear, the film changing to the
/// other or stopping).
#[derive(Clone, Debug)]
struct Schedule {
    answers: Vec<(Answer, u64)>,
    arrivals: Vec<(u64, Arrival)>,
}

/// What the session answers a question with ([`TvAnswer`]'s kinds).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Good,
    Failed,
    NotAsked,
}

#[derive(Clone, Copy, Debug)]
enum Arrival {
    Still,
    Clear,
    Change,
    Stop,
}

/// One painted frame as a line: when, what she's doing, where, how she
/// looks, what her script shows, her generator, her next wake, and every
/// cell that differs from the real frame (a kitty image's cell as
/// "image": which image is drawing, not behaviour).
fn line(guest: &Guest, frame: &Buffer, real: &Buffer, now: u64) -> String {
    let her = match &guest.state {
        State::Visiting(visit) => {
            let osaka = &visit.osaka;
            format!(
                "{} {} {} {:?} {:?} {:?}",
                osaka.act_name(),
                osaka.x,
                osaka.y,
                osaka.facing,
                osaka.appearance(now),
                osaka.prop(now),
            )
        }
        _ => "away".to_owned(),
    };
    let mut cells = String::new();
    let width = usize::from(real.area.width);
    for (i, (got, want)) in frame.content.iter().zip(&real.content).enumerate() {
        if got != want {
            let cell = if got.symbol().contains('\x1b') {
                "image".to_owned()
            } else {
                format!("{got:?}")
            };
            cells.push_str(&format!("|{} {} {cell}", i % width, i / width));
        }
    }
    format!(
        "{now} {her} rng={:x} wake={:?} {cells}",
        guest.rng.0,
        guest.next_tick(now)
    )
}

/// What a run of [`fed_films`] gives: its lines, and in line art each
/// paint's TV look (the films drawn, and any static between them).
struct Run {
    lines: Vec<String>,
    /// When each chat line came.
    chats: Vec<u64>,
    /// At each paint where her TV was drawn: when, her act's start,
    /// whether that act is a watch of her programme, how the TV looked,
    /// and the film held then.
    tv: Vec<(u64, u64, bool, graphics::Look, Option<Ed2kHash>)>,
    /// When a still reached her (delivering).
    stills: Vec<u64>,
    /// Every still made, by its id: the file it's of.
    films: HashMap<FilmId, Ed2kHash>,
    /// When each question went (the shell's sends).
    asks: Vec<u64>,
    /// While she wanted a still: from when, to when (the end of the run
    /// for the last).
    wanting: Vec<(u64, u64)>,
}

/// Her fed afternoon in `room` for `minutes`, the shell's film feed run
/// beside her through the calls the UI loop makes ([`TvFeed::turn`]
/// each turn, [`TvFeed::deliver`] for each answer, which comes after its
/// delay), plus `schedule`'s arrivals. With `deliver`, each still and
/// clear reaches her; without, the same inputs come and go and she's
/// painted at the same moments (the client draws on every input), but no
/// picture reaches her (a good answer is delivered as a failed one: the
/// feed's state after either is the same).
fn fed_films(
    room: &Room,
    seed: u64,
    graphics: bool,
    mood: Mood,
    minutes: u64,
    schedule: &Schedule,
    deliver: bool,
) -> Run {
    let mut guest = fed_afternoon(room, seed, graphics, mood);
    if let Some(graphics) = guest.graphics.as_mut() {
        graphics.take_looks();
    }
    let (real, mut view) = (room.real.clone(), room.view.clone());
    let mut feed = TvFeed::default();
    let mut held = Some(FILM);
    // Stills made so far (each its own), answers given so far, and the
    // answer pending (when, to which question, and what).
    let (mut made, mut answered) = (0u32, 0usize);
    let mut pending: Option<(u64, TvAsk, Answer)> = None;
    let mut out = Run {
        lines: Vec::new(),
        chats: Vec::new(),
        tv: Vec::new(),
        stills: Vec::new(),
        films: HashMap::new(),
        asks: Vec::new(),
        wanting: Vec::new(),
    };
    let mut wanted_since: Option<u64> = None;
    let mut now = 0;
    let end = minutes * 60_000;
    while now < end {
        let tick = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        let (mut step, chat) = room.step_from(now, tick);
        // Cut at the next arrival or answer, so each comes at its time.
        let next = schedule
            .arrivals
            .iter()
            .map(|&(t, _)| t)
            .chain(pending.as_ref().map(|&(t, ..)| t))
            .filter(|&t| t > now)
            .min();
        let mut chat = chat;
        if let Some(t) = next
            && t - now < step
        {
            step = t - now;
            chat = false;
        }
        now += step;
        let mut input = false;
        if chat {
            // An input: the client draws on it.
            input = true;
            view.chat_mark.synced += 1;
            out.chats.push(now);
        }
        for &(_, arrival) in schedule.arrivals.iter().filter(|&&(t, _)| t == now) {
            input = true;
            match arrival {
                Arrival::Still => {
                    made += 1;
                    let still = test_picture(held.unwrap_or(FILM), made);
                    out.films.insert(still.id(), still.file());
                    if deliver {
                        guest.set_tv_picture(Some(still));
                        out.stills.push(now);
                    }
                }
                Arrival::Clear => {
                    if deliver {
                        guest.set_tv_picture(None);
                    }
                }
                Arrival::Change => held = Some(if held == Some(FILM) { OTHER } else { FILM }),
                Arrival::Stop => held = None,
            }
        }
        if let Some((t, ask, kind)) = pending.take() {
            if t <= now {
                input = true;
                let answer = match kind {
                    Answer::Good if deliver => {
                        made += 1;
                        let still = test_picture(ask.file, made);
                        out.films.insert(still.id(), still.file());
                        TvAnswer::Still(still)
                    }
                    Answer::Good | Answer::Failed => TvAnswer::Failed,
                    Answer::NotAsked => TvAnswer::NotAsked,
                };
                if feed.deliver(&mut guest, ask, answer, now) {
                    out.stills.push(now);
                }
            } else {
                pending = Some((t, ask, kind));
            }
        }
        // The UI loop's turn: follow the film held, ask if due.
        let mut sent = None;
        let closed = feed.turn(&mut guest, held, now, |ask| {
            sent = Some(ask);
            Sent::Gone
        });
        assert!(!closed);
        if let Some(ask) = sent {
            out.asks.push(now);
            let (kind, delay) = schedule.answers[answered % schedule.answers.len()];
            answered += 1;
            pending = Some((now + delay, ask, kind));
        }
        if guest.advance(now) || input {
            let frame = paint(&mut guest, &real, &view, now);
            out.lines.push(line(&guest, &frame, &real, now));
            if let Some(graphics) = guest.graphics.as_mut()
                && let State::Visiting(visit) = &guest.state
            {
                let osaka = &visit.osaka;
                let watching = osaka.tv_bound() && osaka.seat().is_some();
                for look in graphics.take_looks() {
                    if matches!(look, graphics::Look::Tv(_) | graphics::Look::Film(..)) {
                        out.tv
                            .push((now, osaka.act_started(), watching, look, held));
                    }
                }
            }
        }
        match (wanted_since, guest.tv_wants_picture()) {
            (None, true) => wanted_since = Some(now),
            (Some(since), false) => {
                out.wanting.push((since, now));
                wanted_since = None;
            }
            _ => {}
        }
    }
    if let Some(since) = wanted_since {
        out.wanting.push((since, end));
    }
    out
}

/// How long each run of the purity property is (four sims a case: kept
/// short, off the gate's long pole).
const PURITY_MINUTES: u64 = 4;

fn schedule_strategy() -> impl Strategy<Value = Schedule> {
    let arrival = prop_oneof![
        3 => Just(Arrival::Still),
        2 => Just(Arrival::Clear),
        1 => Just(Arrival::Change),
        1 => Just(Arrival::Stop),
    ];
    let answer = prop_oneof![
        3 => Just(Answer::Good),
        1 => Just(Answer::Failed),
        1 => Just(Answer::NotAsked),
    ];
    (
        proptest::collection::vec((answer, 0u64..4_000), 1..6),
        proptest::collection::vec((1u64..PURITY_MINUTES * 60_000, arrival), 0..10),
    )
        .prop_map(|(answers, arrivals)| Schedule { answers, arrivals })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        dessplay_core::test_support::proptest_cases(8)
    ))]

    /// The purity rule (phase 5c D7): whatever stills arrive, whenever,
    /// good, failed or not asked, answered late or after the question was
    /// given up, and however the film changes, her trace (what she
    /// does, where, how she looks, her generator, her wakes) and every
    /// cell of her ASCII frames are byte-identical with and without the
    /// pictures reaching her, in both modes. Only which image her TV's
    /// cells hold differs.
    #[test]
    fn her_film_never_moves_her(
        seed in 0u64..1_000,
        mood in 0usize..4,
        schedule in schedule_strategy(),
    ) {
        let room = tv_home();
        let mood = Mood::ALL[mood];
        for graphics in [false, true] {
            let with = fed_films(&room, seed, graphics, mood, PURITY_MINUTES, &schedule, true);
            let without = fed_films(&room, seed, graphics, mood, PURITY_MINUTES, &schedule, false);
            prop_assert_eq!(with.lines.len(), without.lines.len(), "graphics={}", graphics);
            for (a, b) in with.lines.iter().zip(&without.lines) {
                prop_assert_eq!(a, b, "graphics={}", graphics);
            }
        }
    }
}

/// The once-a-minute refresh (the user's Q2), pinned here by its literal
/// rather than the feed's constant.
const A_MINUTE: u64 = 60_000;
/// The slack on it: one of the driver's steps (at most a second).
const STEP: u64 = 1_000;

/// The film is drawn: on fed afternoons in her TV-only home, lazy (her
/// watches linger, up to two minutes), each answer 150 ms late, her TV
/// shows a still in place of her programme while she watches. While she
/// wants a still, one is asked for at once, then again once a minute
/// has passed, no sooner and no later (stills reach her at least a
/// minute apart); each cuts in at the paint it arrives at, straight
/// from the one before (never static between: static is only her
/// switching it on), and a failed refresh keeps the still she shows.
/// The drawn half of D7's stillness rule, beside
/// `no_long_act_flips_faster_than_a_frame`, which checks her model.
/// Without stills, the same watches show their programmes.
#[test]
fn a_long_watch_takes_a_fresh_still_once_a_minute() {
    let room = tv_home();
    let (mut filmed, mut refreshed) = (0, 0);
    for answers in [
        vec![(Answer::Good, 150)],
        vec![(Answer::Good, 150), (Answer::Failed, 150)],
    ] {
        let schedule = Schedule {
            answers,
            arrivals: Vec::new(),
        };
        for seed in 0..4 {
            let run = fed_films(&room, seed, true, Mood::Lazy, 20, &schedule, true);
            // Painted at each chat line, as the client draws on every
            // input.
            let painted: Vec<u64> = run
                .lines
                .iter()
                .map(|line| line.split(' ').next().unwrap().parse().unwrap())
                .collect();
            assert!(!run.chats.is_empty(), "seed {seed}: no chat");
            for &t in &run.chats {
                assert!(
                    painted.contains(&t),
                    "seed {seed}: no paint at the line at {t}"
                );
            }
            for pair in run.stills.windows(2) {
                assert!(
                    pair[1] - pair[0] >= A_MINUTE,
                    "seed {seed}: stills at {} and {}",
                    pair[0],
                    pair[1]
                );
            }
            // The questions: none sooner than a minute after the last,
            // and while she wants a still, none later (the first at once
            // when the minute allows).
            for pair in run.asks.windows(2) {
                assert!(pair[1] - pair[0] >= A_MINUTE, "seed {seed}: asks {pair:?}");
            }
            for &(from, to) in &run.wanting {
                let mut due = run
                    .asks
                    .iter()
                    .rev()
                    .find(|&&t| t <= from)
                    .map_or(from, |&t| (t + A_MINUTE).max(from));
                while due + STEP <= to {
                    assert!(
                        run.asks.iter().any(|&t| t >= due && t <= due + STEP),
                        "seed {seed}: wanting {from}..{to}, no question by {}: {:?}",
                        due + STEP,
                        run.asks
                    );
                    let at = run.asks.iter().find(|&&t| t >= due).copied().unwrap_or(due);
                    due = at + A_MINUTE;
                }
            }
            let mut acts: Vec<(u64, Vec<(u64, graphics::Look)>)> = Vec::new();
            for &(t, since, watching, look, _) in &run.tv {
                if !watching {
                    continue;
                }
                match acts.last_mut() {
                    Some((s, looks)) if *s == since => looks.push((t, look)),
                    _ => acts.push((since, vec![(t, look)])),
                }
            }
            for (since, looks) in &acts {
                let films: Vec<(u64, FilmId)> = looks
                    .iter()
                    .filter_map(|&(t, look)| match look {
                        graphics::Look::Film(id, _) => Some((t, id)),
                        _ => None,
                    })
                    .collect();
                if films.is_empty() {
                    continue;
                }
                filmed += 1;
                let first = films[0].0;
                for &(t, look) in looks.iter().filter(|&&(t, _)| t > first) {
                    assert!(
                        matches!(look, graphics::Look::Film(..)),
                        "seed {seed}: the act from {since} drew {look:?} at {t}, after its film at {first}"
                    );
                }
                for t in films
                    .windows(2)
                    .filter(|w| w[0].1 != w[1].1)
                    .map(|w| w[1].0)
                {
                    assert!(
                        run.stills.contains(&t),
                        "seed {seed}: the act from {since} changed its film at {t}, not as a still came"
                    );
                    refreshed += 1;
                }
            }
            let plain = fed_films(
                &room,
                seed,
                true,
                Mood::Lazy,
                20,
                &Schedule {
                    answers: vec![(Answer::Failed, 150)],
                    arrivals: Vec::new(),
                },
                true,
            );
            assert!(
                plain
                    .tv
                    .iter()
                    .all(|(_, _, _, look, _)| !matches!(look, graphics::Look::Film(..))),
                "seed {seed}: no still, no film"
            );
        }
    }
    println!("watches filmed {filmed}, fresh stills mid-watch {refreshed}");
    assert!(filmed >= 8, "her TV showed the film in {filmed} watches");
    assert!(refreshed >= 2, "no long watch took a fresh still");
}

/// The shell's wiring of the feed ([`TvFeed::turn`], [`TvFeed::deliver`],
/// as the UI loop calls them): her TV only ever draws a still of the
/// film held when it's drawn. A still of a film since changed, arriving
/// late (or after its question was given up), never reaches her; a
/// change of film clears her stills at once, so the programme's card is
/// back until a still of the new film comes; a question the player
/// wasn't asked is asked again within seconds.
#[test]
fn her_tv_draws_only_the_held_films_stills() {
    let room = tv_home();
    let (mut drawn, mut cards_after_change) = (0, 0);
    for seed in 0..3 {
        let schedule = Schedule {
            answers: vec![
                (Answer::Good, 2_500),
                (Answer::NotAsked, 40),
                (Answer::Good, 3_600),
                (Answer::Good, 300),
                (Answer::Failed, 900),
            ],
            // A change, then another while the question it set off is
            // out (its answer, of the film before, comes late).
            arrivals: (1..40)
                .flat_map(|i| {
                    let at = i * 29_000 + seed * 1_100;
                    let then = if i % 7 == 0 {
                        Arrival::Stop
                    } else {
                        Arrival::Change
                    };
                    [(at, Arrival::Change), (at + 1_200, then)]
                })
                .collect(),
        };
        let run = fed_films(&room, seed, true, Mood::Lazy, 20, &schedule, true);
        let mut held_before = Some(FILM);
        let mut since_change = false;
        for &(t, _, _, look, held) in &run.tv {
            if held != held_before {
                since_change = true;
                held_before = held;
            }
            match look {
                graphics::Look::Film(id, _) => {
                    drawn += 1;
                    since_change = false;
                    assert_eq!(
                        held,
                        Some(run.films[&id]),
                        "seed {seed}: at {t}, a still of another film"
                    );
                }
                graphics::Look::Tv(_) if since_change => cards_after_change += 1,
                _ => {}
            }
        }
    }
    println!("films drawn {drawn}, cards after a change {cards_after_change}");
    assert!(drawn > 0, "no film drawn");
    assert!(cards_after_change > 0, "no change came while her TV showed");
}

/// A session gone (its queue closed) ends the UI loop, rather than
/// leaving the question due with nothing waking for it but at once (the
/// loop spun).
#[test]
fn a_closed_session_ends_the_loop_without_spinning() {
    let room = tv_home();
    let mut guest = fed_afternoon(&room, 3, true, Mood::Lazy);
    let mut feed = TvFeed::default();
    let mut now = 0;
    while !guest.tv_wants_picture() {
        now += 1_000;
        assert!(now < 30 * 60_000, "she never wanted a still");
        if guest.advance(now) {
            paint(&mut guest, &room.real, &room.view, now);
        }
    }
    assert!(feed.turn(&mut guest, Some(FILM), now, |_| Sent::Closed));
    assert!(
        feed.wake_in(now, true).is_some_and(|d| !d.is_zero()),
        "a question that couldn't go wakes the loop later, not at once"
    );
    assert!(!feed.turn(&mut guest, Some(FILM), now, |_| Sent::Full));
    assert!(feed.wake_in(now, true).is_some_and(|d| !d.is_zero()));
}

/// Her TV wants a still only in line art, only while she's on her way
/// to watch her programme (walking to her seat at the TV), watching it
/// or flicking through the channels to rest on it (not the shopping
/// channel), only of a real TV (not one still in its box; she makes no
/// TV of text), and never in ASCII.
#[test]
fn her_tv_wants_a_still_only_for_her_programme_in_line_art() {
    let room = tv_home();
    let (mut walking, mut surfing, mut boxed) = (0, 0, 0);
    for (graphics, shopping, seed) in [false, true]
        .into_iter()
        .flat_map(|g| [(g, false, 3), (g, true, 3), (g, false, 5), (g, false, 8)])
    {
        let mut guest = fed_afternoon(&room, seed, graphics, Mood::Lazy);
        if shopping {
            guest.shop_now = true;
        }
        let mut wanted = 0;
        let mut now = 0;
        while now < 10 * 60_000 {
            let step = guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            now += step;
            if guest.advance(now) {
                paint(&mut guest, &room.real, &room.view, now);
            }
            let wants = guest.tv_wants_picture();
            let State::Visiting(visit) = &mut guest.state else {
                assert!(!wants);
                continue;
            };
            let osaka = &visit.osaka;
            let shown = osaka.plays().map(|play| play.own);
            if wants {
                wanted += 1;
                assert!(graphics, "never in ASCII");
                assert!(
                    !matches!(
                        shown,
                        Some(script::ScriptId::Shopping | script::ScriptId::FirstSunrise)
                    ),
                    "at {now}: {shown:?}"
                );
            }
            if graphics && matches!(shown, Some(script::ScriptId::Watch)) {
                assert!(wants, "at {now}: watching, but no still wanted");
            }
            if graphics && matches!(shown, Some(script::ScriptId::Surf)) {
                surfing += 1;
                assert!(wants, "at {now}: flicking through, but no still wanted");
            }
            let summary = osaka.act_summary();
            if graphics && summary.starts_with("Walk") && summary.ends_with("for Watch (Tv)") {
                walking += 1;
                assert!(wants, "at {now}: {summary}, but no still wanted");
            }
            if wants {
                // The same moment, her TV still boxed: none wanted.
                let tv = visit
                    .shown
                    .iter_mut()
                    .find(|s| s.item == Furniture::Tv)
                    .expect("her TV");
                tv.boxed = true;
                let boxed_wants = guest.tv_wants_picture();
                let State::Visiting(visit) = &mut guest.state else {
                    unreachable!()
                };
                for s in visit.shown.iter_mut().filter(|s| s.item == Furniture::Tv) {
                    s.boxed = false;
                }
                assert!(!boxed_wants, "at {now}: a boxed TV wants a still");
                boxed += 1;
            }
        }
        assert_eq!(
            wanted > 0,
            graphics,
            "graphics={graphics} shopping={shopping} seed={seed}"
        );
    }
    println!("walking {walking}, surfing {surfing}, boxed {boxed}");
    assert!(walking > 0 && surfing > 0 && boxed > 0);
}
