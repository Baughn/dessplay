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
            State::Arriving(..) => "arriving".to_owned(),
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
/// does). Each event, and each cue, paints her there, as the client
/// draws after every input (`ui::shell`'s loop), so she sees it then,
/// not at her next wake. The census drivers do the same
/// (`census::Room::step_from`).
///
/// Within a moment it keeps the shell's order (`ui::shell`'s loop):
/// [`Guest::advance`] to the moment first, then [`Guest::activity`] for
/// a key press there, then the paint, so what fell due by that moment
/// happened before the input came
/// (`the_golden_driver_advances_her_before_input_like_the_shell`). A
/// [`Guest::cue`] keeps the same order by convention, not to match a
/// client: the shell has no cues, and the stage (`examples/houseguest.rs`)
/// cues on a key while her clock still stands at its last draw, so a cue
/// on the very moment something falls due is a tie there, either order.
///
/// It isn't the client in one way (docs/testing-strategy.md, the golden
/// driver): it doesn't paint at the shell's ~10 Hz snapshot redraws
/// during playback (the errand scenes play), a cadence the session sets,
/// not she; so what a paint does to her ([`Guest::paint`]: observing, a
/// nudge falling due, an errand's progress) comes only at the paints it
/// makes.
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
        let advanced = guest.advance(now);
        if input {
            guest.activity(now);
        }
        let cue = cues.iter().find(|&&(t, _)| t <= now && t > now - step);
        if let Some(&(_, scene)) = cue {
            guest.cue(scene);
        }
        // The client draws after every input: each event is one (a chat
        // line, text arriving, a key press, a focus change).
        let event = events.iter().any(|times| times.contains(&now));
        if advanced || input || event || cue.is_some() {
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

/// The golden driver paints her at every event's own time, as the client
/// draws after every input (`ui::shell`'s loop: a chat line, text
/// arriving, a key press or a focus change is a `UiInput`, and a draw
/// follows each), not at her next wake: in both modes, on the stage room
/// with chat lines, presses and a focus change at odd moments no wake of
/// hers would hit, and a cue.
#[test]
fn the_golden_driver_paints_at_every_event() {
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
            let presses = [7_777, 12_345];
            let focus = [23_456, 47_111];
            let cues = [(20_011, Scene::Sneeze)];
            let until = 60_000;
            let mut trace = Trace {
                hash: Fnv::new(),
                lines: Some(Vec::new()),
            };
            drive(
                &mut guest,
                &mut trace,
                until,
                &cues,
                &[&chats, &presses, &focus],
                |now, step| {
                    let mark = ChatMark {
                        synced: chats.iter().filter(|&&t| t <= now).count(),
                        ..ChatMark::default()
                    };
                    let view = IdleView {
                        chat_mark: mark,
                        ..view.clone()
                    };
                    (real.clone(), view, arrived_within(&presses, now, step))
                },
            );
            let painted: Vec<u64> = trace
                .lines
                .as_ref()
                .expect("traced")
                .iter()
                .map(|line| line.split(' ').next().unwrap().parse().unwrap())
                .collect();
            for &t in chats.iter().chain(&presses).chain(&focus) {
                assert!(
                    painted.contains(&t),
                    "{at}: no paint at the event at {t} ms"
                );
            }
        }
    }
}

/// A traced run of `drive` on Tuesday from `h:m`, a visitor on
/// [`home_screen`] with [`home_at`]'s home, the client idle but for a key
/// press at each of `presses`, and the stage's `cues`, to `until`.
/// Returns her, and her painted frames as `(ms, what she is)`.
fn tuesday_run(
    seed: u64,
    graphics: bool,
    (h, m): (u16, u16),
    until: u64,
    presses: &[u64],
    cues: &[(u64, Scene)],
) -> (Guest, Vec<(u64, String)>) {
    let (real, view) = home_screen();
    let mut guest = home_at(seed, routine::GameTime { day: 1, h, m }, graphics);
    let mut trace = Trace {
        hash: Fnv::new(),
        lines: Some(Vec::new()),
    };
    drive(
        &mut guest,
        &mut trace,
        until,
        cues,
        &[presses],
        |now, step| {
            (
                real.clone(),
                view.clone(),
                arrived_within(presses, now, step),
            )
        },
    );
    let frames = trace
        .lines
        .expect("traced")
        .iter()
        .map(|line| {
            let mut words = line.split(' ');
            let at = words.next().unwrap().parse().unwrap();
            (at, words.next().unwrap().to_owned())
        })
        .collect();
    (guest, frames)
}

/// Whether a traced frame shows her not here: her empty home, nothing, or
/// her rain.
fn not_here(what: &str) -> bool {
    matches!(what, "away" | "absent" | "leaving")
}

/// The golden driver advances her to a moment before it tells her of a
/// key press there, as the shell does (`ui::shell`'s loop advances her
/// as it takes each input, then handles it, then draws): what fell due
/// by that moment happened before the input came. It cues the stage in
/// the same order, by convention (the stage cues on a key while her
/// clock stands at its last draw, a tie at the very moment; see
/// [`drive`]). In both modes, a visitor:
/// - at the very moment school ends (12:45), out and the client idle
///   until then: her coming home is asked as it falls due, so a key
///   press at that moment finds her on her way and doesn't call it off
///   (A6: input since doesn't), where told first she'd have found the
///   client busy and her empty home rained out;
/// - at the very moment she goes out through her door for school
///   (08:15): a cue there, advanced first, finds her gone and brings her
///   in; cued first, it went to the visit ending under it.
#[test]
fn the_golden_driver_advances_her_before_input_like_the_shell() {
    for graphics in [false, true] {
        for seed in 0..2u64 {
            let at = format!("graphics={graphics} seed={seed}");
            // As school ends, from a run with no input: her first frame
            // here after her empty home.
            let (_, frames) = tuesday_run(seed, graphics, (12, 40), 120_000, &[], &[]);
            assert!(
                frames.iter().any(|(_, what)| what == "away"),
                "{at}: her home stood empty at school"
            );
            let home = frames
                .iter()
                .skip_while(|(_, what)| what != "away")
                .find(|(_, what)| !not_here(what))
                .unwrap_or_else(|| panic!("{at}: she came home from school"))
                .0;
            let (_, pressed) = tuesday_run(seed, graphics, (12, 40), home, &[home], &[]);
            let last = pressed.last().expect("painted");
            assert!(
                last.0 == home && !not_here(&last.1),
                "{at}: a key press as school ends at {home} ms calls off her coming home: {last:?}"
            );
            // As she goes out for school, from a run with no input: her
            // first frame not here after one of her here.
            let (_, frames) = tuesday_run(seed, graphics, (8, 10), 120_000, &[], &[]);
            let gone = frames
                .iter()
                .skip_while(|(_, what)| not_here(what))
                .find(|(_, what)| not_here(what))
                .unwrap_or_else(|| panic!("{at}: she went out for school"))
                .0;
            let cue = [(gone, Scene::Sneeze)];
            let (_, cued) = tuesday_run(seed, graphics, (8, 10), gone, &[], &cue);
            let last = cued.last().expect("painted");
            assert!(
                last.0 == gone && !not_here(&last.1),
                "{at}: a cue as she goes out at {gone} ms, advanced first, doesn't find her gone: {last:?}"
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
            (0, 0x82a43521ce2d85a9, 0x5324ad0ab77e294d),
            (1, 0x9ff22574a22965dc, 0x2dffbc80fe0fecf2),
            (2, 0x1968b506412a510e, 0xfddbedcec339d99b),
            (3, 0xa51ad78330ce9b7e, 0x91b65c314fc22b97),
        ],
    );
}

#[test]
fn golden_resident() {
    check(
        "resident",
        resident,
        &[
            (0, 0x7dad8ef2e34e5a7c, 0xbbf7b7857c11fc23),
            (1, 0xf193ef339b3b80c5, 0x9e3e3480ace7b635),
            (2, 0x18e391964aa079e9, 0x4a36b186f0dad3e7),
            (3, 0xc1bfe6a0a0eb5531, 0x219f8c000012e97a),
        ],
    );
}

#[test]
fn golden_furnished_home() {
    check(
        "furnished",
        furnished,
        &[
            (0, 0x3d7631b2dab10a61, 0x7639b3c5251c12f5),
            (1, 0xd50ca4ac2ab6868d, 0xd8b394fb0d7ccff2),
            (2, 0xea0954023fa2121d, 0x937d5165f32f7168),
            (3, 0x2f4fd9c8c2dd190e, 0xe90d229123649d8d),
        ],
    );
}

#[test]
fn golden_errand() {
    check(
        "errand",
        errand,
        &[
            (0, 0x2ea7fca9dab09e54, 0x62569ff3c4e80001),
            (1, 0x0189aaa246e31f89, 0x8837834bca303667),
            (2, 0x4ce68ad344556355, 0xa1b27ef17e3b691b),
            (3, 0x11862e9fc4708882, 0x4a1f23cf13c1aa17),
        ],
    );
}

#[test]
fn golden_homework_evening() {
    check(
        "evening",
        homework_evening,
        &[
            (0, 0x061838ff3f4ebb25, 0x3f9b12351b6540d2),
            (1, 0x6b94cda040511219, 0xea7f128aed37d778),
            (2, 0xc0bb663d312fc2d0, 0x252bcf86fdcf35e1),
            (3, 0xf295ea0bf24b0127, 0x3244fbf05b4664c3),
        ],
    );
}

#[test]
fn golden_tucked_in() {
    check(
        "tucked",
        tucked_in,
        &[
            (0, 0x8dcfa40b486b2ec1, 0xcbdf4149316b216b),
            (1, 0xb5e3cbaf805c521f, 0x09c0227001b2c019),
            (2, 0x6ea0823da6e1fd6d, 0xf5e87665c6e1836f),
            (3, 0x639b7670f76146e2, 0xccff93e05fcbef5e),
        ],
    );
}

/// Re-recorded in the door batch's step 4b: work goes out by her door
/// (every seed, both modes, first differs as she stops at her door's
/// spot (3, 16) on her way to work and opens it, where she walked on off
/// the screen's edge).
#[test]
fn golden_weekend() {
    check(
        "weekend",
        weekend,
        &[
            (0, 0x8abc3f341512e9fc, 0x03bd5ff97a628bee),
            (1, 0x6e71302cca23acee, 0xca1d3fb0011cff7c),
            (2, 0x7f7f1c0ce10971ce, 0xbd3545e8677a4fe3),
            (3, 0x9ab549fe0548ff43, 0x0da300f40538a4d0),
        ],
    );
}

/// Re-recorded in the door batch's step 3: her Away door stands at its
/// space by her door's wall, not where she went out (the traces first
/// differ at the first `away` frame, in its cells only). Again in step
/// 4a: she walks to her door to go out (the traces first differ at her
/// set-off, a `Walk` where her door used to open at her feet).
#[test]
fn golden_school_morning() {
    check(
        "school",
        school_morning,
        &[
            (0, 0x91075bdec21b3928, 0x848a7905623507b4),
            (1, 0x0f128be02e54c273, 0x6d547e08b421c6e2),
            (2, 0x64ed2451177b3d6f, 0xf1bc13ffb1852e11),
            (3, 0x77580f827acc6c9e, 0x805e919c3bcddff3),
        ],
    );
}

/// Re-recorded in the door batch's step 3: her Away door, and her
/// coming home, at its space (Users' left on `home_screen`, spot (3, 16)),
/// not the middle (the traces differ from the first frame).
#[test]
fn golden_home_from_school() {
    check(
        "home",
        home_from_school,
        &[
            (0, 0x57d801e451d6d919, 0x99b1d3cc8ee740da),
            (1, 0xb2309a261c8fe65b, 0xd001cf6dd53aac69),
            (2, 0xd01fad3f4e1bbbc4, 0x5a38302ec795bc8b),
            (3, 0xe0bfbe0ac513afac, 0xf3e147b040f3e30c),
        ],
    );
}

/// Re-recorded in the door batch's step 3: she dashes in out of her door
/// at its space (3, 16), not the middle (the traces differ from the first
/// frame). Again in step 4a: out again, she walks back to her door (the
/// traces first differ at her set-off, after "Forgot my lunch!").
#[test]
fn golden_dash_home() {
    check(
        "dash",
        dash_home,
        &[
            (0, 0xdda02d0adc807b20, 0x52b53bc3b5dddaa3),
            (1, 0xb2e0362c93ef47a0, 0x244cb8cbcf4b6863),
            (2, 0x68ffd923bfa9eb29, 0x51197acb6b4a7b43),
            (3, 0xa1ec79ac1f6f261d, 0x7fa41a266b0269ee),
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
/// she walked; the trace diff is in that commit), and step 12c's driver,
/// which paints her at every event (each seed and mode first differs by a
/// frame inserted at a chat line's own time, her look up at it shown then,
/// not at her next wake; the trace diff is in that commit), and the door
/// batch's step 2, her door's space (each seed and mode the same until
/// her first parcel comes in, about 170 s in: its box stands six columns
/// along, past her door's space, so she walks to it there, and what
/// follows shifts; her record saves her door's wall; the trace diff is in
/// that commit), with its review's fixes (seeds 0 and 1 in ASCII and 1
/// and 2 in line art: one frame each, at a line she says on after her
/// look up, which now shows on a frame past her look's end; nothing
/// else changed; the trace diff is in that commit), and the door batch's
/// step 4b, work through her door (seed 0, line art: walking to work she
/// opens her door at (96, 8) where she walked on off the screen's edge;
/// seeds 1 and 3, ASCII: her way to her door on another floor is a door
/// in space, and she comes out of it there and goes on through her door,
/// where before she went to work through a door where she stood (the run
/// ended in its gap); seed 1, line art: her way to work cut short, the
/// shift's length drawn as she set off moved a later beat's timing; the
/// rest unchanged; the trace diff is in that commit).
const UNFED_STAGE: [(u64, u64, u64); 4] = [
    (0, 0x708733793f400284, 0x941bf0a431f5b066),
    (1, 0xa4a798e2a1a79945, 0x51c67cb9d4c2829e),
    (2, 0xec15f92e2f11cc74, 0xf73152a21a71ba81),
    (3, 0xaf26d1f42948b458, 0xc45aa53bb13f470a),
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
/// that commit), and step 12c's driver, which paints her at every event
/// (seeds 0 to 2 in both modes first differ by a frame inserted at a chat
/// line's or text's own time, her look up at it shown then, not at her
/// next wake; seed 3 in both modes by one at a focus change, 33915 ms;
/// the trace diff is in that commit), and the door batch's step 2, her
/// door's space (each seed and mode first differs at its first painted
/// frame, her sofa or TV standing six columns along from her door's
/// space; what she does follows from there; her record saves her door's
/// wall; the trace diff is in that commit).
const UNFED_RESIDENT: [(u64, u64, u64); 4] = [
    (0, 0xae83572b8f74f265, 0x85d8066280988717),
    (1, 0xa27439f50cffa068, 0x4434802f6b7eaeeb),
    (2, 0xec6ab8140f10edef, 0xa35e3e8936345270),
    (3, 0xee91647cff7780b8, 0x409e2de896d6fed3),
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
/// decision, choosing otherwise; the trace diff is in that commit), and
/// step 12c's driver, which paints her at every event (each seed and mode
/// first differs by a frame inserted at a chat line's own time, her look
/// up at it shown then, not at her next wake; the trace diff is in that
/// commit), and the door batch's step 2, her door's space (each seed and
/// mode first differs at its first frame, the pieces by her door's wall
/// standing six columns along; what she does follows from there; her
/// record saves her door's wall; the trace diff is in that commit), with
/// its review's fixes (each seed and mode first differs at its first
/// frame, in the cells only: a stage gift is judged where it's laid, so
/// her pieces stand elsewhere; the trace diff is in that commit), and the
/// door batch's step 4b, work through her door (seed 0, both modes: on
/// her way to work she stops at her door's spot (3, 16) and goes through
/// it, where she walked on to the screen's edge; seed 1, both modes:
/// her way to work cut by a chat line, the shift's length is now drawn
/// as she sets off, so the stream that times her standing blinks moved,
/// first showing on the frame after the look (a blink no longer shown);
/// seeds 2 and 3 unchanged; the trace diff is in that commit).
const UNFED_FURNISHED: [(u64, u64, u64); 4] = [
    (0, 0xd68c121f21096143, 0xe5248391f31579c5),
    (1, 0xe11d39f391f8b0a6, 0xc6712cdb6f6c3b91),
    (2, 0x926059ace43e7151, 0x9d52a9c8d4e177e1),
    (3, 0x79407de0d1328d6f, 0xcc2d79b8d69e779b),
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
/// diff is in that commit), and step 12c's driver, which paints her at
/// every event (each seed and mode first differs by a frame inserted at an
/// event's own time: seeds 0 and 2 while she's still away, seeds 1 and 3
/// with her in the room, her answer or her step shown then, not at her
/// next wake; the trace diff is in that commit).
const UNFED_ERRAND: [(u64, u64, u64); 4] = [
    (0, 0x2ea7fca9dab09e54, 0x62569ff3c4e80001),
    (1, 0x0189aaa246e31f89, 0x8837834bca303667),
    (2, 0x4ce68ad344556355, 0xa1b27ef17e3b691b),
    (3, 0x464bd292b9257dd1, 0xbd9bbe268ca64baf),
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
