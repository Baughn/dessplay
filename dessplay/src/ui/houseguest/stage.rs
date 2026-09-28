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
}

impl Scene {
    /// Every scene, in menu order.
    pub const ALL: [Scene; 15] = [
        Self::Arrive,
        Self::Pull,
        Self::Swap,
        Self::Sneeze,
        Self::ClimbUp,
        Self::ClimbDown,
        Self::Drop,
        Self::Sit,
        Self::LieBack,
        Self::LieFront,
        Self::Jacks,
        Self::ToeTouch,
        Self::Stretch,
        Self::Gaze,
        Self::Muse,
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
            Self::Sit => "sit",
            Self::LieBack => "lie on back",
            Self::LieFront => "lie on front",
            Self::Jacks => "jumping jacks",
            Self::ToeTouch => "toe touches",
            Self::Stretch => "stretch",
            Self::Gaze => "gaze",
            Self::Muse => "muse",
        }
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
        Scene::ClimbUp | Scene::ClimbDown | Scene::Drop => {
            let wanted = |link: &&Link| {
                let from = terrain.platforms.get(link.from).map_or(0, |p| p.y);
                let to = terrain.platforms.get(link.to).map_or(0, |p| p.y);
                match (scene, link.route) {
                    (Scene::ClimbUp, Route::Climb) => to < from,
                    (Scene::ClimbDown, Route::Climb) => to > from,
                    (Scene::Drop, Route::Drop { .. }) => true,
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
