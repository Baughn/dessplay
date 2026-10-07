//! Golden trajectories: ten scenes, four seeds each, in ASCII and line
//! art, hashed frame by frame; and the first four again with her routine
//! never reaching her (the `UNFED_*` tables: as she was before the clock
//! fed her). A refactor that means to change nothing she does keeps every
//! hash (re-record them when behaviour is meant to change). With
//! `HOUSEGUEST_GOLDEN_TRACE` set to a directory, each run also writes its
//! frames there, one line each, to diff against.

use super::*;

/// FNV-1a, 64-bit: stable across Rust versions, unlike `DefaultHasher`.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// One run's trace: a line per painted frame.
struct Trace {
    hash: Fnv,
    lines: Option<Vec<String>>,
}

impl Trace {
    fn new() -> Self {
        Self {
            hash: Fnv::new(),
            lines: std::env::var_os("HOUSEGUEST_GOLDEN_TRACE").map(|_| Vec::new()),
        }
    }

    /// Record the frame painted at `now`: her (what she's doing, where,
    /// how she looks) and every cell that differs from the real frame.
    fn frame(&mut self, guest: &Guest, frame: &Buffer, real: &Buffer, now: u64) {
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
                    // Her placement alone, as it was recorded before her
                    // door and the pieces travelled with it.
                    visit.image.as_ref().and_then(|i| i.figure.her())
                )
            }
            State::Leaving(_) => "leaving".to_owned(),
            State::Arriving(_) => "arriving".to_owned(),
            State::Absent => "absent".to_owned(),
            State::Away(_) => "away".to_owned(),
        };
        let mut cells = Fnv::new();
        let width = usize::from(real.area.width);
        for (i, (got, want)) in frame.content.iter().zip(&real.content).enumerate() {
            if got != want {
                // A kitty image's id is random: where it's placed counts,
                // and what it shows is her appearance.
                let cell = if got.symbol().contains('\x1b') {
                    "image".to_owned()
                } else {
                    format!("{got:?}")
                };
                cells.bytes(format!("{} {} {cell}", i % width, i / width).as_bytes());
            }
        }
        let line = format!("{now} {her} {:016x}", cells.0);
        self.hash.bytes(line.as_bytes());
        self.hash.bytes(b"\n");
        if let Some(lines) = &mut self.lines {
            lines.push(line);
        }
    }

    /// The run's hash, with her record at the end, as it's saved;
    /// written out as `name` when tracing.
    fn finish(mut self, guest: &Guest, name: &str) -> u64 {
        let home = guest.ledger.to_json();
        self.hash.bytes(home.as_bytes());
        if let (Some(lines), Some(dir)) = (self.lines, std::env::var_os("HOUSEGUEST_GOLDEN_TRACE"))
        {
            let path = std::path::Path::new(&dir).join(format!("{name}.txt"));
            let mut text = lines.join("\n");
            text.push('\n');
            text.push_str(&home);
            std::fs::write(&path, text).unwrap();
        }
        self.hash.0
    }
}

/// A deterministic spread of `n` times from `from`, `lo..hi` ms apart.
fn times(seed: u64, n: u64, from: u64, (lo, hi): (u64, u64)) -> Vec<u64> {
    let mut rng = Rng(seed ^ 0x0060_1de2);
    let mut at = from;
    (0..n)
        .map(|_| {
            at += rng.range(lo, hi);
            at
        })
        .collect()
}

/// Step the guest as the shell would until `until`, painting whenever it
/// says the screen could change, with `world(now, step)` the frame, its
/// view, and whether local input arrives on this step; the stage cues
/// each of `cues` when its time comes.
///
/// `events` are the times at which `world` changes (a chat line, a key
/// press, a focus change, text arriving): a step that would cross one
/// (or a cue's time) is cut at it, so each comes into the world at its
/// own moment, not at the first step past it, whose length her wakes
/// set (a wake that changes only how she looks would move what she
/// does). A key press or a cue paints there; a chat line or a focus
/// change she sees at her next paint, which her wakes time (the client
/// paints on a chat line: making this driver do so is open, step 8c's
/// review). The census drivers do the same (`census::Room::step_from`).
fn drive(
    guest: &mut Guest,
    trace: &mut Trace,
    until: u64,
    cues: &[(u64, Scene)],
    events: &[&[u64]],
    mut world: impl FnMut(u64, u64) -> (Buffer, IdleView, bool),
) {
    let mut now = 0;
    let (real, view, _) = world(0, 0);
    let frame = paint(guest, &real, &view, now);
    trace.frame(guest, &frame, &real, now);
    while now < until {
        let tick = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
        let step = step_to(now, tick, cues, events);
        now += step;
        let (real, view, input) = world(now, step);
        if input {
            guest.activity(now);
        }
        let cue = cues.iter().find(|&&(t, _)| t <= now && t > now - step);
        if let Some(&(_, scene)) = cue {
            guest.cue(scene);
        }
        if guest.advance(now) || input || cue.is_some() {
            let frame = paint(guest, &real, &view, now);
            trace.frame(guest, &frame, &real, now);
        }
    }
}

/// The step from `now`: `tick` long, but cut at the next of `cues`' or
/// `events`' times if it would cross one.
fn step_to(now: u64, tick: u64, cues: &[(u64, Scene)], events: &[&[u64]]) -> u64 {
    let next = cues
        .iter()
        .map(|&(t, _)| t)
        .chain(events.iter().flat_map(|times| times.iter().copied()))
        .filter(|&t| t > now)
        .min();
    next.map_or(tick, |t| tick.min(t - now))
}

fn arrived_within(times: &[u64], now: u64, step: u64) -> bool {
    times.iter().any(|&t| t <= now && t > now - step)
}

/// The stage room: she's cued to make a sofa, then into every kind of
/// job, while chat keeps arriving.
fn stage_room(seed: u64, graphics: bool, fed: bool) -> u64 {
    let mut ui = stage_ui();
    let (real, view) = real_frame(&mut ui, 100, 30);
    let mut guest = guest_of(seed, fed);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::MakeSofa);
    let chats = times(seed, 12, 0, (15_000, 40_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    let cues = [
        (45_000, Scene::Swap),
        (90_000, Scene::Sneeze),
        (130_000, Scene::Pull),
        (170_000, Scene::Shopping),
        (215_000, Scene::MakeBed),
        (255_000, Scene::Parcel),
    ];
    drive(
        &mut guest,
        &mut trace,
        300_000,
        &cues,
        &[&chats],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            (real.clone(), view, false)
        },
    );
    trace.finish(
        &guest,
        &format!("stage-{seed}-{graphics}{}", unfed_tag(fed)),
    )
}

/// A resident with a sofa and a TV, through focus changes and key
/// presses, with text in the chat to get up to mischief with.
fn resident(seed: u64, graphics: bool, fed: bool) -> u64 {
    let (w, h) = (100, 30);
    let mut guest = guest_of(seed, fed);
    if graphics {
        guest.set_picker(kitty());
    }
    for (item, nook, at) in [
        (Furniture::Sofa, Nook::Users, 200),
        (Furniture::Tv, Nook::Users, 700),
        (Furniture::Bed, Nook::Playlist, 500),
    ] {
        let _ = guest
            .ledger
            .home
            .add(room::Prop::new(item, nook, at, sprite::Facing::Right));
    }
    let mut real = rooms(w, h);
    // Chat lines down the chat pane, the last few just above its floor.
    let text: Vec<(u16, u16, String)> = (0..8)
        .map(|i| {
            (
                2 + i * 3 % 20,
                4 + i * 2,
                "so what did you think".to_owned(),
            )
        })
        .chain((0..3).map(|i| (2, h - 7 + i, "same, wait for me".to_owned())))
        .collect();
    scatter(&mut real, &text, &[]);
    let panes = nooks(w, h);
    let focus_at = times(seed, 6, 10_000, (15_000, 35_000));
    // Cued at mischief in the chat, then caught at it.
    let cues = [
        (20_000, Scene::Pull),
        (60_000, Scene::Swap),
        (100_000, Scene::Sneeze),
        (140_000, Scene::MakeSofa),
    ];
    let mut presses = times(seed ^ 1, 6, 5_000, (8_000, 30_000));
    presses.extend(cues.iter().map(|&(t, _)| t + 2_500 + seed * 700));
    let chats = times(seed ^ 2, 6, 0, (15_000, 40_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        180_000,
        &cues,
        &[&chats, &focus_at, &presses],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            // Each focus change cycles: none, the Users pane, the chat, the
            // Playlist pane.
            let changes = focus_at.iter().filter(|&&t| t <= now).count();
            let focus = match changes % 4 {
                0 => None,
                1 => Some(panes[1].1),
                2 => Some(panes[0].1),
                _ => Some(panes[2].1),
            };
            let view = IdleView {
                chat_mark: mark,
                ..resident_view(w, h, focus)
            };
            (real.clone(), view, arrived_within(&presses, now, step))
        },
    );
    trace.finish(
        &guest,
        &format!("resident-{seed}-{graphics}{}", unfed_tag(fed)),
    )
}

/// A furnished home over ten minutes from Monday 16:00, with the odd
/// chat line: long enough to go to work unfed; fed, her job's closed on a
/// school day (see [`weekend`]).
fn furnished(seed: u64, graphics: bool, fed: bool) -> u64 {
    let (real, view) = home_screen();
    let mut guest = guest_of(seed, fed);
    if graphics {
        guest.set_picker(kitty());
    }
    guest.cue(Scene::Arrive);
    paint(&mut guest, &real, &view, 0);
    for item in [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Bed,
        Furniture::Desk,
    ] {
        guest.give(item);
        paint(&mut guest, &real, &view, 0);
    }
    let chats = times(seed, 10, 0, (30_000, 90_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        600_000,
        &[],
        &[&chats],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            (real.clone(), view, false)
        },
    );
    trace.finish(
        &guest,
        &format!("furnished-{seed}-{graphics}{}", unfed_tag(fed)),
    )
}

/// Scrolled back while messages arrive: she comes to poke the accordion
/// (a visitor on even seeds, a resident on odd ones).
fn errand(seed: u64, graphics: bool, fed: bool) -> u64 {
    let (w, h) = (100, 30);
    let resident = seed % 2 == 1;
    let mut guest = guest_of(seed, fed);
    if graphics {
        guest.set_picker(kitty());
    }
    let chat = nooks(w, h)[0].1;
    let arrivals = times(seed, 4, 0, (5_000, 20_000));
    let presses = times(seed ^ 1, 3, 70_000, (5_000, 20_000));
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        150_000,
        &[],
        &[&arrivals, &presses],
        |now, step| {
            let unseen = arrivals.iter().filter(|&&a| a <= now).count();
            let (real, accordion) = accordion_room(w, h, unseen);
            let base = IdleView {
                busy: Some(Busy::Playing),
                resident,
                focus: resident.then_some(chat),
                chat,
                nooks: nooks(w, h)[1..].to_vec(),
                ..view(bottom_strip(w, h))
            };
            let view = scrolled_back(base, accordion, unseen);
            (real, view, arrived_within(&presses, now, step))
        },
    );
    trace.finish(
        &guest,
        &format!("errand-{seed}-{graphics}{}", unfed_tag(fed)),
    )
}

/// A scene: its seed, whether in line art, and whether her routine is
/// fed to her (else [`Guest::unfed`]).
type SceneFn = fn(u64, bool, bool) -> u64;

/// A guest from `seed`, her routine fed to her or not.
fn guest_of(seed: u64, fed: bool) -> Guest {
    let guest = Guest::new(seed);
    if fed { guest } else { guest.unfed() }
}

/// What an unfed run's trace file name ends with.
fn unfed_tag(fed: bool) -> &'static str {
    if fed { "" } else { "-unfed" }
}

/// A home she has visited once, on [`home_screen`], her clock at `at`
/// (her routine fed), with her sofa, TV and desk in the Users pane and
/// her bed in the Playlist pane: she's absent, and arrives once the idle
/// gate opens.
fn home_at(seed: u64, at: routine::GameTime, graphics: bool) -> Guest {
    let mut ledger = Ledger::new_at(seed, at);
    for (item, nook, x) in [
        (Furniture::Sofa, Nook::Users, 50),
        (Furniture::Tv, Nook::Users, 400),
        (Furniture::Desk, Nook::Users, 850),
        (Furniture::Bed, Nook::Playlist, 500),
    ] {
        assert!(
            ledger
                .home
                .add(room::Prop::new(item, nook, x, sprite::Facing::Right))
        );
    }
    let mut guest = Guest::restore(ledger);
    if graphics {
        guest.set_picker(kitty());
    }
    guest
}

/// Monday 20:30, homework time (a school night): ten minutes of her
/// evening at home, with the odd chat line.
fn homework_evening(seed: u64, graphics: bool, _: bool) -> u64 {
    let (real, view) = home_screen();
    let at = routine::GameTime {
        day: 0,
        h: 20,
        m: 30,
    };
    let mut guest = home_at(seed, at, graphics);
    let chats = times(seed, 6, 0, (40_000, 120_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        600_000,
        &[],
        &[&chats],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            (real.clone(), view, false)
        },
    );
    trace.finish(&guest, &format!("evening-{seed}-{graphics}"))
}

/// Monday 23:00, past bedtime: she comes tucked in, and stirs at a chat
/// line or two through five minutes of her night.
fn tucked_in(seed: u64, graphics: bool, _: bool) -> u64 {
    let (real, view) = home_screen();
    let at = routine::GameTime {
        day: 0,
        h: 23,
        m: 0,
    };
    let mut guest = home_at(seed, at, graphics);
    let chats = times(seed, 2, 30_000, (40_000, 100_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        300_000,
        &[],
        &[&chats],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            (real.clone(), view, false)
        },
    );
    trace.finish(&guest, &format!("tucked-{seed}-{graphics}"))
}

/// Saturday 10:00, a day off (her part-time job open until 17:00): ten
/// minutes of her morning at home, with the odd chat line.
fn weekend(seed: u64, graphics: bool, _: bool) -> u64 {
    let (real, view) = home_screen();
    let at = routine::GameTime {
        day: 5,
        h: 10,
        m: 0,
    };
    let mut guest = home_at(seed, at, graphics);
    let chats = times(seed, 6, 0, (40_000, 120_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        600_000,
        &[],
        &[&chats],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            (real.clone(), view, false)
        },
    );
    trace.finish(&guest, &format!("weekend-{seed}-{graphics}"))
}

/// Three minutes of her clock from `at` in her home, with the odd chat
/// line.
fn three_minutes_from(seed: u64, graphics: bool, at: routine::GameTime) -> (Guest, Trace) {
    let (real, view) = home_screen();
    let mut guest = home_at(seed, at, graphics);
    let chats = times(seed, 3, 20_000, (30_000, 60_000));
    let mut mark = ChatMark::default();
    let mut trace = Trace::new();
    drive(
        &mut guest,
        &mut trace,
        180_000,
        &[],
        &[&chats],
        |now, step| {
            if arrived_within(&chats, now, step) {
                mark.synced += 1;
            }
            let view = IdleView {
                chat_mark: mark,
                ..view.clone()
            };
            (real.clone(), view, false)
        },
    );
    (guest, trace)
}

/// Tuesday 08:10, a school morning: she's up, and at 08:15 goes out
/// through her door; her home stands empty, her door closed where she
/// went out.
fn school_morning(seed: u64, graphics: bool, _: bool) -> u64 {
    let at = routine::GameTime {
        day: 1,
        h: 8,
        m: 10,
    };
    let (guest, trace) = three_minutes_from(seed, graphics, at);
    trace.finish(&guest, &format!("school-{seed}-{graphics}"))
}

/// Tuesday 12:40, at school: her home stands empty, and at 12:45 she
/// comes home out of her door.
fn home_from_school(seed: u64, graphics: bool, _: bool) -> u64 {
    let at = routine::GameTime {
        day: 1,
        h: 12,
        m: 40,
    };
    let (guest, trace) = three_minutes_from(seed, graphics, at);
    trace.finish(&guest, &format!("home-{seed}-{graphics}"))
}

/// A dash home (D3a), as it falls for `seed`: from a minute before its
/// first school day's dash (a Tuesday or later), her home standing empty
/// with a fridge in it, three minutes of her clock: out of her closed
/// door, to the fridge for her lunch, and out again by her door.
fn dash_home(seed: u64, graphics: bool, _: bool) -> u64 {
    let (day, minute) = (1..1000)
        .find_map(|day| {
            let school = routine::weekday(day).num_days_from_monday() < 5;
            brain::dash(seed, day)
                .filter(|_| school)
                .map(|minute| (day, minute))
        })
        .expect("a school day with a dash home");
    let at = routine::GameTime {
        day,
        h: (minute - 1) / 60,
        m: (minute - 1) % 60,
    };
    let (real, view) = home_screen();
    let mut guest = home_at(seed, at, graphics);
    assert!(guest.ledger.home.add(room::Prop::new(
        Furniture::Fridge,
        Nook::Playlist,
        900,
        sprite::Facing::Right
    )));
    let mut trace = Trace::new();
    drive(&mut guest, &mut trace, 180_000, &[], &[], |_, _| {
        (real.clone(), view.clone(), false)
    });
    trace.finish(&guest, &format!("dash-{seed}-{graphics}"))
}

/// Every run's hash, for a table of `(seed, ASCII, line art)`.
fn hashes(scene: SceneFn, fed: bool) -> Vec<(u64, u64, u64)> {
    (0..4)
        .map(|seed| (seed, scene(seed, false, fed), scene(seed, true, fed)))
        .collect()
}

/// Her routine fed to her: the tables as she is now.
fn check(name: &str, scene: SceneFn, want: &[(u64, u64, u64)]) {
    check_fed(name, scene, true, want);
}

/// Her routine never reaching her ([`Guest::unfed`]): the tables as she
/// was before the clock fed her (phase 5b A5). No step may move them
/// without a trace diff in its commit.
fn check_unfed(name: &str, scene: SceneFn, want: &[(u64, u64, u64)]) {
    check_fed(name, scene, false, want);
}

fn check_fed(name: &str, scene: SceneFn, fed: bool, want: &[(u64, u64, u64)]) {
    let got = hashes(scene, fed);
    assert!(
        got == want,
        "{name}: her trajectories changed; if that's meant, re-record:\n{}",
        got.iter()
            .map(|(seed, a, b)| format!("    ({seed}, {a:#018x}, {b:#018x}),"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The golden driver lands a step on every event's own time and every
/// cue's (as the census drivers do), however her wakes fall, in both
/// modes: on the stage room, where her wakes are densest, with events at
/// odd moments no wake of hers would hit, a few at once and one past the
/// end.
#[test]
fn the_golden_driver_steps_to_every_event() {
    for graphics in [false, true] {
        for seed in 0..2u64 {
            let at = format!("graphics={graphics} seed={seed}");
            let mut ui = stage_ui();
            let (real, view) = real_frame(&mut ui, 100, 30);
            let mut guest = guest_of(seed, true);
            if graphics {
                guest.set_picker(kitty());
            }
            let chats = times(seed, 8, 0, (3_001, 9_007));
            let presses = [7_777, 7_777, 12_345, 33_333];
            let cues = [(20_011, Scene::Sneeze), (41_003, Scene::Swap)];
            let until = 60_000;
            let mut stepped = Vec::new();
            let mut trace = Trace::new();
            drive(
                &mut guest,
                &mut trace,
                until,
                &cues,
                &[&chats, &presses, &[until + 1]],
                |now, step| {
                    stepped.push((now, step));
                    (real.clone(), view.clone(), false)
                },
            );
            let landed: Vec<u64> = stepped.iter().map(|&(now, _)| now).collect();
            let due = chats
                .iter()
                .chain(&presses)
                .chain(cues.iter().map(|(t, _)| t))
                .filter(|&&t| t <= until);
            for &t in due {
                assert!(landed.contains(&t), "{at}: no step lands on {t} ms");
            }
            assert!(
                stepped.windows(2).all(|w| w[1].0 == w[0].0 + w[1].1),
                "{at}: steps add up"
            );
        }
    }
}

#[test]
fn golden_stage_room() {
    check(
        "stage",
        stage_room,
        &[
            (0, 0x5b42688d70dbe2d4, 0xb276f5d89314d03c),
            (1, 0x7c953da8be804f4f, 0x4d80275cdaf7e110),
            (2, 0x1182fcdddaa8dd07, 0xce09d5b63f289204),
            (3, 0x4cb005da5ee2fe59, 0x172fbbec1d7b6bd5),
        ],
    );
}

#[test]
fn golden_resident() {
    check(
        "resident",
        resident,
        &[
            (0, 0xcc7119817e9176f9, 0x569146435fa5228c),
            (1, 0x5a94514c88ced161, 0xa4ea3e8f228015b1),
            (2, 0xb5cc6cd8f98d0ecc, 0x6204e6730cbdde27),
            (3, 0xe4dcda4adf95ff77, 0x3dabbe4d4830102e),
        ],
    );
}

#[test]
fn golden_furnished_home() {
    check(
        "furnished",
        furnished,
        &[
            (0, 0x5590f92a2a836544, 0xdc3dbb092c72efc3),
            (1, 0xd830fa40de068476, 0x8bdc2ac94c696c5d),
            (2, 0x988ead2f6b7c0243, 0x9a3eb2caf36dff57),
            (3, 0x43ffa7cb07911db2, 0xbd41d3949e44b179),
        ],
    );
}

#[test]
fn golden_errand() {
    check(
        "errand",
        errand,
        &[
            (0, 0x7ef41b6f44466ee1, 0x21d600e1fc53a86a),
            (1, 0xdf91d60dbf2f6bd5, 0xe63c37ac36f0e2ca),
            (2, 0x853a5f641f5d8378, 0x95ae070706e62dc6),
            (3, 0x219ec7d8688cc95e, 0x76564208074cbeb2),
        ],
    );
}

#[test]
fn golden_homework_evening() {
    check(
        "evening",
        homework_evening,
        &[
            (0, 0x876ea56886723d82, 0xa5cab324d4c159c6),
            (1, 0x37b33a754a2dd3b0, 0xfc559d7cb1c2b4f1),
            (2, 0x6ec06ff0c7873fef, 0x704d05253160bc17),
            (3, 0x5c406cbce954ccc6, 0xef32fc05675ffef4),
        ],
    );
}

#[test]
fn golden_tucked_in() {
    check(
        "tucked",
        tucked_in,
        &[
            (0, 0xba09db43923a93ae, 0x30c0edb9d18e94bb),
            (1, 0x3cb861c6934daed1, 0x43718fdc5d1aa0f2),
            (2, 0xdfc846fd62ec9a27, 0xe2c5ec2d0bc9326e),
            (3, 0x23829cc8c48b959f, 0x69b70050d8b5df9a),
        ],
    );
}

#[test]
fn golden_weekend() {
    check(
        "weekend",
        weekend,
        &[
            (0, 0x6e294eaa17d2587b, 0xaf59f7ea51f2540e),
            (1, 0x43c7d9594c8de10a, 0x653578e3e2894656),
            (2, 0xdafd0b8b80eba12c, 0x658cb4be30aaaa8b),
            (3, 0x82f1158a7ca39832, 0x5929fb648f45f962),
        ],
    );
}

#[test]
fn golden_school_morning() {
    check(
        "school",
        school_morning,
        &[
            (0, 0x34abba24308b502d, 0x61df31d116ac8109),
            (1, 0x85a1267f399e61a1, 0x13abefdca5376cc5),
            (2, 0x36876fecb42493bf, 0x23d07a5dd9a875b6),
            (3, 0x67d3c2766feac3e2, 0xc7d3d4fa034582db),
        ],
    );
}

#[test]
fn golden_home_from_school() {
    check(
        "home",
        home_from_school,
        &[
            (0, 0xedede266d0e28c59, 0x657805fbe5642300),
            (1, 0xb9bec406c451f100, 0x1e2bca2cb68bd798),
            (2, 0xab383fe9d3985a63, 0xbefb74800cb09c2a),
            (3, 0x332570b8d4bc1eb6, 0xc477f13e33dddeec),
        ],
    );
}

#[test]
fn golden_dash_home() {
    check(
        "dash",
        dash_home,
        &[
            (0, 0x146fdfcb4445418c, 0xa31153cf283a6bbd),
            (1, 0x973488674fe497a8, 0x629d2676800bfd3d),
            (2, 0x6cf8b1ebaaf4ec62, 0x07cf674663a0282f),
            (3, 0xc14b4e9f2925d69d, 0xd66b50ed236a52dc),
        ],
    );
}

// ---- Unfed: as she was before the clock fed her (A5) ----

/// The stage room's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her door:
/// its trace diff is in that commit), and step 5b's parcels (one waits
/// while she's out of sight, and for her "I'm home!" from work: seeds 1 and
/// 2 in ASCII, out at work as the stage sends a parcel; the trace diff is
/// in that commit), and phase 5c step 5's shorter watch (5 s after a chat
/// line, not 15: each seed and mode first differs 5 s after a line; the
/// trace diff is in that commit), and step 6's looking up in place (each
/// seed and mode first differs at a chat line during a use, where she looks
/// up from it or, dozing, stirs; the trace diff is in that commit), and
/// step 8's slow blink (on a pose she holds, a frame inserted as each blink
/// begins and one as it ends; gazing, one as her "ooh" goes after 3 s:
/// the blink's wakes let a chat line or a cue, which the harness delivers
/// at the step that crosses it, land sooner, and the "ooh" going is an
/// event of hers at which she acts on what changed sooner, so what follows
/// shifts; seed 0 in line art and seed 3 in ASCII then choose otherwise),
/// and its review's credit fix (an easing lands on her needs as they are:
/// seeds 0, 1 and 3 in both modes and 2 in ASCII first differ at a
/// decision, choosing otherwise; the trace diffs are in that commit), and
/// step 10a's floor acts, paper desk and makeshift draw (each seed and
/// mode first differs at her first decision where making is three times
/// as likely, with no real piece of its kind), with its review's stand
/// under a live watch facing the chat (seed 1 in line art: landing from a
/// climb a line came during, she faces the chat at once, not a beat
/// later; nothing else changed; the trace diffs are in that commit), and
/// step 10b's borrowed line (seeds 0, 1 and 3 in ASCII first differ at a
/// decision with a line to borrow a strip of on offer, choosing
/// otherwise; the trace diff is in that commit), with its review's fixes
/// (seed 1 in ASCII: the stage placing her mid-read, the strip is back in
/// its line the frame she's placed, not the next; one frame, nothing else
/// changed; the trace diff is in that commit), and phase 5c step 8c's
/// driver, which cuts a step at each event's own time (each seed and
/// mode first differs at the swap cued at 45 s, where it was painted at
/// the first step past it; the trace diff is in that commit), and step
/// 8c's stillness levers shipped (each seed and mode first differs as
/// her lounge on the sofa she made runs on, longer, where it ended and
/// she walked; the trace diff is in that commit).
const UNFED_STAGE: [(u64, u64, u64); 4] = [
    (0, 0xc1c76b41e25adc6a, 0xb28f1b52a5210b71),
    (1, 0x470d571a004dbe1b, 0x72e4b1da28372f69),
    (2, 0xcccd9bebcb893f2c, 0x9a33fb96d73f0e67),
    (3, 0xf3e5e32c8234215c, 0x35a820c379fb3a8f),
];

/// The resident's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her door:
/// its trace diff is in that commit), and phase 5c step 1's rain-out fix
/// (the tick a focused pane's rain ends on is drawn: one frame inserted at
/// each rain's end, every seed and mode, nothing else changed; the trace
/// diff is in that commit), and step 5's shorter watch (5 s after a chat
/// line, not 15: each seed and mode first differs 5 s after a line, or, a
/// line come as she climbed, at her landing past its watch; the trace diff
/// is in that commit), and step 6's looking up in place (each seed and mode
/// first differs at a chat line during a use, where she looks up from it
/// or, dozing, stirs; the trace diff is in that commit), and step 8's slow
/// blink (on a pose she holds, a frame inserted as each blink begins and
/// one as it ends; gazing, one as her "ooh" goes after 3 s: the blink's
/// wakes let a chat line or a cue, which the harness delivers at the step
/// that crosses it, land sooner, and the "ooh" going is an event of hers
/// at which she acts on what changed sooner, so what follows shifts), and
/// its review's look fix (a line come mid-blink ends the blink: seed 1 in
/// ASCII, its one frame of a blink under her "!" gone; the trace diffs are
/// in that commit), and step 10a's floor acts (seeds 0, 1 and 3 in both
/// modes first differ at her first decision with reading on her back on
/// offer, choosing otherwise; the trace diff is in that commit), and step
/// 10b's borrowed line and keys catching her out tearing chat text (seeds
/// 0, 1 and 3 in ASCII, 1 and 3 in line art, first differ at a decision
/// with a line to borrow a strip of on offer, choosing otherwise; seed 2
/// in both modes and 0 in line art at a key press while she tears chat
/// text for furniture: caught out, where she had torn on; the trace diff
/// is in that commit), with its review's fixes (seed 1 in both modes and
/// 3 in line art: a key press mid-pull in the chat, her line is put back
/// and the same frame's heave no longer pulls it out again from home; one
/// frame, nothing else changed; the trace diff is in that commit), and
/// step 8c's driver, which cuts a step at each event's own time (each
/// seed and mode first differs at a key press or a cue, at its own time
/// where it was the first step past it; the trace diff is in that
/// commit), and step 8c's stillness levers shipped (each seed and mode
/// first differs at a decision, choosing otherwise; the trace diff is in
/// that commit).
const UNFED_RESIDENT: [(u64, u64, u64); 4] = [
    (0, 0xc68d1f83436c6aa0, 0x78221d66d01b129e),
    (1, 0x31b7823432e31b21, 0x0307fe7d3c7ad87d),
    (2, 0xd20e22fee8f3d344, 0xbcdc86c480e2453e),
    (3, 0x77765beced7f5792, 0xaefc5faef1a6111d),
];

/// The furnished home's tables at the end of phase 5b step 3, but for step
/// 4b's fix to her part-time job's end (a shift cut short, out or back,
/// ends as she next decides: its trace diff is in that commit), and phase
/// 5c step 5's shorter watch (5 s after a chat line, not 15: each seed and
/// mode first differs 5 s after a line; the trace diff is in that commit),
/// and step 6's looking up in place (each seed and mode first differs at a
/// chat line during a use, where she looks up from it; with its review
/// fixes, seed 0 looks up from her homework still facing her desk, and seed
/// 3 looks up only once she has said what's wrong with her home; the trace
/// diff is in that commit), and step 8's slow blink (on a pose she holds, a
/// frame inserted as each blink begins and one as it ends; gazing, one as
/// her "ooh" goes after 3 s: the blink's wakes let a chat line or a cue,
/// which the harness delivers at the step that crosses it, land sooner,
/// and the "ooh" going is an event of hers at which she acts on what
/// changed sooner, so what follows shifts), and its review's fixes (a
/// blink's tail after her look is over no longer shows: seed 3, both
/// modes; and an easing lands on her needs as they are: every seed and
/// mode first differs at a decision, choosing otherwise; the trace diffs
/// are in that commit), and step 10a's floor acts and cross-legged TV
/// (seeds 1-3 in both modes first differ at a watch from beside the TV,
/// cross-legged where she sat hugging her knees; seed 0 at her first
/// decision with reading on her back on offer, choosing otherwise; the
/// trace diff is in that commit), and step 8c's driver, which cuts a
/// step at each event's own time (seed 1 in both modes: every frame the
/// same, her empty home's once-a-second steps re-phased at a chat line,
/// so the run's last step ends at another moment and her ledger's clock
/// reads a minute on; the trace diff is in that commit), and step 8c's
/// stillness levers shipped (each seed and mode first differs at a
/// decision, choosing otherwise; the trace diff is in that commit).
const UNFED_FURNISHED: [(u64, u64, u64); 4] = [
    (0, 0x682c2854e2fff2cd, 0x32efbb5dd79c55da),
    (1, 0xa80d35ab1e0df0a8, 0x3c295f186eea94dd),
    (2, 0x9a5ecc376ee257ed, 0xa7983878310ebd4b),
    (3, 0x7cd109c9288bed43, 0x422fbcab6a159117),
];

/// The errand's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit), and phase 5c step 1's
/// rain-out fix (the tick a focused pane's rain ends on is drawn: one
/// frame inserted at the rain's end, seeds 1 and 3 (the residents) in
/// both modes, nothing else changed; the trace diff is in that commit),
/// and step 8's quieter gaze (one frame inserted as her "ooh" goes after
/// 3 s, seeds 1 and 3 in both modes, nothing else changed; the trace
/// diff is in that commit), and step 10a's floor acts (seeds 1 and 3 in
/// both modes first differ at her first decision with reading on her back
/// on offer, choosing otherwise; the trace diff is in that commit), and
/// step 8c's driver, which cuts a step at each event's own time (each
/// seed and mode first differs at a key press, at its own time where it
/// was the first step past it; the trace diff is in that commit), and
/// step 8c's stillness levers shipped (seeds 1 and 3, the residents, in
/// both modes first differ at a decision, choosing otherwise; the trace
/// diff is in that commit).
const UNFED_ERRAND: [(u64, u64, u64); 4] = [
    (0, 0x7ef41b6f44466ee1, 0x21d600e1fc53a86a),
    (1, 0xdf91d60dbf2f6bd5, 0xe63c37ac36f0e2ca),
    (2, 0x853a5f641f5d8378, 0x95ae070706e62dc6),
    (3, 0xb298d0d13167c6a3, 0xdac720f2ade0f23e),
];

#[test]
fn golden_unfed_stage_room() {
    check_unfed("unfed stage", stage_room, &UNFED_STAGE);
}

#[test]
fn golden_unfed_resident() {
    check_unfed("unfed resident", resident, &UNFED_RESIDENT);
}

#[test]
fn golden_unfed_furnished_home() {
    check_unfed("unfed furnished", furnished, &UNFED_FURNISHED);
}

#[test]
fn golden_unfed_errand() {
    check_unfed("unfed errand", errand, &UNFED_ERRAND);
}
