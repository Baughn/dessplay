//! Osaka herself: where she is, what she's doing, and when that next
//! changes. Every activity is a finite, wall-clock-timed step, so any
//! instant is a safe place to cut the visit short.

use super::Rng;
use super::art::DoorFrame;
use super::brain::{self, Kind, Need, Needs};
use super::layer::Placed;
use super::room::{Furniture, Seat, Use};
use super::scenes::{Job, LayerOp, Side};
use super::sprite::{self, Face, Facing, HEIGHT, Pose, SpriteCell};
use super::terrain::{Link, Route, Terrain};

/// What the current frame offers her beyond walking around.
#[derive(Clone, Debug, Default)]
pub(super) struct Chances {
    /// Lines she could pull.
    pub pulls: Vec<super::scenes::Pull>,
    /// Letters she could swap.
    pub swaps: Vec<super::scenes::Swap>,
    /// Glyphs a sneeze where she stands would knock loose.
    pub loose: Vec<(u16, u16)>,
    /// Her furniture, and where she'd go to use it.
    pub seats: Vec<Seat>,
    /// What the shopping channel would sell her, were she to watch now.
    pub advert: Option<Furniture>,
    /// She has a home (to leave for work, and come back to).
    pub furnished: bool,
}

/// Something she did to her home (the guest keeps the record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HomeEvent {
    /// Bought off the shopping channel: on order from this moment (the
    /// scene's start), whatever interrupts it.
    Bought(Furniture),
    /// Out of its box.
    Unpacked(Furniture),
}

impl Chances {
    fn offers(&self, job: &Job) -> bool {
        match job {
            Job::Pull(p) => self.pulls.contains(p),
            Job::Swap(s) => self.swaps.contains(s),
            Job::Use(seat) => self.seats.contains(seat),
        }
    }
}

/// One brace-and-heave cycle, stretched for longer lines.
fn heave_ms(glyphs: usize) -> u64 {
    600 * (40 + glyphs as u64) / 40
}

/// Milliseconds per cell walked (3 cells/s: dreamy, not brisk).
const WALK_MS: u64 = 333;
/// Milliseconds per row climbed.
const CLIMB_MS: u64 = 500;
/// Gravity in rows/s² — honest for a four-row-tall person.
const GRAVITY: f64 = 28.0;
/// How long she lies dazed after a real fall.
const DAZED_MS: u64 = 1500;
/// How long she peers over an edge.
const PEER_MS: u64 = 1200;
/// "!" then "?" when a chat message arrives.
const SURPRISED_MS: u64 = 1200;
const LOOK_MS: u64 = 4000;
/// A conversation keeps her watching until it's been quiet this long.
const WATCH_MS: u64 = 60_000;
const BLINK_MS: u64 = 150;
/// Reaching for two letters (and back again).
const FIDDLE_MS: u64 = 700;
/// How long a swap stays before she swaps it back.
const SWAP_KEPT_MS: (u64, u64) = (7000, 14_000);
/// "a... a..." before the sneeze, then the recoil.
const WINDUP_MS: u64 = 1400;
const RECOIL_MS: u64 = 600;
/// Knocked glyphs drop a row this often, at most `FALL_ROWS` rows.
const DROP_MS: u64 = 90;
const FALL_ROWS: u64 = 4;
/// After a sneeze: a moment's "...", then one glyph back per beat.
const OOPS_MS: u64 = 900;
const PUT_BACK_MS: u64 = 400;
/// Choices remembered for the cooldown.
const RECENT: usize = 3;
/// A refused put-back is retried this many times.
const RETRIES: u8 = 5;

/// What she says on first finding her feet.
const GREETING: &str = "Nice to meet you.";
/// After a hard landing.
const OK: &str = "...I'm OK.";
/// Things she says when spacing out (each ≤ 24 characters).
pub(super) const MUSINGS: [&str; 12] = [
    "I wish I were a bird.",
    "Why is the sky blue?",
    "Sata andagi!",
    "Melon bread...",
    "Black spots on white?",
    "Or white on black...",
    "Escalator? Elevator?",
    "Feels like I could fly.",
    "Which hand's left...",
    "Chiyo-chan's dad...",
    "Nanja-kora.",
    "Oh my gah.",
];

/// How long she keeps saying `text`.
fn speech_ms(text: &str) -> u64 {
    1200 + 60 * text.chars().count() as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Stand {
        until: u64,
    },
    SpaceOut {
        until: u64,
    },
    Walk {
        to: i32,
        then: Option<Link>,
    },
    Peer {
        until: u64,
        then: Option<Link>,
    },
    Climb {
        to_y: i32,
    },
    Fall {
        from_y: i32,
        since: u64,
        to_y: i32,
    },
    Dazed {
        until: u64,
    },
    Look {
        surprised_until: u64,
        until: u64,
    },
    /// Pulling `task`: bracing, or heaving (stepping back with the line).
    Pull {
        offset: u16,
        goal: u16,
        heaving: bool,
    },
    /// Done pulling: a pleased moment.
    Admire {
        until: u64,
    },
    /// Reaching for the letters of a swap (`back`: to undo it).
    Swap {
        until: u64,
        back: bool,
    },
    /// Giggling at a swap she made; she undoes it at `revert`.
    Giggle {
        until: u64,
        revert: u64,
    },
    /// Whistling, looking anywhere but at the swapped letters.
    Innocent {
        until: u64,
        revert: u64,
    },
    /// A sneeze: the windup, then (`knocked`) the recoil.
    Sneeze {
        since: u64,
        knocked: bool,
    },
    /// Picking up what the sneeze knocked loose.
    PutBack {
        since: u64,
        until: u64,
    },
    /// An activity on the spot.
    Idle {
        what: Activity,
        since: u64,
        until: u64,
    },
    /// Clambering over a divider: over to `column`, along the pole to
    /// `to_y`, then over to `to_x` on the new floor.
    Clamber {
        column: i32,
        to_y: i32,
        to_x: i32,
    },
    /// Walking off the screen to `to`, to come back in at `enter`.
    Out {
        to: i32,
        enter: i32,
        to_y: i32,
        to_x: i32,
    },
    /// Off screen until `until`; then in from `enter`, to `to_x`.
    Away {
        until: u64,
        enter: i32,
        to_y: i32,
        to_x: i32,
    },
    /// Through a door in space from where she stands to `to` (see
    /// [`DOOR`]), away for `gap` ms between the doors.
    Door {
        since: u64,
        to: (i32, i32),
        gap: u64,
    },
    /// Back from work with her shopping.
    Home {
        until: u64,
    },
    /// Using a piece of her furniture (the task is its seat). Watching
    /// the TV, `advert` is what the shopping channel is selling her.
    Use {
        what: Use,
        since: u64,
        until: u64,
        advert: Option<Furniture>,
    },
}

/// One beat of going through a door: the door (if shown), whether she
/// is, whether it's the far end yet, and for how long.
struct DoorBeat {
    door: Option<DoorFrame>,
    her: bool,
    there: bool,
    ms: u64,
}

/// A door appears, she steps through, it shuts and goes; a door appears
/// where she's going, she steps out, it shuts and goes.
const DOOR: [DoorBeat; 13] = [
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: true,
        there: false,
        ms: 600,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: true,
        there: false,
        ms: 300,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: true,
        there: false,
        ms: 700,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: false,
        there: false,
        ms: 400,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: false,
        there: false,
        ms: 250,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: false,
        there: false,
        ms: 350,
    },
    DoorBeat {
        door: None,
        her: false,
        there: false,
        ms: 600,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: false,
        there: true,
        ms: 400,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: false,
        there: true,
        ms: 250,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: false,
        there: true,
        ms: 350,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: true,
        there: true,
        ms: 600,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: true,
        there: true,
        ms: 300,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: true,
        there: true,
        ms: 400,
    },
];

/// The door beat `elapsed` ms in, and when the next begins; `None` once
/// it's over. `gap` stretches the time between the doors (she's away).
fn door_beat(elapsed: u64, gap: u64) -> Option<(&'static DoorBeat, u64)> {
    let mut end = 0;
    DOOR.iter().find_map(|beat| {
        end += if beat.door.is_none() {
            beat.ms.max(gap)
        } else {
            beat.ms
        };
        (elapsed < end).then_some((beat, end))
    })
}

/// She goes to work after this long into a visit, at the earliest.
const WORK_AFTER_MS: u64 = 3 * 60_000;
/// How long a shift lasts (ms range).
const SHIFT_MS: (u64, u64) = (60_000, 180_000);
/// Back from work, showing what she brought.
const HOME_MS: u64 = 3000;
/// What she says, back from work.
const HOME: &str = "I'm home!";

/// What she says stepping out of a door.
const THROUGH: &str = "Where was I?";

/// The fridge stands open this long at the start of a snack.
pub(super) const FRIDGE_OPEN_MS: u64 = 1500;

/// When the cat bites, into a petting that lasts `length` ms.
pub(super) fn bite_at(length: u64) -> u64 {
    length * 7 / 10
}

/// How long she keeps at `what` (ms range).
fn use_duration(what: Use) -> (u64, u64) {
    match what {
        Use::Lounge => (15_000, 30_000),
        Use::Nap => (30_000, 60_000),
        Use::Sleep => (60_000, 180_000),
        Use::Homework => (30_000, 60_000),
        Use::Watch => (20_000, 45_000),
        Use::Unpack => (4_000, 6_000),
        Use::Read => (20_000, 40_000),
        Use::Snack => (6_000, 9_000),
        Use::Pet => (6_000, 9_000),
    }
}

/// Animation frame period for `what`.
const USE_FRAME_MS: u64 = 1400;

/// How she looks `elapsed` ms into `what`, which lasts `length` ms
/// (with `advert` on the TV).
fn use_look(
    what: Use,
    advert: Option<Furniture>,
    elapsed: u64,
    length: u64,
) -> (Pose, Face, Option<Bubble>) {
    let frame = (elapsed / USE_FRAME_MS % 2) as u8;
    match what {
        Use::Lounge => (Pose::Lounge, Face::Vacant, None),
        Use::Nap => (Pose::Nap(frame), Face::Blink, Some(Bubble::Zzz)),
        Use::Sleep => (Pose::Sleep(frame), Face::Blink, Some(Bubble::Zzz)),
        // Writing for the first half, then nodding off onto the paper.
        Use::Homework => {
            if elapsed < length / 2 {
                (Pose::Homework(frame), Face::Vacant, None)
            } else if elapsed < length * 3 / 4 {
                (Pose::Homework(2), Face::Blink, Some(Bubble::Dots))
            } else {
                (Pose::Homework(3), Face::Blink, Some(Bubble::Zzz))
            }
        }
        Use::Watch => match advert {
            // Hooked, then sold.
            Some(_) if elapsed < length * 2 / 5 => (Pose::Sit, Face::Curious, Some(Bubble::Ooh)),
            Some(item) if elapsed < length * 3 / 5 => {
                (Pose::Sit, Face::Happy, Some(Bubble::Say(item.pitch())))
            }
            _ => (Pose::Sit, Face::Curious, None),
        },
        Use::Read => (Pose::Read(frame), Face::Vacant, None),
        // A look in the fridge, then the melon bread.
        Use::Snack if elapsed < FRIDGE_OPEN_MS => (Pose::Side, Face::Curious, None),
        Use::Snack => (Pose::Eat(frame), Face::Happy, None),
        // Petting the cat, who has had quite enough.
        Use::Pet if elapsed < bite_at(length) => (Pose::Pet(0), Face::Happy, Some(Bubble::Hum)),
        Use::Pet => (Pose::Pet(1), Face::Surprised, Some(Bubble::Say("Ow!"))),
        // Bent over the box, rummaging.
        Use::Unpack => {
            let bubble = (elapsed > length * 3 / 5).then_some(Bubble::Ooh);
            (Pose::ToeTouch(frame), Face::Happy, bubble)
        }
    }
}

/// Something to do on the spot that isn't staring at the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Activity {
    Sit,
    LieBack,
    LieFront,
    Jacks,
    ToeTouch,
    Stretch,
    Gaze,
}

impl Activity {
    pub const ALL: [Activity; 7] = [
        Self::Sit,
        Self::LieBack,
        Self::LieFront,
        Self::Jacks,
        Self::ToeTouch,
        Self::Stretch,
        Self::Gaze,
    ];

    /// How long she keeps at it (ms range).
    fn duration(self) -> (u64, u64) {
        match self {
            Self::Sit => (10_000, 25_000),
            Self::LieBack => (15_000, 40_000),
            Self::LieFront => (10_000, 25_000),
            Self::Jacks => (4_000, 8_000),
            Self::ToeTouch => (5_000, 9_000),
            Self::Stretch => (2_000, 4_000),
            Self::Gaze => (4_000, 10_000),
        }
    }

    /// Animation frame period; 0 for a held pose.
    fn period(self) -> u64 {
        match self {
            Self::LieBack => 1400,
            Self::LieFront => 500,
            Self::Jacks => 450,
            Self::ToeTouch => 900,
            Self::Sit | Self::Stretch | Self::Gaze => 0,
        }
    }

    fn look(self, frame: u8) -> (Pose, Face, Option<Bubble>) {
        match self {
            Self::Sit => (Pose::Sit, Face::Vacant, None),
            Self::LieBack => (Pose::LieBack(frame), Face::Blink, Some(Bubble::Zzz)),
            Self::LieFront => (Pose::LieFront(frame), Face::Happy, Some(Bubble::Hum)),
            Self::Jacks => (Pose::Jack(frame), Face::Happy, Some(Bubble::Count)),
            Self::ToeTouch => (Pose::ToeTouch(frame), Face::Vacant, None),
            Self::Stretch => (Pose::Stretch, Face::Blink, Some(Bubble::Stretch)),
            Self::Gaze => (Pose::Gaze, Face::Curious, Some(Bubble::Ooh)),
        }
    }
}

/// A speech or thought bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bubble {
    Dots,
    Bang,
    Huh,
    Hehe,
    Zzz,
    Hum,
    Count,
    Stretch,
    Ooh,
    Achoo,
    Chu,
    /// Something she says (≤ 24 characters).
    Say(&'static str),
}

impl Bubble {
    pub fn text(self) -> &'static str {
        match self {
            Self::Dots => "...",
            Self::Bang => "!",
            Self::Huh => "?",
            Self::Hehe => "hehe",
            Self::Zzz => "zzz",
            Self::Hum => "~",
            Self::Count => "1, 2!",
            Self::Stretch => "nnn~",
            Self::Ooh => "ooh",
            Self::Achoo => "a...",
            Self::Chu => "chu!",
            Self::Say(text) => text,
        }
    }
}

/// The houseguest.
#[derive(Clone, Debug)]
pub(super) struct Osaka {
    /// Anchor column (her middle).
    pub x: i32,
    /// The floor row she stands on; her body is the rows above it.
    pub y: i32,
    pub facing: Facing,
    act: Act,
    /// When the current act next changes her pose.
    act_due: u64,
    next_blink: u64,
    blink_until: u64,
    /// While a chat conversation continues she stands watching it.
    watch_until: u64,
    watch_x: i32,
    /// A job she's walking to on this floor, or doing.
    task: Option<Job>,
    /// A job on another floor she's making her way towards.
    goal: Option<Job>,
    /// The pole she's climbing (column).
    pole: i32,
    /// Layer changes for the next paint to apply.
    ops: Vec<LayerOp>,
    /// Changes to her home for the guest to record.
    events: Vec<HomeEvent>,
    /// Layer changes due later, whatever she's doing by then: undoing
    /// mischief is scheduled when it's made, so nothing can strand it.
    pending: Vec<(u64, LayerOp)>,
    /// What she's saying, and until when (over her act's own bubble).
    speech: Option<(&'static str, u64)>,
    /// She has said hello (or "I'm OK", which does as well).
    greeted: bool,
    /// When this visit began.
    arrived: u64,
    /// Out at her part-time job (or on her way there or back).
    at_work: bool,
    /// She's been to work this visit (once is plenty).
    worked: bool,
    needs: Needs,
    /// Her last few choices (repeating herself is discouraged).
    recent: Vec<Kind>,
    /// When she last chose.
    decided: u64,
    /// Every choice she made (tests read it).
    #[cfg(test)]
    pub choices: Vec<Kind>,
}

impl Osaka {
    fn new(x: i32, y: i32, facing: Facing, act: Act, now: u64, rng: &mut Rng) -> Self {
        let mut osaka = Self {
            x,
            y,
            facing,
            act,
            act_due: now,
            next_blink: now + rng.range(4000, 9000),
            blink_until: 0,
            watch_until: 0,
            watch_x: x,
            task: None,
            goal: None,
            pole: x,
            ops: Vec::new(),
            events: Vec::new(),
            pending: Vec::new(),
            speech: None,
            greeted: false,
            arrived: now,
            at_work: false,
            worked: false,
            needs: Needs::default(),
            recent: Vec::new(),
            decided: now,
            #[cfg(test)]
            choices: Vec::new(),
        };
        osaka.act_due = osaka.first_due(now);
        osaka
    }

    /// She enters: walking in from a screen edge a floor reaches, or
    /// dropping in from above onto a platform with a clear fall. `None`
    /// when there's nowhere to be.
    pub fn arrive(now: u64, terrain: &Terrain, width: i32, rng: &mut Rng) -> Option<Self> {
        let entries: Vec<(i32, i32, Facing, i32)> =
            terrain
                .platforms
                .iter()
                .flat_map(|p| {
                    let target = p.x0 + rng.below((p.x1 - p.x0 + 1) as u64) as i32;
                    let left = (p.edge_left && p.x0 <= sprite::WIDTH / 2).then_some((
                        -sprite::WIDTH,
                        p.y,
                        Facing::Right,
                        target,
                    ));
                    let right = (p.edge_right && p.x1 >= width - 1 - sprite::WIDTH / 2)
                        .then_some((width + sprite::WIDTH, p.y, Facing::Left, target));
                    [left, right]
                })
                .flatten()
                .collect();
        if !entries.is_empty() && rng.below(2) == 0 {
            let pick = rng.below(entries.len() as u64) as usize;
            let (x, y, facing, to) = *entries.get(pick)?;
            tracing::trace!(x, y, to, "houseguest walks in");
            return Some(Self::new(
                x,
                y,
                facing,
                Act::Walk { to, then: None },
                now,
                rng,
            ));
        }
        for _ in 0..16 {
            let pick = rng.below(terrain.platforms.len().max(1) as u64) as usize;
            let platform = terrain.platforms.get(pick)?;
            let x = platform.x0 + rng.below((platform.x1 - platform.x0 + 1) as u64) as i32;
            if terrain.landing(x, 0) == Some(pick) {
                tracing::trace!(x, y = platform.y, "houseguest drops in");
                let act = Act::Fall {
                    from_y: 0,
                    since: now,
                    to_y: platform.y,
                };
                return Some(Self::new(x, 0, Facing::Right, act, now, rng));
            }
        }
        if let Some(&(x, y, facing, to)) = entries.first() {
            return Some(Self::new(
                x,
                y,
                facing,
                Act::Walk { to, then: None },
                now,
                rng,
            ));
        }
        // Nowhere to walk in from or drop onto: she's simply there,
        // blinking, as if she'd been home all along.
        let pick = rng.below(terrain.platforms.len() as u64) as usize;
        let platform = terrain.platforms.get(pick)?;
        let x = platform.x0 + rng.below((platform.x1 - platform.x0 + 1) as u64) as i32;
        let until = now + rng.range(2000, 5000);
        Some(Self::new(
            x,
            platform.y,
            Facing::Right,
            Act::Stand { until },
            now,
            rng,
        ))
    }

    /// Test fixture: standing at `(x, y)`, about to decide.
    #[cfg(test)]
    pub fn standing_at(x: i32, y: i32, now: u64, rng: &mut Rng) -> Self {
        Self::new(
            x,
            y,
            Facing::Right,
            Act::Stand { until: now + 100 },
            now,
            rng,
        )
    }

    /// Test fixture: reach for `swap` now (she must stand at its spot).
    #[cfg(test)]
    pub fn swap_now(&mut self, swap: super::scenes::Swap, now: u64) {
        self.facing = side_facing(swap.side);
        self.task = Some(Job::Swap(swap));
        self.set(
            Act::Swap {
                until: now + FIDDLE_MS,
                back: false,
            },
            now,
        );
    }

    /// Whether any layer change is still queued.
    #[cfg(test)]
    pub fn owes_anything(&self) -> bool {
        self.owes()
    }

    /// Start a sneeze now.
    pub fn sneeze_now(&mut self, now: u64) {
        self.set(
            Act::Sneeze {
                since: now,
                knocked: false,
            },
            now,
        );
    }

    fn first_due(&self, now: u64) -> u64 {
        match self.act {
            Act::Stand { until }
            | Act::SpaceOut { until }
            | Act::Peer { until, .. }
            | Act::Dazed { until }
            | Act::Admire { until }
            | Act::Swap { until, .. }
            | Act::Giggle { until, .. }
            | Act::Innocent { until, .. }
            | Act::PutBack { until, .. }
            | Act::Home { until } => until,
            Act::Sneeze { since, knocked } => {
                since + WINDUP_MS + if knocked { RECOIL_MS } else { 0 }
            }
            Act::Idle { what, since, until } => next_frame(what, since, now).min(until),
            Act::Use { since, until, .. } => {
                (since + (now.saturating_sub(since) / USE_FRAME_MS + 1) * USE_FRAME_MS).min(until)
            }
            Act::Clamber { column, to_y, .. } => {
                if self.x == column && self.y != to_y {
                    now + CLIMB_MS
                } else {
                    now + WALK_MS
                }
            }
            Act::Out { .. } => now + WALK_MS,
            Act::Away { until, .. } => until,
            Act::Door { since, gap, .. } => {
                door_beat(now.saturating_sub(since), gap).map_or(now, |(_, end)| since + end)
            }
            Act::Look {
                surprised_until, ..
            } => surprised_until,
            Act::Walk { .. } => now + WALK_MS,
            Act::Pull { .. } => {
                let glyphs = match &self.task {
                    Some(Job::Pull(t)) => t.cells.len(),
                    _ => 0,
                };
                now + heave_ms(glyphs) / 2
            }
            Act::Climb { .. } => now + CLIMB_MS,
            Act::Fall { from_y, since, .. } => fall_time(since, (self.y - from_y + 1) as u64),
        }
    }

    /// When her pose, speech, or the text layer next changes.
    pub fn due(&self) -> u64 {
        let hush = self.speech.map_or(u64::MAX, |(_, until)| until);
        self.pose_due().min(self.pending_due()).min(hush)
    }

    /// Say `text` for a while, over whatever bubble her act shows.
    pub fn say(&mut self, text: &'static str, now: u64) {
        tracing::debug!(text, "houseguest says");
        self.speech = Some((text, now + speech_ms(text)));
    }

    /// Space out, maybe saying one of her musings first.
    pub fn muse(&mut self, now: u64, rng: &mut Rng) {
        let line = MUSINGS.get(rng.below(MUSINGS.len() as u64) as usize);
        if let Some(line) = line {
            self.say(line, now);
        }
        self.set(
            Act::SpaceOut {
                until: now + rng.range(6000, 14_000),
            },
            now,
        );
    }

    fn pose_due(&self) -> u64 {
        let blinking = matches!(self.act, Act::Stand { .. });
        if blinking {
            let blink = if self.blink_until > self.next_blink {
                self.blink_until
            } else {
                self.next_blink
            };
            self.act_due.min(blink)
        } else {
            self.act_due
        }
    }

    fn pending_due(&self) -> u64 {
        self.pending
            .iter()
            .map(|(due, _)| *due)
            .min()
            .unwrap_or(u64::MAX)
    }

    fn schedule(&mut self, due: u64, op: LayerOp) {
        self.pending.push((due, op));
    }

    /// Whether some mischief is still waiting to be undone.
    fn owes(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Run every event due by `now`. Returns whether her pose changed.
    pub fn tick(&mut self, now: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) -> bool {
        let mut changed = false;
        for _ in 0..64 {
            let due = self.due();
            if due > now {
                return changed;
            }
            changed = true;
            if let Some((_, until)) = self.speech
                && until == due
            {
                self.speech = None;
                continue;
            }
            if self.pending_due() == due {
                // Every op due now, in the order they were scheduled.
                let (now_due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
                    .into_iter()
                    .partition(|(d, _)| *d == due);
                self.pending = later;
                self.ops.extend(now_due.into_iter().map(|(_, op)| op));
                continue;
            }
            if matches!(self.act, Act::Stand { .. }) && due < self.act_due {
                if self.blink_until > self.next_blink {
                    self.blink_until = 0;
                } else {
                    self.blink_until = due + BLINK_MS;
                    self.next_blink = due + rng.range(4000, 9000);
                }
                continue;
            }
            self.fire(due, terrain, chances, rng);
        }
        // Far behind (a suspended laptop): resume from now.
        self.act_due = self.act_due.max(now);
        self.next_blink = self.next_blink.max(now);
        changed
    }

    fn set(&mut self, act: Act, at: u64) {
        tracing::trace!(?act, x = self.x, y = self.y, "houseguest act");
        self.act = act;
        self.act_due = self.first_due(at);
    }

    fn fire(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        match self.act {
            Act::Idle { what, since, until } => {
                if at >= until {
                    self.decide(at, terrain, chances, rng);
                } else {
                    self.act_due = next_frame(what, since, at).min(until);
                }
            }
            Act::Clamber { column, to_y, to_x } => {
                if self.y != to_y && self.x != column {
                    self.x += (column - self.x).signum();
                } else if self.y != to_y {
                    self.y += (to_y - self.y).signum();
                } else if self.x != to_x {
                    self.x += (to_x - self.x).signum();
                }
                if (self.x, self.y) == (to_x, to_y) {
                    self.set(Act::Stand { until: at + 800 }, at);
                } else {
                    self.act_due = self.first_due(at);
                }
            }
            Act::Out {
                to,
                enter,
                to_y,
                to_x,
            } => {
                self.x += (to - self.x).signum();
                if self.x == to {
                    tracing::debug!(work = self.at_work, "houseguest: stepped out");
                    let away = if self.at_work {
                        rng.range(SHIFT_MS.0, SHIFT_MS.1)
                    } else {
                        rng.range(4000, 12_000)
                    };
                    self.set(
                        Act::Away {
                            until: at + away,
                            enter,
                            to_y,
                            to_x,
                        },
                        at,
                    );
                } else {
                    self.act_due = at + WALK_MS;
                }
            }
            Act::Away {
                enter, to_y, to_x, ..
            } => {
                tracing::debug!("houseguest: back");
                self.x = enter;
                self.y = to_y;
                self.facing = toward(enter, to_x);
                self.set(
                    Act::Walk {
                        to: to_x,
                        then: None,
                    },
                    at,
                );
            }
            Act::Door { since, to, gap } => match door_beat(at.saturating_sub(since), gap) {
                Some((beat, end)) => {
                    if beat.there && (self.x, self.y) != to {
                        (self.x, self.y) = to;
                    }
                    self.act_due = since + end;
                }
                None => {
                    (self.x, self.y) = to;
                    if !self.home_from_work(at) {
                        self.say(THROUGH, at);
                        self.decide(at, terrain, chances, rng);
                    }
                }
            },
            Act::Home { .. } => self.decide(at, terrain, chances, rng),
            Act::Use {
                what, since, until, ..
            } => {
                if at >= until {
                    if let (Use::Unpack, Some(Job::Use(seat))) = (what, &self.task) {
                        tracing::info!(item = ?seat.item, "houseguest: unpacked");
                        self.events.push(HomeEvent::Unpacked(seat.item));
                    }
                    self.decide(at, terrain, chances, rng);
                } else {
                    let frame =
                        since + (at.saturating_sub(since) / USE_FRAME_MS + 1) * USE_FRAME_MS;
                    self.act_due = frame.min(until);
                }
            }
            Act::Dazed { .. } => {
                // Said in place of a hello when it's her entrance.
                if !self.greeted || rng.below(2) == 0 {
                    self.greeted = true;
                    self.say(OK, at);
                }
                self.decide(at, terrain, chances, rng)
            }
            Act::Stand { .. } | Act::SpaceOut { .. } | Act::Admire { .. } | Act::PutBack { .. } => {
                self.decide(at, terrain, chances, rng)
            }
            Act::Swap { back: true, .. } => {
                self.task = None;
                self.set(
                    Act::SpaceOut {
                        until: at + rng.range(1500, 3000),
                    },
                    at,
                );
            }
            Act::Swap { back: false, .. } => {
                let Some(Job::Swap(swap)) = self.task.clone() else {
                    return self.decide(at, terrain, chances, rng);
                };
                let (a, b) = (swap.a, swap.b);
                let revert = at + rng.range(SWAP_KEPT_MS.0, SWAP_KEPT_MS.1);
                tracing::debug!(?a, ?b, "houseguest: swapping two letters");
                self.ops.push(LayerOp::Swap { a, b });
                // Back where they were shown: home, or along a line she
                // pulled.
                self.schedule(
                    revert,
                    LayerOp::Restore {
                        to: vec![a, b],
                        tries: 0,
                    },
                );
                self.set(
                    Act::Giggle {
                        until: at + 1500,
                        revert,
                    },
                    at,
                );
            }
            Act::Giggle { revert, .. } => {
                let until = revert.saturating_sub(FIDDLE_MS).max(at);
                self.set(Act::Innocent { until, revert }, at);
            }
            Act::Innocent { revert, .. } => {
                // Nobody noticed. She quietly puts it right.
                if let Some(task) = &self.task {
                    self.facing = side_facing(task.side());
                }
                self.set(
                    Act::Swap {
                        until: revert.max(at),
                        back: true,
                    },
                    at,
                );
            }
            Act::Sneeze {
                since,
                knocked: false,
            } => {
                self.sneeze(at, chances, rng);
                self.set(
                    Act::Sneeze {
                        since,
                        knocked: true,
                    },
                    at,
                );
            }
            Act::Sneeze { knocked: true, .. } => {
                let last = self
                    .pending
                    .iter()
                    .filter(|(_, op)| matches!(op, LayerOp::Restore { .. }))
                    .map(|(due, _)| *due)
                    .max();
                match last {
                    Some(until) => self.set(Act::PutBack { since: at, until }, at),
                    None => self.decide(at, terrain, chances, rng),
                }
            }
            Act::Look {
                surprised_until,
                until,
            } => {
                if at <= surprised_until && until > at {
                    self.act_due = until;
                } else {
                    self.decide(at, terrain, chances, rng);
                }
            }
            Act::Walk { to, then } => {
                if self.x != to {
                    let step = (to - self.x).signum();
                    let next = self.x + step;
                    let inside = terrain.platform_at(next, self.y).is_some();
                    let entering = terrain.platform_at(self.x, self.y).is_none() && !inside;
                    if !inside && !entering {
                        return self.decide(at, terrain, chances, rng);
                    }
                    self.x = next;
                }
                if self.x != to {
                    self.act_due = at + WALK_MS;
                    return;
                }
                if let Some(job) = &self.task
                    && job.spot() == (self.x, self.y)
                {
                    self.facing = side_facing(job.side());
                    if let Job::Use(seat) = *job {
                        self.facing = seat.facing;
                        let (lo, hi) = use_duration(seat.what);
                        tracing::debug!(?seat, "houseguest: using her furniture");
                        // The shopping channel: she's bought it the moment
                        // it comes on.
                        let advert = chances.advert.filter(|_| seat.what == Use::Watch);
                        if let Some(item) = advert {
                            tracing::info!(?item, "houseguest: bought off the shopping channel");
                            self.events.push(HomeEvent::Bought(item));
                        }
                        return self.set(
                            Act::Use {
                                what: seat.what,
                                since: at,
                                until: at + rng.range(lo, hi),
                                advert,
                            },
                            at,
                        );
                    }
                    let Job::Pull(task) = job else {
                        // At the word: reach for the letters.
                        return self.set(
                            Act::Swap {
                                until: at + FIDDLE_MS,
                                back: false,
                            },
                            at,
                        );
                    };
                    // At the line's end: brace. She reels in any slack
                    // first, then heaves it 2–7 cells further.
                    let goal = task.gap + rng.range(2, 8) as u16;
                    tracing::debug!(
                        row = task.row,
                        glyphs = task.cells.len(),
                        side = ?task.side,
                        cells = goal,
                        "houseguest: pulling a line"
                    );
                    return self.set(
                        Act::Pull {
                            offset: 0,
                            goal,
                            heaving: false,
                        },
                        at,
                    );
                }
                self.task = None;
                match then {
                    Some(Link {
                        to,
                        route: Route::Clamber { column },
                        pole,
                        ..
                    }) => {
                        let Some(target) = terrain.platforms.get(to) else {
                            return self.decide(at, terrain, chances, rng);
                        };
                        self.pole = pole;
                        let to_x = target.clamp(column);
                        tracing::debug!(column, to_y = target.y, "houseguest: clambering over");
                        self.set(
                            Act::Clamber {
                                column,
                                to_y: target.y,
                                to_x,
                            },
                            at,
                        );
                    }
                    Some(Link {
                        to,
                        route: Route::Around { out, enter },
                        ..
                    }) => {
                        let Some(target) = terrain.platforms.get(to) else {
                            return self.decide(at, terrain, chances, rng);
                        };
                        // In at the end nearest where she comes back.
                        let to_x = if enter < 0 { target.x0 } else { target.x1 };
                        self.facing = toward(self.x, out);
                        self.set(
                            Act::Out {
                                to: out,
                                enter,
                                to_y: target.y,
                                to_x,
                            },
                            at,
                        );
                    }
                    Some(link) if link.route == Route::Climb => {
                        let to_y = terrain.platforms.get(link.to).map_or(self.y, |p| p.y);
                        // She faces the pole and climbs it.
                        self.pole = link.pole;
                        if link.pole != self.x {
                            self.facing = toward(self.x, link.pole);
                        }
                        self.set(Act::Climb { to_y }, at);
                    }
                    Some(link) => self.set(
                        Act::Peer {
                            until: at + PEER_MS,
                            then: Some(link),
                        },
                        at,
                    ),
                    None if self.home_from_work(at) => {}
                    None => {
                        let at_edge = terrain.platform_at(self.x, self.y).and_then(|i| {
                            let p = terrain.platforms.get(i)?;
                            (p.edge_left && self.x == p.x0 || p.edge_right && self.x == p.x1)
                                .then_some(())
                        });
                        if at_edge.is_some() && rng.below(2) == 0 {
                            self.set(
                                Act::Peer {
                                    until: at + PEER_MS,
                                    then: None,
                                },
                                at,
                            );
                        } else {
                            self.decide(at, terrain, chances, rng);
                        }
                    }
                }
            }
            Act::Peer { then, .. } => match then {
                Some(Link {
                    to,
                    route: Route::Drop { over },
                    ..
                }) => {
                    self.x = over;
                    let to_y = terrain.platforms.get(to).map_or(self.y, |p| p.y);
                    self.set(
                        Act::Fall {
                            from_y: self.y,
                            since: at,
                            to_y,
                        },
                        at,
                    );
                }
                _ => {
                    self.facing = flip(self.facing);
                    self.decide(at, terrain, chances, rng);
                }
            },
            Act::Pull {
                offset,
                goal,
                heaving,
            } => {
                let Some(Job::Pull(task)) = self.task.clone() else {
                    return self.decide(at, terrain, chances, rng);
                };
                if !heaving {
                    // The heave: the line comes to her hands (reeling in
                    // the slack), then she steps back and it follows.
                    let step = task.side.step();
                    if offset >= task.gap {
                        let next = self.x + step;
                        let room = terrain.platform_at(next, self.y).is_some()
                            && terrain.clear(next, self.y);
                        if !room {
                            tracing::debug!("houseguest: out of floor, done pulling");
                            return self.finish_pull(at);
                        }
                        self.x = next;
                    }
                    let offset = offset + 1;
                    self.ops.push(LayerOp::Pull {
                        row: task.row,
                        cells: task.cells.clone(),
                        offset: (i32::from(offset) * step) as i16,
                    });
                    self.set(
                        Act::Pull {
                            offset,
                            goal,
                            heaving: true,
                        },
                        at,
                    );
                } else if offset >= goal {
                    self.finish_pull(at);
                } else {
                    self.set(
                        Act::Pull {
                            offset,
                            goal,
                            heaving: false,
                        },
                        at,
                    );
                }
            }
            Act::Climb { to_y } => {
                self.y += (to_y - self.y).signum();
                if self.y == to_y {
                    self.set(Act::Stand { until: at + 800 }, at);
                } else {
                    self.act_due = at + CLIMB_MS;
                }
            }
            Act::Fall {
                from_y,
                since,
                to_y,
            } => {
                let fallen = rows_fallen(since, at).max(self.y - from_y + 1);
                self.y = (from_y + fallen).min(to_y);
                if self.y >= to_y {
                    if to_y - from_y >= 3 {
                        self.set(
                            Act::Dazed {
                                until: at + DAZED_MS,
                            },
                            at,
                        );
                    } else {
                        self.set(Act::Stand { until: at + 600 }, at);
                    }
                } else {
                    self.act_due = fall_time(since, (self.y - from_y + 1) as u64);
                }
            }
        }
    }

    fn finish_pull(&mut self, at: u64) {
        self.task = None;
        self.set(Act::Admire { until: at + 2000 }, at);
    }

    /// Layer changes queued since the last paint.
    pub fn take_ops(&mut self) -> Vec<LayerOp> {
        std::mem::take(&mut self.ops)
    }

    /// What she did to her home since last asked.
    pub fn take_events(&mut self) -> Vec<HomeEvent> {
        std::mem::take(&mut self.events)
    }

    /// The paint refused `op` (the frame didn't allow it). A put-back is
    /// tried again shortly. Mischief that never happened owes nothing:
    /// what was queued to follow it is cancelled, and a swap she'd have
    /// giggled at leaves her puzzled instead.
    pub fn refused(&mut self, now: u64, op: LayerOp) {
        match op {
            LayerOp::Restore { to, tries } if tries < RETRIES => self.schedule(
                now + PUT_BACK_MS,
                LayerOp::Restore {
                    to,
                    tries: tries + 1,
                },
            ),
            LayerOp::Swap { .. } | LayerOp::Knock { .. } => {
                let gone = op.sources();
                self.pending
                    .retain(|(_, queued)| !queued.sources().iter().any(|c| gone.contains(c)));
                let giggling = matches!(
                    self.act,
                    Act::Giggle { .. } | Act::Innocent { .. } | Act::Swap { .. }
                );
                if matches!(op, LayerOp::Swap { .. }) && giggling {
                    tracing::debug!("houseguest: the letters moved before she could swap them");
                    self.task = None;
                    self.set(
                        Act::Look {
                            surprised_until: now,
                            until: now + LOOK_MS / 2,
                        },
                        now,
                    );
                }
            }
            _ => {}
        }
    }

    /// "chu!": knock 2–4 glyphs beside her loose, let them fall, and
    /// schedule putting each back.
    fn sneeze(&mut self, at: u64, chances: &Chances, rng: &mut Rng) {
        let mut loose = chances.loose.clone();
        let count = (rng.range(2, 5) as usize).min(loose.len());
        let mut knocked = Vec::with_capacity(count);
        for _ in 0..count {
            let pick = rng.below(loose.len() as u64) as usize;
            knocked.push(loose.swap_remove(pick));
        }
        tracing::debug!(knocked = knocked.len(), "houseguest: sneezed");
        let putting_back = at + RECOIL_MS + OOPS_MS;
        for (i, &source) in knocked.iter().enumerate() {
            let dir = if i32::from(source.0) < self.x { -1 } else { 1 };
            self.ops.push(LayerOp::Knock { source, dir });
            for row in 1..=FALL_ROWS {
                self.schedule(at + row * DROP_MS, LayerOp::Fall { source });
            }
            self.schedule(
                putting_back + i as u64 * PUT_BACK_MS,
                LayerOp::Restore {
                    to: vec![Placed::home(source)],
                    tries: 0,
                },
            );
        }
    }

    /// Where she's using a piece of her furniture, if she is.
    pub fn seat(&self) -> Option<Seat> {
        match (self.act, &self.task) {
            (Act::Use { .. }, Some(Job::Use(seat))) => Some(*seat),
            _ => None,
        }
    }

    /// The piece she was using went into the closet: she's back on her
    /// feet where it was, blinking.
    pub fn lost_seat(&mut self, now: u64) {
        if let (Act::Use { .. }, Some(Job::Use(seat))) = (self.act, self.task.take()) {
            tracing::debug!(?seat, "houseguest: her furniture went away under her");
            self.set(
                Act::Look {
                    surprised_until: now + SURPRISED_MS,
                    until: now + LOOK_MS / 2,
                },
                now,
            );
        }
    }

    /// Using a piece: where, and since and until when.
    pub fn use_span(&self) -> Option<(Seat, u64, u64)> {
        match (self.act, &self.task) {
            (Act::Use { since, until, .. }, Some(Job::Use(seat))) => Some((*seat, since, until)),
            _ => None,
        }
    }

    /// Watching the TV: since when, and what the shopping channel is
    /// selling (for what's on screen).
    pub fn watching(&self) -> Option<(u64, Option<Furniture>)> {
        match self.act {
            Act::Use {
                what: Use::Watch,
                since,
                advert,
                ..
            } => Some((since, advert)),
            _ => None,
        }
    }

    /// Whether she's using `item` (inside it or beside it).
    pub fn using(&self) -> Option<Furniture> {
        match (self.act, &self.task) {
            (Act::Use { .. }, Some(Job::Use(seat))) => Some(seat.item),
            _ => None,
        }
    }

    /// The text she was pulling changed under her (someone scrolled the
    /// chat): she lets go and stares.
    pub fn lost_grip(&mut self, now: u64) {
        if matches!(self.task, Some(Job::Pull(_))) {
            self.task = None;
            tracing::trace!("houseguest lost her grip");
            self.set(
                Act::Look {
                    surprised_until: now,
                    until: now + LOOK_MS / 2,
                },
                now,
            );
        }
    }

    /// Choose what to do next, standing somewhere valid.
    fn decide(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        self.task = None;
        let Some(here) = terrain.platform_at(self.x, self.y) else {
            return self.set(Act::Stand { until: at + 1000 }, at);
        };
        if at < self.watch_until {
            self.facing = toward(self.x, self.watch_x);
            return self.set(
                Act::Stand {
                    until: self.watch_until.min(at + 5000),
                },
                at,
            );
        }
        if !self.greeted {
            self.greeted = true;
            self.say(GREETING, at);
        }
        // Her needs move on with the time since she last chose.
        self.needs
            .pass(at.saturating_sub(self.decided), !chances.pulls.is_empty());
        self.decided = at;
        // Still making for a job on another floor, while it's on offer.
        if let Some(job) = self.goal.take().filter(|g| chances.offers(g))
            && self.go_to(job, here, terrain, at)
        {
            return;
        }
        let links: Vec<Link> = terrain
            .links
            .iter()
            .filter(|l| l.from == here)
            .copied()
            .collect();
        let mut offers = vec![Kind::Stand, Kind::SpaceOut, Kind::Sneeze, Kind::Walk];
        offers.extend(Activity::ALL.iter().map(|&a| Kind::Idle(a)));
        // With no way off this floor, travelling means a door in space.
        offers.push(Kind::Travel);
        // A home to leave, a while into the visit, once.
        if chances.furnished && !self.worked && at >= self.arrived + WORK_AFTER_MS {
            offers.push(Kind::Work);
        }
        if !chances.pulls.is_empty() {
            offers.push(Kind::Pull);
        }
        for seat in &chances.seats {
            let kind = Kind::Use(seat.what);
            if !offers.contains(&kind) {
                offers.push(kind);
            }
        }
        // One piece of mischief at a time: no new swap while one is owed.
        if !chances.swaps.is_empty() && !self.owes() {
            offers.push(Kind::Swap);
        }
        while let Some((i, top)) = brain::choose(&offers, &self.needs, &self.recent, rng) {
            let kind = offers.remove(i);
            if self.start(kind, here, &links, terrain, chances, at, rng) {
                tracing::debug!(
                    ?kind,
                    needs = %self.needs.summary(),
                    ?top,
                    "houseguest: decided"
                );
                self.recent.push(kind);
                #[cfg(test)]
                self.choices.push(kind);
                if self.recent.len() > RECENT {
                    self.recent.remove(0);
                }
                if let Some((need, amount)) = kind.serves() {
                    self.needs.serve(need, amount);
                }
                return;
            }
            tracing::debug!(?kind, "houseguest: couldn't after all");
        }
        self.set(Act::Stand { until: at + 2000 }, at);
    }

    /// Start `kind` from platform `here`. False when it turns out not to
    /// be possible (nowhere else to walk, no way to the job).
    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        kind: Kind,
        here: usize,
        links: &[Link],
        terrain: &Terrain,
        chances: &Chances,
        at: u64,
        rng: &mut Rng,
    ) -> bool {
        let act = match kind {
            Kind::Stand => Act::Stand {
                until: at + rng.range(2000, 5000),
            },
            Kind::SpaceOut => {
                if rng.below(3) == 0 {
                    self.muse(at, rng);
                    return true;
                }
                Act::SpaceOut {
                    until: at + rng.range(6000, 14_000),
                }
            }
            Kind::Sneeze => Act::Sneeze {
                since: at,
                knocked: false,
            },
            Kind::Idle(what) => self.idle_act(what, at, rng),
            Kind::Walk => {
                let Some(p) = terrain.platforms.get(here) else {
                    return false;
                };
                let to = p.x0 + rng.below((p.x1 - p.x0 + 1) as u64) as i32;
                if to == self.x {
                    return false;
                }
                self.facing = toward(self.x, to);
                Act::Walk { to, then: None }
            }
            Kind::Work => {
                let out = links
                    .iter()
                    .find(|l| matches!(l.route, Route::Around { .. }))
                    .copied();
                self.go_to_work(out, at, rng);
                return true;
            }
            Kind::Travel => {
                if let Some(&link) = links.get(rng.below(links.len() as u64) as usize) {
                    self.travel(link, at);
                    return true;
                }
                // Stuck here: through a door to anywhere else.
                let others: Vec<_> = terrain
                    .platforms
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| i != here)
                    .map(|(_, p)| p)
                    .collect();
                let Some(p) = others.get(rng.below(others.len() as u64) as usize) else {
                    return false;
                };
                let x = p.x0 + rng.below((p.x1 - p.x0 + 1) as u64) as i32;
                self.through_door((x, p.y), at);
                return true;
            }
            Kind::Use(what) => {
                let seats: Vec<Seat> = chances
                    .seats
                    .iter()
                    .filter(|s| s.what == what)
                    .copied()
                    .collect();
                let Some(&seat) = seats.get(rng.below(seats.len() as u64) as usize) else {
                    return false;
                };
                return self.go_to(Job::Use(seat), here, terrain, at);
            }
            Kind::Pull | Kind::Swap => {
                // A few tries at one she can get to.
                for _ in 0..8 {
                    let job = if kind == Kind::Pull {
                        let pick = rng.below(chances.pulls.len() as u64) as usize;
                        chances.pulls.get(pick).cloned().map(Job::Pull)
                    } else {
                        let pick = rng.below(chances.swaps.len() as u64) as usize;
                        chances.swaps.get(pick).cloned().map(Job::Swap)
                    };
                    let Some(job) = job else {
                        return false;
                    };
                    if self.go_to(job, here, terrain, at) {
                        return true;
                    }
                }
                return false;
            }
        };
        self.set(act, at);
        true
    }

    /// Head for `job`: straight there on this floor, or along the first
    /// link of a route to its floor. False when there's no way there.
    fn go_to(&mut self, job: Job, here: usize, terrain: &Terrain, at: u64) -> bool {
        let (x, y) = job.spot();
        let there = terrain.platform_at(x, y);
        if there == Some(here) {
            tracing::debug!(?job, "houseguest: walking to a job");
            self.pursue(job, at);
            return true;
        }
        match there.map(|there| route(terrain, here, there)) {
            Some(Some(link)) => {
                tracing::debug!(?job, via = ?link.route, "houseguest: heading for a job on another floor");
                self.goal = Some(job);
                self.travel(link, at);
                true
            }
            // No way there: a door in space, straight to it.
            Some(None) => {
                self.goal = Some(job);
                self.through_door((x, y), at);
                true
            }
            None => false,
        }
    }

    /// Her needs.
    pub fn needs(&self) -> &Needs {
        &self.needs
    }

    /// Stage: make `need` pressing.
    pub fn press(&mut self, need: Need) {
        self.needs.serve(need, -1.0);
    }

    /// Walk to `job`'s spot on this floor and do it.
    pub fn pursue(&mut self, job: Job, at: u64) {
        let (x, _) = job.spot();
        if x != self.x {
            self.facing = toward(self.x, x);
        }
        self.task = Some(job);
        self.set(Act::Walk { to: x, then: None }, at);
    }

    /// Walk to `link` on this floor and take it (climb or drop).
    pub fn travel(&mut self, link: Link, at: u64) {
        if link.x != self.x {
            self.facing = toward(self.x, link.x);
        }
        self.set(
            Act::Walk {
                to: link.x,
                then: Some(link),
            },
            at,
        );
    }

    /// Do `what` on the spot.
    pub fn idle(&mut self, what: Activity, at: u64, rng: &mut Rng) {
        let act = self.idle_act(what, at, rng);
        self.set(act, at);
    }

    fn idle_act(&mut self, what: Activity, at: u64, rng: &mut Rng) -> Act {
        let (lo, hi) = what.duration();
        if matches!(what, Activity::Sit | Activity::LieBack | Activity::LieFront) {
            // Sitting and lying face either way.
            self.facing = if rng.below(2) == 0 {
                Facing::Left
            } else {
                Facing::Right
            };
        }
        Act::Idle {
            what,
            since: at,
            until: at + rng.range(lo, hi),
        }
    }

    /// Put her at `(x, y)`, standing, having forgotten what she was up
    /// to (mischief she owes is still undone on schedule).
    pub fn place(&mut self, x: i32, y: i32, at: u64) {
        self.x = x;
        self.y = y;
        self.task = None;
        self.goal = None;
        self.watch_until = 0;
        self.speech = None;
        self.at_work = false;
        self.set(Act::Stand { until: at + 1000 }, at);
    }

    /// A chat message arrived: stop and look at it.
    pub fn look(&mut self, now: u64, chat_x: i32) {
        self.watch_until = now + WATCH_MS;
        // Someone's here: whatever she knocked over or swapped goes back
        // at once, in order.
        self.pending.sort_by_key(|(due, _)| *due);
        for (due, _) in &mut self.pending {
            *due = now;
        }
        // Out, or on her way: she'll see it when she's back.
        if matches!(
            self.act,
            Act::Out { .. } | Act::Away { .. } | Act::Door { .. }
        ) {
            return;
        }
        if !matches!(self.act, Act::Fall { .. } | Act::Climb { .. }) {
            // She lets go of whatever she was pulling.
            self.task = None;
        }
        self.watch_x = chat_x;
        if matches!(self.act, Act::Fall { .. } | Act::Climb { .. }) {
            return; // She looks once she has landed (decide watches).
        }
        self.facing = toward(self.x, chat_x);
        self.set(
            Act::Look {
                surprised_until: now + SURPRISED_MS,
                until: now + LOOK_MS,
            },
            now,
        );
    }

    /// Re-anchor to a freshly read terrain (a resize, a scrolled day
    /// separator). Returns false when there's nowhere left to be.
    pub fn settle(&mut self, now: u64, terrain: &Terrain) -> bool {
        match self.act {
            // Off screen, or between doors: nothing here to fall off.
            Act::Out { .. } | Act::Away { .. } | Act::Door { .. } => return true,
            Act::Clamber { .. } => {
                if terrain.clear(self.x, self.y) {
                    return true;
                }
            }
            Act::Climb { to_y } => {
                if terrain.platform_at(self.x, to_y).is_some() && terrain.clear(self.x, self.y) {
                    return true;
                }
            }
            Act::Fall { to_y, .. } => match terrain.landing(self.x, self.y) {
                Some(landing) => {
                    let landing_y = terrain.platforms.get(landing).map_or(to_y, |p| p.y);
                    if landing_y != to_y
                        && let Act::Fall { to_y, .. } = &mut self.act
                    {
                        *to_y = landing_y;
                    }
                    return true;
                }
                None if self.y < HEIGHT => return true, // still above the screen
                None => {}
            },
            // Walking in (on arrival, or back from stepping out): until her
            // box is wholly on screen and over the floor, there's no floor
            // under her, and that's fine.
            Act::Walk { to, .. }
                if terrain.platform_at(self.x, self.y).is_none()
                    && self.offscreen(terrain)
                    && terrain.platform_at(to, self.y).is_some() =>
            {
                return true;
            }
            _ => {
                if terrain.platform_at(self.x, self.y).is_some() {
                    if let Act::Walk { to, then } = &mut self.act
                        && let Some(here) = terrain.platform_at(self.x, self.y)
                        && let Some(p) = terrain.platforms.get(here)
                        && !p.contains(*to)
                    {
                        *to = p.clamp(*to);
                        *then = None;
                    }
                    return true;
                }
            }
        }
        // The floor went away under her.
        if let Some(landing) = terrain.landing(self.x, self.y) {
            let to_y = terrain.platforms.get(landing).map_or(self.y, |p| p.y);
            self.set(
                Act::Fall {
                    from_y: self.y,
                    since: now,
                    to_y,
                },
                now,
            );
            return true;
        }
        let nearest = terrain
            .platforms
            .iter()
            .min_by_key(|p| (p.clamp(self.x) - self.x).abs() + (p.y - self.y).abs());
        let Some(p) = nearest else {
            return false;
        };
        self.x = p.clamp(self.x);
        self.y = p.y;
        self.set(
            Act::Dazed {
                until: now + DAZED_MS,
            },
            now,
        );
        true
    }

    /// Whether any of her box is off the screen's sides.
    fn offscreen(&self, terrain: &Terrain) -> bool {
        let half = sprite::WIDTH / 2;
        self.x - half < 0 || self.x + half >= terrain.width()
    }

    /// Her current sprite cells and bubble.
    pub fn picture(&self, now: u64) -> (Vec<SpriteCell>, Option<Bubble>) {
        let (pose, face, bubble) = self.appearance(now);
        (sprite::cells(pose, self.facing, face), bubble)
    }

    /// The box row her hands work at for the current job.
    fn hands_row(&self) -> u8 {
        self.task.as_ref().map_or(1, Job::box_row)
    }

    fn facing_sign(&self) -> i32 {
        match self.facing {
            Facing::Left => -1,
            Facing::Right => 1,
        }
    }

    /// Whether she is standing on a floor (not climbing or falling), so
    /// her line art includes the floor under her feet.
    pub fn standing(&self) -> bool {
        !matches!(
            self.act,
            Act::Climb { .. } | Act::Fall { .. } | Act::Clamber { .. }
        )
    }

    /// Whether she's out of sight (through a door, or stepped out).
    pub fn hidden(&self, now: u64) -> bool {
        match self.act {
            Act::Away { .. } => true,
            Act::Door { since, gap, .. } => {
                door_beat(now.saturating_sub(since), gap).is_some_and(|(beat, _)| !beat.her)
            }
            _ => false,
        }
    }

    /// The door she's going through, if any, as it looks at `now`.
    pub fn door(&self, now: u64) -> Option<DoorFrame> {
        match self.act {
            Act::Door { since, gap, .. } => door_beat(now.saturating_sub(since), gap)?.0.door,
            _ => None,
        }
    }

    /// Go through a door in space to `(x, y)`, ignoring whatever lies
    /// between: her way out when there's no other.
    pub fn through_door(&mut self, to: (i32, i32), at: u64) {
        tracing::debug!(from = ?(self.x, self.y), ?to, "houseguest: a door in space");
        self.set(
            Act::Door {
                since: at,
                to,
                gap: 0,
            },
            at,
        );
    }

    /// Off to her part-time job: out at a screen edge if her floor
    /// reaches one (`out`), else through a door, and back in 1–3
    /// minutes with her shopping.
    pub fn go_to_work(&mut self, out: Option<Link>, at: u64, rng: &mut Rng) {
        tracing::info!("houseguest: off to work");
        self.at_work = true;
        self.worked = true;
        match out {
            Some(link) => self.travel(link, at),
            None => {
                let gap = rng.range(SHIFT_MS.0, SHIFT_MS.1);
                let here = (self.x, self.y);
                self.set(
                    Act::Door {
                        since: at,
                        to: here,
                        gap,
                    },
                    at,
                );
            }
        }
    }

    /// Back from work (if she was at work): "I'm home!", showing her
    /// shopping. Returns whether she was.
    fn home_from_work(&mut self, at: u64) -> bool {
        if !std::mem::take(&mut self.at_work) {
            return false;
        }
        tracing::info!("houseguest: back from work");
        self.say(HOME, at);
        self.set(
            Act::Home {
                until: at + HOME_MS,
            },
            at,
        );
        true
    }

    /// Her pose, face and bubble at `now`.
    pub fn appearance(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
        let (pose, face, bubble) = self.acting(now);
        let speech = self
            .speech
            .filter(|&(_, until)| now < until)
            .map(|(text, _)| Bubble::Say(text));
        (pose, face, speech.or(bubble))
    }

    fn acting(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
        match self.act {
            Act::Stand { .. } => {
                let blink = now < self.blink_until;
                let face = if blink { Face::Blink } else { Face::Vacant };
                // Watching the chat she stands side-on, facing it.
                let pose = if now < self.watch_until {
                    Pose::Side
                } else {
                    Pose::Stand
                };
                (pose, face, None)
            }
            Act::Idle { what, since, .. } => {
                let frame = now
                    .saturating_sub(since)
                    .checked_div(what.period())
                    .map_or(0, |n| (n % 2) as u8);
                what.look(frame)
            }
            Act::Use {
                what,
                since,
                until,
                advert,
            } => use_look(
                what,
                advert,
                now.saturating_sub(since),
                until.saturating_sub(since),
            ),
            Act::SpaceOut { .. } => (Pose::Stand, Face::Vacant, Some(Bubble::Dots)),
            Act::Home { until } => {
                let frame = (until.saturating_sub(now) / 700 % 2) as u8;
                (Pose::Carry(frame), Face::Pleased, None)
            }
            Act::Clamber { column, to_y, .. } if self.x == column && self.y != to_y => {
                let pole = ((self.pole - self.x) * self.facing_sign()).clamp(-3, 3) as i8;
                let frame = self.y.rem_euclid(2) as u8;
                (Pose::Climb { frame, pole }, Face::Vacant, None)
            }
            Act::Clamber { .. } | Act::Out { .. } | Act::Away { .. } => {
                (Pose::Walk(self.x.rem_euclid(4) as u8), Face::Vacant, None)
            }
            Act::Door { since, gap, .. } => {
                let there = door_beat(now.saturating_sub(since), gap).is_some_and(|(b, _)| b.there);
                let face = if there { Face::Pleased } else { Face::Curious };
                (Pose::Stand, face, None)
            }
            Act::Walk { .. } => (Pose::Walk(self.x.rem_euclid(4) as u8), Face::Vacant, None),
            Act::Peer { .. } => (Pose::Peer, Face::Vacant, None),
            Act::Climb { .. } => {
                let pole = ((self.pole - self.x) * self.facing_sign()).clamp(-3, 3) as i8;
                let frame = self.y.rem_euclid(2) as u8;
                (Pose::Climb { frame, pole }, Face::Vacant, None)
            }
            Act::Fall { .. } => (Pose::Fall, Face::Surprised, None),
            Act::Dazed { .. } => (Pose::Dazed, Face::Vacant, None),
            Act::Pull { heaving, .. } => (
                Pose::Pull {
                    heaving,
                    row: self.hands_row(),
                },
                Face::Vacant,
                None,
            ),
            Act::Swap { back, .. } => {
                let face = if back { Face::Vacant } else { Face::Curious };
                let pose = Pose::Pull {
                    heaving: false,
                    row: self.hands_row(),
                };
                (pose, face, None)
            }
            Act::Giggle { .. } => (Pose::Stand, Face::Pleased, Some(Bubble::Hehe)),
            Act::Innocent { .. } => (Pose::Gaze, Face::Happy, Some(Bubble::Hum)),
            Act::Sneeze { knocked: false, .. } => (Pose::Gaze, Face::Blink, Some(Bubble::Achoo)),
            Act::Sneeze { knocked: true, .. } => {
                (Pose::ToeTouch(0), Face::Blink, Some(Bubble::Chu))
            }
            Act::PutBack { since, .. } => {
                let oops = now < since + OOPS_MS;
                let frame = (now.saturating_sub(since) / PUT_BACK_MS % 2) as u8;
                let bubble = oops.then_some(Bubble::Dots);
                (Pose::ToeTouch(frame), Face::Vacant, bubble)
            }
            Act::Admire { .. } => (Pose::Stand, Face::Pleased, Some(Bubble::Hehe)),
            Act::Look {
                surprised_until, ..
            } => {
                if now < surprised_until {
                    (Pose::Side, Face::Surprised, Some(Bubble::Bang))
                } else {
                    (Pose::Side, Face::Curious, Some(Bubble::Huh))
                }
            }
        }
    }
}

fn side_facing(side: Side) -> Facing {
    match side {
        Side::Left => Facing::Left,
        Side::Right => Facing::Right,
    }
}

fn flip(facing: Facing) -> Facing {
    match facing {
        Facing::Left => Facing::Right,
        Facing::Right => Facing::Left,
    }
}

fn toward(from: i32, to: i32) -> Facing {
    if to < from {
        Facing::Left
    } else {
        Facing::Right
    }
}

/// Rows fallen `since..at` under gravity.
fn rows_fallen(since: u64, at: u64) -> i32 {
    let t = at.saturating_sub(since) as f64 / 1000.0;
    (0.5 * GRAVITY * t * t).floor() as i32
}

/// When the fall started at `since` crosses its `rows`th row.
fn fall_time(since: u64, rows: u64) -> u64 {
    since + ((2.0 * rows as f64 / GRAVITY).sqrt() * 1000.0).ceil() as u64
}

/// The first link on a shortest route from platform `from` to `to`.
fn route(terrain: &Terrain, from: usize, to: usize) -> Option<Link> {
    let mut first: Vec<Option<Link>> = vec![None; terrain.platforms.len()];
    let mut seen = vec![false; terrain.platforms.len()];
    let mut queue = std::collections::VecDeque::from([from]);
    if let Some(s) = seen.get_mut(from) {
        *s = true;
    }
    while let Some(at) = queue.pop_front() {
        if at == to {
            return first.get(at).copied().flatten();
        }
        for link in terrain.links.iter().filter(|l| l.from == at) {
            if seen.get(link.to).copied().unwrap_or(true) {
                continue;
            }
            if let Some(s) = seen.get_mut(link.to) {
                *s = true;
            }
            let via = first.get(at).copied().flatten().unwrap_or(*link);
            if let Some(slot) = first.get_mut(link.to) {
                *slot = Some(via);
            }
            queue.push_back(link.to);
        }
    }
    None
}

/// The next animation-frame boundary of `what` after `now`, or far away
/// for a held pose.
fn next_frame(what: Activity, since: u64, now: u64) -> u64 {
    let period = what.period();
    if period == 0 {
        return u64::MAX;
    }
    since + (now.saturating_sub(since) / period + 1) * period
}
