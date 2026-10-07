//! The film on her TV (phase 5c D7): a still of what's playing, made from
//! a player screenshot and held on the TV's glass while she watches.
//!
//! Everything here but [`box_scale`] runs off the UI thread (the
//! blocking pool, run.rs), on a frame the player wrote to the TV's own
//! private screenshot slot ([`crate::screenshot::Slot`]): the frame
//! never leaves the process, and nothing here logs pixels. The guest only
//! ever receives the finished [`TvPicture`], through
//! [`Guest::set_tv_picture`](super::Guest::set_tv_picture), and only her
//! drawing reads it: what she does never depends on it.

use std::sync::Arc;

use dessplay_core::types::Ed2kHash;
use image::{Rgba, RgbaImage};

/// The treated still's size, in pixels: the glass's 58:46, and larger
/// than any glass a terminal's cells give (about 27×22 pixels at 9×19
/// cells; see `art::render_film`), so painting only ever scales it down,
/// once, to the glass's whole-pixel rectangle at the cell size of the
/// day. A new cell size never needs a new screenshot.
pub const SOURCE: (u32, u32) = (128, 102);
/// The glass's aspect (`art::GLASS`, 58 × 46 units).
const ASPECT: f32 = 58.0 / 46.0;
/// The centre crop is zoomed this much past the glass's aspect: faces
/// come up bigger, and most hardsub bands and letterbox mattes fall
/// outside it (the mock's column 7, the user's choice).
const ZOOM: f32 = 1.3;
/// One levels stretch from luma's 2nd to 98th percentile, the same map
/// on all three channels (so hues keep), its gain at most this.
const GAIN_MAX: f32 = 3.0;
/// Saturation, after the stretch.
const SATURATION: f32 = 1.25;
/// A frame darker than this (mean luma, of 255) is black: a fade, a
/// cut, a player between files. Measured before the stretch.
const BLACK_MEAN: f32 = 20.0;
/// A frame whose luma spreads less than this (2nd to 98th percentile)
/// is flat: a title card's fill, a blank. Measured before the stretch.
const FLAT_SPREAD: f32 = 24.0;

/// A treated still's identity: a hash of its pixels, so the same frame
/// twice (a paused film) is the same image, cached once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FilmId(u64);

/// A still of the film for her TV: which frame (its pixels' hash), of
/// which file, and its pixels ([`SOURCE`] large). No `Debug`: nothing
/// prints it.
#[derive(Clone)]
pub struct TvPicture {
    id: FilmId,
    file: Ed2kHash,
    image: Arc<RgbaImage>,
}

/// Why a frame makes no picture (logged at debug; never pixels).
#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    /// It wouldn't decode (within the decoder's caps).
    Decode(String),
    /// Black, or nearly.
    Black,
    /// One flat colour, or nearly.
    Flat,
}

impl TvPicture {
    /// The picture of a screenshot's bytes, a frame of `file`: decoded
    /// within the chat images' caps, then [`treat`]ed. Blocking.
    pub fn from_frame(file: Ed2kHash, bytes: &[u8]) -> Result<Self, Rejected> {
        let decoded = crate::chat_images::decode_capped(bytes).map_err(Rejected::Decode)?;
        Ok(Self::of(file, treat(&decoded.into_rgba8())?))
    }

    /// A picture of `file` holding `image` as it is (already treated).
    pub(super) fn of(file: Ed2kHash, image: RgbaImage) -> Self {
        Self {
            id: FilmId(fnv(image.as_raw())),
            file,
            image: Arc::new(image),
        }
    }

    /// The file it's a frame of.
    pub fn file(&self) -> Ed2kHash {
        self.file
    }

    pub(super) fn id(&self) -> FilmId {
        self.id
    }

    pub(super) fn image(&self) -> &RgbaImage {
        &self.image
    }
}

/// FNV-1a, 64-bit.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Luma of an RGB pixel, 0–255.
fn luma(p: &Rgba<u8>) -> f32 {
    0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])
}

/// The still her TV shows of `frame`: centred, cropped to the glass's
/// aspect and zoomed [`ZOOM`]×, box-scaled to [`SOURCE`], one levels
/// stretch from its luma, then saturation ×[`SATURATION`]. A black or
/// flat frame is rejected (measured on the scaled still, before the
/// stretch, whose gain would hide a flat one).
pub fn treat(frame: &RgbaImage) -> Result<RgbaImage, Rejected> {
    if frame.width() == 0 || frame.height() == 0 {
        return Err(Rejected::Flat);
    }
    let mut still = centre(frame, ZOOM);
    let mut lumas: Vec<f32> = still.pixels().map(luma).collect();
    let mean = lumas.iter().sum::<f32>() / lumas.len() as f32;
    if mean < BLACK_MEAN {
        return Err(Rejected::Black);
    }
    lumas.sort_by(f32::total_cmp);
    let at = |q: f32| lumas[((lumas.len() - 1) as f32 * q).round() as usize];
    let (lo, hi) = (at(0.02), at(0.98));
    if hi - lo < FLAT_SPREAD {
        return Err(Rejected::Flat);
    }
    let gain = (255.0 / (hi - lo)).min(GAIN_MAX);
    for pixel in still.pixels_mut() {
        let stretched = [0, 1, 2].map(|c| ((f32::from(pixel[c]) - lo) * gain).clamp(0.0, 255.0));
        let y = 0.299 * stretched[0] + 0.587 * stretched[1] + 0.114 * stretched[2];
        for (c, v) in stretched.into_iter().enumerate() {
            pixel[c] = (y + (v - y) * SATURATION).clamp(0.0, 255.0).round() as u8;
        }
        pixel[3] = 255;
    }
    Ok(still)
}

/// The centre of `frame` (not empty) at the glass's aspect, zoomed
/// `zoom`×, box-scaled to [`SOURCE`].
fn centre(frame: &RgbaImage, zoom: f32) -> RgbaImage {
    let (w, h) = (frame.width() as f32, frame.height() as f32);
    let (cw, ch) = if w / h > ASPECT {
        (h * ASPECT, h)
    } else {
        (w, w / ASPECT)
    };
    let (cw, ch) = ((cw / zoom).max(1.0), (ch / zoom).max(1.0));
    let (left, top) = (((w - cw) / 2.0) as u32, ((h - ch) / 2.0) as u32);
    let crop = image::imageops::crop_imm(frame, left, top, cw as u32, ch as u32).to_image();
    box_scale(&crop, SOURCE.0, SOURCE.1)
}

/// `src` scaled to `w × h` by area: each output pixel the mean of the
/// source it covers, fractional edges weighted (a box filter, as the
/// mock's). Separable: across, then down.
pub(crate) fn box_scale(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    if sw == 0 || sh == 0 {
        return RgbaImage::new(w as u32, h as u32);
    }
    // For each output index, the source indices it covers and how much.
    let spans = |from: usize, to: usize| -> Vec<Vec<(usize, f32)>> {
        let scale = from as f32 / to as f32;
        (0..to)
            .map(|i| {
                let (a, b) = (i as f32 * scale, (i + 1) as f32 * scale);
                let mut out = Vec::new();
                let mut j = a.floor() as usize;
                while (j as f32) < b && j < from {
                    let cover = (b.min(j as f32 + 1.0) - a.max(j as f32)).max(0.0);
                    if cover > 0.0 {
                        out.push((j, cover));
                    }
                    j += 1;
                }
                if out.is_empty() {
                    out.push((j.min(from - 1), 1.0));
                }
                out
            })
            .collect()
    };
    let (across, down) = (spans(sw, w), spans(sh, h));
    let raw = src.as_raw();
    // Across: sh rows of w pixels, as f32 RGBA.
    let mut mid = vec![0f32; sh * w * 4];
    for y in 0..sh {
        for (x, span) in across.iter().enumerate() {
            let total: f32 = span.iter().map(|&(_, c)| c).sum();
            for &(sx, cover) in span {
                let at = (y * sw + sx) * 4;
                for c in 0..4 {
                    mid[(y * w + x) * 4 + c] += f32::from(raw[at + c]) * cover / total;
                }
            }
        }
    }
    let mut out = RgbaImage::new(w as u32, h as u32);
    for (y, span) in down.iter().enumerate() {
        let total: f32 = span.iter().map(|&(_, c)| c).sum();
        for x in 0..w {
            let mut acc = [0f32; 4];
            for &(sy, cover) in span {
                for (c, a) in acc.iter_mut().enumerate() {
                    *a += mid[(sy * w + x) * 4 + c] * cover / total;
                }
            }
            out.put_pixel(
                x as u32,
                y as u32,
                Rgba(acc.map(|v| v.round().clamp(0.0, 255.0) as u8)),
            );
        }
    }
    out
}

/// A generated frame for tests and the review sheet (never a real
/// film's): a sky-to-ground gradient with a sun, a hill, and a bright
/// marker left of centre (to tell which way it's drawn), `width ×
/// height`; frame `n` has its own colours (six turns) and sun (a hundred
/// places along the sky), so each of 600 is a still of its own.
#[cfg(test)]
pub(crate) fn test_frame(width: u32, height: u32, n: u32) -> RgbaImage {
    let hue = (n % 6) as u8;
    let sun_x = 0.45 + 0.4 * ((n / 6 * 37) % 100) as f32 / 100.0;
    RgbaImage::from_fn(width, height, |x, y| {
        let (fx, fy) = (x as f32 / width as f32, y as f32 / height as f32);
        let sun = ((fx - sun_x).powi(2) + (fy - 0.3).powi(2)).sqrt() < 0.1;
        let hill = fy > 0.7 - 0.15 * (fx * 3.0).sin();
        let mut rgb = if sun {
            [250, 220, 90]
        } else if hill {
            [40, (90.0 + 80.0 * fx) as u8, 50]
        } else {
            [(80.0 + 100.0 * fy) as u8, (120.0 + 60.0 * fy) as u8, 220]
        };
        if fx > 0.32 && fx < 0.42 && fy > 0.3 && fy < 0.45 {
            rgb = [230, 30, 40];
        }
        rgb.rotate_left(usize::from(hue % 3));
        if hue >= 3 {
            rgb.swap(0, 1);
        }
        Rgba([rgb[0], rgb[1], rgb[2], 255])
    })
}

/// A still of a generated frame ([`test_frame`] `n`) of
/// `file`, for tests.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn test_picture(file: Ed2kHash, n: u32) -> TvPicture {
    TvPicture::of(
        file,
        treat(&test_frame(320, 180, n)).expect("a test frame makes a still"),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn encode(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image.clone())
            .into_rgb8()
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        bytes
    }

    const FILE: Ed2kHash = Ed2kHash([7; 16]);

    /// A frame (a JPEG, as mpv writes) makes a still of [`SOURCE`] size,
    /// opaque, of its file; the same frame twice is the same picture,
    /// and another frame another.
    #[test]
    fn a_frame_makes_a_still_of_its_file() {
        let frame = encode(&test_frame(640, 360, 0));
        let picture = TvPicture::from_frame(FILE, &frame).unwrap();
        assert_eq!(picture.image().dimensions(), SOURCE);
        assert!(picture.image().pixels().all(|p| p[3] == 255));
        assert_eq!(picture.file(), FILE);
        let again = TvPicture::from_frame(FILE, &frame).unwrap();
        assert_eq!(picture.id(), again.id(), "a paused film is one picture");
        let other = TvPicture::from_frame(FILE, &encode(&test_frame(640, 360, 1))).unwrap();
        assert_ne!(picture.id(), other.id());
    }

    /// Black and flat frames make no picture (a fade, a blank, a title
    /// card's fill), however the stretch would lift them; and bytes that
    /// aren't an image make none.
    #[test]
    fn black_and_flat_frames_are_rejected() {
        let flat = |rgb: [u8; 3]| {
            encode(&RgbaImage::from_pixel(
                320,
                180,
                Rgba([rgb[0], rgb[1], rgb[2], 255]),
            ))
        };
        assert_eq!(
            TvPicture::from_frame(FILE, &flat([4, 4, 6])).err(),
            Some(Rejected::Black)
        );
        assert_eq!(
            TvPicture::from_frame(FILE, &flat([120, 90, 200])).err(),
            Some(Rejected::Flat)
        );
        // Dark but for faint stripes: black.
        let dim = RgbaImage::from_fn(320, 180, |x, _| {
            let v = if x % 20 < 10 { 6 } else { 22 };
            Rgba([v, v, v, 255])
        });
        assert_eq!(
            TvPicture::from_frame(FILE, &encode(&dim)).err(),
            Some(Rejected::Black)
        );
        // A soft gradient spanning less than the flatness floor.
        let soft = RgbaImage::from_fn(320, 180, |x, _| {
            let v = 100 + (x * 15 / 320) as u8;
            Rgba([v, v, v, 255])
        });
        assert_eq!(
            TvPicture::from_frame(FILE, &encode(&soft)).err(),
            Some(Rejected::Flat)
        );
        assert!(matches!(
            TvPicture::from_frame(FILE, b"not an image"),
            Err(Rejected::Decode(_))
        ));
    }

    /// The crop is the frame's centre at the glass's aspect, zoomed
    /// 1.3×: a frame whose outer band (letterbox mattes, a hardsub
    /// strip) differs from its middle shows only the middle; and the
    /// stretch lifts a dim scene's range (by at most 3×), keeping its hue.
    #[test]
    fn the_still_is_the_centre_lifted() {
        // A 16:9 frame: magenta bars top and bottom (a tenth each: the
        // crop keeps the middle 554 of its 720 rows), a dim two-tone
        // middle.
        let frame = RgbaImage::from_fn(1280, 720, |x, y| {
            if !(72..648).contains(&y) {
                Rgba([255, 0, 255, 255])
            } else if x < 640 {
                Rgba([30, 40, 70, 255])
            } else {
                Rgba([70, 80, 110, 255])
            }
        });
        let still = treat(&frame).unwrap();
        assert!(
            still
                .pixels()
                .all(|p| !(p[0] > 200 && p[1] < 60 && p[2] > 200)),
            "no matte in the crop"
        );
        let lumas: Vec<f32> = still.pixels().map(luma).collect();
        let (lo, hi) = lumas
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
        // The middle's lumas, 40 apart, stretched by the most gain there
        // is (3×) and pulled down to black at the low end.
        assert!(
            lo < 16.0 && hi - lo > 100.0 && hi - lo < 130.0,
            "lifted, by no more than 3×: {lo}..{hi}"
        );
        // The blue stays bluest.
        let p = still.get_pixel(SOURCE.0 - 1, SOURCE.1 / 2);
        assert!(p[2] > p[0] && p[2] > p[1], "{p:?}");
    }

    /// The user's treatment, by its numbers (the mock's column 7, with
    /// the Q1 answer's saturation): the crop is the centre zoomed 1.3×
    /// (the frame's rings at 1.25× and 1.35× tell 1.2 and 1.4 from it),
    /// and saturation is ×1.25 about each pixel's luma (a frame already
    /// spanning black to white, so the stretch leaves it be).
    #[test]
    fn the_treatment_is_the_users_choice() {
        // At the glass's aspect: magenta outside the 1.25× crop, cyan
        // from there to the 1.35× crop, grey within.
        let (w, h) = (580u32, 460u32);
        let inside = |x: u32, y: u32, zoom: f32| {
            let (cw, ch) = (w as f32 / zoom, h as f32 / zoom);
            let (dx, dy) = (
                (x as f32 + 0.5 - w as f32 / 2.0).abs(),
                (y as f32 + 0.5 - h as f32 / 2.0).abs(),
            );
            dx < cw / 2.0 && dy < ch / 2.0
        };
        let rings = RgbaImage::from_fn(w, h, |x, y| {
            if !inside(x, y, 1.25) {
                Rgba([255, 0, 255, 255])
            } else if !inside(x, y, 1.35) {
                Rgba([0, 255, 255, 255])
            } else {
                Rgba([128, 128, 128, 255])
            }
        });
        let magenta = |p: &Rgba<u8>| p[0] > p[1].saturating_add(20);
        let cyan = |p: &Rgba<u8>| p[1] > p[0].saturating_add(40);
        for (zoom, ok) in [(1.2, false), (ZOOM, true), (1.4, false)] {
            let still = centre(&rings, zoom);
            let no_matte = !still.pixels().any(magenta);
            let ring = (0..SOURCE.1).all(|y| cyan(still.get_pixel(0, y)))
                && (0..SOURCE.0).all(|x| cyan(still.get_pixel(x, 0)));
            assert_eq!(
                no_matte && ring,
                ok,
                "zoom {zoom}: no matte {no_matte}, ring {ring}"
            );
        }
        assert_eq!(ZOOM, 1.3);
        // Black, a colour, white: the stretch's 2nd to 98th percentile
        // is 0 to 255 already.
        let bands = RgbaImage::from_fn(640, 360, |x, _| match x * 3 / 640 {
            0 => Rgba([0, 0, 0, 255]),
            1 => Rgba([100, 150, 200, 255]),
            _ => Rgba([255, 255, 255, 255]),
        });
        let still = treat(&bands).unwrap();
        // Luma 140.75: each channel 1.25× as far from it.
        let p = still.get_pixel(SOURCE.0 / 2, SOURCE.1 / 2);
        for (c, want) in [90u8, 152, 215].into_iter().enumerate() {
            assert!(p[c].abs_diff(want) <= 1, "channel {c}: {p:?}, want {want}");
        }
    }

    /// Box scaling keeps a flat colour flat and averages a checker to
    /// grey, at fractional ratios both ways.
    #[test]
    fn box_scaling_averages_by_area() {
        let flat = RgbaImage::from_pixel(37, 23, Rgba([10, 200, 30, 255]));
        for (w, h) in [(5, 3), (36, 22), (50, 40)] {
            let out = box_scale(&flat, w, h);
            assert!(out.pixels().all(|p| p.0 == [10, 200, 30, 255]), "{w}×{h}");
        }
        let checker = RgbaImage::from_fn(64, 64, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 254 };
            Rgba([v, v, v, 255])
        });
        let out = box_scale(&checker, 7, 5);
        assert!(
            out.pixels().all(|p| p[0].abs_diff(127) <= 8),
            "{:?}",
            out.get_pixel(0, 0)
        );
    }
}
