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
            (0, 0x7f83fec767e74862, 0x1794b93ffc8d66d8),
            (1, 0xd09959b194fcc713, 0x8536be75145c26d8),
            (2, 0x9591c00017d66914, 0x79e04ffa0cf74262),
            (3, 0x67269c72721ff0fb, 0xd0481ce9d956912a),
        ],
    );
}

#[test]
fn golden_resident() {
    check(
        "resident",
        resident,
        &[
            (0, 0xe4ea2684cb927940, 0xc5924ae43095cb89),
            (1, 0x4d55b70cce594b8e, 0x38712f820f2c9484),
            (2, 0x950e8274aad0baa5, 0x19a6e8a505632e3d),
            (3, 0x508efaa9039319cb, 0x2f85593bf91dd66c),
        ],
    );
}

#[test]
fn golden_furnished_home() {
    check(
        "furnished",
        furnished,
        &[
            (0, 0x78e58429f3a27a2a, 0xfa72b74bcd513170),
            (1, 0xd9dd7e616e7d6dd1, 0x2aabb1351b20e89b),
            (2, 0xe7ac43051b49c57a, 0xe0a6bf4d2bc40e77),
            (3, 0x195e3cf9e6f66b02, 0x4a1f34cf3a57f2d0),
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
            (1, 0x25af335efba3a77b, 0xa19648a98946d58e),
            (2, 0x1e3dd842f4ca4668, 0x20d410417a6fc48e),
            (3, 0x7b72e3b19661f820, 0x80cd5be7de17a456),
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
            (1, 0x2b7d28f3784a5dc7, 0x2cb59310a593e59c),
            (2, 0x774073234f16be7e, 0xce0d3b4cbec3c985),
            (3, 0x0e940ff5e6294baa, 0x256afa3dcadf273e),
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
            (0, 0xc78a2230b908b857, 0x0121e3d716edbf8b),
            (1, 0xe6f13b1ffa672c63, 0x861f622b965f241f),
            (2, 0x2c94f7654da230f2, 0x324a1c03b6ca5b35),
            (3, 0xd99eff5817f454a4, 0xdf66e086a8fdac40),
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
            (3, 0x457e0f12b590c556, 0xe54342c5fcee6435),
        ],
    );
}

#[test]
fn golden_home_from_school() {
    check(
        "home",
        home_from_school,
        &[
            (0, 0x000795126138beb6, 0xb650bf0ab5ebd845),
            (1, 0x9e3cf147d29514ae, 0x6ea3e6ed9619c3f9),
            (2, 0xfe06d9555e35472d, 0x5875f8f39085d2ff),
            (3, 0xcdd830678a84442f, 0x684bcfe2aaf4e49b),
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
/// decision, choosing otherwise; the trace diffs are in that commit).
const UNFED_STAGE: [(u64, u64, u64); 4] = [
    (0, 0x6ced8c8ab211dfbd, 0xa560392316f63baa),
    (1, 0xa51be88372d650ee, 0xf080a21cdf07145a),
    (2, 0x385a1a6f622c5d83, 0x079d7aabf37478de),
    (3, 0x803711c1f86db531, 0x4443c8c3c14e3f6b),
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
/// in that commit).
const UNFED_RESIDENT: [(u64, u64, u64); 4] = [
    (0, 0x03f9b708a79947a0, 0xef873c49a1716468),
    (1, 0xac73d82a1936dac7, 0xe524039d4d473c83),
    (2, 0x982f046e42548829, 0xd7697fc6058fdd89),
    (3, 0xfdae7253a9775155, 0xc9363c4d2ac5acda),
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
/// are in that commit).
const UNFED_FURNISHED: [(u64, u64, u64); 4] = [
    (0, 0x5195e1f31ed5dd66, 0xe138dd7fdb0fac4c),
    (1, 0xc688b8e0180077b0, 0xb973162ad46b24d8),
    (2, 0x46335ed7766b40ed, 0x7581949368fe0ba6),
    (3, 0x64786cea82132375, 0xbb5bdf25945d10b2),
];

/// The errand's tables at the end of phase 5b step 3, but for step 5a's
/// hidden-goodbye fix (no placement recorded of her hidden behind her
/// door: its trace diff is in that commit), and phase 5c step 1's
/// rain-out fix (the tick a focused pane's rain ends on is drawn: one
/// frame inserted at the rain's end, seeds 1 and 3 (the residents) in
/// both modes, nothing else changed; the trace diff is in that commit),
/// and step 8's quieter gaze (one frame inserted as her "ooh" goes after
/// 3 s, seeds 1 and 3 in both modes, nothing else changed; the trace
/// diff is in that commit).
const UNFED_ERRAND: [(u64, u64, u64); 4] = [
    (0, 0xa115268dd34741af, 0x84a48e0354dec3ca),
    (1, 0x25af335efba3a77b, 0xa19648a98946d58e),
    (2, 0x1e3dd842f4ca4668, 0x20d410417a6fc48e),
    (3, 0xc7e40f76fbc03603, 0xb020c495bc09f166),
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
