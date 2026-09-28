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
/// Prop units per cell (her scale at a 9 × 19 px cell), so her
/// furniture shares her line weights.
const CELL_UNITS: (f32, f32) = (20.0, 42.0);
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
    // Frontal: both arms over everything (hands can reach past the
    // head). Profile: far limbs behind the body, near limbs in front.
    let body = if rig.profile {
        format!(
            r##"{legs_behind}<g transform="rotate({lean} 50 {HIP_Y})">{far_arm}{hair}<use href="#{skirt}"/><use href="#p-torso"/>{head}</g>{legs_before}<g transform="rotate({lean} 50 {HIP_Y})">{cushion}{near_arm}</g>"##,
            legs_behind = if rig.seated || rig.legs_front {
                String::new()
            } else {
                leg(0) + &leg(1)
            },
            legs_before = if rig.seated {
                format!(
                    r##"<g transform="rotate({} 50 {HIP_Y})"><use href="#p-seated-legs"/></g>"##,
                    rig.lean
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
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS_W} {CANVAS_H}" color="{line}">{PARTS}<g{mirror}>{stool}<g transform="translate({shift} {bob}) translate(50 {HIP_Y}) scale({scale}) rotate({turn}) translate(-50 -{HIP_Y})">{body}</g></g></svg>"##,
        stool = if rig.stool {
            format!(
                r##"<use href="#stool" transform="translate({} 0)"/>"##,
                rig.shift
            )
        } else {
            String::new()
        },
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
    }
}

/// `layer` of a piece as an SVG document, in the piece's own frame.
fn layer_scene(prop: Furniture, layer: Layer, facing: Facing, line: &str) -> String {
    let (w, h) = prop_frame(prop);
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({w} 0) scale(-1 1)""#),
    };
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

/// A prop's frame in SVG units.
fn prop_frame(prop: Furniture) -> (f32, f32) {
    let (cols, rows) = prop.footprint();
    (
        f32::from(cols) * CELL_UNITS.0,
        f32::from(rows) * CELL_UNITS.1,
    )
}

/// Rasterize `svg`, whose frame is `(fw, fh)` units, into a `width ×
/// height` image: scaled uniformly, centred, bottom-aligned.
fn rasterize(svg: &str, (fw, fh): (f32, f32), width: u32, height: u32) -> Option<image::RgbaImage> {
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
            ("wave", Rig::waving(true)),
            ("wave", Rig::waving(false)),
            ("sofa sit", Rig::sofa_sit(Expression::Smile)),
            ("sofa nap", Rig::sofa_nap(Expression::Blink)),
            ("bed sleep", Rig::bed_sleep(0)),
            ("bed sleep", Rig::bed_sleep(1)),
            ("homework", Rig::homework(0, Expression::Vacant)),
            ("homework", Rig::homework(1, Expression::Vacant)),
            ("homework", Rig::homework(2, Expression::Vacant)),
        ]);
        out
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
            let (cols, rows) = prop.footprint();
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
                    let (cols, prop_rows) = prop.footprint();
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
            let (cols, rows) = prop.footprint();
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
