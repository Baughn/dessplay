//! The houseguest stage: cue any of her scenes on demand, in a spot of
//! the room where it works, instead of waiting for luck. Drives the
//! `houseguest` example and the tests (which share its rooms).
//!
//! A cue is applied at the next paint, where the frame, terrain and her
//! chances are known: she is placed on a floor where the scene is
//! possible (a few cells from the spot, so the walk there shows) and
//! told to do it. Afterwards she carries on as usual.

use std::time::Duration;

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::Rect;

use super::osaka::{Activity, Chances};
use super::room::Use;
use super::scenes::{self, Job, Side};
use super::sprite::WIDTH;
use super::terrain::{Link, Route, Terrain};
use super::{Rng, Visit};
use crate::config::{Houseguest, Settings};
use crate::ui::app::{Ui, UiSnapshot};
use dessplay_core::types::UserId;

/// Something she can be asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// A fresh visit: she walks or drops in.
    Arrive,
    /// Pull a line of text along.
    Pull,
    /// Swap two letters of a word, and later back.
    Swap,
    /// Sneeze glyphs loose, then put them back.
    Sneeze,
    /// Climb a pole to a higher floor.
    ClimbUp,
    /// Climb a pole to a lower floor.
    ClimbDown,
    /// Peer over a ledge's end and hop down.
    Drop,
    /// Clamber over a divider to a floor on its other side.
    Clamber,
    /// Step out at a screen edge and come back in at another.
    StepOut,
    /// A door in space, to another floor.
    Door,
    /// Sit hugging her knees.
    Sit,
    /// Doze on her back.
    LieBack,
    /// Lie on her front, feet kicking.
    LieFront,
    /// Jumping jacks.
    Jacks,
    /// Toe touches.
    ToeTouch,
    /// A big stretch.
    Stretch,
    /// Gaze up at something.
    Gaze,
    /// Space out, saying something first.
    Muse,
    /// Sit on her sofa (she gets one if she has none).
    Lounge,
    /// Nap on her sofa, hugging the cushion.
    Nap,
    /// Sleep in her bed.
    Sleep,
    /// Homework at her desk.
    Homework,
    /// Watch her TV.
    Watch,
    /// A parcel arrives with her next piece, and she unpacks it.
    Parcel,
    /// The shopping channel comes on while she watches, and she buys.
    Shopping,
    /// Off to her part-time job (she gets a sofa if her home is empty).
    Work,
}

impl Scene {
    /// Every scene, in menu order.
    pub const ALL: [Scene; 26] = [
        Self::Arrive,
        Self::Pull,
        Self::Swap,
        Self::Sneeze,
        Self::ClimbUp,
        Self::ClimbDown,
        Self::Drop,
        Self::Clamber,
        Self::StepOut,
        Self::Door,
        Self::Sit,
        Self::LieBack,
        Self::LieFront,
        Self::Jacks,
        Self::ToeTouch,
        Self::Stretch,
        Self::Gaze,
        Self::Muse,
        Self::Lounge,
        Self::Nap,
        Self::Sleep,
        Self::Homework,
        Self::Watch,
        Self::Parcel,
        Self::Shopping,
        Self::Work,
    ];

    /// A short menu label.
    pub fn name(self) -> &'static str {
        match self {
            Self::Arrive => "arrive",
            Self::Pull => "pull a line",
            Self::Swap => "swap letters",
            Self::Sneeze => "sneeze",
            Self::ClimbUp => "climb up",
            Self::ClimbDown => "climb down",
            Self::Drop => "hop down",
            Self::Clamber => "clamber over",
            Self::StepOut => "step out",
            Self::Door => "door in space",
            Self::Sit => "sit",
            Self::LieBack => "lie on back",
            Self::LieFront => "lie on front",
            Self::Jacks => "jumping jacks",
            Self::ToeTouch => "toe touches",
            Self::Stretch => "stretch",
            Self::Gaze => "gaze",
            Self::Muse => "muse",
            Self::Lounge => "sit on the sofa",
            Self::Nap => "nap on the sofa",
            Self::Sleep => "sleep in bed",
            Self::Homework => "homework",
            Self::Watch => "watch TV",
            Self::Parcel => "a parcel",
            Self::Shopping => "shopping channel",
            Self::Work => "part-time job",
        }
    }

    /// What she does with her furniture in this scene.
    pub(super) fn furniture(self) -> Option<Use> {
        Some(match self {
            Self::Lounge => Use::Lounge,
            Self::Nap => Use::Nap,
            Self::Sleep => Use::Sleep,
            Self::Homework => Use::Homework,
            Self::Watch | Self::Shopping => Use::Watch,
            Self::Parcel => Use::Unpack,
            _ => return None,
        })
    }

    fn activity(self) -> Option<Activity> {
        Some(match self {
            Self::Sit => Activity::Sit,
            Self::LieBack => Activity::LieBack,
            Self::LieFront => Activity::LieFront,
            Self::Jacks => Activity::Jacks,
            Self::ToeTouch => Activity::ToeTouch,
            Self::Stretch => Activity::Stretch,
            Self::Gaze => Activity::Gaze,
            _ => return None,
        })
    }
}

/// A need the stage can make pressing, to provoke what answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// She gets sleepy.
    Sleepy,
    /// She gets restless.
    Restless,
    /// She wants to tidy.
    Tidy,
    /// She feels mischievous.
    Mischief,
}

impl Want {
    pub(super) fn need(self) -> super::brain::Need {
        use super::brain::Need;
        match self {
            Self::Sleepy => Need::Sleepy,
            Self::Restless => Need::Restless,
            Self::Tidy => Need::Tidy,
            Self::Mischief => Need::Mischief,
        }
    }
}

/// How far from the spot she starts, so the walk there shows.
const APPROACH: i32 = 6;

/// Place her where `scene` works and start it. `Ok` describes what she's
/// doing; `Err` says why this room offers no spot for it.
pub(super) fn direct(
    scene: Scene,
    buf: &Buffer,
    protected: &[Rect],
    visit: &mut Visit,
    chances: &Chances,
    now: u64,
    rng: &mut Rng,
) -> Result<String, String> {
    let (terrain, osaka) = (&visit.terrain, &mut visit.osaka);
    let name = scene.name();
    match scene {
        Scene::Arrive => Ok("arriving".into()),
        Scene::Pull | Scene::Swap => {
            let jobs: Vec<Job> = if scene == Scene::Pull {
                chances.pulls.iter().cloned().map(Job::Pull).collect()
            } else {
                chances.swaps.iter().cloned().map(Job::Swap).collect()
            };
            let job = pick(&jobs, rng).ok_or_else(|| format!("{name}: no text in reach here"))?;
            let (x, y) = job.spot();
            // Approach from the side away from the text.
            let away = match job.side() {
                Side::Left => 1,
                Side::Right => -1,
            };
            let start = approach(terrain, x, y, away);
            osaka.place(start, y, now);
            osaka.pursue(job, now);
            Ok(format!("{name} at ({x}, {y})"))
        }
        _ if scene.furniture().is_some() => {
            let what = scene.furniture();
            let seats: Vec<_> = chances
                .seats
                .iter()
                .filter(|s| Some(s.what) == what)
                .copied()
                .collect();
            let seat = pick(&seats, rng)
                .ok_or_else(|| format!("{name}: no room for her furniture here"))?;
            let job = Job::Use(seat);
            let away = match job.side() {
                Side::Left => 1,
                Side::Right => -1,
            };
            let start = approach(terrain, seat.x, seat.y, away);
            osaka.place(start, seat.y, now);
            osaka.pursue(job, now);
            Ok(format!("{name} at ({}, {})", seat.x, seat.y))
        }
        Scene::Sneeze => {
            let spot = terrain
                .platforms
                .iter()
                .flat_map(|p| (p.x0..=p.x1).map(move |x| (x, p.y)))
                .filter(|&(x, y)| terrain.clear(x, y))
                .map(|(x, y)| (scenes::loose(buf, protected, x, y).len(), x, y))
                .filter(|&(n, ..)| n >= 2)
                .max_by_key(|&(n, x, y)| (n.min(6), -(x + y)));
            let (n, x, y) = spot.ok_or_else(|| format!("{name}: no glyphs to knock loose"))?;
            osaka.place(x, y, now);
            osaka.sneeze_now(now);
            Ok(format!("{name} at ({x}, {y}), {n} glyphs in reach"))
        }
        Scene::Work => {
            // From a floor reaching a screen edge if there is one (she
            // walks out), else anywhere (she takes the door).
            let out = terrain
                .links
                .iter()
                .find(|l| matches!(l.route, Route::Around { .. }))
                .copied();
            let (x, y) = match out {
                Some(link) => {
                    let y = terrain.platforms.get(link.from).map_or(0, |p| p.y);
                    let middle = terrain
                        .platforms
                        .get(link.from)
                        .map_or(link.x, |p| (p.x0 + p.x1) / 2);
                    (
                        approach(terrain, link.x, y, if middle < link.x { -1 } else { 1 }),
                        y,
                    )
                }
                None => terrain
                    .platforms
                    .iter()
                    .map(|p| ((p.x0 + p.x1) / 2, p.y))
                    .find(|&(x, y)| terrain.clear(x, y))
                    .ok_or_else(|| format!("{name}: nowhere to stand"))?,
            };
            osaka.place(x, y, now);
            let how = if out.is_some() { "walking" } else { "by door" };
            osaka.go_to_work(out, now, rng);
            Ok(format!("{name}, {how}"))
        }
        Scene::Door => {
            let here = terrain.platform_at(osaka.x, osaka.y);
            let floors: Vec<_> = terrain
                .platforms
                .iter()
                .enumerate()
                .filter(|&(i, _)| Some(i) != here)
                .map(|(_, p)| ((p.x0 + p.x1) / 2, p.y))
                .collect();
            let to = pick(&floors, rng).ok_or_else(|| format!("{name}: nowhere else to go"))?;
            if here.is_none() {
                let start = terrain
                    .platforms
                    .iter()
                    .map(|p| ((p.x0 + p.x1) / 2, p.y))
                    .find(|&spot| spot != to)
                    .ok_or_else(|| format!("{name}: nowhere to stand"))?;
                osaka.place(start.0, start.1, now);
            }
            osaka.through_door(to, now);
            Ok(format!("{name} to ({}, {})", to.0, to.1))
        }
        Scene::ClimbUp | Scene::ClimbDown | Scene::Drop | Scene::Clamber | Scene::StepOut => {
            let wanted = |link: &&Link| {
                let from = terrain.platforms.get(link.from).map_or(0, |p| p.y);
                let to = terrain.platforms.get(link.to).map_or(0, |p| p.y);
                match (scene, link.route) {
                    (Scene::ClimbUp, Route::Climb) => to < from,
                    (Scene::ClimbDown, Route::Climb) => to > from,
                    (Scene::Drop, Route::Drop { .. }) => true,
                    (Scene::Clamber, Route::Clamber { .. }) => true,
                    (Scene::StepOut, Route::Around { .. }) => true,
                    _ => false,
                }
            };
            let links: Vec<Link> = terrain.links.iter().filter(wanted).copied().collect();
            let link = pick(&links, rng).ok_or_else(|| format!("{name}: no such way here"))?;
            let y = terrain.platforms.get(link.from).map_or(0, |p| p.y);
            // Approach from inside the floor (drops are at its ends).
            let middle = terrain
                .platforms
                .get(link.from)
                .map_or(link.x, |p| (p.x0 + p.x1) / 2);
            let away = if middle < link.x { -1 } else { 1 };
            let start = approach(terrain, link.x, y, away);
            osaka.place(start, y, now);
            osaka.travel(link, now);
            Ok(format!("{name} at ({}, {y})", link.x))
        }
        _ => {
            if terrain.platform_at(osaka.x, osaka.y).is_none() {
                let spot = terrain
                    .platforms
                    .iter()
                    .map(|p| ((p.x0 + p.x1) / 2, p.y))
                    .find(|&(x, y)| terrain.clear(x, y))
                    .ok_or_else(|| format!("{name}: nowhere to stand"))?;
                osaka.place(spot.0, spot.1, now);
            }
            match scene.activity() {
                Some(what) => osaka.idle(what, now, rng),
                None => osaka.muse(now, rng),
            }
            Ok(name.into())
        }
    }
}

fn pick<T: Clone>(items: &[T], rng: &mut Rng) -> Option<T> {
    items.get(rng.below(items.len() as u64) as usize).cloned()
}

/// Up to [`APPROACH`] cells from `x` in direction `away`, as far as the
/// floor stays standable.
fn approach(terrain: &Terrain, x: i32, y: i32, away: i32) -> i32 {
    let mut at = x;
    for _ in 0..APPROACH {
        let next = at + away;
        if terrain.platform_at(next, y).is_none() || !terrain.clear(next, y) {
            break;
        }
        at = next;
    }
    // Never so close to the screen edge that she'd be half off it.
    at.max(WIDTH / 2)
}

// ---- Rooms ----

/// The idle delay the stage rooms are set up with.
pub const DELAY: Duration = Duration::from_secs(5);

/// The real UI with the default layout, idle, and nothing in it.
pub fn real_ui() -> Ui {
    let mut ui = Ui::with_setup(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("test".into()),
            houseguest: Houseguest::After(DELAY),
            ..Settings::default()
        },
        vec![],
        false,
    );
    ui.apply_snapshot(UiSnapshot::default());
    ui
}

/// The real UI with `n` short chat messages (alternating senders).
pub fn chatty_ui(n: usize) -> Ui {
    chat_ui((0..n).map(|i| format!("line {i}")))
}

/// The stage's room: the real UI with an evening's worth of chat.
pub fn stage_ui() -> Ui {
    // The last four lines sit right above the chat's floor, where she
    // stands: the longer one at her chest is the one she can reach (her
    // box must fit beside it, clear of the lines around it).
    const LINES: [&str; 16] = [
        "is everyone here?",
        "almost, brb",
        "what are we watching",
        "episode four",
        "ok",
        "the cat sat",
        "no spoilers",
        "yes",
        "why is the sky blue",
        "osaka would know",
        "snacks ready",
        "hi all",
        "did you hear that",
        "same",
        "starting soon",
        "wait for me",
    ];
    chat_ui((0..40).map(|i| LINES[i % LINES.len()].to_string()))
}

pub(super) fn chat_ui(lines: impl Iterator<Item = String>) -> Ui {
    let mut ui = real_ui();
    let senders = ["kim", "bob", "ana"];
    let view = dessplay_core::state::StateView {
        chat: lines
            .enumerate()
            .map(|(i, text)| dessplay_core::types::ChatMessage {
                timestamp: dessplay_core::types::SharedTimestamp(1_000 + i as u64 * 60_000),
                sender: UserId::new(senders[i % 2 + i / 7 % 2]),
                text,
            })
            .collect(),
        ..Default::default()
    };
    ui.apply_snapshot(UiSnapshot {
        view: std::sync::Arc::new(view),
        ..UiSnapshot::default()
    });
    ui
}
