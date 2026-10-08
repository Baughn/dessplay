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
use super::room::{Furniture, Use};
use super::scenes::{self, Job, Side};
use super::script::{Cue, ScriptId, SpliceId};
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
    /// Doze off sitting up, head sinking onto her knees (she only
    /// settles into it, from sitting).
    SitDoze,
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
    /// Homework lying on the floor, on her front, writing (where she has
    /// no desk), nodding off.
    FloorHomework,
    /// Homework lying on the floor, on her back, reading the set text,
    /// nodding off under the book.
    BookHomework,
    /// Reading lying on her back (where she has no bookshelf).
    LieRead,
    /// Dozing under the book, open over her eyes (she only settles into
    /// it, from reading on her back).
    BookDoze,
    /// Sit in front of the window, chin in her hands (she only settles
    /// into it, from leaning on its sill).
    UnderSill,
    /// Lie on her back watching the clouds (she only settles into it,
    /// under her window).
    CloudWatch,
    /// Space out, saying something first.
    Muse,
    /// Space out, telling a riddle (and answering it herself).
    Riddle,
    /// Sit on her sofa (she gets one if she has none).
    Lounge,
    /// Nap on her sofa, hugging the cushion.
    Nap,
    /// Sleep in her bed.
    Sleep,
    /// Her night's sleep in her bed: until her wake time if it's bedtime,
    /// else as long as a day's sleep (chat only stirs her).
    Night,
    /// Homework at her desk.
    Homework,
    /// Watch her TV.
    Watch,
    /// A parcel arrives with her next piece, and she unpacks it.
    Parcel,
    /// The shopping channel comes on while she watches, and she buys.
    Shopping,
    /// Watching her TV, she flicks through the channels.
    Surf,
    /// Off to her part-time job (she gets a sofa if her home is empty).
    Work,
    /// Reading beside her bookshelf.
    Read,
    /// Borrowing a strip of a line to read beside the tear, then sliding
    /// it back (as she does where she has no bookshelf).
    Borrow,
    /// A snack from her fridge.
    Snack,
    /// Petting the cat in his bed (he's home for it).
    Pet,
    /// Before her homework, she splits a pair of chopsticks cleanly.
    ChopsticksClean,
    /// Before her homework, she splits a pair of chopsticks badly.
    ChopsticksBad,
    /// After a snack, a sata andagi.
    Andagi,
    /// Tear text off a line and crumple it into a makeshift sofa.
    MakeSofa,
    /// Tear text off a line and crumple it into a makeshift bed.
    MakeBed,
    /// Tear text off a line and crumple it into a paper desk, then do
    /// her homework at it.
    MakeDesk,
    /// Her sofa turned away from her TV (she gets both if she hasn't
    /// them): she's felt it, and turns it round.
    Arrange,
    /// She dashes home from school for her lunch: in through her door,
    /// to her fridge (she gets one if she has none), and (at school time)
    /// out again.
    DashIn,
    /// She dashes home from school, and can't think what for: in through
    /// her door, a moment wondering, and (at school time) out again.
    DashForgot,
    /// Setsubun's beans, thrown on the spot (her calendar's on Feb 3).
    Setsubun,
    /// The first sunrise of the year on her TV (her calendar's on Jan 1;
    /// she gets a TV if she has none).
    FirstSunrise,
    /// The Dream, rare: her night's sleep in her bed (she gets one if she
    /// has none), dreamt from the start ("Hello everynyan...").
    Dream,
    /// After a snack, rare: that was the last melon bread (she gets a
    /// fridge if she has none).
    NoMelon,
    /// Spacing out, rare: which one's the escalator.
    Escalator,
    /// After lounging on her sofa, rare: a scary story (she gets a sofa
    /// if she has none; whatever the hour).
    Scary,
    /// She looks out of her window at the sky (she gets one if she has
    /// none), saying what she sees.
    LookOut,
    /// Spacing out, a glance up at her wall clock, saying the hour,
    /// roughly (three o'clock, without her clock to tell her).
    ClockGlance,
}

impl Scene {
    /// Every scene, in menu order.
    pub const ALL: [Scene; 57] = [
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
        Self::SitDoze,
        Self::LieFront,
        Self::Jacks,
        Self::ToeTouch,
        Self::Stretch,
        Self::Gaze,
        Self::FloorHomework,
        Self::BookHomework,
        Self::LieRead,
        Self::BookDoze,
        Self::UnderSill,
        Self::CloudWatch,
        Self::Muse,
        Self::Riddle,
        Self::Lounge,
        Self::Nap,
        Self::Sleep,
        Self::Night,
        Self::Homework,
        Self::Watch,
        Self::Parcel,
        Self::Shopping,
        Self::Surf,
        Self::Work,
        Self::Read,
        Self::Borrow,
        Self::Snack,
        Self::Pet,
        Self::ChopsticksClean,
        Self::ChopsticksBad,
        Self::Andagi,
        Self::MakeSofa,
        Self::MakeBed,
        Self::MakeDesk,
        Self::Arrange,
        Self::DashIn,
        Self::DashForgot,
        Self::Setsubun,
        Self::FirstSunrise,
        Self::Dream,
        Self::NoMelon,
        Self::Escalator,
        Self::Scary,
        Self::LookOut,
        Self::ClockGlance,
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
            Self::SitDoze => "doze sitting up",
            Self::LieFront => "lie on front",
            Self::Jacks => "jumping jacks",
            Self::ToeTouch => "toe touches",
            Self::Stretch => "stretch",
            Self::Gaze => "gaze",
            Self::FloorHomework => "homework on the floor",
            Self::BookHomework => "homework on her back",
            Self::LieRead => "read on her back",
            Self::BookDoze => "doze under the book",
            Self::UnderSill => "sit under the window",
            Self::CloudWatch => "watch the clouds",
            Self::Muse => "muse",
            Self::Riddle => "tell a riddle",
            Self::Lounge => "sit on the sofa",
            Self::Nap => "nap on the sofa",
            Self::Sleep => "sleep in bed",
            Self::Night => "a night's sleep",
            Self::Homework => "homework",
            Self::Watch => "watch TV",
            Self::Parcel => "a parcel",
            Self::Shopping => "shopping channel",
            Self::Surf => "channel surfing",
            Self::Work => "part-time job",
            Self::Read => "read",
            Self::Borrow => "read a borrowed line",
            Self::Snack => "snack",
            Self::Pet => "pet the cat",
            Self::ChopsticksClean => "chopsticks, clean split",
            Self::ChopsticksBad => "chopsticks, bad split",
            Self::Andagi => "sata andagi",
            Self::MakeSofa => "make a sofa of text",
            Self::MakeBed => "make a bed of text",
            Self::MakeDesk => "make a desk of text",
            Self::Arrange => "turn the sofa round",
            Self::DashIn => "dash home for lunch",
            Self::DashForgot => "dash home, forgetful",
            Self::Setsubun => "Setsubun's beans",
            Self::FirstSunrise => "first sunrise",
            Self::Dream => "the Dream",
            Self::NoMelon => "no melon bread left",
            Self::Escalator => "which one's the escalator",
            Self::Scary => "scary story time",
            Self::LookOut => "look out of the window",
            Self::ClockGlance => "glance at the clock",
        }
    }

    /// What she does with her furniture in this scene.
    pub(super) fn furniture(self) -> Option<Use> {
        Some(match self {
            Self::Lounge | Self::Scary => Use::Lounge,
            Self::Nap => Use::Nap,
            Self::Sleep | Self::Night | Self::Dream => Use::Sleep,
            Self::Homework | Self::ChopsticksClean | Self::ChopsticksBad => Use::Homework,
            Self::Watch | Self::Shopping | Self::Surf | Self::FirstSunrise => Use::Watch,
            Self::Parcel => Use::Unpack,
            Self::Read => Use::Read,
            Self::Snack | Self::Andagi | Self::DashIn | Self::NoMelon => Use::Snack,
            Self::Pet => Use::Pet,
            Self::LookOut => Use::LookOut,
            _ => return None,
        })
    }

    /// What it has her play, forced rather than rolled: each script
    /// that shares its piece with another (a plain watch, surfing, the
    /// shopping channel; a day's sleep, her night's; a snack, the lunch
    /// she dashes home for: cued to one, she plays none of the others), a
    /// riddle (musing, she might not tell one), and each splice (the
    /// chopsticks on the branch named). Wildcard-free, so a new scene
    /// says.
    pub(super) fn cue(self) -> Option<Cue> {
        match self {
            Self::ChopsticksClean => Some(Cue::Splice(SpliceId::Chopsticks, Some(0))),
            Self::ChopsticksBad => Some(Cue::Splice(SpliceId::Chopsticks, Some(1))),
            Self::Andagi => Some(Cue::Splice(SpliceId::Andagi, None)),
            Self::Watch => Some(Cue::Script(ScriptId::Watch)),
            Self::Shopping => Some(Cue::Script(ScriptId::Shopping)),
            Self::Surf => Some(Cue::Script(ScriptId::Surf)),
            Self::Riddle => Some(Cue::Script(ScriptId::Riddle)),
            // A day's sleep and her night's both play on her bed.
            Self::Sleep => Some(Cue::Script(ScriptId::Sleep)),
            Self::Night => Some(Cue::Script(ScriptId::Night)),
            // A snack and the lunch she dashes home for both play on her
            // fridge; dashed home with one to hand, she still forgets,
            // cued to.
            Self::Snack => Some(Cue::Script(ScriptId::Snack)),
            Self::DashIn => Some(Cue::Script(ScriptId::DashLunch)),
            Self::DashForgot => Some(Cue::Script(ScriptId::DashForgot)),
            // Her calendar's, on the stage whatever the date: the beans
            // spacing out (as a riddle is), the sunrise on a watch.
            Self::Setsubun => Some(Cue::Script(ScriptId::Setsubun)),
            Self::FirstSunrise => Some(Cue::Script(ScriptId::FirstSunrise)),
            // Her rare things, forced whatever's open today: the Dream on
            // her bed (as her night is), the escalator spacing out (as a
            // riddle is), the melon bread and the scary story as the
            // splices they are.
            Self::Dream => Some(Cue::Script(ScriptId::Dream)),
            Self::Escalator => Some(Cue::Script(ScriptId::Escalator)),
            Self::NoMelon => Some(Cue::Splice(SpliceId::NoMelon, None)),
            Self::Scary => Some(Cue::Splice(SpliceId::Scary, None)),
            // A glance at the clock spacing out (as a riddle is).
            Self::ClockGlance => Some(Cue::Script(ScriptId::ClockGlance)),
            Self::Arrive
            | Self::Pull
            | Self::Swap
            | Self::Sneeze
            | Self::ClimbUp
            | Self::ClimbDown
            | Self::Drop
            | Self::Clamber
            | Self::StepOut
            | Self::Door
            | Self::Sit
            | Self::LieBack
            | Self::SitDoze
            | Self::LieFront
            | Self::Jacks
            | Self::ToeTouch
            | Self::Stretch
            | Self::Gaze
            | Self::FloorHomework
            | Self::BookHomework
            | Self::LieRead
            | Self::BookDoze
            | Self::UnderSill
            | Self::CloudWatch
            | Self::Muse
            | Self::Lounge
            | Self::Nap
            | Self::Homework
            | Self::Parcel
            | Self::Work
            | Self::Read
            | Self::Borrow
            | Self::Pet
            | Self::MakeSofa
            | Self::MakeBed
            | Self::MakeDesk
            | Self::Arrange
            | Self::LookOut => None,
        }
    }

    fn activity(self) -> Option<Activity> {
        Some(match self {
            Self::Sit => Activity::Sit,
            Self::LieBack => Activity::LieBack,
            Self::SitDoze => Activity::SitDoze,
            Self::LieFront => Activity::LieFront,
            Self::Jacks => Activity::Jacks,
            Self::ToeTouch => Activity::ToeTouch,
            Self::Stretch => Activity::Stretch,
            Self::Gaze => Activity::Gaze,
            Self::FloorHomework => Activity::FloorHomework,
            Self::LieRead => Activity::LieRead,
            Self::BookDoze => Activity::BookDoze,
            Self::UnderSill => Activity::UnderSill,
            Self::CloudWatch => Activity::CloudWatch,
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
    /// She gets peckish.
    Hungry,
    /// She wants her home right (what she's felt is wrong with it).
    Nesting,
}

impl Want {
    pub(super) fn need(self) -> super::brain::Need {
        use super::brain::Need;
        match self {
            Self::Sleepy => Need::Sleepy,
            Self::Restless => Need::Restless,
            Self::Tidy => Need::Tidy,
            Self::Mischief => Need::Mischief,
            Self::Hungry => Need::Hungry,
            Self::Nesting => Need::Nesting,
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
    // The frame as it is (her tick may not have seen it yet): where her
    // clock hangs, for a glance up at it.
    osaka.take_in(chances);
    let name = scene.name();
    osaka.cue(scene.cue());
    match scene {
        Scene::Arrive => Ok("arriving".into()),
        // In through her door (where her home and the frame stand it,
        // the door batch): what she forgot, she goes for as she first
        // decides (her routine sends her out after, at school time).
        // Just dashed in, she's through it already.
        Scene::DashIn | Scene::DashForgot => {
            if osaka.dashing() {
                return Ok(format!("{name}: in through her door"));
            }
            let Some(door) = chances.door else {
                // Not left to wait for a later snack.
                osaka.cue(None);
                return Err(format!("{name}: nowhere for her door to stand"));
            };
            let (x, y) = door.spot();
            osaka.dash_through((x, y), door.into_room(), now);
            Ok(format!("{name}: in through her door at ({x}, {y})"))
        }
        // The sofa was set up turned from the TV as the frame was read,
        // and she's felt it: she sets off to lift it.
        Scene::Arrange => {
            let repair = *chances
                .repairs
                .first()
                .ok_or_else(|| format!("{name}: no way to put it right here"))?;
            let at = visit
                .shown
                .iter()
                .find(|s| s.item == repair.piece && s.scrap.is_none())
                .ok_or_else(|| format!("{name}: her {} isn't out", repair.piece.spec().name))?;
            let ((x, y), side) = super::reach(at, terrain, osaka.x)
                .ok_or_else(|| format!("{name}: nowhere to stand to lift it"))?;
            let away = match side {
                Side::Left => 1,
                Side::Right => -1,
            };
            let start = approach(terrain, x, y, away);
            osaka.place(start, y, now);
            osaka.lift(
                scenes::Lift {
                    repair,
                    trials: super::rules::Trials::default(),
                    x,
                    y,
                    side,
                },
                now,
            );
            Ok(format!("{name}: {}", repair.label()))
        }
        Scene::Pull | Scene::Swap | Scene::Borrow => {
            let jobs: Vec<Job> = match scene {
                Scene::Pull => chances.pulls.iter().cloned().map(Job::Pull).collect(),
                Scene::Borrow => chances.borrows.iter().cloned().map(Job::Borrow).collect(),
                _ => chances.swaps.iter().cloned().map(Job::Swap).collect(),
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
        Scene::MakeSofa | Scene::MakeBed | Scene::MakeDesk => {
            let item = match scene {
                Scene::MakeSofa => Furniture::Sofa,
                Scene::MakeBed => Furniture::Bed,
                _ => Furniture::Desk,
            };
            let builds: Vec<_> = chances
                .builds
                .iter()
                .filter(|b| b.piece.item == item)
                .cloned()
                .collect();
            let build = pick(&builds, rng)
                .ok_or_else(|| format!("{name}: no text to tear, or no room for it"))?;
            let job = Job::Build(build);
            let (x, y) = job.spot();
            let away = match job.side() {
                Side::Left => 1,
                Side::Right => -1,
            };
            let start = approach(terrain, x, y, away);
            osaka.place(start, y, now);
            osaka.pursue(job, now);
            Ok(format!("{name} from ({x}, {y})"))
        }
        Scene::Sneeze => {
            let spot = terrain
                .platforms
                .iter()
                .flat_map(|p| (p.x0..=p.x1).map(move |x| (x, p.y)))
                .filter(|&(x, y)| terrain.restful(x, y))
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
                    .find(|&(x, y)| terrain.restful(x, y))
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
                    .find(|&(x, y)| terrain.restful(x, y))
                    .ok_or_else(|| format!("{name}: nowhere to stand"))?;
                osaka.place(spot.0, spot.1, now);
            }
            match scene.activity() {
                Some(what) => osaka.idle(what, now, rng),
                None if scene == Scene::BookHomework => osaka.floor_homework(true, now, rng),
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
            // A visitor: a resident keeps out of the focused pane, and
            // the chat is focused.
            houseguest_resident: false,
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
    room_ui(
        (0..40).map(|i| LINES[i % LINES.len()].to_string()),
        &PLAYLIST,
    )
}

/// The stage's playlist: the text she tears off for furniture.
const PLAYLIST: [&str; 6] = [
    "Azumanga Daioh - 01.mkv",
    "Azumanga Daioh - 02.mkv",
    "Frieren - 12.mkv",
    "Haibane Renmei - 03.mkv",
    "Yotsuba to! - 07.mkv",
    "Mushishi - 05.mkv",
];

pub(super) fn chat_ui(lines: impl Iterator<Item = String>) -> Ui {
    room_ui(lines, &[])
}

/// The real UI with chat `lines` and a playlist of `files`.
fn room_ui(lines: impl Iterator<Item = String>, files: &[&str]) -> Ui {
    use dessplay_core::playlist::NewPlaylistEntry;
    use dessplay_core::state::CrdtState;
    use dessplay_core::types::{ActorId, Ed2kHash, SharedTimestamp};
    let mut ui = real_ui();
    let mut state = CrdtState::new();
    for (i, &file) in files.iter().enumerate() {
        state.push_playlist_entry(
            ActorId::SERVER,
            SharedTimestamp(1 + i as u64),
            NewPlaylistEntry {
                hash: Ed2kHash([i as u8 + 1; 16]),
                added_by: UserId::new("kim"),
                filename: file.into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
    }
    let senders = ["kim", "bob", "ana"];
    let view = dessplay_core::state::StateView {
        playlist: state.view().playlist,
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
