//! Osaka as line art: the SVG parts in `art/osaka.svg`, posed by a small
//! rig and rendered with resvg to the pixel size of her 5×4-cell box.
//! Terminals with a graphics protocol show these instead of the ASCII
//! sprite.

use resvg::tiny_skia;
use resvg::usvg;

use super::sprite::{Face, Facing, Pose};

const PARTS: &str = include_str!("art/osaka.svg");
/// The parts' canvas; feet rest on its bottom edge.
const CANVAS_W: f32 = 100.0;
const CANVAS_H: f32 = 160.0;
const HIP_Y: f32 = 120.0;
const HIPS: [f32; 2] = [44.5, 55.5];
const THIGH: f32 = 18.0;
const SHOULDER_Y: f32 = 75.0;
const SHOULDERS: [f32; 2] = [35.5, 64.5];
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
}

impl Expression {
    fn id(self) -> &'static str {
        match self {
            Self::Vacant => "face-vacant",
            Self::Blink => "face-blink",
            Self::Surprised => "face-surprised",
            Self::Smile => "face-smile",
            Self::Dizzy => "face-dizzy",
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
            arms: [(8.0, -4.0), (-8.0, 4.0)],
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
            arms: [(arm(0) + 4.0, 0.0), (arm(1), 0.0)],
            legs,
        }
    }

    /// Climbing a pole `pole` cells ahead of her (in profile, facing it):
    /// both hands on the pole, one high and one low, alternating; the
    /// knee on the high hand's side lifted against it.
    fn climbing(frame: u8, pole: i8, expression: Expression) -> Self {
        let cell = CANVAS_W / 5.0;
        let pole_x = (CANVAS_W / 2.0 + f32::from(pole) * cell).clamp(58.0, CANVAS_W - 2.0);
        let shift = if pole <= 0 { 0.0 } else { 6.0 };
        // Her arms are short (chibi): aim the high hand nearly straight
        // up and the low one at her waist so the alternation reads.
        let (high, low) = (-20.0, 100.0);
        let reach = |i: usize, y: f32| aim((P_SHOULDERS[i] + shift, P_SHOULDER_Y), (pole_x, y));
        let (arms, legs) = if frame.is_multiple_of(2) {
            (
                [(reach(0, low), 0.0), (reach(1, high), 0.0)],
                [(6.0, 0.0), (-62.0, 78.0)],
            )
        } else {
            (
                [(reach(0, high), 0.0), (reach(1, low), 0.0)],
                [(-62.0, 78.0), (6.0, 0.0)],
            )
        };
        Self {
            expression,
            lean: 6.0,
            tilt: -6.0,
            bob: 0.0,
            shift,
            profile: true,
            arms,
            legs,
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
    let head = format!(
        r##"<g transform="rotate({tilt} {nx} {ny})"><use href="#{prefix}hair-back"/><use href="#{prefix}face"/><use href="#{prefix}{face}"/><use href="#{prefix}hair-front"/></g>"##,
        tilt = rig.tilt,
        nx = NECK.0,
        ny = NECK.1,
        face = rig.expression.id(),
    );
    // Frontal: both arms over everything (hands can reach past the
    // head). Profile: far limbs behind the body, near limbs in front.
    let body = if rig.profile {
        format!(
            r##"{far_leg}{near_leg}<g transform="rotate({lean} 50 {HIP_Y})">{far_arm}<use href="#p-skirt"/><use href="#p-torso"/>{head}{near_arm}</g>"##,
            far_leg = leg(0),
            near_leg = leg(1),
            lean = rig.lean,
            far_arm = arm(0),
            near_arm = arm(1),
        )
    } else {
        format!(
            r##"{leg0}{leg1}<g transform="rotate({lean} 50 {HIP_Y})"><use href="#skirt"/><use href="#torso"/>{head}{arm0}{arm1}</g>"##,
            leg0 = leg(0),
            leg1 = leg(1),
            lean = rig.lean,
            arm0 = arm(0),
            arm1 = arm(1),
        )
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS_W} {CANVAS_H}" color="{line}">{PARTS}<g{mirror}><g transform="translate({shift} {bob})">{body}</g></g></svg>"##,
        shift = rig.shift,
        bob = rig.bob,
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
    let svg = scene(rig, facing, line);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let scale = (width as f32 / CANVAS_W).min(height as f32 / CANVAS_H);
    let dx = (width as f32 - CANVAS_W * scale) / 2.0;
    let dy = height as f32 - CANVAS_H * scale;
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
            ("wave", Rig::waving(true)),
            ("wave", Rig::waving(false)),
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
            row_h * poses.len() as u32 / 2 + row_h,
            image::Rgba([13, 17, 23, 255]),
        );
        for (index, (_, rig)) in poses.iter().enumerate() {
            let (col, row) = (index as u32 % 2, index as u32 / 2);
            let mut x0 = col * col_w;
            let y0 = row * row_h;
            for &s in &scales {
                let (w, h) = cell(s);
                let facing = if index % 3 == 2 {
                    Facing::Left
                } else {
                    Facing::Right
                };
                let image = render(rig, facing, LINE, w * 5, h * 4).unwrap();
                let (bx, by) = (x0 + w, y0 + row_h - h * 5);
                // Cell grid (faint) and the floor border line mid-row.
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
        sheet.save(path).unwrap();
    }
}
