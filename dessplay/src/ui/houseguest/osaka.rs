//! Osaka herself: where she is, what she's doing, and when that next
//! changes. Every activity is a finite, wall-clock-timed step, so any
//! instant is a safe place to cut the visit short.

use super::Rng;
use super::scenes::{LayerOp, Pull, Side};
use super::sprite::{self, Face, Facing, HEIGHT, Pose, SpriteCell};
use super::terrain::{Link, Route, Terrain};

/// What the current frame offers her beyond walking around.
#[derive(Clone, Debug, Default)]
pub(super) struct Chances {
    /// Lines she could pull.
    pub pulls: Vec<Pull>,
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
}

/// A speech or thought bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bubble {
    Dots,
    Bang,
    Huh,
    Hehe,
}

impl Bubble {
    pub fn text(self) -> &'static str {
        match self {
            Self::Dots => "...",
            Self::Bang => "!",
            Self::Huh => "?",
            Self::Hehe => "hehe",
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
    /// A pull she's walking to on this floor, or doing.
    task: Option<Pull>,
    /// A pull on another floor she's making her way towards.
    goal: Option<Pull>,
    /// The pole she's climbing (column).
    pole: i32,
    /// Layer changes for the next paint to apply.
    ops: Vec<LayerOp>,
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

    fn first_due(&self, now: u64) -> u64 {
        match self.act {
            Act::Stand { until }
            | Act::SpaceOut { until }
            | Act::Peer { until, .. }
            | Act::Dazed { until }
            | Act::Admire { until } => until,
            Act::Look {
                surprised_until, ..
            } => surprised_until,
            Act::Walk { .. } => now + WALK_MS,
            Act::Pull { .. } => {
                now + self.task.as_ref().map_or(600, |t| heave_ms(t.cells.len())) / 2
            }
            Act::Climb { .. } => now + CLIMB_MS,
            Act::Fall { from_y, since, .. } => fall_time(since, (self.y - from_y + 1) as u64),
        }
    }

    /// When her pose next changes.
    pub fn due(&self) -> u64 {
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

    /// Run every event due by `now`. Returns whether her pose changed.
    pub fn tick(&mut self, now: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) -> bool {
        let mut changed = false;
        for _ in 0..64 {
            let due = self.due();
            if due > now {
                return changed;
            }
            changed = true;
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
            Act::Stand { .. } | Act::SpaceOut { .. } | Act::Dazed { .. } | Act::Admire { .. } => {
                self.decide(at, terrain, chances, rng)
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
                if let Some(task) = &self.task
                    && task.x == self.x
                    && task.y == self.y
                {
                    // At the line's end: face it and brace.
                    self.facing = match task.side {
                        Side::Left => Facing::Left,
                        Side::Right => Facing::Right,
                    };
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
                let Some(task) = self.task.clone() else {
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

    /// The text she was pulling changed under her (someone scrolled the
    /// chat): she lets go and stares.
    pub fn lost_grip(&mut self, now: u64) {
        if self.task.take().is_some() {
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
        let kept = self.goal.take().filter(|g| chances.pulls.contains(g));
        let fresh = kept.is_none() && !chances.pulls.is_empty() && rng.below(100) < 40;
        let target = kept.or_else(|| {
            fresh
                .then(|| {
                    let pick = rng.below(chances.pulls.len() as u64) as usize;
                    chances.pulls.get(pick).cloned()
                })
                .flatten()
        });
        if let Some(pull) = target {
            let there = terrain.platform_at(pull.x, pull.y);
            if there == Some(here) {
                tracing::debug!(
                    row = pull.row,
                    to = pull.x,
                    offered = chances.pulls.len(),
                    "houseguest: walking to a line to tidy"
                );
                if pull.x != self.x {
                    self.facing = toward(self.x, pull.x);
                }
                let to = pull.x;
                self.task = Some(pull);
                return self.set(Act::Walk { to, then: None }, at);
            }
            match there.and_then(|there| route(terrain, here, there)) {
                Some(link) => {
                    tracing::debug!(
                        row = pull.row,
                        via = ?link.route,
                        offered = chances.pulls.len(),
                        "houseguest: heading for a line on another floor"
                    );
                    self.goal = Some(pull);
                    if link.x != self.x {
                        self.facing = toward(self.x, link.x);
                    }
                    return self.set(
                        Act::Walk {
                            to: link.x,
                            then: Some(link),
                        },
                        at,
                    );
                }
                None => tracing::debug!(
                    row = pull.row,
                    offered = chances.pulls.len(),
                    "houseguest: a line to tidy, but no way there"
                ),
            }
        } else if chances.pulls.is_empty() {
            tracing::trace!("houseguest: nothing to tidy");
        }
        let roll = rng.below(100);
        let act = if roll < 30 {
            Act::Stand {
                until: at + rng.range(3000, 12_000),
            }
        } else if roll < 40 {
            Act::SpaceOut {
                until: at + rng.range(8000, 20_000),
            }
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

    /// A chat message arrived: stop and look at it.
    pub fn look(&mut self, now: u64, chat_x: i32) {
        self.watch_until = now + WATCH_MS;
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
                (
                    Pose::Stand,
                    if blink { Face::Blink } else { Face::Vacant },
                    None,
                )
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
            Act::Pull { heaving, .. } => {
                let row = self.task.as_ref().map_or(1, Pull::box_row);
                (Pose::Pull { heaving, row }, Face::Vacant, None)
            }
            Act::Admire { .. } => (Pose::Stand, Face::Pleased, Some(Bubble::Hehe)),
            Act::Look {
                surprised_until, ..
            } => {
                if now < surprised_until {
                    (Pose::Stand, Face::Surprised, Some(Bubble::Bang))
                } else {
                    (Pose::Stand, Face::Vacant, Some(Bubble::Huh))
                }
            }
        }
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
