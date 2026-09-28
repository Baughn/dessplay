//! Osaka herself: where she is, what she's doing, and when that next
//! changes. Every activity is a finite, wall-clock-timed step, so any
//! instant is a safe place to cut the visit short.

use super::Rng;
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
}

impl Chances {
    fn offers(&self, job: &Job) -> bool {
        match job {
            Job::Pull(p) => self.pulls.contains(p),
            Job::Swap(s) => self.swaps.contains(s),
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
/// A refused put-back is retried this many times.
const RETRIES: u8 = 5;

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
}

/// Something to do on the spot that isn't staring at the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    const ALL: [Activity; 7] = [
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
    /// When this visit began (early visits favour activities).
    arrived: u64,
    /// Layer changes for the next paint to apply.
    ops: Vec<LayerOp>,
    /// Layer changes due later, whatever she's doing by then: undoing
    /// mischief is scheduled when it's made, so nothing can strand it.
    pending: Vec<(u64, LayerOp)>,
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
            arrived: now,
            ops: Vec::new(),
            pending: Vec::new(),
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
            | Act::PutBack { until, .. } => until,
            Act::Sneeze { since, knocked } => {
                since + WINDUP_MS + if knocked { RECOIL_MS } else { 0 }
            }
            Act::Idle { what, since, until } => next_frame(what, since, now).min(until),
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

    /// When her pose (or the text layer) next changes.
    pub fn due(&self) -> u64 {
        self.pose_due().min(self.pending_due())
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
            Act::Stand { .. }
            | Act::SpaceOut { .. }
            | Act::Dazed { .. }
            | Act::Admire { .. }
            | Act::PutBack { .. } => self.decide(at, terrain, chances, rng),
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
                let a = (swap.pair.0, swap.row);
                let b = (swap.pair.1, swap.row);
                let revert = at + rng.range(SWAP_KEPT_MS.0, SWAP_KEPT_MS.1);
                tracing::debug!(row = swap.row, ?swap.pair, "houseguest: swapping two letters");
                self.ops.push(LayerOp::Swap { a, b });
                self.schedule(
                    revert,
                    LayerOp::Restore {
                        sources: vec![a, b],
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
                    // At the line's end: brace.
                    let goal = rng.range(3, 9) as u16;
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
                    // The heave: she steps back and the line follows.
                    let step = task.side.step();
                    let next = self.x + step;
                    let room =
                        terrain.platform_at(next, self.y).is_some() && terrain.clear(next, self.y);
                    if !room {
                        tracing::debug!("houseguest: out of floor, done pulling");
                        return self.finish_pull(at);
                    }
                    self.x = next;
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

    /// The paint refused `op` (the frame didn't allow it). A put-back is
    /// tried again shortly. Mischief that never happened owes nothing:
    /// what was queued to follow it is cancelled, and a swap she'd have
    /// giggled at leaves her puzzled instead.
    pub fn refused(&mut self, now: u64, op: LayerOp) {
        match op {
            LayerOp::Restore { sources, tries } if tries < RETRIES => self.schedule(
                now + PUT_BACK_MS,
                LayerOp::Restore {
                    sources,
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
                    sources: vec![source],
                    tries: 0,
                },
            );
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
        let platform = terrain.platforms.get(here).copied();
        // Tidying: keep making for a line still on offer, or maybe pick
        // one anywhere she can reach.
        let kept = self.goal.take().filter(|g| chances.offers(g));
        // One piece of mischief at a time: no new swap while one is owed.
        let swaps = if self.owes() {
            &[][..]
        } else {
            &chances.swaps[..]
        };
        let offered = chances.pulls.len() + swaps.len();
        let fresh = kept.is_none() && offered > 0 && rng.below(100) < 40;
        let target = kept.or_else(|| {
            if !fresh {
                return None;
            }
            // Mostly tidying; now and then a swap.
            let swap = !swaps.is_empty() && (chances.pulls.is_empty() || rng.below(100) < 30);
            if swap {
                let pick = rng.below(swaps.len() as u64) as usize;
                swaps.get(pick).cloned().map(Job::Swap)
            } else {
                let pick = rng.below(chances.pulls.len() as u64) as usize;
                chances.pulls.get(pick).cloned().map(Job::Pull)
            }
        });
        if let Some(job) = target {
            let (x, y) = job.spot();
            let there = terrain.platform_at(x, y);
            if there == Some(here) {
                tracing::debug!(?job, offered, "houseguest: walking to a job");
                return self.pursue(job, at);
            }
            match there.and_then(|there| route(terrain, here, there)) {
                Some(link) => {
                    tracing::debug!(
                        ?job,
                        via = ?link.route,
                        offered,
                        "houseguest: heading for a job on another floor"
                    );
                    self.goal = Some(job);
                    return self.travel(link, at);
                }
                None => tracing::debug!(?job, offered, "houseguest: a job, but no way there"),
            }
        } else if offered == 0 {
            tracing::trace!("houseguest: nothing to tidy");
        }
        // Early in a visit she's busier; staring at the viewer is short and
        // rare either way.
        let early = at.saturating_sub(self.arrived) < 300_000;
        let busy = if early { 40 } else { 28 };
        let roll = rng.below(100);
        let act = if roll < 8 {
            Act::Stand {
                until: at + rng.range(2000, 5000),
            }
        } else if roll < 16 {
            Act::SpaceOut {
                until: at + rng.range(6000, 14_000),
            }
        } else if roll < 19 {
            Act::Sneeze {
                since: at,
                knocked: false,
            }
        } else if roll < 19 + busy {
            let pick = rng.below(Activity::ALL.len() as u64) as usize;
            let what = Activity::ALL.get(pick).copied().unwrap_or(Activity::Gaze);
            self.idle_act(what, at, rng)
        } else {
            let links: Vec<&Link> = terrain.links.iter().filter(|l| l.from == here).collect();
            let travel = roll >= 75 && !links.is_empty();
            let (to, then) = if travel {
                let link = **links
                    .get(rng.below(links.len() as u64) as usize)
                    .unwrap_or(&links[0]);
                (link.x, Some(link))
            } else {
                let to = platform.map_or(self.x, |p| {
                    p.x0 + rng.below((p.x1 - p.x0 + 1) as u64) as i32
                });
                (to, None)
            };
            if to == self.x && then.is_none() {
                Act::Stand {
                    until: at + rng.range(2000, 6000),
                }
            } else {
                if to != self.x {
                    self.facing = toward(self.x, to);
                }
                Act::Walk { to, then }
            }
        };
        tracing::debug!(?act, roll, "houseguest: pottering");
        self.set(act, at);
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
            Act::Walk { .. }
                if terrain.platform_at(self.x, self.y).is_none() && self.offscreen(terrain) =>
            {
                return true; // walking in
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

    fn offscreen(&self, terrain: &Terrain) -> bool {
        !(0..terrain.width()).contains(&self.x)
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
        !matches!(self.act, Act::Climb { .. } | Act::Fall { .. })
    }

    /// Her pose, face and bubble at `now`.
    pub fn appearance(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
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
            Act::SpaceOut { .. } => (Pose::Stand, Face::Vacant, Some(Bubble::Dots)),
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
