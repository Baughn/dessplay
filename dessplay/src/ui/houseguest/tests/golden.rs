//! Golden trajectories: nine scenes, four seeds each, in ASCII and line
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
fn drive(
    guest: &mut Guest,
    trace: &mut Trace,
    until: u64,
    cues: &[(u64, Scene)],
    mut world: impl FnMut(u64, u64) -> (Buffer, IdleView, bool),
) {
    let mut now = 0;
    let (real, view, _) = world(0, 0);
    let frame = paint(guest, &real, &view, now);
    trace.frame(guest, &frame, &real, now);
    while now < until {
        let step = guest
            .next_tick(now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000);
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
    drive(&mut guest, &mut trace, 300_000, &cues, |now, step| {
        if arrived_within(&chats, now, step) {
            mark.synced += 1;
        }
        let view = IdleView {
            chat_mark: mark,
            ..view.clone()
        };
        (real.clone(), view, false)
    });
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
    drive(&mut guest, &mut trace, 180_000, &cues, |now, step| {
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
    });
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
    drive(&mut guest, &mut trace, 600_000, &[], |now, step| {
        if arrived_within(&chats, now, step) {
            mark.synced += 1;
        }
        let view = IdleView {
            chat_mark: mark,
            ..view.clone()
        };
        (real.clone(), view, false)
    });
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
    drive(&mut guest, &mut trace, 150_000, &[], |now, step| {
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
    });
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
    drive(&mut guest, &mut trace, 600_000, &[], |now, step| {
        if arrived_within(&chats, now, step) {
            mark.synced += 1;
        }
        let view = IdleView {
            chat_mark: mark,
            ..view.clone()
        };
        (real.clone(), view, false)
    });
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
    drive(&mut guest, &mut trace, 300_000, &[], |now, step| {
        if arrived_within(&chats, now, step) {
            mark.synced += 1;
        }
        let view = IdleView {
            chat_mark: mark,
            ..view.clone()
        };
        (real.clone(), view, false)
    });
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
    drive(&mut guest, &mut trace, 600_000, &[], |now, step| {
        if arrived_within(&chats, now, step) {
            mark.synced += 1;
        }
        let view = IdleView {
            chat_mark: mark,
            ..view.clone()
        };
        (real.clone(), view, false)
    });
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
    drive(&mut guest, &mut trace, 180_000, &[], |now, step| {
        if arrived_within(&chats, now, step) {
            mark.synced += 1;
        }
        let view = IdleView {
            chat_mark: mark,
            ..view.clone()
        };
        (real.clone(), view, false)
    });
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

#[test]
fn golden_stage_room() {
    check(
        "stage",
        stage_room,
        &[
            (0, 0xf0c8e581c411846a, 0x3dc444745d86df61),
            (1, 0xae04cfc95045af79, 0x25b0a880ce4d67e1),
            (2, 0xdce6aa718de2d517, 0xeb9ef4bebb7695b6),
            (3, 0x95b51d88911f193f, 0x3d09061717aef140),
        ],
    );
}

#[test]
fn golden_resident() {
    check(
        "resident",
        resident,
        &[
            (0, 0xe17c989309e27351, 0x22e5e646d231e359),
            (1, 0xe6d87d159971dfb3, 0x918b7cf7d48c11b7),
            (2, 0xe80789b1a3371b4d, 0x64d92d1e905bc905),
            (3, 0x7c996adf16ce6ae1, 0x54628e1921043571),
        ],
    );
}

#[test]
fn golden_furnished_home() {
    check(
        "furnished",
        furnished,
        &[
            (0, 0xce411351d40b8db0, 0x704a66641848f382),
            (1, 0xdf957f9a38ed293a, 0x36a820fdf5a2da93),
            (2, 0xaa7b2ceb0bf8b5bc, 0x68ef8c97aaef29b5),
            (3, 0xc9279ea0f38a8416, 0x44aefdc2afe74571),
        ],
    );
}

#[test]
fn golden_errand() {
    check(
        "errand",
        errand,
        &[
            (0, 0xa115268dd34741af, 0x84a48e0354dec3ca),
            (1, 0x60d4f971230730d2, 0xb92429282cd46ef0),
            (2, 0x1e3dd842f4ca4668, 0x20d410417a6fc48e),
            (3, 0x0b3934b5b2e4d1c0, 0x47f20eb30403a5d2),
        ],
    );
}

#[test]
fn golden_homework_evening() {
    check(
        "evening",
        homework_evening,
        &[
            (0, 0xc714add6b7035e9a, 0xaf1bbbbda0fa2149),
            (1, 0xd8350653eb6c8d0f, 0xf986e1a0f8174e65),
            (2, 0xad3a7e6a9cff564d, 0x5c55d0f1198d1566),
            (3, 0x857f177b56066f57, 0xf887861180fac779),
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
            (0, 0x3fb754fd56078d46, 0xa6cc921ccabf64aa),
            (1, 0x27792fa4bfae67ed, 0xe9375b7e8b8631dc),
            (2, 0x265af502895ee3f2, 0x31a531f225ae7477),
            (3, 0x951005ed7743aba5, 0x1dccf0464b05e7e7),
        ],
    );
}

#[test]
fn golden_school_morning() {
    check(
        "school",
        school_morning,
        &[
            (0, 0xb2b7d8e0e05c2bd7, 0x74112a43a0b0c060),
            (1, 0x044b15882e3f3676, 0xe51f160e52a5ce00),
            (2, 0xe29a2ca7ee7f9ab0, 0xe28753339037aa01),
            (3, 0xd87dbd5deeefd2c6, 0x1f68258f09ef2813),
        ],
    );
}

#[test]
fn golden_home_from_school() {
    check(
        "home",
        home_from_school,
        &[
            (0, 0x57528e5f189b1820, 0xae6a109bbb7a9ba9),
            (1, 0x0ceee9de88786799, 0x3cbf367270361eeb),
            (2, 0x49590f25ae93c4ea, 0x2911bbd589dbbb90),
            (3, 0x16817c2b767a8a57, 0xd23eb48164ba0ee4),
        ],
    );
}

// ---- Unfed: as she was before the clock fed her (A5) ----

/// The stage room's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit), and step 5b's parcels (one
/// waits while she's out of sight, and for her "I'm home!" from work:
/// seeds 1 and 2 in ASCII, out at work as the stage sends a parcel; the
/// trace diff is in that commit).
const UNFED_STAGE: [(u64, u64, u64); 4] = [
    (0, 0xe6077be20df9456b, 0x2b8c5fb5ebcffa8d),
    (1, 0xa208eaff8a9cc01d, 0xf2310f324a023361),
    (2, 0xbdba4006540e022b, 0x4e2fb6c3442e1d11),
    (3, 0xe2ff1e903886c076, 0x83fff4ee2141d60f),
];

/// The resident's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit).
const UNFED_RESIDENT: [(u64, u64, u64); 4] = [
    (0, 0x7500d48c817323dc, 0xe9ccfad21660aec7),
    (1, 0xc9a36f2147c27b57, 0x38bbc612ffbcfc2b),
    (2, 0x9204f86377c803b0, 0xcd7b645d6efb9997),
    (3, 0xf1fe032ad00023fe, 0x9818dfe4c8fad305),
];

/// The furnished home's tables at the end of phase 5b step 3, but for
/// step 4b's fix to her part-time job's end (a shift cut short, out or
/// back, ends as she next decides: its trace diff is in that commit).
const UNFED_FURNISHED: [(u64, u64, u64); 4] = [
    (0, 0xb1d8e0d2d3efee1b, 0x15bd5f15742432e7),
    (1, 0x91fffe61c77c6a47, 0x4b674f554f1af2ad),
    (2, 0x484952bd84f65872, 0x919783c7eaacf2b5),
    (3, 0x6203639870ff41a9, 0xdf31ce48e46e5201),
];

/// The errand's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit).
const UNFED_ERRAND: [(u64, u64, u64); 4] = [
    (0, 0xa115268dd34741af, 0x84a48e0354dec3ca),
    (1, 0x60d4f971230730d2, 0xb92429282cd46ef0),
    (2, 0x1e3dd842f4ca4668, 0x20d410417a6fc48e),
    (3, 0x07f3cf925a3cfb31, 0xf621c3073f8725cf),
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
