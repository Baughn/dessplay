//! Semantic tones -> concrete styles, in one place.

use std::sync::{Mutex, OnceLock};

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::style::{Color, Modifier, Style};

use palette::{FromColor, Hsluv, Lab, Srgb, color_difference::Ciede2000};

use super::props::Tone;

/// Color depth reported by the terminal backend. Production detects this
/// once when the terminal starts; tests inject it so rendering never depends
/// on process-global environment variables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorDepth {
    /// ANSI/indexed colors only. Keep the user's terminal theme intact.
    #[default]
    Limited,
    /// 24-bit RGB. DessPlay supplies its own coherent dark theme.
    TrueColor,
}

impl ColorDepth {
    /// Detect the production terminal's advertised color depth.
    pub fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM").ok();
        let term = std::env::var("TERM").ok();
        Self::detect_from(
            crossterm::style::available_color_count(),
            colorterm.as_deref(),
            term.as_deref(),
        )
    }

    fn detect_from(color_count: u16, colorterm: Option<&str>, term: Option<&str>) -> Self {
        if color_count == u16::MAX
            || colorterm.is_some_and(advertises_truecolor)
            || term.is_some_and(advertises_truecolor)
        {
            Self::TrueColor
        } else {
            Self::Limited
        }
    }
}

fn advertises_truecolor(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    matches!(value.as_str(), "truecolor" | "24bit") || value.ends_with("-direct")
}

// Visually distinct hues; deliberately avoids plain Red/Green/Blue/Gray,
// which carry ready-state meaning elsewhere in the UI. This is also the
// finite speaker palette on limited-color terminals.
const USER_PALETTE: [Color; 10] = [
    Color::Cyan,
    Color::Magenta,
    Color::LightCyan,
    Color::LightMagenta,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightRed,
    Color::Indexed(208), // orange
    Color::Indexed(141), // lavender
];

/// Number of simultaneously distinct speaker colors available without RGB.
pub const LIMITED_SPEAKER_CAPACITY: usize = USER_PALETTE.len();

/// App background used when the terminal advertises 24-bit color.
pub const TRUECOLOR_BACKGROUND: Color = Color::Rgb(13, 17, 23);

/// Default text color paired with [`TRUECOLOR_BACKGROUND`].
pub const TRUECOLOR_FOREGROUND: Color = Color::Rgb(230, 237, 243);

/// Muted text color paired with [`TRUECOLOR_BACKGROUND`]. True-color mode
/// materializes `DIM` into this foreground instead of relying on SGR 2,
/// whose interaction with explicit RGB colors varies between terminals.
pub const TRUECOLOR_MUTED_FOREGROUND: Color = Color::Rgb(139, 148, 158);

/// Distinct speakers remain active for this rolling wall-clock window.
pub const SPEAKER_WINDOW_MILLIS: u64 = 5 * 60 * 1_000;

/// Style for a tone.
pub fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Good => Style::default().fg(Color::Green),
        Tone::Blocked => Style::default().fg(Color::Red),
        Tone::Paused => Style::default().fg(Color::Yellow),
        Tone::Transfer => Style::default().fg(Color::Blue),
        Tone::Idle => Style::default().fg(Color::DarkGray),
        Tone::Muted => Style::default().add_modifier(Modifier::DIM),
        Tone::Normal => Style::default(),
    }
}

/// Border color for a pane, focused vs not. Single source of truth so the
/// chat input border can match the rest of its pane.
pub fn border_color(focused: bool) -> Color {
    if focused {
        Color::Yellow
    } else {
        Color::DarkGray
    }
}

/// Border style for a pane, focused vs not.
pub fn border_style(focused: bool) -> Style {
    Style::default().fg(border_color(focused))
}

/// A deterministic, auto-generated color for a username, so names are
/// visually distinguishable in chat and the Users pane. Same name always
/// maps to the same color (FNV-1a over the bytes, into a fixed palette).
pub fn user_style(name: &str) -> Style {
    // FNV-1a, hand-rolled for determinism across runs (RandomState is seeded).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Style::default().fg(USER_PALETTE[(hash % USER_PALETTE.len() as u64) as usize])
}

#[derive(Clone, Copy)]
struct GeneratedSpeakerColor {
    rgb: [u8; 3],
    lab: Lab,
}

static SPEAKER_TRUECOLOR_PALETTE: OnceLock<Mutex<Vec<GeneratedSpeakerColor>>> = OnceLock::new();

/// Progressive, perceptually spaced speaker color for an RGB terminal.
///
/// Every new slot considers a deterministic batch of HSLuv candidates and
/// keeps the one whose nearest existing color is farthest away in CIEDE2000.
/// The progressive cache makes old slots stable and imposes no application
/// cap; quantization happens before comparison so tested distance matches the
/// RGB bytes sent to the terminal.
pub fn speaker_truecolor(slot: usize) -> Style {
    let palette = SPEAKER_TRUECOLOR_PALETTE.get_or_init(|| Mutex::new(Vec::new()));
    let mut palette = palette
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while palette.len() <= slot {
        let next = next_speaker_truecolor(&palette);
        palette.push(next);
    }
    let [red, green, blue] = palette[slot].rgb;
    Style::default().fg(Color::Rgb(red, green, blue))
}

fn next_speaker_truecolor(existing: &[GeneratedSpeakerColor]) -> GeneratedSpeakerColor {
    const CANDIDATES_PER_BATCH: u64 = 64;
    if existing.is_empty() {
        return generated_speaker_color(25.0, 88.0, 76.0);
    }

    let slot = existing.len() as u64;
    let mut batch = 0_u64;
    loop {
        let start = slot
            .wrapping_mul(CANDIDATES_PER_BATCH)
            .wrapping_add(batch.wrapping_mul(CANDIDATES_PER_BATCH));
        let mut best: Option<(GeneratedSpeakerColor, f32)> = None;
        for offset in 0..CANDIDATES_PER_BATCH {
            let first = splitmix64(start.wrapping_add(offset));
            let second = splitmix64(first);
            let third = splitmix64(second);
            let candidate = generated_speaker_color(
                360.0 * unit_interval(first),
                40.0 + 60.0 * unit_interval(second),
                58.0 + 32.0 * unit_interval(third),
            );
            if existing.iter().any(|color| color.rgb == candidate.rgb) {
                continue;
            }
            let nearest = existing
                .iter()
                .map(|color| candidate.lab.difference(color.lab))
                .fold(f32::INFINITY, f32::min);
            if best
                .as_ref()
                .is_none_or(|(_, best_distance)| nearest > *best_distance)
            {
                best = Some((candidate, nearest));
            }
        }
        if let Some((color, _)) = best {
            return color;
        }
        // A practical prefix will always find a free candidate in its first
        // batch. Continuing deterministically avoids turning that assumption
        // into an application-level speaker cap.
        batch = batch.wrapping_add(1);
    }
}

fn generated_speaker_color(hue: f32, saturation: f32, lightness: f32) -> GeneratedSpeakerColor {
    let rgb: Srgb = Srgb::from_color(Hsluv::new(hue, saturation, lightness));
    let rgb: Srgb<u8> = rgb.into_format();
    let normalized: Srgb = rgb.into_format();
    GeneratedSpeakerColor {
        rgb: [rgb.red, rgb.green, rgb.blue],
        lab: Lab::from_color(normalized),
    }
}

fn splitmix64(value: u64) -> u64 {
    let mut value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_interval(value: u64) -> f32 {
    (value >> 40) as f32 / (1_u32 << 24) as f32
}

/// Apply the terminal-depth-specific presentation to a completed frame.
/// Limited terminals retain their configured terminal theme. RGB terminals
/// get one app-wide dark palette, including every pane, modal and overlay.
pub fn apply_color_depth(buffer: &mut Buffer, depth: ColorDepth) {
    if depth == ColorDepth::Limited {
        return;
    }
    for cell in &mut buffer.content {
        if cell.modifier.contains(Modifier::DIM) {
            cell.fg = TRUECOLOR_MUTED_FOREGROUND;
            cell.modifier.remove(Modifier::DIM);
        } else {
            cell.fg = dark_foreground(cell.fg);
        }
        // DessPlay owns the whole alternate-screen canvas in true-color
        // mode. A single explicit background makes contrast deterministic
        // instead of guessing the user's terminal theme.
        cell.bg = TRUECOLOR_BACKGROUND;
    }
}

fn dark_foreground(color: Color) -> Color {
    match color {
        Color::Reset => TRUECOLOR_FOREGROUND,
        // An explicit ANSI Black foreground would disappear on the forced
        // dark canvas. Map it to the subdued readable tone instead.
        Color::Black => TRUECOLOR_MUTED_FOREGROUND,
        Color::Red => Color::Rgb(248, 81, 73),
        Color::Green => Color::Rgb(86, 211, 100),
        Color::Yellow => Color::Rgb(227, 179, 65),
        Color::Blue => Color::Rgb(88, 166, 255),
        Color::Magenta => Color::Rgb(219, 97, 162),
        Color::Cyan => Color::Rgb(86, 212, 221),
        Color::Gray => Color::Rgb(177, 186, 196),
        Color::DarkGray => TRUECOLOR_MUTED_FOREGROUND,
        Color::LightRed => Color::Rgb(255, 123, 114),
        Color::LightGreen => Color::Rgb(126, 231, 135),
        Color::LightYellow => Color::Rgb(242, 204, 96),
        Color::LightBlue => Color::Rgb(121, 192, 255),
        Color::LightMagenta => Color::Rgb(255, 128, 191),
        Color::LightCyan => Color::Rgb(165, 243, 252),
        Color::White => Color::Rgb(240, 246, 252),
        Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
        Color::Indexed(208) => Color::Rgb(255, 166, 87),
        Color::Indexed(141) => Color::Rgb(188, 140, 255),
        Color::Indexed(index) => xterm_rgb(index),
    }
}

fn xterm_rgb(index: u8) -> Color {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let (red, green, blue) = match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let offset = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (
                component(offset / 36),
                component((offset / 6) % 6),
                component(offset % 6),
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    Color::Rgb(red, green, blue)
}

/// Style for a directory row in the file browser (files stay plain, so
/// directories read at a glance).
pub fn directory() -> Style {
    Style::default().fg(Color::Cyan)
}

/// Style for selection highlight within a focused pane.
pub fn highlight_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Dim style for decoration lines (departed/seeders, group headings).
pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rgb(color: Color) -> palette::Srgb<f32> {
        match color {
            Color::Rgb(red, green, blue) => palette::Srgb::new(red, green, blue).into_format(),
            other => {
                assert!(
                    matches!(other, Color::Rgb(..)),
                    "expected RGB, got {other:?}"
                );
                palette::Srgb::new(0.0, 0.0, 0.0)
            }
        }
    }

    fn speaker_foreground(slot: usize) -> Color {
        let foreground = speaker_truecolor(slot).fg;
        assert!(foreground.is_some(), "speaker style needs a foreground");
        foreground.unwrap_or(Color::Reset)
    }

    #[test]
    fn truecolor_detection_accepts_backend_and_direct_terminal_hints() {
        assert_eq!(
            ColorDepth::detect_from(u16::MAX, None, Some("xterm-256color")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::detect_from(256, None, Some("xterm-direct")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::detect_from(256, Some(""), Some("tmux-direct")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::detect_from(256, Some("truecolor"), Some("screen")),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn truecolor_detection_keeps_ordinary_indexed_terminals_limited() {
        assert_eq!(
            ColorDepth::detect_from(256, None, Some("xterm-256color")),
            ColorDepth::Limited
        );
        assert_eq!(
            ColorDepth::detect_from(8, Some("unknown"), Some("dumb")),
            ColorDepth::Limited
        );
    }

    #[test]
    fn truecolor_buffer_mapping_supplies_a_complete_dark_theme() {
        use tuirealm::ratatui::{buffer::Buffer, layout::Rect};

        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer[(0, 0)].fg = Color::Reset;
        buffer[(0, 0)].bg = Color::Reset;
        buffer[(1, 0)].fg = Color::Red;
        buffer[(1, 0)].bg = Color::Blue;
        buffer[(2, 0)].fg = Color::Indexed(208);
        buffer[(2, 0)].bg = Color::Indexed(141);
        buffer[(3, 0)].fg = Color::Rgb(1, 2, 3);
        buffer[(3, 0)].bg = Color::Rgb(4, 5, 6);

        apply_color_depth(&mut buffer, ColorDepth::TrueColor);

        assert_eq!(buffer[(0, 0)].fg, TRUECOLOR_FOREGROUND);
        assert_eq!(buffer[(0, 0)].bg, TRUECOLOR_BACKGROUND);
        assert!(matches!(buffer[(1, 0)].fg, Color::Rgb(..)));
        assert_eq!(buffer[(1, 0)].bg, TRUECOLOR_BACKGROUND);
        assert!(matches!(buffer[(2, 0)].fg, Color::Rgb(..)));
        assert_eq!(buffer[(2, 0)].bg, TRUECOLOR_BACKGROUND);
        assert_eq!(buffer[(3, 0)].fg, Color::Rgb(1, 2, 3));
        assert_eq!(buffer[(3, 0)].bg, TRUECOLOR_BACKGROUND);
    }

    proptest! {
        /// SGR DIM is the compatibility problem; materializing it must not
        /// erase independent presentation such as the selected-row reverse
        /// or the known-offline italic style.
        #[test]
        fn truecolor_materializes_dim_without_disturbing_other_modifiers(
            bold in any::<bool>(),
            italic in any::<bool>(),
            reversed in any::<bool>(),
            underlined in any::<bool>(),
        ) {
            use tuirealm::ratatui::{buffer::Buffer, layout::Rect};

            let mut retained = Modifier::empty();
            if bold {
                retained.insert(Modifier::BOLD);
            }
            if italic {
                retained.insert(Modifier::ITALIC);
            }
            if reversed {
                retained.insert(Modifier::REVERSED);
            }
            if underlined {
                retained.insert(Modifier::UNDERLINED);
            }

            let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
            buffer[(0, 0)].modifier = retained | Modifier::DIM;

            apply_color_depth(&mut buffer, ColorDepth::TrueColor);

            prop_assert_eq!(buffer[(0, 0)].fg, TRUECOLOR_MUTED_FOREGROUND);
            prop_assert_eq!(buffer[(0, 0)].modifier, retained);
        }
    }

    #[test]
    fn limited_buffer_mapping_is_a_noop() {
        use tuirealm::ratatui::{buffer::Buffer, layout::Rect};

        let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
        buffer[(0, 0)].fg = Color::Indexed(208);
        buffer[(0, 0)].bg = Color::Reset;
        buffer[(0, 0)].modifier = Modifier::DIM | Modifier::ITALIC;
        let original = buffer.clone();

        apply_color_depth(&mut buffer, ColorDepth::Limited);

        assert_eq!(buffer, original);
    }

    #[test]
    fn truecolor_speaker_palette_has_no_practical_prefix_cap() {
        let colors: std::collections::HashSet<_> = (0..256).map(speaker_foreground).collect();

        assert_eq!(colors.len(), 256);
        assert!(colors.iter().all(|color| matches!(color, Color::Rgb(..))));
    }

    #[test]
    fn truecolor_speakers_meet_wcag_text_contrast() {
        use palette::color_difference::Wcag21RelativeContrast;

        let background = rgb(TRUECOLOR_BACKGROUND);
        for slot in 0..256 {
            let foreground = rgb(speaker_foreground(slot));
            let contrast = foreground.relative_contrast(background);
            assert!(
                contrast >= 4.5,
                "speaker slot {slot} has only {contrast:.2}:1 contrast"
            );
        }
    }

    #[test]
    fn dark_theme_semantic_text_colors_meet_wcag_contrast() {
        use palette::color_difference::Wcag21RelativeContrast;

        let background = rgb(TRUECOLOR_BACKGROUND);
        let semantic = [
            Color::Reset,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::DarkGray,
            Color::Cyan,
            Color::Magenta,
            Color::LightCyan,
            Color::LightMagenta,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightRed,
            Color::Gray,
            Color::White,
            Color::Indexed(208),
            Color::Indexed(141),
        ];
        for source in semantic {
            let foreground = rgb(dark_foreground(source));
            let contrast = foreground.relative_contrast(background);
            assert!(
                contrast >= 4.5,
                "dark-theme {source:?} has only {contrast:.2}:1 contrast"
            );
        }
        let muted_contrast = rgb(TRUECOLOR_MUTED_FOREGROUND).relative_contrast(background);
        assert!(
            muted_contrast >= 4.5,
            "muted true-color text has only {muted_contrast:.2}:1 contrast"
        );
    }

    #[test]
    fn practical_truecolor_prefixes_remain_perceptually_separate() {
        use palette::{FromColor, Lab, color_difference::Ciede2000};

        for (count, required_distance) in [(32, 10.5), (128, 5.5), (256, 4.25)] {
            let colors: Vec<Lab> = (0..count)
                .map(|slot| Lab::from_color(rgb(speaker_foreground(slot))))
                .collect();
            let minimum = colors
                .iter()
                .enumerate()
                .flat_map(|(left_index, left)| {
                    colors[left_index + 1..]
                        .iter()
                        .map(move |right| left.difference(*right))
                })
                .fold(f32::INFINITY, f32::min);
            assert!(
                minimum >= required_distance,
                "closest pair in prefix {count} has CIEDE2000 distance {minimum:.2}"
            );
        }
    }

    #[test]
    fn user_style_is_deterministic() {
        assert_eq!(user_style("Baughn").fg, user_style("Baughn").fg);
        assert_eq!(user_style("Nero").fg, user_style("Nero").fg);
    }

    #[test]
    fn user_style_spreads_across_palette() {
        // A handful of distinct names should land on more than one color
        // (a constant function would fail this).
        let names = ["Baughn", "Nero", "Quickshot", "Dagger", "Kim", "nas"];
        let colors: std::collections::HashSet<_> = names.iter().map(|n| user_style(n).fg).collect();
        assert!(colors.len() > 1, "all names mapped to the same color");
    }
}
