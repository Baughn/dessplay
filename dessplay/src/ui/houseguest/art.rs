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
            // Facing right, a forward swing is counter-clockwise.
            Pose::Walk(frame) => {
                let (legs, arms, bob) = match frame % 4 {
                    0 => (
                        [(-18.0, 0.0), (16.0, 24.0)],
                        [(14.0, -8.0), (-14.0, -10.0)],
                        0.0,
                    ),
                    1 => (
                        [(-2.0, 0.0), (4.0, 18.0)],
                        [(4.0, -4.0), (-4.0, -4.0)],
                        -2.0,
                    ),
                    2 => (
                        [(16.0, 24.0), (-18.0, 0.0)],
                        [(-14.0, -10.0), (14.0, -8.0)],
                        0.0,
                    ),
                    _ => (
                        [(4.0, 18.0), (-2.0, 0.0)],
                        [(-4.0, -4.0), (4.0, -4.0)],
                        -2.0,
                    ),
                };
                Self {
                    legs,
                    arms,
                    bob,
                    ..stand
                }
            }
            Pose::Climb(frame) => {
                let (arms, legs) = if frame % 2 == 0 {
                    (
                        [(172.0, -18.0), (-150.0, 30.0)],
                        [(-12.0, 34.0), (6.0, 0.0)],
                    )
                } else {
                    (
                        [(150.0, -30.0), (-172.0, 18.0)],
                        [(-6.0, 0.0), (12.0, -34.0)],
                    )
                };
                Self {
                    arms,
                    legs,
                    ..stand
                }
            }
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
                lean: 10.0,
                tilt: 10.0,
                arms: [(12.0, -6.0), (-128.0, -96.0)],
                ..stand
            },
        }
    }

    /// Pulling a line on box row `row`: body shifted toward the line and
    /// leaning back, both hands reaching the box edge at that row (a
    /// standing image spans ~4.5 rows of the canvas's 160 units).
    fn pulling(heaving: bool, row: u8, expression: Expression) -> Self {
        const ROW_UNITS: f32 = CANVAS_H / 4.5;
        let shift = 16.0;
        let target = (CANVAS_W, (f32::from(row) + 0.5) * ROW_UNITS);
        let aim = |shoulder: f32| {
            let dx = target.0 - (shoulder + shift);
            let dy = target.1 - SHOULDER_Y;
            -dx.atan2(dy).to_degrees()
        };
        // Facing right, the leading leg braces forward (towards the line,
        // counter-clockwise) and the trailing one bends behind her.
        let (lean, legs) = if heaving {
            (-16.0, [(20.0, 24.0), (-30.0, 0.0)])
        } else {
            (-8.0, [(12.0, 10.0), (-18.0, 0.0)])
        };
        Self {
            expression,
            lean,
            tilt: -4.0,
            bob: 0.0,
            shift,
            arms: [(aim(SHOULDERS[0]) + 6.0, 0.0), (aim(SHOULDERS[1]), 0.0)],
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
    let legs: String = (0..2)
        .map(|i| {
            limb(
                (HIPS[i], HIP_Y),
                rig.legs[i],
                "leg-upper",
                "leg-lower",
                THIGH,
            )
        })
        .collect();
    let arms: Vec<String> = (0..2)
        .map(|i| {
            limb(
                (SHOULDERS[i], SHOULDER_Y),
                rig.arms[i],
                "arm-upper",
                "arm-lower",
                UPPER_ARM,
            )
        })
        .collect();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS_W} {CANVAS_H}" color="{line}">{PARTS}<g{mirror}><g transform="translate({shift} {bob})">{legs}<g transform="rotate({lean} 50 {HIP_Y})"><use href="#skirt"/><use href="#torso"/><g transform="rotate({tilt} {nx} {ny})"><use href="#hair-back"/><use href="#face"/><use href="#{face}"/><use href="#hair-front"/></g>{arm0}{arm1}</g></g></g></svg>"##,
        bob = rig.bob,
        shift = rig.shift,
        lean = rig.lean,
        arm0 = arms[0],
        arm1 = arms[1],
        tilt = rig.tilt,
        nx = NECK.0,
        ny = NECK.1,
        face = rig.expression.id(),
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
            ("climb", Rig::for_pose(Pose::Climb(0), Face::Vacant)),
            ("climb", Rig::for_pose(Pose::Climb(1), Face::Vacant)),
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
