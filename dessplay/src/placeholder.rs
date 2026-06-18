//! The not-watching placeholder image (design.md, Placeholder Image).
//!
//! When a user is set to Not Watching because they don't have the
//! current file, their player shows a generated PNG instead of a stale
//! frame: the filename, "You don't have this file", and the session
//! status. Rendered with the embedded DejaVu Sans (see
//! `assets/DejaVu-LICENSE`), so it works on a bare system.

use std::io;
use std::path::Path;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgb, RgbImage};

/// DejaVu Sans, vendored — the placeholder must render without any
/// system font lookup.
const FONT_BYTES: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// Output dimensions. Players scale the image to the window anyway;
/// 720p keeps text crisp without a multi-megabyte PNG.
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
/// Horizontal margin text must fit within.
const MARGIN: f32 = 48.0;

const BACKGROUND: Rgb<u8> = Rgb([16, 20, 24]);
const TITLE_COLOR: Rgb<u8> = Rgb([235, 235, 235]);
const BODY_COLOR: Rgb<u8> = Rgb([165, 175, 185]);

const TITLE_SCALE: f32 = 44.0;
const BODY_SCALE: f32 = 30.0;
/// Vertical gap between lines, as a fraction of the line height.
const LINE_GAP: f32 = 0.45;

/// Render the placeholder and write it as a PNG. The first line is the
/// title (larger, brighter); the rest are body text. Empty lines are
/// spacing.
pub fn render_to(path: &Path, lines: &[String]) -> io::Result<()> {
    let image = render(lines)?;
    image
        .save(path)
        .map_err(|e| io::Error::other(format!("writing placeholder: {e}")))
}

/// Render the placeholder image in memory.
pub fn render(lines: &[String]) -> io::Result<RgbImage> {
    let font = FontRef::try_from_slice(FONT_BYTES)
        .map_err(|e| io::Error::other(format!("embedded font failed to parse: {e}")))?;
    let mut image = RgbImage::from_pixel(WIDTH, HEIGHT, BACKGROUND);

    // Measure the stack first so it can be vertically centered.
    let mut heights = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let scale = fit_scale(&font, line, base_scale(index));
        let scaled = font.as_scaled(scale);
        heights.push(scaled.ascent() - scaled.descent());
    }
    let total: f32 = heights.iter().map(|h| h * (1.0 + LINE_GAP)).sum();
    let mut y = ((HEIGHT as f32 - total) / 2.0).max(MARGIN);

    for (index, line) in lines.iter().enumerate() {
        let scale = fit_scale(&font, line, base_scale(index));
        let scaled = font.as_scaled(scale);
        let baseline = y + scaled.ascent();
        if !line.is_empty() {
            let width = line_width(&font, scale, line);
            let x = ((WIDTH as f32 - width) / 2.0).max(MARGIN);
            let color = if index == 0 { TITLE_COLOR } else { BODY_COLOR };
            draw_line(&mut image, &font, scale, line, x, baseline, color);
        }
        y += heights[index] * (1.0 + LINE_GAP);
    }
    Ok(image)
}

fn base_scale(line_index: usize) -> f32 {
    if line_index == 0 {
        TITLE_SCALE
    } else {
        BODY_SCALE
    }
}

/// Shrink a line's scale until it fits the image width (long filenames
/// — shrinking beats wrapping a name nobody can break sensibly).
fn fit_scale(font: &FontRef, line: &str, base: f32) -> PxScale {
    let available = WIDTH as f32 - 2.0 * MARGIN;
    let mut scale = base;
    while scale > 14.0 && line_width(font, PxScale::from(scale), line) > available {
        scale *= 0.9;
    }
    PxScale::from(scale)
}

fn line_width(font: &FontRef, scale: PxScale, line: &str) -> f32 {
    let scaled = font.as_scaled(scale);
    let mut width = 0.0;
    let mut previous: Option<ab_glyph::GlyphId> = None;
    for c in line.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = previous {
            width += scaled.kern(prev, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

fn draw_line(
    image: &mut RgbImage,
    font: &FontRef,
    scale: PxScale,
    line: &str,
    x: f32,
    baseline: f32,
    color: Rgb<u8>,
) {
    let scaled = font.as_scaled(scale);
    // The text region; nothing is ever drawn into the margins (a
    // pathological filename that overflows even the minimum scale is
    // clipped, not splattered across the edge).
    let right_limit = WIDTH as f32 - MARGIN;
    let mut caret = x;
    let mut previous: Option<ab_glyph::GlyphId> = None;
    for c in line.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = previous {
            caret += scaled.kern(prev, id);
        }
        if caret >= right_limit {
            break; // overflowed the text region; stop.
        }
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(caret, baseline));
        caret += scaled.h_advance(id);
        previous = Some(id);
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue; // whitespace has no outline
        };
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, coverage| {
            let px = bounds.min.x as i32 + gx as i32;
            let py = bounds.min.y as i32 + gy as i32;
            if (px as f32) < MARGIN || (px as f32) >= right_limit || py < 0 || py >= HEIGHT as i32 {
                return;
            }
            let pixel = image.get_pixel_mut(px as u32, py as u32);
            for channel in 0..3 {
                let bg = pixel.0[channel] as f32;
                let fg = color.0[channel] as f32;
                pixel.0[channel] = (bg + (fg - bg) * coverage).round() as u8;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn lines(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn renders_visible_text_on_the_background() {
        let image = render(&lines(&[
            "[Frieren] Sousou no Frieren - 01.mkv",
            "You don't have this file",
            "",
            "Watching: baughn, nero",
        ]))
        .unwrap();
        assert_eq!(image.dimensions(), (WIDTH, HEIGHT));
        let lit = image.pixels().filter(|p| **p != BACKGROUND).count();
        assert!(lit > 500, "expected drawn glyphs, got {lit} non-bg pixels");
    }

    #[test]
    fn very_long_filenames_shrink_to_fit_instead_of_clipping() {
        let long = "X".repeat(300);
        let image = render(&lines(&[&long])).unwrap();
        // Nothing may be drawn in the margins (clipping would put
        // glyphs at the very edge).
        for y in 0..HEIGHT {
            for x in [0, WIDTH - 1] {
                assert_eq!(*image.get_pixel(x, y), BACKGROUND);
            }
        }
    }

    #[test]
    fn render_to_writes_a_decodable_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("placeholder.png");
        render_to(&path, &lines(&["ep1.mkv", "You don't have this file"])).unwrap();
        let loaded = image::open(&path).unwrap();
        assert_eq!(loaded.width(), WIDTH);
    }
}
