//! Osaka as line art: the SVG parts in `art/osaka.svg`, posed by a small
//! rig and rendered with resvg to the pixel size of her 5×4-cell box.
//! Terminals with a graphics protocol show these instead of the ASCII
//! sprite.

use resvg::tiny_skia;
use resvg::usvg;

use super::room::Furniture;
use super::sprite::{Face, Facing, Pose};

const PARTS: &str = include_str!("art/osaka.svg");
const PROPS: &str = include_str!("art/props.svg");
const DOOR: &str = include_str!("art/door.svg");
const DELIVERY: &str = include_str!("art/delivery.svg");
/// A sata andagi in her hand, whole and bitten (see [`Rig::eating_food`]).
const ANDAGI: [&str; 2] = ["andagi", "andagi-bitten"];
/// Prop units per cell (her scale at a 9 × 19 px cell), so her
/// furniture shares her line weights.
pub(super) const CELL_UNITS: (f32, f32) = (20.0, 42.0);
/// The parts' canvas; feet rest on its bottom edge.
const CANVAS_W: f32 = 100.0;
const CANVAS_H: f32 = 160.0;
const HIP_Y: f32 = 120.0;
const HIPS: [f32; 2] = [44.5, 55.5];
const THIGH: f32 = 18.0;
const SHOULDER_Y: f32 = 75.0;
const SHOULDERS: [f32; 2] = [33.5, 66.5];
const UPPER_ARM: f32 = 13.0;
const NECK: (f32, f32) = (50.0, 70.0);
/// Profile pivots: shoulders and hips stacked, far limb first.
const P_SHOULDERS: [f32; 2] = [49.0, 52.0];
const P_SHOULDER_Y: f32 = 76.0;
const P_HIPS: [f32; 2] = [48.0, 52.5];
/// The paper desk's top in her canvas (see `scrap::DESK`), where her
/// paper lies when she does homework at it.
const PAPER_DESK_TOP: f32 = 112.0;
/// A standing image spans ~4.5 rows of the canvas's 160 units.
const ROW_UNITS: f32 = CANVAS_H / 4.5;

/// Her expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Expression {
    Vacant,
    Blink,
    Surprised,
    Smile,
    Dizzy,
    Happy,
    Curious,
    /// Let down: heavy lids over low eyes, mouth turned down.
    Droop,
}

impl Expression {
    fn id(self) -> &'static str {
        match self {
            Self::Vacant => "face-vacant",
            Self::Blink => "face-blink",
            Self::Surprised => "face-surprised",
            Self::Smile => "face-smile",
            Self::Dizzy => "face-dizzy",
            Self::Happy => "face-happy",
            Self::Curious => "face-curious",
            Self::Droop => "face-droop",
        }
    }
}

/// The rig angle (degrees clockwise from hanging down) that points from
/// `from` at `to`.
fn aim(from: (f32, f32), to: (f32, f32)) -> f32 {
    -(to.0 - from.0).atan2(to.1 - from.1).to_degrees()
}

/// Joint angles in degrees, clockwise on screen; limbs hang straight
/// down at 0. Index 0 is the limb on the viewer's left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rig {
    pub expression: Expression,
    /// Whole upper body about the hips.
    pub lean: f32,
    /// Head about the neck.
    pub tilt: f32,
    /// Vertical offset in canvas units (negative is up).
    pub bob: f32,
    /// Horizontal offset in canvas units, toward the side she faces.
    pub shift: f32,
    /// Seen from the side (facing right before mirroring): index 0 is her
    /// far limb, drawn behind her.
    pub profile: bool,
    /// The whole figure turned about her hips (degrees clockwise): −90
    /// lies her on her back, 90 on her stomach (in profile).
    pub turn: f32,
    /// The whole figure's scale (lying down she must fit the box width).
    pub scale: f32,
    /// Legs drawn in front of her skirt (knees up in front of her).
    pub legs_front: bool,
    /// Sitting on her stool (drawn under her, at the canvas floor).
    pub stool: bool,
    /// Hugging the sofa's throw cushion to her chest.
    pub cushion: bool,
    /// Sitting on something (in profile): her skirt drapes forward over
    /// her lap and her legs are the seated part (thighs along the seat,
    /// shins hanging); `legs` is ignored.
    pub seated: bool,
    /// Sitting cross-legged on the floor (in profile): her legs are the
    /// crossed-legs part, out in front of her skirt; `legs` is ignored.
    pub crossed: bool,
    /// Cradling a bundle of leeks across her chest (frontal only): drawn
    /// in front of her body, under her arms, whose hands hold it.
    pub leeks: bool,
    /// Something in her near hand: a part of `art/osaka.svg`, placed at
    /// `(x, y)` (her own, pre-bob coordinates) and turned `angle`
    /// degrees; drawn over her body and head, under her near arm.
    pub hold: Option<(&'static str, f32, f32, f32)>,
    /// Something lying on the floor or a surface in front of her (her
    /// paper): a part of `art/osaka.svg` placed at `(x, y)` in canvas
    /// coordinates (not bobbed, turned or scaled with her) and turned
    /// `angle` degrees; drawn behind her, so her hands go over it.
    pub ground: Option<(&'static str, f32, f32, f32)>,
    /// (shoulder, elbow).
    pub arms: [(f32, f32); 2],
    /// (hip, knee).
    pub legs: [(f32, f32); 2],
}

impl Rig {
    fn standing(expression: Expression) -> Self {
        Self {
            expression,
            lean: 0.0,
            tilt: 0.0,
            bob: 0.0,
            shift: 0.0,
            profile: false,
            turn: 0.0,
            scale: 1.0,
            legs_front: false,
            stool: false,
            cushion: false,
            seated: false,
            crossed: false,
            leeks: false,
            hold: None,
            ground: None,
            arms: [(12.0, -4.0), (-12.0, 4.0)],
            legs: [(3.0, 0.0), (-3.0, 0.0)],
        }
    }

    /// The rig for a sprite pose (drawn facing right; see [`render`] for
    /// mirroring).
    pub fn for_pose(pose: Pose, face: Face) -> Self {
        let expression = match face {
            Face::Vacant => Expression::Vacant,
            Face::Blink => Expression::Blink,
            Face::Surprised => Expression::Surprised,
            Face::Pleased => Expression::Smile,
            Face::Happy => Expression::Happy,
            Face::Curious => Expression::Curious,
            Face::Droop => Expression::Droop,
        };
        let stand = Self::standing(expression);
        match pose {
            Pose::Stand => stand,
            // In profile, facing right: a forward swing is
            // counter-clockwise. Index 0 is the far limb.
            Pose::Walk(frame) => {
                let (legs, arms, bob) = match frame % 4 {
                    0 => (
                        [(18.0, 22.0), (-22.0, 4.0)],
                        [(-20.0, -14.0), (22.0, -6.0)],
                        0.0,
                    ),
                    1 => (
                        [(4.0, 26.0), (-2.0, 0.0)],
                        [(-6.0, -8.0), (6.0, -4.0)],
                        -2.5,
                    ),
                    2 => (
                        [(-22.0, 4.0), (18.0, 22.0)],
                        [(22.0, -6.0), (-20.0, -14.0)],
                        0.0,
                    ),
                    _ => (
                        [(-2.0, 0.0), (4.0, 26.0)],
                        [(6.0, -4.0), (-6.0, -8.0)],
                        -2.5,
                    ),
                };
                Self {
                    legs,
                    arms,
                    bob,
                    profile: true,
                    ..stand
                }
            }
            Pose::Climb { frame, pole } => Self::climbing(frame, pole, expression),
            // Sitting on the floor in profile, hugging her knees.
            Pose::Sit => Self {
                bob: 34.0,
                lean: -4.0,
                tilt: 6.0,
                profile: true,
                legs_front: true,
                legs: [(-146.0, 124.0), (-138.0, 118.0)],
                arms: [(-26.0, -20.0), (-18.0, -24.0)],
                ..stand
            },
            // On her back, knees up, hands behind her head; one foot
            // swinging.
            Pose::LieBack(frame) => Self {
                turn: -90.0,
                scale: 0.74,
                // Head towards the back edge: keep it inside the box.
                shift: 30.0,
                bob: 17.0,
                tilt: -6.0,
                profile: true,
                legs: if frame % 2 == 0 {
                    [(-62.0, 118.0), (-48.0, 104.0)]
                } else {
                    [(-62.0, 118.0), (-30.0, 70.0)]
                },
                arms: [(168.0, 128.0), (160.0, 134.0)],
                ..stand
            },
            // On her stomach, chin on her hands, feet kicking.
            Pose::LieFront(frame) => Self {
                turn: 90.0,
                scale: 0.74,
                shift: -16.0,
                bob: 17.0,
                tilt: -78.0,
                profile: true,
                legs: if frame % 2 == 0 {
                    [(0.0, -118.0), (0.0, -76.0)]
                } else {
                    [(0.0, -80.0), (0.0, -120.0)]
                },
                arms: [(-150.0, 110.0), (-140.0, 100.0)],
                ..stand
            },
            Pose::Jack(frame) => {
                if frame % 2 == 0 {
                    Self {
                        arms: [(14.0, 0.0), (-14.0, 0.0)],
                        legs: [(2.0, 0.0), (-2.0, 0.0)],
                        ..stand
                    }
                } else {
                    Self {
                        bob: -6.0,
                        arms: [(132.0, 10.0), (-132.0, -10.0)],
                        legs: [(18.0, 0.0), (-18.0, 0.0)],
                        ..stand
                    }
                }
            }
            Pose::ToeTouch(frame) => {
                if frame % 2 == 0 {
                    Self {
                        profile: true,
                        arms: [(-160.0, 0.0), (-170.0, 0.0)],
                        legs: [(0.0, 0.0), (0.0, 0.0)],
                        ..stand
                    }
                } else {
                    Self {
                        profile: true,
                        lean: 52.0,
                        shift: -32.0,
                        tilt: -24.0,
                        arms: [(-50.0, 0.0), (-60.0, 0.0)],
                        legs: [(0.0, 0.0), (0.0, 0.0)],
                        ..stand
                    }
                }
            }
            Pose::Stretch => Self {
                bob: -2.0,
                arms: [(150.0, -20.0), (-150.0, 20.0)],
                legs: [(1.0, 0.0), (-1.0, 0.0)],
                ..stand
            },
            Pose::Side => Self {
                profile: true,
                arms: [(6.0, -8.0), (-4.0, -10.0)],
                ..stand
            },
            Pose::Lounge => Self::sofa_sit(expression),
            Pose::Nap(_) => Self::sofa_nap(expression),
            Pose::Sleep(frame) => Self::bed_sleep(frame),
            // Writing (two frames), nodding off, asleep on the paper.
            Pose::Homework(frame) => Self::homework(frame.saturating_sub(1), expression),
            Pose::Carry(frame) => Self::carrying(frame, expression),
            Pose::Read(frame) => Self::reading(frame, expression),
            Pose::Eat(frame) => Self::eating(frame, expression),
            // Once bitten, it stays bitten: chewing, she holds what's
            // left of it where she held it whole.
            Pose::EatAndagi(frame @ 0..=1) => Self::eating_food(frame, ANDAGI, expression),
            Pose::EatAndagi(_) => Self::eating_food(0, [ANDAGI[1]; 2], expression),
            Pose::Pet(frame) => Self::petting(frame, expression),
            Pose::Chopsticks(frame) => Self::chopsticks(frame, expression),
            Pose::FloorHomework(frame) => Self::floor_homework(frame, expression),
            Pose::PaperDesk(frame) => Self::paper_desk(frame, true, expression),
            Pose::ReadStrip(frame) => Self::reading_strip(frame, expression),
            Pose::CrossLegged => Self::cross_legged(expression),
            Pose::SillLean => Self::sill_lean(expression),
            Pose::UnderSill => Self::under_sill(expression),
            Pose::SitDoze(frame) => Self::sit_doze(frame),
            Pose::LieRead(frame) => Self::lie_read(frame, expression),
            // Lying back as she dozes, held, her eyes open on the sky.
            Pose::CloudWatch => Self::for_pose(Pose::LieBack(0), face),
            Pose::Gaze => Self {
                profile: true,
                tilt: -16.0,
                arms: [(26.0, -36.0), (18.0, -44.0)],
                ..stand
            },
            Pose::Fall => Self {
                expression: Expression::Surprised,
                arms: [(150.0, -25.0), (-150.0, 25.0)],
                legs: [(-22.0, 12.0), (22.0, -12.0)],
                ..stand
            },
            Pose::Dazed => Self {
                expression: Expression::Dizzy,
                tilt: 12.0,
                arms: [(22.0, 0.0), (-22.0, 0.0)],
                legs: [(6.0, 0.0), (-6.0, 0.0)],
                ..stand
            },
            Pose::Pull { heaving, row } => Self::pulling(heaving, row, expression),
            Pose::Peer => Self {
                lean: 14.0,
                tilt: 12.0,
                profile: true,
                arms: [(18.0, -10.0), (-118.0, -100.0)],
                legs: [(10.0, 6.0), (-6.0, 0.0)],
                ..stand
            },
        }
    }

    /// Pulling a line on box row `row`, in profile facing it: leaning
    /// back, both hands reaching the box edge at that row.
    fn pulling(heaving: bool, row: u8, expression: Expression) -> Self {
        // Her shoulders sit mid-box in profile and her arms are short:
        // shift her toward the line (leaning back keeps her face inside
        // the box) so her hands meet its edge.
        let shift = 22.0;
        let target = (CANVAS_W, (f32::from(row) + 0.5) * ROW_UNITS);
        let arm = |i: usize| aim((P_SHOULDERS[i] + shift, P_SHOULDER_Y), target);
        // The leading leg braces forward, the trailing one bends behind.
        let (lean, legs) = if heaving {
            (-16.0, [(26.0, 26.0), (-30.0, 0.0)])
        } else {
            (-8.0, [(14.0, 12.0), (-18.0, 0.0)])
        };
        Self {
            expression,
            lean,
            tilt: -4.0,
            bob: 0.0,
            shift,
            profile: true,
            turn: 0.0,
            scale: 1.0,
            legs_front: false,
            stool: false,
            cushion: false,
            seated: false,
            crossed: false,
            leeks: false,
            hold: None,
            ground: None,
            arms: [(arm(0) + 4.0, 0.0), (arm(1), 0.0)],
            legs,
        }
    }

    /// Climbing a pole `pole` cells ahead of her (in profile, facing it):
    /// both hands on the pole, one high and one low, alternating; the
    /// knee on the high hand's side lifted against it.
    fn climbing(frame: u8, pole: i8, expression: Expression) -> Self {
        // Hanging from her handholds: body straight, legs hanging, one
        // foot pressed against the pole.
        let cell = CANVAS_W / 5.0;
        let pole_x = (CANVAS_W / 2.0 + f32::from(pole) * cell).clamp(58.0, CANVAS_W - 2.0);
        let shift = if pole <= 0 { 0.0 } else { 6.0 };
        // Her arms are short (chibi): aim the high hand nearly straight
        // up and the low one at her shoulders so the alternation reads.
        let (high, low) = (-40.0, 70.0);
        let reach = |i: usize, y: f32| aim((P_SHOULDERS[i] + shift, P_SHOULDER_Y), (pole_x, y));
        let (arms, legs) = if frame.is_multiple_of(2) {
            (
                [(reach(0, low), 0.0), (reach(1, high), 0.0)],
                [(2.0, 0.0), (-24.0, 34.0)],
            )
        } else {
            (
                [(reach(0, high), 0.0), (reach(1, low), 0.0)],
                [(-24.0, 34.0), (2.0, 0.0)],
            )
        };
        Self {
            expression,
            lean: 0.0,
            tilt: -8.0,
            bob: 0.0,
            shift,
            profile: true,
            turn: 0.0,
            scale: 1.0,
            legs_front: false,
            stool: false,
            cushion: false,
            seated: false,
            crossed: false,
            leeks: false,
            hold: None,
            ground: None,
            arms,
            legs,
        }
    }

    /// Sitting on the sofa's seat in profile, leaning back a little,
    /// hands in her lap and her feet dangling short of the floor. Drawn
    /// centred on the sofa's column 4.
    pub fn sofa_sit(expression: Expression) -> Self {
        // Hips on the seat cushion (52 units above the floor): thighs
        // along the seat, knees at its front, shins hanging.
        let lap = (68.0, 112.0);
        let arm = |i: usize| aim((P_SHOULDERS[i], P_SHOULDER_Y), lap);
        Self {
            bob: -12.0,
            profile: true,
            seated: true,
            arms: [(arm(0) + 4.0, -10.0), (arm(1), -14.0)],
            ..Self::standing(expression)
        }
    }

    /// Napping on the sofa's seat on her back, knees drawn up, hugging
    /// its throw cushion. Drawn centred on the sofa's column 4, with the
    /// sofa's `Bare` layer behind her (the cushion is in her arms).
    pub fn sofa_nap(expression: Expression) -> Self {
        Self {
            turn: -90.0,
            scale: 0.74,
            shift: 30.0,
            bob: -24.0,
            tilt: -6.0,
            profile: true,
            cushion: true,
            legs: [(-34.0, 64.0), (-24.0, 56.0)],
            arms: [(-70.0, -70.0), (-60.0, -80.0)],
            ..Self::standing(expression)
        }
    }

    /// Asleep in bed on her back, head on the pillow, one hand up by her
    /// cheek; the quilt (the bed's `Front` layer) goes over her. `frame`
    /// breathes. Drawn centred on the bed's column 3.
    pub fn bed_sleep(frame: u8) -> Self {
        Self {
            turn: -90.0,
            scale: 0.74,
            shift: 30.0,
            bob: if frame.is_multiple_of(2) { -5.0 } else { -5.8 },
            tilt: -10.0,
            profile: true,
            legs: [(-4.0, 0.0), (0.0, 0.0)],
            arms: [(10.0, 0.0), (160.0, 150.0)],
            ..Self::standing(Expression::Blink)
        }
    }

    /// Homework: on her stool at the desk, in profile facing it, writing;
    /// from `frame` 1 she nods off, head sinking, until at 2+ she's
    /// asleep on the paper. Drawn centred on the desk's column 7 (a cell
    /// past its end) and facing it (the desk faces right, she faces
    /// left).
    pub fn homework(frame: u8, expression: Expression) -> Self {
        // Nodding off she leans in, so she sits back to keep her head in
        // her box.
        let (lean, tilt, shift, expression) = match frame {
            0 => (6.0, 10.0, -16.0, expression),
            1 => (14.0, 22.0, -18.0, Expression::Blink),
            _ => (20.0, 38.0, -27.0, Expression::Blink),
        };
        let desk = (86.0, 98.0);
        let arm = |i: usize| aim((P_SHOULDERS[i] + shift, P_SHOULDER_Y), desk) - lean;
        Self {
            expression,
            lean,
            tilt,
            bob: -4.0,
            shift,
            profile: true,
            stool: true,
            legs: [(-86.0, 84.0), (-92.0, 92.0)],
            arms: [(arm(0), 0.0), (arm(1), 0.0)],
            ..Self::standing(expression)
        }
    }

    /// Before homework, on her stool at the desk in profile facing it,
    /// splitting a pair of disposable chopsticks held up in front of her
    /// chest: `frame` 0 still joined, both hands on the pair; 1 split
    /// cleanly, one stick in each hand, her head up; 2 split badly, her
    /// hands sagging, her head bowed over them. The face is the caller's
    /// (Happy for the clean split, Droop for the bad one). Drawn where
    /// homework is.
    pub fn chopsticks(frame: u8, expression: Expression) -> Self {
        // Arm angles are solved for the hands to land on the sticks'
        // grips (`hold` places the part's (0, 0) between them).
        // They lean forward, into the gap between her face and the desk
        // lamp's shade; split badly, they sag lower with her hands.
        let (part, tilt, arms, (x, y, angle)) = match frame {
            0 => (
                "chopsticks",
                8.0,
                [(-46.6, -34.8), (-44.4, -57.8)],
                (72.0, 86.0, 30.0),
            ),
            1 => (
                "chopsticks-clean",
                -4.0,
                [(-32.6, -57.2), (-40.2, -37.6)],
                (72.0, 86.0, 22.0),
            ),
            _ => (
                "chopsticks-bad",
                10.0,
                [(-19.7, -53.3), (-31.2, -22.7)],
                (69.0, 92.0, 40.0),
            ),
        };
        Self {
            tilt,
            bob: -4.0,
            shift: -16.0,
            profile: true,
            stool: true,
            legs: [(-86.0, 84.0), (-92.0, 92.0)],
            arms,
            hold: Some((part, x, y, angle)),
            ..Self::standing(expression)
        }
    }

    /// Home from her part-time job with her shopping: a bundle of leeks
    /// cradled across her chest, cut ends at her viewer-left hip, green
    /// tops fanning up past her viewer-right shoulder. Her viewer-left
    /// forearm comes across under the bundle; her viewer-right hand grips
    /// the stalks at her chest. Frame 1 bobs up, pleased.
    pub fn carrying(frame: u8, expression: Expression) -> Self {
        Self {
            arms: [(0.0, -40.0), (-11.8, 109.0)],
            bob: if frame.is_multiple_of(2) { 0.0 } else { -4.0 },
            leeks: true,
            ..Self::standing(expression)
        }
    }

    /// Reading beside her bookshelf: sitting on the floor, knees up, an
    /// open book held up in front of her face. `frame` 1 turns a page,
    /// her head tipped a little further. Drawn facing the shelf, her box
    /// a cell clear of it.
    pub fn reading(frame: u8, expression: Expression) -> Self {
        let turning = frame % 2 == 1;
        let book = (84.0, 70.0);
        let arm = |i: usize| aim((P_SHOULDERS[i], P_SHOULDER_Y), book);
        Self {
            bob: 34.0,
            lean: -4.0,
            tilt: if turning { 12.0 } else { 8.0 },
            profile: true,
            legs_front: true,
            legs: [(-146.0, 124.0), (-138.0, 118.0)],
            arms: [(arm(0) + 6.0, -22.0), (arm(1) + 4.0, -26.0)],
            hold: Some((
                if turning { "book-turn" } else { "book" },
                book.0,
                book.1,
                -8.0,
            )),
            ..Self::standing(expression)
        }
    }

    /// A snack from her fridge: standing side-on, a melon bread held up
    /// to her mouth; `frame` 1 is the bite, eyes shut, munching.
    pub fn eating(frame: u8, expression: Expression) -> Self {
        Self::eating_food(frame, ["melon-bread", "melon-bread-bitten"], expression)
    }

    /// [`Rig::eating`] with another food in her hand: `food` is the held
    /// part whole and bitten (e.g. `["andagi", "andagi-bitten"]`), each
    /// drawn centred on (0, 0) within the melon bread's radius.
    pub fn eating_food(frame: u8, food: [&'static str; 2], expression: Expression) -> Self {
        let biting = frame % 2 == 1;
        // The food at her mouth (which the bite's head tilt carries a
        // little forward), her near hand gripping its lower back edge:
        // the arm angles are solved for the hand to land there.
        let (food_at, arm) = if biting {
            ((81.0, 62.0), (-98.5, -23.9))
        } else {
            ((80.0, 61.0), (-97.2, -29.3))
        };
        Self {
            expression: if biting {
                Expression::Blink
            } else {
                expression
            },
            profile: true,
            tilt: if biting { 4.0 } else { 0.0 },
            bob: if biting { -1.0 } else { 0.0 },
            arms: [(6.0, -8.0), arm],
            hold: Some((food[usize::from(biting)], food_at.0, food_at.1, 0.0)),
            ..Self::standing(expression)
        }
    }

    /// Petting the cat in its bed: kneeling side-on, leaning in, her near
    /// hand reaching out low toward it. `frame` 1: bitten — the hand
    /// yanked back up, leaning away, surprised ("Ow!").
    pub fn petting(frame: u8, expression: Expression) -> Self {
        let bitten = frame % 2 == 1;
        // Sitting on the floor, knees up (as she reads), leaning in to
        // reach low over the bed's rim; bitten, she rears back.
        let (lean, near, far, expression) = if bitten {
            (
                -10.0,
                (
                    aim((P_SHOULDERS[1], P_SHOULDER_Y), (60.0, 34.0)) - 10.0,
                    -80.0,
                ),
                (-30.0, -30.0),
                Expression::Surprised,
            )
        } else {
            (
                16.0,
                (
                    aim((P_SHOULDERS[1], P_SHOULDER_Y), (112.0, 122.0)) - 16.0,
                    -4.0,
                ),
                (-40.0, -20.0),
                expression,
            )
        };
        Self {
            expression,
            bob: 34.0,
            lean,
            tilt: if bitten { -8.0 } else { 10.0 },
            // Leaning in, she sits back so her head stays in her box.
            shift: if bitten { 0.0 } else { -12.0 },
            profile: true,
            legs_front: true,
            legs: [(-146.0, 124.0), (-138.0, 118.0)],
            arms: [far, near],
            ..Self::standing(expression)
        }
    }

    /// Homework on the floor (no desk): lying on her front, head up, a
    /// paper on the floor in front of her (its near edge under her chin,
    /// most of it out past her face: her chibi arms reach no further)
    /// and a pencil in her near hand, feet up behind her (`frame` 0–1:
    /// the pencil moves); at 2 she's dozed off, her head down on her
    /// arms, the paper still out in front of her.
    pub fn floor_homework(frame: u8, expression: Expression) -> Self {
        let base = Self {
            turn: 90.0,
            scale: 0.74,
            bob: 30.0,
            profile: true,
            ground: Some(("paper-floor", 74.0, 156.0, 0.0)),
            ..Self::standing(expression)
        };
        // Arm angles solved for her hands to land on the paper's near
        // edge; the pencil there (in her own, unturned coordinates)
        // leaning back toward her, its point on the paper.
        match frame {
            0 | 1 => {
                let (near, pencil) = if frame == 0 {
                    ((-102.0, -34.0), (73.9, 63.8, -102.0))
                } else {
                    ((-126.0, -2.0), (72.9, 60.2, -88.0))
                };
                Self {
                    shift: -38.0,
                    lean: -18.0,
                    tilt: -20.0,
                    legs: [(0.0, 120.0), (0.0, 80.0)],
                    arms: [(-104.0, -8.0), near],
                    hold: Some(("pencil", pencil.0, pencil.1, pencil.2)),
                    ..base
                }
            }
            _ => Self {
                expression: Expression::Blink,
                shift: -40.0,
                lean: -10.0,
                tilt: -40.0,
                legs: [(0.0, 90.0), (0.0, 50.0)],
                arms: [(-96.0, -56.0), (-160.0, 40.0)],
                ..base
            },
        }
    }

    /// Reading on the floor on her back, knees up, an open book held up
    /// over her face in both hands (`frame` 1 turns a page); at 2 she's
    /// dozed off with it lying open on her face, her arms down. For
    /// floor reading, or floor homework with a book.
    pub fn lie_read(frame: u8, expression: Expression) -> Self {
        let base = Self {
            turn: -90.0,
            scale: 0.74,
            shift: 30.0,
            bob: 17.0,
            tilt: -6.0,
            profile: true,
            legs: [(-62.0, 118.0), (-48.0, 104.0)],
            ..Self::standing(expression)
        };
        // Her hands reach only to her face's height (her head is big),
        // so she holds the book by its near end, tipped open toward her
        // eyes just over her face (its place in her own, unturned frame;
        // a quarter turn stands it ridge-up over her). Dozing, it lies
        // open on her face turned side-on to the viewer (`book-cover`),
        // covering her eyes, which is why one sleeps under a book.
        match frame {
            0 | 1 => Self {
                arms: [(-110.0, -2.0), (-112.0, -4.0)],
                hold: Some((
                    if frame == 0 {
                        "book-side"
                    } else {
                        "book-side-turn"
                    },
                    89.2,
                    48.4,
                    115.0,
                )),
                ..base
            },
            _ => Self {
                expression: Expression::Blink,
                arms: [(8.0, -12.0), (-30.0, -60.0)],
                hold: Some(("book-cover", 73.0, 45.7, 90.0)),
                ..base
            },
        }
    }

    /// Homework at her paper desk: kneeling (or, `kneel` false,
    /// cross-legged) on the floor beside the low cube of crumpled text,
    /// side-on facing it, writing on a paper on its top (`frame` 0–1);
    /// nodding off (2); asleep on the cube, her head on her arms (3).
    /// Drawn with her box centred a column past the cube's end, facing
    /// it, as at the desk.
    pub fn paper_desk(frame: u8, kneel: bool, expression: Expression) -> Self {
        // (shift, lean, tilt, far arm, near arm): the arms are solved for
        // her hands to land on the paper on the cube's top.
        type Frame = (f32, f32, f32, (f32, f32), (f32, f32));
        let (shift, lean, tilt, far, near): Frame = match (kneel, frame) {
            (true, 0) => (-4.0, 12.0, 14.0, (-28.0, -66.0), (-38.0, -60.0)),
            (true, 1) => (-4.0, 12.0, 16.0, (-28.0, -66.0), (-62.0, -30.0)),
            (true, 2) => (-18.0, 16.0, 24.0, (-72.0, -6.0), (-78.0, 0.0)),
            (true, _) => (-30.0, 24.0, 34.0, (-94.0, -2.0), (-100.0, 0.0)),
            (false, 0) => (-4.0, 12.0, 10.0, (-32.0, -88.0), (-42.0, -80.0)),
            (false, 1) => (-4.0, 12.0, 12.0, (-32.0, -88.0), (-62.0, -56.0)),
            (false, 2) => (-18.0, 16.0, 22.0, (-64.0, -46.0), (-72.0, -36.0)),
            (false, _) => (-30.0, 22.0, 32.0, (-104.0, -2.0), (-106.0, -8.0)),
        };
        let (bob, legs) = if kneel {
            (20.0, [(-52.0, 140.0), (-46.0, 136.0)])
        } else {
            (26.0, [(0.0, 0.0); 2])
        };
        Self {
            expression: if frame >= 2 {
                Expression::Blink
            } else {
                expression
            },
            lean,
            tilt,
            bob,
            shift,
            profile: true,
            legs_front: kneel,
            crossed: !kneel,
            legs,
            arms: [far, near],
            ground: Some(("paper", 82.0, PAPER_DESK_TOP - 2.0, 0.0)),
            ..Self::standing(expression)
        }
    }

    /// Reading a strip of text she tore off a line, sitting on the floor
    /// as she reads a book: the strip held up in front of her face in
    /// both hands; `frame` 1 reads along it, her head tipped further.
    pub fn reading_strip(frame: u8, expression: Expression) -> Self {
        let along = frame % 2 == 1;
        // Held out over her knees in both hands, her head bowed over it;
        // reading along, she tips it and follows it with her head.
        let strip = if along { (78.0, 88.0) } else { (76.0, 86.0) };
        let arm = |i: usize| aim((P_SHOULDERS[i], P_SHOULDER_Y), strip);
        Self {
            hold: Some(("strip", strip.0, strip.1, if along { 4.0 } else { -8.0 })),
            tilt: if along { 24.0 } else { 18.0 },
            arms: [(arm(0) + 6.0, -22.0), (arm(1) + 4.0, -26.0)],
            ..Self::reading(0, expression)
        }
    }

    /// Sitting cross-legged on the floor, side-on, hands on her knees,
    /// her head up a little (watching the TV beside her).
    pub fn cross_legged(expression: Expression) -> Self {
        // Her legs crossed out in front of her (the crossed-legs part);
        // her hands in her lap (her arms don't reach her knees).
        Self {
            bob: 26.0,
            lean: -2.0,
            tilt: -4.0,
            profile: true,
            crossed: true,
            legs: [(0.0, 0.0); 2],
            arms: [(-46.0, 44.0), (-36.0, 22.0)],
            ..Self::standing(expression)
        }
    }

    /// Leaning on the window sill (the window hung low, its sill at her
    /// chest), side-on facing it: elbows on the sill, chin in her hands,
    /// gazing out (a long daydream hold, unlike the stiff `Gaze`). Arm
    /// angles solved for her elbows to rest on the sill and her hands to
    /// cup her chin.
    pub fn sill_lean(expression: Expression) -> Self {
        Self {
            profile: true,
            lean: 8.0,
            tilt: -6.0,
            shift: 4.0,
            arms: [(-82.0, -72.0), (-84.0, -74.0)],
            legs: [(2.0, 0.0), (-6.0, 10.0)],
            ..Self::standing(expression)
        }
    }

    /// Sitting on the floor under the window, knees up, chin in her
    /// hands, looking up at the sky.
    pub fn under_sill(expression: Expression) -> Self {
        // Arm angles solved for her hands to cup her chin.
        Self {
            bob: 34.0,
            lean: -4.0,
            tilt: -16.0,
            profile: true,
            legs_front: true,
            legs: [(-146.0, 124.0), (-138.0, 118.0)],
            arms: [(-92.0, -74.0), (-86.0, -86.0)],
            ..Self::standing(expression)
        }
    }

    /// Dozing off sitting on the floor, hugging her knees, her head
    /// sinking onto them (`frame` 1 lower); eyes shut.
    pub fn sit_doze(frame: u8) -> Self {
        let low = frame % 2 == 1;
        Self {
            bob: 34.0,
            // Sinking forward, she sits back to keep her head in her box.
            shift: if low { -12.0 } else { -6.0 },
            lean: if low { 12.0 } else { 6.0 },
            tilt: if low { 30.0 } else { 20.0 },
            profile: true,
            legs_front: true,
            legs: [(-146.0, 124.0), (-138.0, 118.0)],
            arms: [(-36.0, -20.0), (-28.0, -24.0)],
            ..Self::standing(Expression::Blink)
        }
    }

    /// Waving goodbye (the dissolve's first beat).
    pub fn waving(raised: bool) -> Self {
        let wave = if raised { -168.0 } else { -150.0 };
        Self {
            arms: [(8.0, -4.0), (wave, if raised { 12.0 } else { -24.0 })],
            ..Self::standing(Expression::Smile)
        }
    }
}

fn limb(
    pivot: (f32, f32),
    (joint, bend): (f32, f32),
    upper: &str,
    lower: &str,
    length: f32,
) -> String {
    format!(
        r##"<g transform="translate({} {}) rotate({joint})"><use href="#{upper}"/><g transform="translate(0 {length}) rotate({bend})"><use href="#{lower}"/></g></g>"##,
        pivot.0, pivot.1
    )
}

/// The posed scene as an SVG document. `line` is the outline colour.
pub(super) fn scene(rig: &Rig, facing: Facing, line: &str) -> String {
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({CANVAS_W} 0) scale(-1 1)""#),
    };
    let (shoulders, shoulder_y, hips, prefix) = if rig.profile {
        (P_SHOULDERS, P_SHOULDER_Y, P_HIPS, "p-")
    } else {
        (SHOULDERS, SHOULDER_Y, HIPS, "")
    };
    let leg = |i: usize| {
        limb(
            (hips[i], HIP_Y),
            rig.legs[i],
            "leg-upper",
            "leg-lower",
            THIGH,
        )
    };
    let arm = |i: usize| {
        limb(
            (shoulders[i], shoulder_y),
            rig.arms[i],
            "arm-upper",
            "arm-lower",
            UPPER_ARM,
        )
    };
    let tilt = |part: String| {
        format!(
            r#"<g transform="rotate({} {} {})">{part}</g>"#,
            rig.tilt, NECK.0, NECK.1
        )
    };
    // The long hair hangs behind her body; face and bangs in front.
    let hair = tilt(format!(r##"<use href="#{prefix}hair-back"/>"##));
    let head = tilt(format!(
        r##"<use href="#{prefix}face"/><use href="#{prefix}{face}"/><use href="#{prefix}hair-front"/>"##,
        face = rig.expression.id(),
    ));
    // The cushion she hugs sits on her chest, under her near arm.
    let cushion = if rig.cushion {
        if rig.profile {
            r##"<use href="#cushion" transform="translate(60 88) rotate(-10)"/>"##
        } else {
            r##"<use href="#cushion" transform="translate(50 90)"/>"##
        }
    } else {
        ""
    };
    // Leeks in her arms: in front of her body, under her arms.
    let leeks = if rig.leeks {
        r##"<use href="#leeks"/>"##
    } else {
        ""
    };
    let held = rig
        .hold
        .map(|(part, x, y, angle)| {
            format!(r##"<use href="#{part}" transform="translate({x} {y}) rotate({angle})"/>"##)
        })
        .unwrap_or_default();
    let cushion = format!("{cushion}{leeks}{held}");
    // Frontal: both arms over everything (hands can reach past the
    // head). Profile: far limbs behind the body, near limbs in front.
    let body = if rig.profile {
        format!(
            r##"{legs_behind}<g transform="rotate({lean} 50 {HIP_Y})">{far_arm}{hair}<use href="#{skirt}"/><use href="#p-torso"/>{head}</g>{legs_before}<g transform="rotate({lean} 50 {HIP_Y})">{cushion}{near_arm}</g>"##,
            legs_behind = if rig.seated || rig.crossed || rig.legs_front {
                String::new()
            } else {
                leg(0) + &leg(1)
            },
            legs_before = if rig.seated || rig.crossed {
                format!(
                    r##"<g transform="rotate({} 50 {HIP_Y})"><use href="#{}"/></g>"##,
                    // Crossed, her legs stay on the floor as she leans.
                    if rig.seated { rig.lean } else { 0.0 },
                    if rig.seated {
                        "p-seated-legs"
                    } else {
                        "p-crossed-legs"
                    }
                )
            } else if rig.legs_front {
                leg(0) + &leg(1)
            } else {
                String::new()
            },
            lean = rig.lean,
            far_arm = arm(0),
            near_arm = arm(1),
            skirt = if rig.seated {
                "p-skirt-seated"
            } else {
                "p-skirt"
            },
            cushion = cushion,
        )
    } else {
        format!(
            r##"<g transform="rotate({lean} 50 {HIP_Y})">{hair}</g>{leg0}{leg1}<g transform="rotate({lean} 50 {HIP_Y})"><use href="#skirt"/><use href="#torso"/>{head}{cushion}{arm0}{arm1}</g>"##,
            leg0 = leg(0),
            leg1 = leg(1),
            lean = rig.lean,
            arm0 = arm(0),
            arm1 = arm(1),
            cushion = cushion,
        )
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS_W} {CANVAS_H}" color="{line}">{PARTS}<g{mirror}>{ground}{stool}<g transform="translate({shift} {bob}) translate(50 {HIP_Y}) scale({scale}) rotate({turn}) translate(-50 -{HIP_Y})">{body}</g></g></svg>"##,
        stool = if rig.stool {
            format!(
                r##"<use href="#stool" transform="translate({} 0)"/>"##,
                rig.shift
            )
        } else {
            String::new()
        },
        ground = rig
            .ground
            .map(|(part, x, y, angle)| {
                format!(r##"<use href="#{part}" transform="translate({x} {y}) rotate({angle})"/>"##)
            })
            .unwrap_or_default(),
        shift = rig.shift,
        bob = rig.bob,
        scale = rig.scale,
        turn = rig.turn,
    )
}

/// Render the posed scene into a `width × height` pixel RGBA image,
/// scaled uniformly and bottom-aligned (her feet on the box's floor).
pub(super) fn render(
    rig: &Rig,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    rasterize(
        &scene(rig, facing, line),
        (CANVAS_W, CANVAS_H),
        width,
        height,
    )
}

/// Her door in space, shut, part-way open, or wide open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum DoorFrame {
    Closed,
    Ajar,
    Open,
}

/// The door as an SVG document in her canvas, hinged on the left facing
/// right (mirrored for left).
fn door_scene(frame: DoorFrame, facing: Facing, line: &str) -> String {
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({CANVAS_W} 0) scale(-1 1)""#),
    };
    let body = match frame {
        DoorFrame::Closed => r##"<use href="#door-leaf-closed"/><use href="#door-frame"/>"##,
        DoorFrame::Ajar => {
            r##"<use href="#door-beyond"/><use href="#door-frame"/><use href="#door-leaf-ajar"/>"##
        }
        DoorFrame::Open => {
            r##"<use href="#door-beyond"/><use href="#door-frame"/><use href="#door-leaf-open"/>"##
        }
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS_W} {CANVAS_H}" color="{line}">{DOOR}<g{mirror}>{body}</g></svg>"##
    )
}

/// Render her door into her box like [`render`]: the same frame and
/// scale, standing where her feet do.
pub(super) fn render_door(
    frame: DoorFrame,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    rasterize(
        &door_scene(frame, facing, line),
        (CANVAS_W, CANVAS_H),
        width,
        height,
    )
}

/// Which parts of a piece to draw, for compositing her into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Layer {
    /// The whole piece, as it stands.
    Whole,
    /// What goes behind her when she uses it (all but the bed's quilt).
    Back,
    /// The back without its loose things (the sofa without its throw
    /// cushion, which she's hugging).
    Bare,
    /// What goes in front of her (the bed's quilt).
    Front,
}

/// The SVG parts of `prop` in `layer`, back to front.
fn parts(prop: Furniture, layer: Layer) -> &'static [&'static str] {
    match (prop, layer) {
        (_, Layer::Front) if prop != Furniture::Bed => &[],
        (Furniture::Sofa, Layer::Whole | Layer::Back) => &["sofa", "sofa-cushion"],
        (Furniture::Sofa, _) => &["sofa"],
        (Furniture::Tv, _) => &["tv", "tv-screen"],
        (Furniture::Bed, Layer::Whole) => &["bed", "bed-quilt"],
        (Furniture::Bed, Layer::Front) => &["bed-quilt-over"],
        (Furniture::Bed, _) => &["bed"],
        (Furniture::Desk, _) => &["desk"],
        (Furniture::Lamp, _) => &["lamp-glow", "lamp", "lamp-shade"],
        (Furniture::Bookshelf, _) => &["bookshelf"],
        (Furniture::Fridge, _) => &["fridge"],
        (Furniture::CatBed, _) => &["cat-bed", "cat-bed-front"],
        (Furniture::Plant, _) => &["plant"],
        (Furniture::Poster, _) => &["poster"],
        // Plain, the hands both point at 12.
        (Furniture::Clock, _) => &["clock", "clock-hand-hour", "clock-hand-minute", "clock-pin"],
        (Furniture::Window, _) => &["window-sky-day", "window"],
    }
}

/// The SVG `transform` attribute that draws a piece facing `facing` in a
/// frame `w` units wide (mirrored facing left), none for a symmetric
/// piece (see [`Furniture::drawn_facing`]).
fn mirror(prop: Furniture, facing: Facing, w: f32) -> String {
    match prop.drawn_facing(facing) {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({w} 0) scale(-1 1)""#),
    }
}

/// `layer` of a piece as an SVG document, in the piece's own frame.
fn layer_scene(prop: Furniture, layer: Layer, facing: Facing, line: &str) -> String {
    let (w, h) = prop_frame(prop);
    let mirror = mirror(prop, facing, w);
    let uses: String = parts(prop, layer)
        .iter()
        .map(|id| format!(r##"<use href="#{id}"/>"##))
        .collect();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" color="{line}">{PROPS}<g{mirror}>{uses}</g></svg>"##
    )
}

/// Render `layer` of a piece like [`render_prop`]; `None` when the
/// layer is empty (only the bed has a front).
pub(super) fn render_prop_layer(
    prop: Furniture,
    layer: Layer,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    if parts(prop, layer).is_empty() {
        return None;
    }
    rasterize(
        &layer_scene(prop, layer, facing, line),
        prop_frame(prop),
        width,
        height,
    )
}

/// The parcel's own frame (floor along its bottom edge).
/// A piece's state, for the pieces that have one (the rest are always
/// `Plain`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum PieceState {
    /// As it stands: the lamp lit, the fridge shut, the cat bed empty.
    Plain,
    /// The lamp switched off.
    LampOff,
    /// The fridge door open, her groceries inside.
    FridgeOpen,
    /// Kamineko asleep in the cat bed.
    Cat,
    /// Kamineko awake, and biting.
    CatBiting,
    /// The wall clock's hands at her time of day.
    Dial(Dial),
    /// The sky outside the window at her time of day.
    Sky(Sky),
}

/// The wall clock's dial, to the quarter-hour: `hour` 0–11, `quarter`
/// 0–3. 48 faces in all, and a new one every 2.5 real minutes (her
/// clock runs six times as fast).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Dial {
    pub hour: u8,
    pub quarter: u8,
}

impl Dial {
    /// The dial showing `minute` of the day, rounded down to its quarter.
    pub(super) fn at(minute: u16) -> Self {
        // Below 12 and 4: they fit.
        Self {
            hour: (minute / 60 % 12) as u8,
            quarter: (minute % 60 / 15) as u8,
        }
    }

    /// The hour and minute hands' angles in degrees, clockwise from 12:
    /// the hour hand creeps on 7.5° a quarter, as a real clock's does.
    fn angles(self) -> (f32, f32) {
        let quarter = f32::from(self.quarter % 4);
        (
            f32::from(self.hour % 12) * 30.0 + quarter * 7.5,
            quarter * 90.0,
        )
    }
}

/// The clock's face centre in its frame, which its hands turn about (on
/// a pixel centre at 9 × 19, so its 1-px hands are crisp at 12, 3, 6
/// and 9).
const CLOCK_CENTRE: (f32, f32) = (30.0, 42.888_89);

/// The clock's parts showing `dial`, back to front: the hands, drawn
/// pointing at 12, turned about the face's centre. Never mirrored.
fn dial_parts(dial: Dial) -> String {
    let (hour, minute) = dial.angles();
    let (cx, cy) = CLOCK_CENTRE;
    format!(
        r##"<use href="#clock"/><use href="#clock-hand-hour" transform="rotate({hour} {cx} {cy})"/><use href="#clock-hand-minute" transform="rotate({minute} {cx} {cy})"/><use href="#clock-pin"/>"##
    )
}

/// The sky through the window, by the time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Sky {
    /// 21:00 to 05:00: navy, a moon and stars, one window still lit.
    Night,
    /// 05:00 to 07:00: pastel bands, the sun just up.
    Dawn,
    /// 07:00 to 17:00: blue, the sun, a cloud, a bird.
    Day,
    /// 17:00 to 19:00: purple over rose over orange, the sun going down.
    Dusk,
    /// 19:00 to 21:00: deep blue, the first star, people home.
    Evening,
}

impl Sky {
    /// Every sky.
    #[cfg(test)]
    pub(super) const ALL: [Self; 5] = [
        Self::Night,
        Self::Dawn,
        Self::Day,
        Self::Dusk,
        Self::Evening,
    ];

    /// The sky at `minute` of the day (phase 5b, sky phases).
    pub(super) fn at(minute: u16) -> Self {
        match minute / 60 {
            5 | 6 => Self::Dawn,
            7..=16 => Self::Day,
            17 | 18 => Self::Dusk,
            19 | 20 => Self::Evening,
            _ => Self::Night,
        }
    }
}
/// The parts of `item` in `state`, back to front.
fn state_parts(item: Furniture, state: PieceState) -> &'static [&'static str] {
    match (item, state) {
        (Furniture::Lamp, PieceState::LampOff) => &["lamp", "lamp-shade-off"],
        (Furniture::Fridge, PieceState::FridgeOpen) => &["fridge-open"],
        (Furniture::CatBed, PieceState::Cat) => &["cat-bed", "kamineko-sleep", "cat-bed-front"],
        (Furniture::CatBed, PieceState::CatBiting) => {
            &["cat-bed", "kamineko-bite", "cat-bed-front"]
        }
        // Each sky clips itself to the glass, behind the frame.
        (Furniture::Window, PieceState::Sky(Sky::Night)) => &["window-sky-night", "window"],
        (Furniture::Window, PieceState::Sky(Sky::Dawn)) => &["window-sky-dawn", "window"],
        (Furniture::Window, PieceState::Sky(Sky::Day)) => &["window-sky-day", "window"],
        (Furniture::Window, PieceState::Sky(Sky::Dusk)) => &["window-sky-dusk", "window"],
        (Furniture::Window, PieceState::Sky(Sky::Evening)) => &["window-sky-evening", "window"],
        _ => parts(item, Layer::Whole),
    }
}

/// `item` in `state`, rendered like [`render_prop_layer`] with
/// `Layer::Whole`; states that don't apply to it render it plain.
pub(super) fn render_piece(
    item: Furniture,
    state: PieceState,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    let (w, h) = prop_frame(item);
    let mirror = mirror(item, facing, w);
    let uses: String = match (item, state) {
        // Turned by the time: drawn, not looked up.
        (Furniture::Clock, PieceState::Dial(dial)) => dial_parts(dial),
        _ => state_parts(item, state)
            .iter()
            .map(|id| format!(r##"<use href="#{id}"/>"##))
            .collect(),
    };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" color="{line}">{PROPS}<g{mirror}>{uses}</g></svg>"##
    );
    rasterize(&svg, (w, h), width, height)
}

const PARCEL: (f32, f32) = (100.0, 80.0);

/// A delivery box holding `item`, as an SVG document in the piece's own
/// frame: centred on its floor, no wider than the piece.
fn parcel_scene(item: Furniture, open: bool, facing: Facing, line: &str) -> String {
    let (w, h) = prop_frame(item);
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({w} 0) scale(-1 1)""#),
    };
    let scale = (w / PARCEL.0).min(1.0);
    let (dx, dy) = ((w - PARCEL.0 * scale) / 2.0, h - PARCEL.1 * scale);
    let id = if open { "parcel-open" } else { "parcel" };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" color="{line}">{DELIVERY}<g{mirror}><g transform="translate({dx} {dy}) scale({scale})"><use href="#{id}"/></g></g></svg>"##
    )
}

/// A delivery box holding `item`, closed or opened, rendered like
/// [`render_prop_layer`] so it stands where the piece will.
pub(super) fn render_parcel(
    item: Furniture,
    open: bool,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    rasterize(
        &parcel_scene(item, open, facing, line),
        prop_frame(item),
        width,
        height,
    )
}

/// What's on her TV: static, or Chiyo-chichi's shopping channel, each
/// with animation frames 0–1; or, flicking through the channels, colour
/// bars and a sunrise; or a programme she watches (all three still).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Channel {
    Snow(u8),
    Shopping(u8),
    ColourBars,
    Sunrise,
    /// A programme, held while she watches (phase 5c D7).
    Programme(Programme),
}

/// The drawn programmes the TV can hold (phase 5c D7): one drawn as each
/// watch begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Programme {
    /// The news: an anchor at the desk, a red ticker.
    News,
    /// The weather: the map, a sun and a rain cloud.
    Weather,
    /// A nature programme: penguins on an ice floe.
    Penguins,
    /// A cooking show: a steaming bowl of ramen.
    Cooking,
}

impl Programme {
    pub const ALL: [Self; 4] = [Self::News, Self::Weather, Self::Penguins, Self::Cooking];

    /// Its picture's part in `art/props.svg`.
    fn id(self) -> &'static str {
        match self {
            Self::News => "tv-news",
            Self::Weather => "tv-weather",
            Self::Penguins => "tv-penguin",
            Self::Cooking => "tv-cooking",
        }
    }
}

/// The TV's glass, in its frame.
const GLASS: (f32, f32, f32, f32) = (26.0, 72.0, 58.0, 46.0);

/// TV static: a grid of grey specks, reshuffled each frame.
fn snow(frame: u8) -> String {
    let (x0, y0, w, h) = GLASS;
    let mut state = 0x2545_F491_4F6C_DD1D_u64 ^ u64::from(frame).wrapping_mul(0x9E37_79B9);
    let mut out = format!(r##"<rect x="{x0}" y="{y0}" width="{w}" height="{h}" fill="#3a3f45"/>"##);
    let step = 2.5;
    let mut y = y0;
    while y < y0 + h {
        let mut x = x0;
        while x < x0 + w {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let grey = 70 + (state % 170) as u8;
            out.push_str(&format!(
                r#"<rect x="{x}" y="{y}" width="{step}" height="{step}" fill="rgb({grey},{grey},{grey})"/>"#
            ));
            x += step;
        }
        y += step;
    }
    out
}

/// A test card's colour bars: seven tall bars over a strip of short
/// ones.
fn colour_bars() -> String {
    const TALL: [&str; 7] = [
        "#c0c0c0", "#c0c000", "#00c0c0", "#00c000", "#c000c0", "#c00000", "#0000c0",
    ];
    const SHORT: [&str; 7] = [
        "#0000c0", "#131313", "#c000c0", "#131313", "#00c0c0", "#131313", "#c0c0c0",
    ];
    let (x0, y0, w, h) = GLASS;
    let bar = w / TALL.len() as f32;
    let tall = h * 0.75;
    let mut out = String::new();
    for (i, (top, bottom)) in TALL.iter().zip(SHORT).enumerate() {
        let x = x0 + bar * i as f32;
        out.push_str(&format!(
            r#"<rect x="{x}" y="{y0}" width="{bar}" height="{tall}" fill="{top}"/><rect x="{x}" y="{}" width="{bar}" height="{}" fill="{bottom}"/>"#,
            y0 + tall,
            h - tall,
        ));
    }
    out
}

/// A sunrise: the sun half up over the sea, under a dawn sky.
fn sunrise() -> String {
    let (x0, y0, w, h) = GLASS;
    let horizon = y0 + h * 0.62;
    let (cx, r) = (x0 + w / 2.0, h * 0.24);
    format!(
        r##"<rect x="{x0}" y="{y0}" width="{w}" height="{h}" fill="#ffb26b"/><rect x="{x0}" y="{y0}" width="{w}" height="{}" fill="#ff8a8a"/><circle cx="{cx}" cy="{horizon}" r="{r}" fill="#fff1a8"/><rect x="{x0}" y="{horizon}" width="{w}" height="{}" fill="#3d6fa8"/><path d="M {} {} h {} M {} {} h {}" stroke="#fff1a8" stroke-width="1.6"/>"##,
        h * 0.3,
        y0 + h - horizon,
        cx - r * 0.8,
        horizon + 4.0,
        r * 1.6,
        cx - r * 0.5,
        horizon + 9.0,
        r,
    )
}

/// The shopping channel's studio: a pink-and-yellow sunburst behind him.
fn sunburst() -> String {
    let (x0, y0, w, h) = GLASS;
    let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0 + 2.0);
    let mut out = format!(r##"<rect x="{x0}" y="{y0}" width="{w}" height="{h}" fill="#ffd66b"/>"##);
    let rays = 14;
    for i in (0..rays).step_by(2) {
        let a0 = std::f32::consts::TAU * i as f32 / rays as f32;
        let a1 = std::f32::consts::TAU * (i + 1) as f32 / rays as f32;
        let r = 60.0;
        out.push_str(&format!(
            r##"<path d="M {cx} {cy} L {} {} L {} {} Z" fill="#ff9db3"/>"##,
            cx + r * a0.cos(),
            cy + r * a0.sin(),
            cx + r * a1.cos(),
            cy + r * a1.sin(),
        ));
    }
    out
}

/// The TV showing `channel`, as an SVG document in its frame.
fn tv_scene(channel: Channel, facing: Facing, line: &str) -> String {
    let (w, h) = prop_frame(Furniture::Tv);
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({w} 0) scale(-1 1)""#),
    };
    let (x0, y0, gw, gh) = GLASS;
    let picture = match channel {
        Channel::Snow(frame) => snow(frame),
        Channel::ColourBars => colour_bars(),
        Channel::Sunrise => sunrise(),
        Channel::Programme(programme) => format!(r##"<use href="#{}"/>"##, programme.id()),
        Channel::Shopping(frame) => {
            // He bobs, and talks on the up-beat.
            let (bob, mouth) = if frame % 2 == 0 {
                (0.0, "chiyo-chichi-hum")
            } else {
                (-1.5, "chiyo-chichi-talk")
            };
            format!(
                r##"{}<g transform="translate(0 {bob})"><use href="#chiyo-chichi"/><use href="#{mouth}"/></g>"##,
                sunburst()
            )
        }
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" color="{line}">{PROPS}{DELIVERY}<defs><clipPath id="tv-glass"><rect x="{x0}" y="{y0}" width="{gw}" height="{gh}" rx="8"/></clipPath></defs><g{mirror}><use href="#tv"/><g clip-path="url(#tv-glass)">{picture}</g><rect x="{x0}" y="{y0}" width="{gw}" height="{gh}" rx="8" fill="none" stroke="currentColor" stroke-width="1.6"/></g></svg>"##
    )
}

/// The whole TV with `channel` on its screen, rendered like
/// [`render_prop_layer`].
pub(super) fn render_tv(
    channel: Channel,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    rasterize(
        &tv_scene(channel, facing, line),
        prop_frame(Furniture::Tv),
        width,
        height,
    )
}

/// The TV's glass's whole-pixel rectangle in a `width × height` image of
/// the TV facing `facing`, as [`rasterize`] places the frame: left, top,
/// width, height, snapped outward (its half-pixel edges covered; the
/// glass's mask trims them).
pub(super) fn glass_rect(facing: Facing, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let (fw, fh) = prop_frame(Furniture::Tv);
    let (x0, y0, gw, gh) = GLASS;
    let scale = (width as f32 / fw).min(height as f32 / fh);
    let dx = (width as f32 - fw * scale) / 2.0;
    let dy = height as f32 - fh * scale;
    let gx = match facing {
        Facing::Right => x0,
        Facing::Left => fw - x0 - gw,
    };
    let left = (gx * scale + dx).floor().max(0.0) as u32;
    let top = (y0 * scale + dy).floor().max(0.0) as u32;
    let right = (((gx + gw) * scale + dx).ceil() as u32).min(width);
    let bottom = (((y0 + gh) * scale + dy).ceil() as u32).min(height);
    (
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
}

/// The whole TV with a still of the film on its glass (phase 5c D7),
/// rendered like [`render_tv`]: the set with nothing on its glass, then
/// `film` box-scaled once to the glass's whole-pixel rectangle at this
/// size and laid in through the glass's rounded mask, **unmirrored** (a
/// film reads one way, whichever way the set faces), then the glass's
/// outline over it. In pixel space, not SVG: resvg here has no raster
/// images, and an `<image>` would be mirrored with the set. No sheen
/// (the user's call).
pub(super) fn render_film(
    film: &image::RgbaImage,
    facing: Facing,
    line: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    let frame = prop_frame(Furniture::Tv);
    let (w, h) = frame;
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({w} 0) scale(-1 1)""#),
    };
    let (x0, y0, gw, gh) = GLASS;
    let svg = |defs: &str, inner: &str| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" color="{line}">{defs}<g{mirror}>{inner}</g></svg>"##
        )
    };
    let glass = format!(r#"x="{x0}" y="{y0}" width="{gw}" height="{gh}" rx="8""#);
    let mut out = rasterize(
        &svg(&format!("{PROPS}{DELIVERY}"), r##"<use href="#tv"/>"##),
        frame,
        width,
        height,
    )?;
    let mask = rasterize(
        &svg("", &format!(r##"<rect {glass} fill="#ffffff"/>"##)),
        frame,
        width,
        height,
    )?;
    let outline = rasterize(
        &svg(
            "",
            &format!(r#"<rect {glass} fill="none" stroke="currentColor" stroke-width="1.6"/>"#),
        ),
        frame,
        width,
        height,
    )?;
    let (left, top, pw, ph) = glass_rect(facing, width, height);
    if pw > 0 && ph > 0 {
        let picture = super::film::box_scale(film, pw, ph);
        for (x, y, p) in picture.enumerate_pixels() {
            let (ox, oy) = (left + x, top + y);
            let cover = f32::from(mask.get_pixel(ox, oy)[3]) / 255.0;
            if cover <= 0.0 {
                continue;
            }
            let under = out.get_pixel_mut(ox, oy);
            for c in 0..3 {
                under[c] =
                    (f32::from(under[c]) * (1.0 - cover) + f32::from(p[c]) * cover).round() as u8;
            }
            under[3] = (f32::from(under[3]) + (255.0 - f32::from(under[3])) * cover).round() as u8;
        }
    }
    image::imageops::overlay(&mut out, &outline, 0, 0);
    Some(out)
}

/// A prop's frame in SVG units.
fn prop_frame(prop: Furniture) -> (f32, f32) {
    let (cols, rows) = prop.spec().footprint;
    (
        f32::from(cols) * CELL_UNITS.0,
        f32::from(rows) * CELL_UNITS.1,
    )
}

/// Rasterize `svg`, whose frame is `(fw, fh)` units, into a `width ×
/// height` image: scaled uniformly, centred, bottom-aligned.
pub(super) fn rasterize(
    svg: &str,
    (fw, fh): (f32, f32),
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let scale = (width as f32 / fw).min(height as f32 / fh);
    let dx = (width as f32 - fw * scale) / 2.0;
    let dy = height as f32 - fh * scale;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut out = image::RgbaImage::new(width, height);
    for (pixel, source) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = source.demultiply();
        *pixel = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const LINE: &str = "#1d1714";

    fn poses() -> Vec<(&'static str, Rig)> {
        let mut out = vec![
            ("stand", Rig::for_pose(Pose::Stand, Face::Vacant)),
            ("blink", Rig::for_pose(Pose::Stand, Face::Blink)),
        ];
        for frame in 0..4 {
            out.push(("walk", Rig::for_pose(Pose::Walk(frame), Face::Vacant)));
        }
        out.extend([
            (
                "climb",
                Rig::for_pose(Pose::Climb { frame: 0, pole: 3 }, Face::Vacant),
            ),
            (
                "climb",
                Rig::for_pose(Pose::Climb { frame: 1, pole: 3 }, Face::Vacant),
            ),
            ("fall", Rig::for_pose(Pose::Fall, Face::Vacant)),
            ("dazed", Rig::for_pose(Pose::Dazed, Face::Vacant)),
            ("peer", Rig::for_pose(Pose::Peer, Face::Vacant)),
            (
                "pull",
                Rig::for_pose(
                    Pose::Pull {
                        heaving: false,
                        row: 1,
                    },
                    Face::Vacant,
                ),
            ),
            (
                "pull",
                Rig::for_pose(
                    Pose::Pull {
                        heaving: true,
                        row: 2,
                    },
                    Face::Vacant,
                ),
            ),
            ("look", Rig::for_pose(Pose::Stand, Face::Surprised)),
            ("sit", Rig::for_pose(Pose::Sit, Face::Vacant)),
            ("lie back", Rig::for_pose(Pose::LieBack(0), Face::Blink)),
            ("lie front", Rig::for_pose(Pose::LieFront(0), Face::Happy)),
            ("jack", Rig::for_pose(Pose::Jack(1), Face::Happy)),
            ("toe", Rig::for_pose(Pose::ToeTouch(1), Face::Vacant)),
            ("stretch", Rig::for_pose(Pose::Stretch, Face::Blink)),
            ("gaze", Rig::for_pose(Pose::Gaze, Face::Curious)),
            ("carry", Rig::carrying(0, Expression::Happy)),
            ("carry", Rig::carrying(1, Expression::Happy)),
            ("read", Rig::reading(0, Expression::Vacant)),
            ("read", Rig::reading(1, Expression::Vacant)),
            ("eat", Rig::eating(0, Expression::Happy)),
            ("eat", Rig::eating(1, Expression::Happy)),
            ("pet", Rig::petting(0, Expression::Smile)),
            ("pet", Rig::petting(1, Expression::Smile)),
            ("wave", Rig::waving(true)),
            ("wave", Rig::waving(false)),
            ("sofa sit", Rig::sofa_sit(Expression::Smile)),
            ("sofa nap", Rig::sofa_nap(Expression::Blink)),
            ("bed sleep", Rig::bed_sleep(0)),
            ("bed sleep", Rig::bed_sleep(1)),
            ("homework", Rig::homework(0, Expression::Vacant)),
            ("homework", Rig::homework(1, Expression::Vacant)),
            ("homework", Rig::homework(2, Expression::Vacant)),
            ("droop", Rig::for_pose(Pose::Stand, Face::Droop)),
            ("droop side", Rig::for_pose(Pose::Side, Face::Droop)),
            ("chopsticks", Rig::chopsticks(0, Expression::Vacant)),
            ("chopsticks", Rig::chopsticks(1, Expression::Happy)),
            ("chopsticks", Rig::chopsticks(2, Expression::Droop)),
            ("eat andagi", Rig::for_pose(Pose::EatAndagi(0), Face::Happy)),
            ("eat andagi", Rig::for_pose(Pose::EatAndagi(1), Face::Happy)),
            ("eat andagi", Rig::for_pose(Pose::EatAndagi(2), Face::Happy)),
            ("floor homework", Rig::floor_homework(0, Expression::Vacant)),
            ("floor homework", Rig::floor_homework(1, Expression::Vacant)),
            ("floor homework", Rig::floor_homework(2, Expression::Vacant)),
            ("paper desk", Rig::paper_desk(0, true, Expression::Vacant)),
            ("paper desk", Rig::paper_desk(1, true, Expression::Vacant)),
            ("paper desk", Rig::paper_desk(2, true, Expression::Vacant)),
            ("paper desk", Rig::paper_desk(3, true, Expression::Vacant)),
            ("paper desk", Rig::paper_desk(0, false, Expression::Vacant)),
            ("paper desk", Rig::paper_desk(3, false, Expression::Vacant)),
            ("read strip", Rig::reading_strip(0, Expression::Vacant)),
            ("read strip", Rig::reading_strip(1, Expression::Vacant)),
            ("cross-legged", Rig::cross_legged(Expression::Curious)),
            ("sill lean", Rig::sill_lean(Expression::Curious)),
            ("under sill", Rig::under_sill(Expression::Curious)),
            ("sit doze", Rig::sit_doze(0)),
            ("sit doze", Rig::sit_doze(1)),
            ("lie read", Rig::lie_read(0, Expression::Vacant)),
            ("lie read", Rig::lie_read(1, Expression::Vacant)),
            ("lie read", Rig::lie_read(2, Expression::Vacant)),
        ]);
        out
    }

    /// The chopsticks, the droop face and the sata andagi for review,
    /// beside the art they join: `HOUSEGUEST_VIGNETTES=/dir cargo test
    /// -p dessplay --lib vignette_sheet -- --ignored` writes
    /// `vignettes-1x.png`, the same pixels scaled 3× nearest-neighbour
    /// (`vignettes-1x-nn3x.png`) and a native 3× render
    /// (`vignettes-3x.png`), over a dark terminal. Rows, left to right:
    /// the held parts at 4× her scale (chopsticks joined, clean, bad;
    /// andagi whole, bitten; the melon bread); her at the desk (homework
    /// for reference, then joined, clean, bad); faces standing and
    /// side-on (vacant, blink, droop); her eating at the open fridge
    /// (melon bread for reference, then the andagi whole and bitten).
    #[test]
    #[ignore = "writes PNGs for review"]
    fn vignette_sheet() {
        enum Shot {
            /// A held part alone, at 4× her scale.
            Part(&'static str),
            /// Her at the desk, as homework places her.
            Desk(Rig),
            /// Her beside the open fridge, facing it.
            Fridge(Rig),
            /// Her alone.
            Her(Rig, Facing),
        }
        const BG: image::Rgba<u8> = image::Rgba([30, 33, 39, 255]);
        const GRID: image::Rgba<u8> = image::Rgba([44, 49, 58, 255]);
        const FLOOR: image::Rgba<u8> = image::Rgba([139, 148, 158, 255]);
        let dir = std::env::var("HOUSEGUEST_VIGNETTES").expect("HOUSEGUEST_VIGNETTES");
        let her = |face| Rig::for_pose(Pose::Stand, face);
        let side = |face| Rig::for_pose(Pose::Side, face);
        let rows: Vec<Vec<Shot>> = vec![
            [
                "chopsticks",
                "chopsticks-clean",
                "chopsticks-bad",
                "andagi",
                "andagi-bitten",
                "melon-bread",
            ]
            .map(Shot::Part)
            .into(),
            vec![
                Shot::Desk(Rig::homework(0, Expression::Vacant)),
                Shot::Desk(Rig::chopsticks(0, Expression::Vacant)),
                Shot::Desk(Rig::chopsticks(1, Expression::Happy)),
                Shot::Desk(Rig::chopsticks(2, Expression::Droop)),
            ],
            vec![
                Shot::Her(her(Face::Vacant), Facing::Right),
                Shot::Her(her(Face::Blink), Facing::Right),
                Shot::Her(her(Face::Droop), Facing::Right),
                Shot::Her(side(Face::Vacant), Facing::Right),
                Shot::Her(side(Face::Blink), Facing::Right),
                Shot::Her(side(Face::Droop), Facing::Right),
            ],
            vec![
                Shot::Fridge(Rig::eating(0, Expression::Happy)),
                Shot::Fridge(Rig::eating_food(0, ANDAGI, Expression::Happy)),
                Shot::Fridge(Rig::eating_food(1, ANDAGI, Expression::Happy)),
            ],
        ];
        let desk = Furniture::Desk.spec().footprint;
        let fridge = Furniture::Fridge.spec().footprint;
        // Each shot's width in cells (her box is 5).
        let cells = |shot: &Shot| -> u32 {
            match shot {
                Shot::Part(_) => 8,
                // Her box is centred on the desk's column 7.
                Shot::Desk(_) => u32::from(desk.0).max(10),
                Shot::Fridge(_) => u32::from(fridge.0) + 6,
                Shot::Her(..) => 5,
            }
        };
        let width = rows
            .iter()
            .map(|row| row.iter().map(|shot| cells(shot) + 1).sum::<u32>() + 1)
            .max()
            .unwrap();
        let render_sheet = |s: u32| {
            let (w, h) = (9 * s, 19 * s);
            let row_h = h * 7;
            let mut sheet = image::RgbaImage::from_pixel(w * width, row_h * rows.len() as u32, BG);
            for (row, shots) in rows.iter().enumerate() {
                let floor = row as u32 * row_h + row_h - h;
                for gx in 0..w * width {
                    for t in 0..s {
                        sheet.put_pixel(gx, floor + h / 2 + t, FLOOR);
                    }
                }
                let mut x0 = w;
                for shot in shots {
                    let span = cells(shot);
                    for gy in 1..=5 {
                        for gx in x0..x0 + w * span {
                            sheet.put_pixel(gx, floor - gy * h, GRID);
                        }
                    }
                    let her_box = |rig: &Rig, facing, x: u32, sheet: &mut image::RgbaImage| {
                        let osaka = render(rig, facing, LINE, w * 5, h * 4 + h / 2).unwrap();
                        image::imageops::overlay(
                            sheet,
                            &osaka,
                            i64::from(x),
                            i64::from(floor - h * 4),
                        );
                    };
                    match shot {
                        Shot::Part(id) => {
                            // 36 × 36 units around the grip, at 4× her
                            // 0.45 px per unit.
                            let side = (36.0 * 0.45 * 4.0 * s as f32) as u32;
                            let svg = format!(
                                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-18 -28 36 36" color="{LINE}">{PARTS}<use href="#{id}"/></svg>"##
                            );
                            let image = rasterize(&svg, (36.0, 36.0), side, side).unwrap();
                            let x = x0 + (w * span - side) / 2;
                            image::imageops::overlay(
                                &mut sheet,
                                &image,
                                i64::from(x),
                                i64::from(floor + h / 2 - side),
                            );
                        }
                        Shot::Desk(rig) => {
                            let (cols, prows) = (u32::from(desk.0), u32::from(desk.1));
                            let (pw, ph) = (w * cols, h * prows + h / 2);
                            let back = render_prop_layer(
                                Furniture::Desk,
                                Layer::Back,
                                Facing::Right,
                                LINE,
                                pw,
                                ph,
                            )
                            .unwrap();
                            image::imageops::overlay(
                                &mut sheet,
                                &back,
                                i64::from(x0),
                                i64::from(floor + h / 2 - ph),
                            );
                            her_box(rig, Facing::Left, x0 + w * 5, &mut sheet);
                        }
                        Shot::Fridge(rig) => {
                            let (cols, prows) = (u32::from(fridge.0), u32::from(fridge.1));
                            let (pw, ph) = (w * cols, h * prows + h / 2);
                            let piece = render_piece(
                                Furniture::Fridge,
                                PieceState::FridgeOpen,
                                Facing::Right,
                                LINE,
                                pw,
                                ph,
                            )
                            .unwrap();
                            image::imageops::overlay(
                                &mut sheet,
                                &piece,
                                i64::from(x0),
                                i64::from(floor - h * prows),
                            );
                            her_box(rig, Facing::Left, x0 + w * (cols + 1), &mut sheet);
                        }
                        Shot::Her(rig, facing) => her_box(rig, *facing, x0, &mut sheet),
                    }
                    x0 += w * (span + 1);
                }
            }
            sheet
        };
        let one = render_sheet(1);
        one.save(format!("{dir}/vignettes-1x.png")).unwrap();
        image::imageops::resize(
            &one,
            one.width() * 3,
            one.height() * 3,
            image::imageops::FilterType::Nearest,
        )
        .save(format!("{dir}/vignettes-1x-nn3x.png"))
        .unwrap();
        render_sheet(3)
            .save(format!("{dir}/vignettes-3x.png"))
            .unwrap();
    }

    /// Lint: no two poses are drawn alike (at any one face), so a pose
    /// wired to another's art (the andagi drawn as the melon bread)
    /// can't pass for its own.
    #[test]
    fn every_pose_is_drawn_its_own_way() {
        use super::super::sprite::ALL;
        for face in [Face::Vacant, Face::Happy] {
            for (i, &a) in ALL.iter().enumerate() {
                for &b in &ALL[i + 1..] {
                    // Frames line art draws as one: her nap (she
                    // breathes in ASCII only), and writing (the pen moves
                    // in ASCII only).
                    // Watching the clouds is lying back with her eyes
                    // open (her face, not her pose, tells it).
                    let shared = matches!(
                        (a, b),
                        (Pose::Nap(_), Pose::Nap(_))
                            | (Pose::Homework(0 | 1), Pose::Homework(0 | 1))
                            | (Pose::LieBack(0), Pose::CloudWatch)
                    );
                    if !shared {
                        assert_ne!(
                            Rig::for_pose(a, face),
                            Rig::for_pose(b, face),
                            "{a:?} and {b:?} ({face:?})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_pose_renders_something_inside_the_box() {
        for (name, rig) in poses() {
            for facing in [Facing::Left, Facing::Right] {
                let image = render(&rig, facing, LINE, 45, 76).expect(name);
                let inked = image.pixels().filter(|p| p.0[3] > 0).count();
                assert!(inked > 400, "{name}: only {inked} pixels");
                // Transparent background: the corners are empty.
                assert_eq!(image.get_pixel(0, 0).0[3], 0, "{name}");
            }
        }
    }

    #[test]
    fn every_door_frame_renders_inside_the_box() {
        for frame in [DoorFrame::Closed, DoorFrame::Ajar, DoorFrame::Open] {
            for facing in [Facing::Left, Facing::Right] {
                let image = render_door(frame, facing, LINE, 45, 76).expect("renders");
                let inked = image.pixels().filter(|p| p.0[3] > 0).count();
                assert!(inked > 800, "{frame:?}: only {inked} pixels");
                // Free-standing: the top corners are empty.
                assert_eq!(image.get_pixel(0, 0).0[3], 0, "{frame:?}");
                assert_eq!(image.get_pixel(44, 0).0[3], 0, "{frame:?}");
            }
        }
    }

    const CHANNELS: [Channel; 10] = [
        Channel::Snow(0),
        Channel::Snow(1),
        Channel::Shopping(0),
        Channel::Shopping(1),
        Channel::ColourBars,
        Channel::Sunrise,
        Channel::Programme(Programme::News),
        Channel::Programme(Programme::Weather),
        Channel::Programme(Programme::Penguins),
        Channel::Programme(Programme::Cooking),
    ];

    /// Every state that applies to each of the second half of the
    /// catalogue.
    const STATES: [(Furniture, PieceState); 13] = [
        (Furniture::Lamp, PieceState::Plain),
        (Furniture::Lamp, PieceState::LampOff),
        (Furniture::Bookshelf, PieceState::Plain),
        (Furniture::Fridge, PieceState::Plain),
        (Furniture::Fridge, PieceState::FridgeOpen),
        (Furniture::CatBed, PieceState::Plain),
        (Furniture::CatBed, PieceState::Cat),
        (Furniture::CatBed, PieceState::CatBiting),
        (Furniture::Clock, PieceState::Plain),
        (
            Furniture::Clock,
            PieceState::Dial(Dial {
                hour: 4,
                quarter: 2,
            }),
        ),
        (Furniture::Window, PieceState::Plain),
        (Furniture::Window, PieceState::Sky(Sky::Dusk)),
        (Furniture::Window, PieceState::Sky(Sky::Night)),
    ];

    /// The dial reads its minute to the quarter, and the sky its phase
    /// by the hour.
    #[test]
    fn the_dial_and_the_sky_read_the_time_of_day() {
        assert_eq!(
            Dial::at(16 * 60 + 44),
            Dial {
                hour: 4,
                quarter: 2
            }
        );
        assert_eq!(Dial::at(0), Dial::at(12 * 60));
        assert_eq!(
            Dial::at(23 * 60 + 59),
            Dial {
                hour: 11,
                quarter: 3
            }
        );
        let phases: Vec<(u16, Sky)> = [
            (0, Sky::Night),
            (4 * 60 + 59, Sky::Night),
            (5 * 60, Sky::Dawn),
            (6 * 60 + 59, Sky::Dawn),
            (7 * 60, Sky::Day),
            (16 * 60 + 59, Sky::Day),
            (17 * 60, Sky::Dusk),
            (18 * 60 + 59, Sky::Dusk),
            (19 * 60, Sky::Evening),
            (20 * 60 + 59, Sky::Evening),
            (21 * 60, Sky::Night),
            (23 * 60 + 59, Sky::Night),
        ]
        .into_iter()
        .collect();
        for (minute, sky) in phases {
            assert_eq!(Sky::at(minute), sky, "{minute}");
        }
        // Every sky shows some time of day; the sky changes only on the
        // hour, the dial only on the quarter-hour.
        for sky in Sky::ALL {
            assert!((0..1440).any(|m| Sky::at(m) == sky), "{sky:?}");
        }
        for minute in 1..1440u16 {
            if minute % 15 != 0 {
                assert_eq!(Dial::at(minute), Dial::at(minute - 1), "{minute}");
            }
            if minute % 60 != 0 {
                assert_eq!(Sky::at(minute), Sky::at(minute - 1), "{minute}");
            }
        }
    }

    /// The hands point where a real clock's do: the minute hand by the
    /// quarter, and the hour hand creeping on 7.5° a quarter toward the
    /// next hour (approved art), never jumping a whole hour at once.
    #[test]
    fn the_hour_hand_creeps_by_quarters() {
        assert_eq!(Dial::at(0).angles(), (0.0, 0.0));
        assert_eq!(Dial::at(3 * 60).angles(), (90.0, 0.0));
        assert_eq!(Dial::at(4 * 60 + 30).angles(), (135.0, 180.0));
        assert_eq!(Dial::at(16 * 60 + 45).angles(), (142.5, 270.0));
        assert_eq!(Dial::at(23 * 60 + 59).angles(), (352.5, 270.0));
        // Each quarter-hour on, the hour hand moves 7.5° and no more,
        // round the dial and back to 12.
        for minute in (15..1440u16).step_by(15) {
            let (was, _) = Dial::at(minute - 15).angles();
            let (now, _) = Dial::at(minute).angles();
            assert_eq!((now - was).rem_euclid(360.0), 7.5, "{minute}");
        }
    }

    /// Every dial and sky inks enough of its footprint, and is drawn the
    /// same whichever way the piece faces, as the guest draws it
    /// ([`render_piece`]: a mirrored dial reads 3:00 as 9:00); and each
    /// quarter of a 12-hour day is its own face.
    #[test]
    fn every_dial_and_sky_renders_inside_its_footprint_unmirrored() {
        let inked = |image: &image::RgbaImage| image.pixels().filter(|p| p.0[3] > 0).count() as u32;
        let both = |item: Furniture, state: PieceState| {
            let (cols, rows) = item.spec().footprint;
            let (w, h) = (u32::from(cols) * 9, u32::from(rows) * 19);
            let [right, left] = [Facing::Right, Facing::Left]
                .map(|facing| render_piece(item, state, facing, LINE, w, h));
            let right = right.unwrap_or_else(|| panic!("{item:?} {state:?} renders"));
            assert!(inked(&right) > w * h / 6, "{item:?} {state:?}");
            assert_eq!(Some(&right), left.as_ref(), "{item:?} {state:?} mirrored");
            right
        };
        let mut faces = std::collections::HashSet::new();
        for minute in (0..24 * 60).step_by(15) {
            let face = both(Furniture::Clock, PieceState::Dial(Dial::at(minute)));
            if minute < 12 * 60 {
                faces.insert(face.into_raw());
            }
        }
        assert_eq!(faces.len(), 48);
        let mut skies = std::collections::HashSet::new();
        for sky in Sky::ALL {
            skies.insert(both(Furniture::Window, PieceState::Sky(sky)).into_raw());
        }
        assert_eq!(skies.len(), 5);
        // Plain, each is a sane look, unmirrored too.
        for item in [Furniture::Clock, Furniture::Window] {
            both(item, PieceState::Plain);
            let (cols, rows) = item.spec().footprint;
            let (w, h) = (u32::from(cols) * 9, u32::from(rows) * 19);
            let [right, left] = [Facing::Right, Facing::Left]
                .map(|facing| render_prop_layer(item, Layer::Whole, facing, LINE, w, h));
            assert_eq!(right, left, "{item:?} mirrored");
        }
    }

    /// The wall clock and the window for review (phase 5b):
    /// `HOUSEGUEST_CLOCK=/dir cargo test -p dessplay --lib clock_sheet --
    /// --ignored` writes `clock-window-1x.png`, the same pixels at 3×
    /// nearest-neighbour (`clock-window-1x-nn3x.png`) and a native 3×
    /// render (`clock-window-3x.png`), over a dark terminal, as the guest
    /// draws them ([`render_piece`]). Hung pieces render at exactly their
    /// footprint, as the game draws a piece off the floor line. Bands,
    /// top to bottom:
    /// 1. the window in its five skies (night, dawn, day, dusk, evening),
    ///    then night facing left (drawn the same);
    /// 2. the dial at 12 hours (columns 12, 1, …, 11) × 4 quarters (rows
    ///    :00, :15, :30, :45);
    /// 3. facings: the clock at 3:00 facing right, facing left (the
    ///    same), and naively mirrored (reads 9:00; underlined red), then
    ///    the window at dusk both ways;
    /// 4. the context: both pieces hung beside the poster over the sofa,
    ///    her in `Gaze` under the window, at 16:00 (day) and 23:00
    ///    (night).
    #[test]
    #[ignore = "writes PNGs for review"]
    fn clock_sheet() {
        enum Draw {
            /// A piece in a state at (x, y), its top row (standing: its
            /// floor row, with the floor row's half cell too).
            Piece(Furniture, PieceState, Facing, (u32, u32), bool),
            /// An SVG in `frame` units over `cols × rows` cells at (x, y).
            Svg(String, (f32, f32), (u32, u32), (u32, u32)),
            /// Her box (5 × 4) at column `x`, on floor row `y`.
            Her(Rig, Facing, (u32, u32)),
            /// A floor line on row `y` from column `x0` to `x1`.
            Floor(u32, u32, u32),
            /// A red underline below row `y`, columns `x0..x1`.
            Wrong(u32, u32, u32),
        }
        const BG: image::Rgba<u8> = image::Rgba([30, 33, 39, 255]);
        const FLOOR: image::Rgba<u8> = image::Rgba([139, 148, 158, 255]);
        const RED: image::Rgba<u8> = image::Rgba([224, 82, 82, 255]);
        let dir = std::env::var("HOUSEGUEST_CLOCK").expect("HOUSEGUEST_CLOCK");
        let hung = |item, state, facing, at| Draw::Piece(item, state, facing, at, false);
        let mut draws = Vec::new();
        // 1. The window.
        for (i, sky) in Sky::ALL.into_iter().enumerate() {
            let at = (1 + 5 * i as u32, 1);
            draws.push(hung(
                Furniture::Window,
                PieceState::Sky(sky),
                Facing::Right,
                at,
            ));
        }
        draws.push(hung(
            Furniture::Window,
            PieceState::Sky(Sky::Night),
            Facing::Left,
            (27, 1),
        ));
        // 2. Every dial.
        for quarter in 0..4u8 {
            for hour in 0..12u8 {
                let (h, q) = (u32::from(hour), u32::from(quarter));
                let dial = PieceState::Dial(Dial { hour, quarter });
                draws.push(hung(
                    Furniture::Clock,
                    dial,
                    Facing::Right,
                    (1 + 4 * h, 4 + 3 * q),
                ));
            }
        }
        // 3. Facings.
        let three = Dial {
            hour: 3,
            quarter: 0,
        };
        draws.push(hung(
            Furniture::Clock,
            PieceState::Dial(three),
            Facing::Right,
            (1, 17),
        ));
        draws.push(hung(
            Furniture::Clock,
            PieceState::Dial(three),
            Facing::Left,
            (5, 17),
        ));
        // What a mirror group would have drawn.
        let (w, h) = prop_frame(Furniture::Clock);
        let naive = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" color="{LINE}">{PROPS}<g transform="translate({w} 0) scale(-1 1)">{}</g></svg>"##,
            dial_parts(three)
        );
        draws.push(Draw::Svg(naive, (w, h), (9, 17), (3, 2)));
        draws.push(Draw::Wrong(19, 9, 12));
        for (x, facing) in [(14, Facing::Right), (19, Facing::Left)] {
            draws.push(hung(
                Furniture::Window,
                PieceState::Sky(Sky::Dusk),
                facing,
                (x, 17),
            ));
        }
        // 4. The context strips: floor on row 27, hung pieces' bottom
        // row at floor − 5 (hang 4).
        let floor = 27u32;
        let gaze = Rig::for_pose(Pose::Gaze, Face::Curious);
        for (x, minute) in [(0u32, 16 * 60), (20, 23 * 60)] {
            let top = floor - 4 - 2;
            draws.push(Draw::Floor(floor, x + 1, x + 20));
            draws.push(Draw::Piece(
                Furniture::Sofa,
                PieceState::Plain,
                Facing::Right,
                (x + 1, floor),
                true,
            ));
            draws.push(hung(
                Furniture::Poster,
                PieceState::Plain,
                Facing::Right,
                (x + 1, top),
            ));
            draws.push(hung(
                Furniture::Clock,
                PieceState::Dial(Dial::at(minute)),
                Facing::Left,
                (x + 6, top),
            ));
            let wx = x + 10;
            draws.push(hung(
                Furniture::Window,
                PieceState::Sky(Sky::at(minute)),
                Facing::Left,
                (wx, top),
            ));
            // Her box a little left of the window's middle, facing it.
            draws.push(Draw::Her(gaze, Facing::Right, (wx + 2 - 3, floor)));
        }
        let (cols, rows) = (49u32, 29u32);
        let render_sheet = |s: u32| {
            let (w, h) = (9 * s, 19 * s);
            let mut sheet = image::RgbaImage::from_pixel(w * cols, h * rows, BG);
            for draw in &draws {
                match draw {
                    Draw::Piece(item, state, facing, at, standing) => {
                        let (pc, pr) = item.spec().footprint;
                        let (pw, mut ph) = (w * u32::from(pc), h * u32::from(pr));
                        let mut top = h * at.1;
                        if *standing {
                            ph += h / 2;
                            top = h * (at.1 - u32::from(pr));
                        }
                        let image =
                            render_piece(*item, *state, *facing, LINE, pw, ph).expect("renders");
                        image::imageops::overlay(
                            &mut sheet,
                            &image,
                            i64::from(w * at.0),
                            i64::from(top),
                        );
                    }
                    Draw::Svg(svg, frame, at, size) => {
                        let image =
                            rasterize(svg, *frame, w * size.0, h * size.1).expect("renders");
                        image::imageops::overlay(
                            &mut sheet,
                            &image,
                            i64::from(w * at.0),
                            i64::from(h * at.1),
                        );
                    }
                    Draw::Her(rig, facing, (x, y)) => {
                        let osaka = render(rig, *facing, LINE, w * 5, h * 4 + h / 2).unwrap();
                        image::imageops::overlay(
                            &mut sheet,
                            &osaka,
                            i64::from(w * x),
                            i64::from(h * (y - 4)),
                        );
                    }
                    Draw::Floor(y, x0, x1) => {
                        for gx in w * x0..w * x1 {
                            for t in 0..s {
                                sheet.put_pixel(gx, h * y + h / 2 + t, FLOOR);
                            }
                        }
                    }
                    Draw::Wrong(y, x0, x1) => {
                        for gx in w * x0..w * x1 {
                            for t in 0..s {
                                sheet.put_pixel(gx, h * y + 2 * s + t, RED);
                            }
                        }
                    }
                }
            }
            sheet
        };
        let one = render_sheet(1);
        one.save(format!("{dir}/clock-window-1x.png")).unwrap();
        image::imageops::resize(
            &one,
            one.width() * 3,
            one.height() * 3,
            image::imageops::FilterType::Nearest,
        )
        .save(format!("{dir}/clock-window-1x-nn3x.png"))
        .unwrap();
        render_sheet(3)
            .save(format!("{dir}/clock-window-3x.png"))
            .unwrap();
    }

    /// A 5 × 7 bitmap of `c` (bit 4 is the left column), for the bubbles
    /// and labels on the stillness sheet: bubbles are terminal text, and
    /// the renderer has no fonts. Only the glyphs the sheet uses.
    fn glyph(c: char) -> [u8; 7] {
        match c {
            '!' => [4, 4, 4, 4, 4, 0, 4],
            '?' => [14, 17, 1, 2, 4, 0, 4],
            '.' => [0, 0, 0, 0, 0, 0, 4],
            '\'' => [4, 4, 8, 0, 0, 0, 0],
            '~' => [0, 0, 8, 21, 2, 0, 0],
            'M' => [17, 27, 21, 21, 17, 17, 17],
            'T' => [31, 4, 4, 4, 4, 4, 4],
            'a' => [0, 0, 14, 1, 15, 17, 15],
            'b' => [16, 16, 22, 25, 17, 17, 30],
            'c' => [0, 0, 14, 16, 16, 17, 14],
            'd' => [1, 1, 13, 19, 17, 17, 15],
            'e' => [0, 0, 14, 17, 31, 16, 14],
            'h' => [16, 16, 22, 25, 17, 17, 17],
            'i' => [4, 0, 12, 4, 4, 4, 14],
            'k' => [16, 16, 18, 20, 24, 20, 18],
            'l' => [12, 4, 4, 4, 4, 4, 14],
            'm' => [0, 0, 26, 21, 21, 21, 17],
            'n' => [0, 0, 22, 25, 17, 17, 17],
            'o' => [0, 0, 14, 17, 17, 17, 14],
            'r' => [0, 0, 22, 25, 16, 16, 16],
            's' => [0, 0, 15, 16, 14, 1, 30],
            't' => [8, 8, 28, 8, 8, 9, 6],
            'u' => [0, 0, 17, 17, 17, 19, 13],
            'w' => [0, 0, 17, 17, 21, 21, 10],
            'z' => [0, 0, 31, 2, 4, 8, 31],
            '1' => [4, 12, 4, 4, 4, 4, 14],
            '2' => [14, 17, 1, 2, 4, 8, 31],
            '3' => [14, 17, 1, 6, 1, 17, 14],
            '4' => [2, 6, 10, 18, 31, 2, 2],
            '5' => [31, 16, 30, 1, 1, 17, 14],
            '6' => [6, 8, 16, 30, 17, 17, 14],
            '7' => [31, 1, 2, 4, 8, 8, 8],
            '8' => [14, 17, 17, 14, 17, 17, 14],
            '9' => [14, 17, 17, 15, 1, 2, 12],
            _ => [0; 7],
        }
    }

    /// Phase 5c's new art for review: `HOUSEGUEST_STILLNESS=/dir cargo
    /// test -p dessplay --lib stillness_sheet -- --ignored` writes
    /// `stillness-1x.png`, the same pixels at 3× nearest-neighbour
    /// (`stillness-1x-nn3x.png`) and a native 3× render
    /// (`stillness-3x.png`), over a dark terminal. Bubbles (terminal
    /// text) are drawn in a stand-in bitmap font. Bands, top to bottom
    /// (numbered on the sheet):
    /// 1. floor homework, the paper out in front of her: writing (two
    ///    frames), the doze, and facing left; then reading on her back,
    ///    the book held up over her face (two frames), and the doze with
    ///    it open on her face;
    /// 2. the paper desk alone, then homework at it kneeling (writing ×2,
    ///    nodding off, asleep), then cross-legged (writing, asleep);
    /// 3. reading a torn strip (two frames, both facings); `Read` with
    ///    the book for reference;
    /// 4. cross-legged before the TV (a programme card on), and the
    ///    current `Sit` there for reference;
    /// 5. the window hung low (hang 1, its sill at her chest), beside a
    ///    sofa and partly behind it: leaning on the sill, chin in her
    ///    hands (both facings); sitting in front of it chin in hands (a
    ///    musing), and dozing there;
    /// 6. cloud-watching (`LieBack`, Curious, held, a musing) lying under
    ///    the low window, against the doze (`LieBack`, Blink, zzz);
    /// 7. the sitting doze (two frames) beside `Sit`;
    /// 8. looking up in place at chat (facing it, on the left): `!` with
    ///    Surprised, then `?` with Curious, sitting, lying back,
    ///    cross-legged and under the window;
    /// 9. the same on the sofa; dozes stir (Blink, `Mm?`);
    /// 10. the programme cards, then the colour bars and the sunrise
    ///     for reference.
    #[test]
    #[ignore = "writes PNGs for review"]
    fn stillness_sheet() {
        use super::super::room::MadeId;
        use super::super::scrap::{self, Scrap};
        use tuirealm::ratatui::style::Color;

        enum Draw {
            /// A piece standing on floor row `y` (left column `x`), or
            /// hung with its top row at `y`.
            Piece(Furniture, PieceState, Facing, (u32, u32), bool),
            /// The TV on `channel`, standing on floor row `y`.
            Tv(Channel, Facing, (u32, u32)),
            /// A made piece, standing on floor row `y`.
            Scrap(Furniture, scrap::Part, Facing, (u32, u32)),
            /// A piece's `Bare` layer (the sofa without the cushion she
            /// hugs), standing on floor row `y`.
            Bare(Furniture, Facing, (u32, u32)),
            /// Her box (5 × 4) at left column `x`, on floor row `y`.
            Her(Rig, Facing, (u32, u32)),
            /// A floor line on row `y` from column `x0` to `x1`.
            Floor(u32, u32, u32),
            /// Terminal text (a bubble) starting at cell (x, y).
            Text(&'static str, (u32, u32)),
            /// A dim label.
            Label(&'static str, (u32, u32)),
        }
        const BG: image::Rgba<u8> = image::Rgba([30, 33, 39, 255]);
        const FLOOR: image::Rgba<u8> = image::Rgba([139, 148, 158, 255]);
        const TEXT: image::Rgba<u8> = image::Rgba([201, 209, 217, 255]);
        const DIM: image::Rgba<u8> = image::Rgba([110, 118, 129, 255]);
        let dir = std::env::var("HOUSEGUEST_STILLNESS").expect("HOUSEGUEST_STILLNESS");
        let glyphs: Vec<(char, Color)> = "Frieren 12"
            .chars()
            .zip(
                [
                    Color::Rgb(201, 209, 217),
                    Color::Rgb(201, 209, 217),
                    Color::Rgb(121, 192, 255),
                    Color::Rgb(201, 209, 217),
                ]
                .into_iter()
                .cycle(),
            )
            .collect();
        let mut cube = Scrap::new(MadeId(0), &glyphs, 7);
        cube.stage = scrap::STAGES;
        let rig = Rig::for_pose;
        let mut draws = Vec::new();
        let band = |k: u32| 7 + 9 * k;
        let label = |draws: &mut Vec<Draw>, text, k: u32| {
            draws.push(Draw::Label(text, (0, band(k) - 6)));
            draws.push(Draw::Floor(band(k), 1, 70));
        };
        // 1. Floor homework, the paper out in front of her; then reading
        // on her back, the book held up over her face, and dozing under it.
        let f = band(0);
        label(&mut draws, "1", 0);
        for (i, frame) in [0u8, 1, 2].into_iter().enumerate() {
            let x = 2 + 7 * i as u32;
            draws.push(Draw::Her(
                rig(Pose::FloorHomework(frame), Face::Vacant),
                Facing::Right,
                (x, f),
            ));
        }
        draws.push(Draw::Text("zzz", (21, f - 2)));
        draws.push(Draw::Her(
            rig(Pose::FloorHomework(0), Face::Vacant),
            Facing::Left,
            (25, f),
        ));
        for (x, frame) in [(34u32, 0u8), (41, 1), (50, 2)] {
            draws.push(Draw::Her(
                rig(Pose::LieRead(frame), Face::Vacant),
                Facing::Right,
                (x, f),
            ));
        }
        draws.push(Draw::Text("zzz", (47, f - 2)));
        // 2. The paper desk: alone, then in use (her box two columns
        // past its end, facing it).
        let f = band(1);
        label(&mut draws, "2", 1);
        draws.push(Draw::Scrap(
            Furniture::Desk,
            scrap::Part::Whole,
            Facing::Right,
            (2, f),
        ));
        let desk = |draws: &mut Vec<Draw>, x: u32, her: Rig| {
            draws.push(Draw::Scrap(
                Furniture::Desk,
                scrap::Part::Back,
                Facing::Right,
                (x, f),
            ));
            draws.push(Draw::Her(her, Facing::Left, (x + 2, f)));
        };
        for (i, frame) in [0u8, 1, 2, 3].into_iter().enumerate() {
            desk(
                &mut draws,
                8 + 9 * i as u32,
                Rig::paper_desk(frame, true, Expression::Vacant),
            );
        }
        draws.push(Draw::Text("zzz", (35, f - 3)));
        for (i, frame) in [0u8, 3].into_iter().enumerate() {
            desk(
                &mut draws,
                45 + 9 * i as u32,
                Rig::paper_desk(frame, false, Expression::Vacant),
            );
        }
        // 3. Reading a torn strip.
        let f = band(2);
        label(&mut draws, "3", 2);
        draws.push(Draw::Her(
            rig(Pose::ReadStrip(0), Face::Vacant),
            Facing::Right,
            (2, f),
        ));
        draws.push(Draw::Her(
            rig(Pose::ReadStrip(1), Face::Vacant),
            Facing::Right,
            (9, f),
        ));
        draws.push(Draw::Her(
            rig(Pose::ReadStrip(0), Face::Vacant),
            Facing::Left,
            (16, f),
        ));
        draws.push(Draw::Her(
            rig(Pose::Read(0), Face::Vacant),
            Facing::Right,
            (27, f),
        ));
        // 4. Cross-legged before the TV, and today's `Sit` there.
        let f = band(3);
        label(&mut draws, "4", 3);
        for (x, channel, her) in [
            (
                2,
                Channel::Programme(Programme::Penguins),
                rig(Pose::CrossLegged, Face::Vacant),
            ),
            (
                16,
                Channel::Programme(Programme::Cooking),
                rig(Pose::CrossLegged, Face::Curious),
            ),
            (32, Channel::Shopping(0), rig(Pose::Sit, Face::Curious)),
        ] {
            draws.push(Draw::Tv(channel, Facing::Right, (x, f)));
            draws.push(Draw::Her(her, Facing::Left, (x + 7, f)));
        }
        // 5. The window hung low (hang 1: its sill at her chest), beside
        // the sofa and partly behind it (the window is drawn first); her
        // face over its middle, leaning on the sill. Then sitting in front
        // of it and dozing there, the settle-ins.
        let f = band(4);
        label(&mut draws, "5", 4);
        let low = |wx: u32| {
            Draw::Piece(
                Furniture::Window,
                PieceState::Sky(Sky::Day),
                Facing::Right,
                (wx, f - 3),
                false,
            )
        };
        draws.push(low(9));
        draws.push(Draw::Piece(
            Furniture::Sofa,
            PieceState::Plain,
            Facing::Right,
            (2, f),
            true,
        ));
        draws.push(Draw::Her(
            rig(Pose::SillLean, Face::Curious),
            Facing::Left,
            (11, f),
        ));
        draws.push(low(20));
        draws.push(Draw::Her(
            rig(Pose::SillLean, Face::Vacant),
            Facing::Right,
            (18, f),
        ));
        draws.push(Draw::Text("~", (24, f - 4)));
        for (wx, her, bubble) in [
            (30u32, rig(Pose::UnderSill, Face::Curious), "~"),
            (40, rig(Pose::SitDoze(1), Face::Blink), "zzz"),
        ] {
            draws.push(low(wx));
            draws.push(Draw::Her(her, Facing::Right, (wx - 1, f)));
            draws.push(Draw::Text(bubble, (wx + 4, f - 4)));
        }
        // 6. Cloud-watching, lying under the (low) window, against the
        // doze on the bare floor.
        let f = band(5);
        label(&mut draws, "6", 5);
        draws.push(Draw::Piece(
            Furniture::Window,
            PieceState::Sky(Sky::Day),
            Facing::Right,
            (3, f - 3),
            false,
        ));
        draws.push(Draw::Her(
            rig(Pose::LieBack(0), Face::Curious),
            Facing::Right,
            (4, f),
        ));
        draws.push(Draw::Text("That cloud's a bun.", (10, f - 2)));
        draws.push(Draw::Her(
            rig(Pose::LieBack(0), Face::Blink),
            Facing::Right,
            (44, f),
        ));
        draws.push(Draw::Text("zzz", (40, f - 2)));
        // 7. The sitting doze beside `Sit`.
        let f = band(6);
        label(&mut draws, "7", 6);
        draws.push(Draw::Her(
            rig(Pose::Sit, Face::Vacant),
            Facing::Right,
            (2, f),
        ));
        draws.push(Draw::Her(
            rig(Pose::SitDoze(0), Face::Blink),
            Facing::Right,
            (9, f),
        ));
        draws.push(Draw::Her(
            rig(Pose::SitDoze(1), Face::Blink),
            Facing::Right,
            (16, f),
        ));
        draws.push(Draw::Text("zzz", (21, f - 3)));
        draws.push(Draw::Her(
            rig(Pose::SitDoze(1), Face::Blink),
            Facing::Left,
            (25, f),
        ));
        // 8–9. Looking up in place: facing the chat (on the left), a `!`
        // then a `?` a cell out from her head.
        let f = band(7);
        label(&mut draws, "8", 7);
        // Lying on her back, her head is already at the chat's end (her
        // facing is toward her feet), so she keeps it.
        let mut x = 4;
        for (pose, head_row, facing) in [
            (Pose::Sit, 3, Facing::Left),
            (Pose::LieBack(0), 1, Facing::Right),
            (Pose::CrossLegged, 3, Facing::Left),
            (Pose::UnderSill, 3, Facing::Left),
        ] {
            for (face, text) in [(Face::Surprised, "!"), (Face::Curious, "?")] {
                draws.push(Draw::Her(rig(pose, face), facing, (x, f)));
                draws.push(Draw::Text(text, (x - 1, f - head_row - 1)));
                x += 8;
            }
        }
        let f = band(8);
        label(&mut draws, "9", 8);
        for (i, (face, text)) in [(Face::Surprised, "!"), (Face::Curious, "?")]
            .into_iter()
            .enumerate()
        {
            let sx = 2 + 11 * i as u32;
            draws.push(Draw::Piece(
                Furniture::Sofa,
                PieceState::Plain,
                Facing::Left,
                (sx, f),
                true,
            ));
            draws.push(Draw::Her(
                rig(Pose::Lounge, face),
                Facing::Left,
                (sx + 2, f),
            ));
            draws.push(Draw::Text(text, (sx + 1, f - 5)));
        }
        // Dozes stir instead.
        draws.push(Draw::Her(
            rig(Pose::LieBack(0), Face::Blink),
            Facing::Right,
            (28, f),
        ));
        draws.push(Draw::Text("Mm?", (25, f - 2)));
        draws.push(Draw::Bare(Furniture::Sofa, Facing::Right, (37, f)));
        draws.push(Draw::Her(
            Rig::sofa_nap(Expression::Blink),
            Facing::Right,
            (39, f),
        ));
        draws.push(Draw::Text("Mm?", (35, f - 3)));
        // 10. The programme cards, then the colour bars and the sunrise.
        let f = band(9);
        label(&mut draws, "10", 9);
        for (i, channel) in Programme::ALL
            .map(Channel::Programme)
            .into_iter()
            .chain([Channel::ColourBars, Channel::Sunrise])
            .enumerate()
        {
            draws.push(Draw::Tv(channel, Facing::Right, (2 + 8 * i as u32, f)));
        }
        let (cols, rows) = (72u32, band(9) + 2);
        let render_sheet = |s: u32| {
            let (w, h) = (9 * s, 19 * s);
            let mut sheet = image::RgbaImage::from_pixel(w * cols, h * rows, BG);
            let put = |sheet: &mut image::RgbaImage, image: &image::RgbaImage, x: u32, y: u32| {
                image::imageops::overlay(sheet, image, i64::from(x), i64::from(y));
            };
            for draw in &draws {
                match draw {
                    Draw::Piece(item, state, facing, at, standing) => {
                        let (pc, pr) = item.spec().footprint;
                        let (pw, mut ph) = (w * u32::from(pc), h * u32::from(pr));
                        let mut top = h * at.1;
                        if *standing {
                            ph += h / 2;
                            top = h * (at.1 - u32::from(pr));
                        }
                        let image =
                            render_piece(*item, *state, *facing, LINE, pw, ph).expect("renders");
                        put(&mut sheet, &image, w * at.0, top);
                    }
                    Draw::Tv(channel, facing, (x, y)) => {
                        let (pc, pr) = Furniture::Tv.spec().footprint;
                        let (pw, ph) = (w * u32::from(pc), h * u32::from(pr) + h / 2);
                        let image = render_tv(*channel, *facing, LINE, pw, ph).expect("renders");
                        put(&mut sheet, &image, w * x, h * (y - u32::from(pr)));
                    }
                    Draw::Scrap(item, part, facing, (x, y)) => {
                        let (pc, pr) = scrap::footprint(*item);
                        let (pw, ph) = (w * u32::from(pc), h * u32::from(pr) + h / 2);
                        if let Some(image) =
                            scrap::render(*item, &cube, *part, *facing, LINE, pw, ph)
                        {
                            put(&mut sheet, &image, w * x, h * (y - u32::from(pr)));
                        }
                    }
                    Draw::Bare(item, facing, (x, y)) => {
                        let (pc, pr) = item.spec().footprint;
                        let (pw, ph) = (w * u32::from(pc), h * u32::from(pr) + h / 2);
                        let image = render_prop_layer(*item, Layer::Bare, *facing, LINE, pw, ph)
                            .expect("renders");
                        put(&mut sheet, &image, w * x, h * (y - u32::from(pr)));
                    }
                    Draw::Her(rig, facing, (x, y)) => {
                        let osaka = render(rig, *facing, LINE, w * 5, h * 4 + h / 2).unwrap();
                        put(&mut sheet, &osaka, w * x, h * (y - 4));
                    }
                    Draw::Floor(y, x0, x1) => {
                        for gx in w * x0..w * x1 {
                            for t in 0..s {
                                sheet.put_pixel(gx, h * y + h / 2 + t, FLOOR);
                            }
                        }
                    }
                    Draw::Text(text, (x, y)) | Draw::Label(text, (x, y)) => {
                        let ink = if matches!(draw, Draw::Text(..)) {
                            TEXT
                        } else {
                            DIM
                        };
                        for (i, c) in text.chars().enumerate() {
                            let (cx, cy) = (w * (x + i as u32) + 2 * s, h * y + 6 * s);
                            for (row, bits) in glyph(c).into_iter().enumerate() {
                                for col in 0..5u32 {
                                    if bits >> (4 - col) & 1 == 1 {
                                        for dy in 0..s {
                                            for dx in 0..s {
                                                sheet.put_pixel(
                                                    cx + col * s + dx,
                                                    cy + row as u32 * s + dy,
                                                    ink,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            sheet
        };
        let one = render_sheet(1);
        one.save(format!("{dir}/stillness-1x.png")).unwrap();
        image::imageops::resize(
            &one,
            one.width() * 3,
            one.height() * 3,
            image::imageops::FilterType::Nearest,
        )
        .save(format!("{dir}/stillness-1x-nn3x.png"))
        .unwrap();
        render_sheet(3)
            .save(format!("{dir}/stillness-3x.png"))
            .unwrap();
    }

    /// The film on her TV for review (phase 5c D7), through the real
    /// path (a JPEG as mpv writes it, decoded and treated, then laid on
    /// the glass): `HOUSEGUEST_FILM_SHEET=/dir cargo test -p dessplay
    /// --lib film_sheet -- --ignored` writes `film-1x.png` and the same
    /// pixels at 3× nearest-neighbour (`film-1x-nn3x.png`), over a dark
    /// terminal. Generated frames (never a real film's), or the frames
    /// named in `HOUSEGUEST_FILM` (comma-separated paths: CC-BY stills,
    /// Sintel or Big Buck Bunny), one row each. Columns: the News card
    /// for reference, the film at 9×19-pixel cells facing right and
    /// left, at 10×20 facing right, and the treated still itself.
    #[test]
    #[ignore = "writes a review sheet: set HOUSEGUEST_FILM_SHEET"]
    fn film_sheet() {
        use super::super::film::{TvPicture, test_frame};
        let Ok(dir) = std::env::var("HOUSEGUEST_FILM_SHEET") else {
            return;
        };
        let file = dessplay_core::types::Ed2kHash([1; 16]);
        let frames: Vec<Vec<u8>> = match std::env::var("HOUSEGUEST_FILM") {
            Ok(paths) => paths
                .split(',')
                .map(|path| std::fs::read(path.trim()).unwrap())
                .collect(),
            Err(_) => (0..4)
                .map(|n| {
                    let mut bytes = Vec::new();
                    image::DynamicImage::ImageRgba8(test_frame(1920, 1080, n * 7))
                        .into_rgb8()
                        .write_to(
                            &mut std::io::Cursor::new(&mut bytes),
                            image::ImageFormat::Jpeg,
                        )
                        .unwrap();
                    bytes
                })
                .collect(),
        };
        const GAP: u32 = 10;
        let (cw, ch) = (6 * 10 + GAP, 4 * 20 + GAP);
        let mut sheet = image::RgbaImage::from_pixel(
            GAP + 5 * cw + 128,
            GAP + frames.len() as u32 * ch.max(102 + GAP),
            image::Rgba([30, 33, 39, 255]),
        );
        for (row, bytes) in frames.iter().enumerate() {
            let y = GAP + row as u32 * ch.max(102 + GAP);
            let picture = match TvPicture::from_frame(file, bytes) {
                Ok(picture) => picture,
                Err(why) => {
                    eprintln!("row {row}: {why:?}");
                    continue;
                }
            };
            let tiles = [
                render_tv(
                    Channel::Programme(Programme::News),
                    Facing::Right,
                    LINE,
                    54,
                    76,
                ),
                render_film(picture.image(), Facing::Right, LINE, 54, 76),
                render_film(picture.image(), Facing::Left, LINE, 54, 76),
                render_film(picture.image(), Facing::Right, LINE, 60, 80),
            ];
            for (col, tile) in tiles.into_iter().enumerate() {
                let tile = tile.unwrap();
                image::imageops::overlay(
                    &mut sheet,
                    &tile,
                    i64::from(GAP + col as u32 * cw),
                    i64::from(y),
                );
            }
            image::imageops::overlay(
                &mut sheet,
                picture.image(),
                i64::from(GAP + 4 * cw),
                i64::from(y),
            );
        }
        sheet.save(format!("{dir}/film-1x.png")).unwrap();
        image::imageops::resize(
            &sheet,
            sheet.width() * 3,
            sheet.height() * 3,
            image::imageops::FilterType::Nearest,
        )
        .save(format!("{dir}/film-1x-nn3x.png"))
        .unwrap();
    }

    #[test]
    fn every_new_piece_renders_inside_its_footprint_in_every_state() {
        for (item, state) in STATES {
            let (cols, rows) = item.spec().footprint;
            let (w, h) = (u32::from(cols) * 9, u32::from(rows) * 19);
            for facing in [Facing::Left, Facing::Right] {
                let image = render_piece(item, state, facing, LINE, w, h)
                    .unwrap_or_else(|| panic!("{item:?} {state:?}"));
                let inked = image.pixels().filter(|p| p.0[3] > 0).count() as u32;
                assert!(inked > w * h / 6, "{item:?} {state:?}: only {inked} pixels");
            }
        }
        // A state changes the drawing only where it applies.
        let plain = |item, state| render_piece(item, state, Facing::Right, LINE, 36, 76);
        assert_ne!(
            plain(Furniture::Lamp, PieceState::Plain),
            plain(Furniture::Lamp, PieceState::LampOff)
        );
        assert_eq!(
            plain(Furniture::Bookshelf, PieceState::Plain),
            plain(Furniture::Bookshelf, PieceState::FridgeOpen)
        );
    }

    /// The second half of the catalogue for review:
    /// `HOUSEGUEST_CATALOGUE=/tmp/catalogue.png cargo test -p dessplay
    /// --lib catalogue_sheet -- --ignored`. Each piece in each state,
    /// then her using each (beside it, facing it, her box a cell clear),
    /// at 1×, 2× and 4× with the cell grid and the floor line.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn catalogue_sheet() {
        let path = std::env::var("HOUSEGUEST_CATALOGUE").expect("HOUSEGUEST_CATALOGUE");
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        let span = |s: u32| cw * s * 13;
        let row_h = ch * 4 * 6;
        let mut rows: Vec<(Furniture, PieceState, Option<Rig>)> = STATES
            .iter()
            .map(|&(item, state)| (item, state, None))
            .collect();
        rows.extend([
            (
                Furniture::Bookshelf,
                PieceState::Plain,
                Some(Rig::reading(0, Expression::Vacant)),
            ),
            (
                Furniture::Bookshelf,
                PieceState::Plain,
                Some(Rig::reading(1, Expression::Vacant)),
            ),
            (
                Furniture::Fridge,
                PieceState::FridgeOpen,
                Some(Rig::eating(0, Expression::Happy)),
            ),
            (
                Furniture::Fridge,
                PieceState::FridgeOpen,
                Some(Rig::eating(1, Expression::Happy)),
            ),
            (
                Furniture::CatBed,
                PieceState::Cat,
                Some(Rig::petting(0, Expression::Smile)),
            ),
            (
                Furniture::CatBed,
                PieceState::CatBiting,
                Some(Rig::petting(1, Expression::Smile)),
            ),
        ]);
        let mut sheet = image::RgbaImage::from_pixel(
            scales.iter().map(|&s| span(s)).sum(),
            row_h * rows.len() as u32,
            image::Rgba([13, 17, 23, 255]),
        );
        for (row, (item, state, her)) in rows.iter().enumerate() {
            let row = row as u32;
            let mut x0 = 0;
            for &s in &scales {
                let (w, h) = (cw * s, ch * s);
                let (cols, prows) = item.spec().footprint;
                let (cols, prows) = (u32::from(cols), u32::from(prows));
                let floor = row * row_h + row_h - h;
                let line = floor + h / 2;
                for gx in 0..span(s) - w {
                    for t in 0..s {
                        sheet.put_pixel(x0 + gx, line + t, image::Rgba([139, 148, 158, 255]));
                    }
                }
                let px = x0 + w;
                for gy in 0..=4 {
                    for gx in 0..w * (cols + 6) {
                        sheet.put_pixel(px + gx, floor - gy * h, image::Rgba([40, 46, 56, 255]));
                    }
                }
                let (pw, ph) = (w * cols, h * prows + h / 2);
                let image = render_piece(*item, *state, Facing::Right, LINE, pw, ph).unwrap();
                image::imageops::overlay(
                    &mut sheet,
                    &image,
                    i64::from(px),
                    i64::from(floor - h * prows),
                );
                if let Some(rig) = her {
                    // Her box a cell clear of the piece, facing it.
                    let hx = px + w * (cols + 1);
                    let osaka = render(rig, Facing::Left, LINE, w * 5, h * 4 + h / 2).unwrap();
                    image::imageops::overlay(
                        &mut sheet,
                        &osaka,
                        i64::from(hx),
                        i64::from(floor - h * 4),
                    );
                }
                x0 += span(s);
            }
        }
        sheet.save(path).unwrap();
    }

    #[test]
    fn every_parcel_and_channel_renders_inside_its_box() {
        for facing in [Facing::Left, Facing::Right] {
            for item in Furniture::ALL {
                let (cols, rows) = item.spec().footprint;
                let (w, h) = (u32::from(cols) * 9, u32::from(rows) * 19);
                for open in [false, true] {
                    let image = render_parcel(item, open, facing, LINE, w, h).expect("renders");
                    let inked = image.pixels().filter(|p| p.0[3] > 0).count();
                    assert!(inked > 400, "{item:?} open={open}: only {inked} pixels");
                    // It stands on the floor, well below the box's top.
                    assert!(
                        (0..w).all(|x| image.get_pixel(x, 0).0[3] == 0),
                        "{item:?} open={open}: reaches the top"
                    );
                }
            }
            let mut shown = Vec::new();
            for channel in CHANNELS {
                let image = render_tv(channel, facing, LINE, 54, 76).expect("renders");
                let inked = image.pixels().filter(|p| p.0[3] > 0).count();
                assert!(inked > 1200, "{channel:?}: only {inked} pixels");
                shown.push(image);
            }
            // Each frame differs from the other.
            assert_ne!(shown[0], shown[1], "static moves");
            assert_ne!(shown[2], shown[3], "he moves");
        }
    }

    /// Deliveries and the TV for review:
    /// `HOUSEGUEST_DELIVERY=/tmp/delivery.png cargo test delivery_sheet -- --ignored`.
    /// Rows: each piece's parcel closed then open (in its footprint); the
    /// TV on each channel frame; her sitting beside it watching the
    /// shopping channel. 1×, 2× and 4×, with the cell grid and floor line.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn delivery_sheet() {
        enum Item {
            Parcel(Furniture, bool),
            Tv(Channel),
            Watching(Channel),
        }
        let path = std::env::var("HOUSEGUEST_DELIVERY").expect("HOUSEGUEST_DELIVERY");
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        let span = |s: u32| cw * s * 16;
        let row_h = ch * 4 * 6;
        let mut items: Vec<Item> = Vec::new();
        for item in Furniture::ALL {
            items.push(Item::Parcel(item, false));
            items.push(Item::Parcel(item, true));
        }
        items.extend(CHANNELS.map(Item::Tv));
        items.push(Item::Watching(Channel::Shopping(0)));
        items.push(Item::Watching(Channel::Shopping(1)));
        let mut sheet = image::RgbaImage::from_pixel(
            scales.iter().map(|&s| span(s)).sum(),
            row_h * items.len() as u32,
            image::Rgba([13, 17, 23, 255]),
        );
        let sitting = Rig::for_pose(Pose::Sit, Face::Curious);
        for (row, item) in items.iter().enumerate() {
            let row = row as u32;
            let mut x0 = 0;
            for &s in &scales {
                let (w, h) = (cw * s, ch * s);
                let piece = match item {
                    Item::Parcel(item, _) => *item,
                    Item::Tv(_) | Item::Watching(_) => Furniture::Tv,
                };
                let (cols, rows) = piece.spec().footprint;
                let (cols, rows) = (u32::from(cols), u32::from(rows));
                let floor = row * row_h + row_h - h;
                let line = floor + h / 2;
                for gx in 0..span(s) - w {
                    for t in 0..s {
                        sheet.put_pixel(x0 + gx, line + t, image::Rgba([139, 148, 158, 255]));
                    }
                }
                let px = x0 + w;
                for gy in 0..=rows {
                    for gx in 0..w * cols {
                        sheet.put_pixel(px + gx, floor - gy * h, image::Rgba([40, 46, 56, 255]));
                    }
                }
                let (pw, ph) = (w * cols, h * rows + h / 2);
                let image = match item {
                    Item::Parcel(item, open) => {
                        render_parcel(*item, *open, Facing::Right, LINE, pw, ph)
                    }
                    Item::Tv(channel) | Item::Watching(channel) => {
                        render_tv(*channel, Facing::Right, LINE, pw, ph)
                    }
                }
                .unwrap();
                image::imageops::overlay(
                    &mut sheet,
                    &image,
                    i64::from(px),
                    i64::from(floor - h * rows),
                );
                if let Item::Watching(_) = item {
                    // Her box centred 3 columns past the TV's right edge.
                    let hx = px + w * (cols + 1);
                    let osaka = render(&sitting, Facing::Left, LINE, w * 5, h * 4 + h / 2).unwrap();
                    image::imageops::overlay(
                        &mut sheet,
                        &osaka,
                        i64::from(hx),
                        i64::from(floor - h * 4),
                    );
                }
                x0 += span(s);
            }
        }
        sheet.save(path).unwrap();
    }

    /// Her door for review:
    /// `HOUSEGUEST_DOOR=/tmp/door.png cargo test door_sheet -- --ignored`.
    /// Rows: each frame on its own, her standing in front of the open
    /// door, her side-on stepping into it. Facing right on the left,
    /// mirrored on the right; 1×, 2× and 4×, with the cell grid and the
    /// floor line.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn door_sheet() {
        let path = std::env::var("HOUSEGUEST_DOOR").expect("HOUSEGUEST_DOOR");
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        let span = |s: u32| cw * s * 7;
        let col_w: u32 = scales.iter().map(|&s| span(s)).sum();
        let row_h = ch * 4 * 6;
        type Scene = (DoorFrame, Option<Rig>);
        let rows: [Scene; 5] = [
            (DoorFrame::Closed, None),
            (DoorFrame::Ajar, None),
            (DoorFrame::Open, None),
            (
                DoorFrame::Open,
                Some(Rig::for_pose(Pose::Stand, Face::Happy)),
            ),
            (
                DoorFrame::Open,
                Some(Rig::for_pose(Pose::Side, Face::Curious)),
            ),
        ];
        let mut sheet = image::RgbaImage::from_pixel(
            col_w * 2,
            row_h * rows.len() as u32,
            image::Rgba([13, 17, 23, 255]),
        );
        for (row, (frame, her)) in rows.iter().enumerate() {
            for (col, facing) in [(0u32, Facing::Right), (1, Facing::Left)] {
                let mut x0 = col * col_w;
                for &s in &scales {
                    let (w, h) = (cw * s, ch * s);
                    let floor = row as u32 * row_h + row_h - h;
                    let line = floor + h / 2;
                    for gx in 0..span(s) {
                        for t in 0..s {
                            sheet.put_pixel(x0 + gx, line + t, image::Rgba([139, 148, 158, 255]));
                        }
                    }
                    let bx = x0 + w;
                    for gy in 0..=4 {
                        for gx in 0..w * 5 {
                            sheet.put_pixel(
                                bx + gx,
                                floor - gy * h,
                                image::Rgba([40, 46, 56, 255]),
                            );
                        }
                    }
                    let top = i64::from(floor - h * 4);
                    let door = render_door(*frame, facing, LINE, w * 5, h * 4 + h / 2).unwrap();
                    image::imageops::overlay(&mut sheet, &door, i64::from(bx), top);
                    if let Some(rig) = her {
                        let osaka = render(rig, facing, LINE, w * 5, h * 4 + h / 2).unwrap();
                        image::imageops::overlay(&mut sheet, &osaka, i64::from(bx), top);
                    }
                    x0 += span(s);
                }
            }
        }
        sheet.save(path).unwrap();
    }

    /// A character sheet for eyeballing the art:
    /// `HOUSEGUEST_SHEET=/tmp/sheet.png cargo test model_sheet -- --ignored`.
    /// Each pose at 1× (9×19 px cells) and 2×, over the dark theme, with
    /// the cell grid and the floor line.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn model_sheet() {
        let path = std::env::var("HOUSEGUEST_SHEET").expect("HOUSEGUEST_SHEET");
        let poses = poses();
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        let cell = |s: u32| (cw * s, ch * s);
        let col_w: u32 = scales.iter().map(|&s| cell(s).0 * 7).sum();
        let row_h = cell(*scales.last().unwrap()).1 * 6;
        let mut sheet = image::RgbaImage::from_pixel(
            col_w * 2,
            row_h * poses.len() as u32,
            image::Rgba([13, 17, 23, 255]),
        );
        // Each pose facing right (left column) and left (right column).
        for (index, (_, rig)) in poses.iter().enumerate() {
            for (col, facing) in [(0u32, Facing::Right), (1, Facing::Left)] {
                let row = index as u32;
                let mut x0 = col * col_w;
                let y0 = row * row_h;
                for &s in &scales {
                    let (w, h) = cell(s);
                    let image = render(rig, facing, LINE, w * 5, h * 4).unwrap();
                    let (bx, by) = (x0 + w, y0 + row_h - h * 5);
                    for gy in 0..=4 {
                        for gx in 0..w * 5 {
                            sheet.put_pixel(bx + gx, by + gy * h, image::Rgba([40, 46, 56, 255]));
                        }
                    }
                    for gx in 0..w * 7 {
                        for t in 0..s {
                            sheet.put_pixel(
                                x0 + gx,
                                by + h * 4 + h / 2 + t,
                                image::Rgba([139, 148, 158, 255]),
                            );
                        }
                    }
                    image::imageops::overlay(&mut sheet, &image, i64::from(bx), i64::from(by));
                    x0 += w * 7;
                }
            }
        }
        sheet.save(path).unwrap();
    }

    #[test]
    fn every_prop_renders_inside_its_footprint() {
        for prop in Furniture::ALL {
            let (cols, rows) = prop.spec().footprint;
            for facing in [Facing::Left, Facing::Right] {
                let (w, h) = (u32::from(cols) * 9, u32::from(rows) * 19);
                let image =
                    render_prop_layer(prop, Layer::Whole, facing, LINE, w, h).expect("renders");
                let inked = image.pixels().filter(|p| p.0[3] > 0).count();
                assert!(inked as u32 > w * h / 4, "{prop:?}: only {inked} pixels");
            }
        }
    }

    /// Her unfilled-furniture alternative: each fill becomes the outline.
    fn unfilled(svg: &str) -> String {
        let mut out = String::new();
        for (i, part) in svg.split('<').enumerate() {
            if i > 0 {
                out.push('<');
            }
            let fill = part
                .find(r##"fill="#"##)
                .map(|at| part[at + 6..at + 13].to_string());
            match fill {
                Some(colour) if !part.starts_with('g') => {
                    let part = part.replace(&format!(r#"fill="{colour}""#), r#"fill="none""#);
                    let part = if part.contains("currentColor") {
                        part.replace(r#"stroke="currentColor""#, &format!(r#"stroke="{colour}""#))
                    } else if part.contains("stroke=") {
                        part
                    } else {
                        part.replacen(' ', &format!(r#" stroke="{colour}" "#), 1)
                    };
                    out.push_str(&part);
                }
                _ => out.push_str(part),
            }
        }
        out
    }

    /// Her furniture for review, beside her for scale:
    /// `HOUSEGUEST_PROPS=/tmp/props.png cargo test props_sheet -- --ignored`.
    /// Each piece filled (top) and as coloured outlines (below), at 1×,
    /// 2× and 4×, over the dark theme, with the cell grid and the floor
    /// line both stand on.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn props_sheet() {
        let path = std::env::var("HOUSEGUEST_PROPS").expect("HOUSEGUEST_PROPS");
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        // Prop, a cell's gap, her, a cell's gap.
        let span = |s: u32| cw * s * 18;
        let row_h = ch * 4 * 6;
        let rows = Furniture::ALL.len() as u32 * 2;
        let mut sheet = image::RgbaImage::from_pixel(
            scales.iter().map(|&s| span(s)).sum(),
            row_h * rows,
            image::Rgba([13, 17, 23, 255]),
        );
        let her = Rig::for_pose(Pose::Stand, Face::Vacant);
        for (index, prop) in Furniture::ALL.into_iter().enumerate() {
            for (variant, outline) in [false, true].into_iter().enumerate() {
                let row = index as u32 * 2 + variant as u32;
                let mut x0 = 0;
                for &s in &scales {
                    let (w, h) = (cw * s, ch * s);
                    let (cols, prop_rows) = prop.spec().footprint;
                    let (cols, prop_rows) = (u32::from(cols), u32::from(prop_rows));
                    // The floor line, half a row below the boxes.
                    let floor = row * row_h + row_h - h;
                    let line = floor + h / 2;
                    for gx in 0..span(s) - w {
                        for t in 0..s {
                            sheet.put_pixel(x0 + gx, line + t, image::Rgba([139, 148, 158, 255]));
                        }
                    }
                    let px = x0 + w;
                    for gy in 0..=prop_rows {
                        for gx in 0..w * cols {
                            sheet.put_pixel(
                                px + gx,
                                floor - gy * h,
                                image::Rgba([40, 46, 56, 255]),
                            );
                        }
                    }
                    let svg = layer_scene(prop, Layer::Whole, Facing::Right, LINE);
                    let svg = if outline { unfilled(&svg) } else { svg };
                    let image =
                        rasterize(&svg, prop_frame(prop), w * cols, h * prop_rows + h / 2).unwrap();
                    image::imageops::overlay(
                        &mut sheet,
                        &image,
                        i64::from(px),
                        i64::from(floor - h * prop_rows),
                    );
                    let hx = px + w * (cols + 1);
                    let osaka = render(&her, Facing::Left, LINE, w * 5, h * 4 + h / 2).unwrap();
                    image::imageops::overlay(
                        &mut sheet,
                        &osaka,
                        i64::from(hx),
                        i64::from(floor - h * 4),
                    );
                    x0 += span(s);
                }
            }
        }
        sheet.save(path).unwrap();
    }

    /// Her using each piece: (piece, layer behind her, her rig, the
    /// piece's column her box is centred on, and her facing when the
    /// piece faces right).
    fn uses() -> Vec<(Furniture, Layer, Rig, i32, Facing)> {
        let mut out = vec![
            (
                Furniture::Sofa,
                Layer::Back,
                Rig::sofa_sit(Expression::Smile),
                4,
                Facing::Right,
            ),
            (
                Furniture::Sofa,
                Layer::Bare,
                Rig::sofa_nap(Expression::Blink),
                4,
                Facing::Right,
            ),
        ];
        for frame in 0..2 {
            out.push((
                Furniture::Bed,
                Layer::Back,
                Rig::bed_sleep(frame),
                3,
                Facing::Right,
            ));
        }
        for frame in 0..3 {
            out.push((
                Furniture::Desk,
                Layer::Back,
                Rig::homework(frame, Expression::Vacant),
                7,
                Facing::Left,
            ));
        }
        out
    }

    /// Her furniture poses for review, composited as the terminal will
    /// show them (the piece's back, her, the piece's front):
    /// `HOUSEGUEST_USE=/tmp/use.png cargo test use_sheet -- --ignored`.
    /// Each use facing right (left half) and mirrored (right half), at
    /// 1×, 2× and 4×, with the cell grid and the floor line.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn use_sheet() {
        let path = std::env::var("HOUSEGUEST_USE").expect("HOUSEGUEST_USE");
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        // Up to 12 cells of scene plus a cell either side.
        let span = |s: u32| cw * s * 14;
        let half: u32 = scales.iter().map(|&s| span(s)).sum();
        let row_h = ch * 4 * 7;
        let uses = uses();
        let mut sheet = image::RgbaImage::from_pixel(
            half * 2,
            row_h * uses.len() as u32,
            image::Rgba([13, 17, 23, 255]),
        );
        for (row, (prop, back, rig, c, her_facing)) in uses.into_iter().enumerate() {
            let (cols, rows) = prop.spec().footprint;
            let (cols, rows) = (i32::from(cols), u32::from(rows));
            for (side, mirrored) in [false, true].into_iter().enumerate() {
                let (facing, c, her) = if mirrored {
                    let flip = match her_facing {
                        Facing::Left => Facing::Right,
                        Facing::Right => Facing::Left,
                    };
                    (Facing::Left, cols - 1 - c, flip)
                } else {
                    (Facing::Right, c, her_facing)
                };
                let mut x0 = side as u32 * half;
                for &s in &scales {
                    let (w, h) = (cw * s, ch * s);
                    // The scene's leftmost column, a cell in from x0.
                    let first = (c - 2).min(0);
                    let origin = |col: i32| {
                        (i64::from(x0) + i64::from(w) * i64::from(col - first + 1)) as u32
                    };
                    let floor = row as u32 * row_h + row_h - h;
                    let line = floor + h / 2;
                    for gx in 0..span(s) - w {
                        for t in 0..s {
                            sheet.put_pixel(x0 + gx, line + t, image::Rgba([139, 148, 158, 255]));
                        }
                    }
                    let last = (c + 3).max(cols);
                    for gy in 0..=rows.max(4) {
                        for gx in origin(first)..origin(last) {
                            sheet.put_pixel(gx, floor - gy * h, image::Rgba([40, 46, 56, 255]));
                        }
                    }
                    let (pw, ph) = (w * cols as u32, h * rows + h / 2);
                    let top = i64::from(floor + h / 2 - ph);
                    let mut layers = Vec::new();
                    let behind = if back == Layer::Back {
                        render_prop_layer(prop, Layer::Back, facing, LINE, pw, ph)
                    } else {
                        render_prop_layer(prop, back, facing, LINE, pw, ph)
                    };
                    layers.extend(behind.map(|i| (i, 0)));
                    let osaka = render(&rig, her, LINE, w * 5, h * 4 + h / 2).unwrap();
                    layers.push((osaka, 1));
                    layers.extend(
                        render_prop_layer(prop, Layer::Front, facing, LINE, pw, ph).map(|i| (i, 0)),
                    );
                    for (image, is_her) in layers {
                        let (x, y) = if is_her == 1 {
                            (origin(c - 2), i64::from(floor - h * 4))
                        } else {
                            (origin(0), top)
                        };
                        image::imageops::overlay(&mut sheet, &image, i64::from(x), y);
                    }
                    x0 += span(s);
                }
            }
        }
        sheet.save(path).unwrap();
    }
}
