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
    drive(&mut guest, &mut trace, 180_000, &[], |_, _| {
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

#[test]
fn golden_stage_room() {
    check(
        "stage",
        stage_room,
        &[
            (0, 0x889dfb317bc6d79e, 0x742ca1584ccc5831),
            (1, 0x51c932ed3406365b, 0x18fcc24936e8ec76),
            (2, 0x656cab3a94977cfd, 0x88c96c8c09ce5c37),
            (3, 0x6543df01a0902857, 0xe70f7f22a9c5edbd),
        ],
    );
}

#[test]
fn golden_resident() {
    check(
        "resident",
        resident,
        &[
            (0, 0x643efaae20d19e4a, 0xe73a092722096412),
            (1, 0xf7a2bb6ab76b3741, 0x77b2d72069c6a6d1),
            (2, 0x674af2bdb4c15c49, 0x4db5dba4b01da43b),
            (3, 0x66c8dfc5555ca333, 0x063c814d6edb6e9c),
        ],
    );
}

#[test]
fn golden_furnished_home() {
    check(
        "furnished",
        furnished,
        &[
            (0, 0x90df96981a3edac9, 0x4d1b5e4062017f80),
            (1, 0x75fbb3a23c1dd4fc, 0x5d5d72a3f4409792),
            (2, 0xa15ecfed7b9234ee, 0x61d2f3c0413e928a),
            (3, 0x96e7f982f9b8379a, 0x713c9005b1bd357b),
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
            (1, 0xe0f41b1ece93757c, 0x827f3f555bf64164),
            (2, 0x1e3dd842f4ca4668, 0x20d410417a6fc48e),
            (3, 0xc2e68cc1c3988003, 0x648adf4edb47be5a),
        ],
    );
}

#[test]
fn golden_homework_evening() {
    check(
        "evening",
        homework_evening,
        &[
            (0, 0xb4332fbae54f5a66, 0xab55063da2ef38d9),
            (1, 0x1c4da6e1a6ede223, 0x183bb725795d304d),
            (2, 0x5da013d615ac9b3d, 0xef40e024300937de),
            (3, 0x19af214c6c9cc8a2, 0xc14a3ab618e513b0),
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
            (0, 0x0da0a3fc03bf9d48, 0xd525110293bb4bea),
            (1, 0x7d10ae0ea20264c5, 0xb767c244d73d55ce),
            (2, 0xcd5b19842da1d401, 0x323a8d330e59fd39),
            (3, 0x55aeb50b0bb3791f, 0xded4742945714aae),
        ],
    );
}

#[test]
fn golden_school_morning() {
    check(
        "school",
        school_morning,
        &[
            (0, 0x78ccce965e42be1f, 0x343dd7e1638e6f82),
            (1, 0x3f90b25eba234732, 0x1ff902c5b3cc9894),
            (2, 0x12714caaef695715, 0x17e75178cc58e115),
            (3, 0x263af0d80a57ee7d, 0x964ab390f8540262),
        ],
    );
}

#[test]
fn golden_home_from_school() {
    check(
        "home",
        home_from_school,
        &[
            (0, 0xf14b3b39e8dc1d28, 0xbdad2644113a5df4),
            (1, 0x7f173be224fa8e9a, 0x379014716b2c2962),
            (2, 0xfe241c5c850650f7, 0x2de579a4ca5d9a1a),
            (3, 0xd301f6e9bf18d652, 0x444d2b6ab597ea61),
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
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit), and step 5b's parcels (one
/// waits while she's out of sight, and for her "I'm home!" from work:
/// seeds 1 and 2 in ASCII, out at work as the stage sends a parcel; the
/// trace diff is in that commit), and phase 5c step 5's shorter watch
/// (5 s after a chat line, not 15: each seed and mode first differs 5 s
/// after a line; the trace diff is in that commit).
const UNFED_STAGE: [(u64, u64, u64); 4] = [
    (0, 0x74b4400889966361, 0x2e7b86ea8de9bdc2),
    (1, 0xee3764a87eb7e577, 0x1fc17c8bac4a8bfc),
    (2, 0x212e412c06111228, 0xcee815927b2945bc),
    (3, 0x51578248e94a3e52, 0x10c0af874e78d72b),
];

/// The resident's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit), and phase 5c step 1's
/// rain-out fix (the tick a focused pane's rain ends on is drawn: one
/// frame inserted at each rain's end, every seed and mode, nothing else
/// changed; the trace diff is in that commit), and step 5's shorter
/// watch (5 s after a chat line, not 15: each seed and mode first
/// differs 5 s after a line, or, a line come as she climbed, at her
/// landing past its watch; the trace diff is in that commit).
const UNFED_RESIDENT: [(u64, u64, u64); 4] = [
    (0, 0x326529d390d3a5c6, 0x82b07905bda21ae9),
    (1, 0xbd4529c24234f374, 0x99a7ee6f2a3cb2c8),
    (2, 0x72767066809dde53, 0xdb5de2f9e14cf0f6),
    (3, 0xc6feb94e7f75ca62, 0x33366fc6128385c4),
];

/// The furnished home's tables at the end of phase 5b step 3, but for
/// step 4b's fix to her part-time job's end (a shift cut short, out or
/// back, ends as she next decides: its trace diff is in that commit),
/// and phase 5c step 5's shorter watch (5 s after a chat line, not 15:
/// each seed and mode first differs 5 s after a line; the trace diff is
/// in that commit).
const UNFED_FURNISHED: [(u64, u64, u64); 4] = [
    (0, 0x36df3515457925ec, 0x767ebc4ef63dde7a),
    (1, 0xc9000edffd8c4e0f, 0xa7b6f996afabf472),
    (2, 0x3b1fb00c9647d2f1, 0x3b6462870453afcc),
    (3, 0x600c573abc6a4a32, 0x0c51d9e2e6311b6a),
];

/// The errand's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit), and phase 5c step 1's
/// rain-out fix (the tick a focused pane's rain ends on is drawn: one
/// frame inserted at the rain's end, seeds 1 and 3 (the residents) in
/// both modes, nothing else changed; the trace diff is in that commit).
const UNFED_ERRAND: [(u64, u64, u64); 4] = [
    (0, 0xa115268dd34741af, 0x84a48e0354dec3ca),
    (1, 0xe0f41b1ece93757c, 0x827f3f555bf64164),
    (2, 0x1e3dd842f4ca4668, 0x20d410417a6fc48e),
    (3, 0x11bd15f445a20504, 0x627e6797aa8f8636),
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
