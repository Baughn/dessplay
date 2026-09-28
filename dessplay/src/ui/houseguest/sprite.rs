//! Osaka's sprites: pure ASCII, 5 wide × 4 tall, defined facing right
//! and mirrored for left. The anchor is bottom-centre: she stands *on*
//! the floor row, so the sprite occupies the four rows above it.

/// Sprite width in cells (the anchor is the middle column).
pub(super) const WIDTH: i32 = 5;
/// Sprite height in cells, all above the floor row.
pub(super) const HEIGHT: i32 = 4;

/// Which way she faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Facing {
    Left,
    Right,
}

/// A body pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pose {
    Stand,
    /// Walk cycle frame 0–3.
    Walk(u8),
    /// Climb frame 0–1 (hands alternate).
    Climb(u8),
    Fall,
    Dazed,
    /// Peering over the edge she faces.
    Peer,
}

/// Her face, drawn into the head of frontal poses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Face {
    Vacant,
    Blink,
    Surprised,
    Pleased,
}

impl Face {
    pub fn glyphs(self) -> [char; 3] {
        match self {
            Self::Vacant => ['.', '_', '.'],
            Self::Blink => ['-', '_', '-'],
            Self::Surprised => ['o', '_', 'o'],
            Self::Pleased => ['^', '_', '^'],
        }
    }
}

const STAND: [&str; 4] = ["(._.)", "/|V|\\", " /_\\ ", " / \\ "];
const WALK: [[&str; 4]; 4] = [
    ["( ._)", "/|V|>", " /_\\ ", " / \\ "],
    ["( ._)", "||V||", " /_\\ ", "  |  "],
    ["( ._)", "<|V|\\", " /_\\ ", " / \\ "],
    ["( ._)", "||V||", " /_\\ ", "  |  "],
];
const CLIMB: [[&str; 4]; 2] = [
    ["(._.)", "\\|V| ", " /_\\ ", " / \\ "],
    ["(._.)", " |V|/", " /_\\ ", " / \\ "],
];
const FALL: [&str; 4] = ["(o_o)", "\\|V|/", " /_\\ ", " / \\ "];
const DAZED: [&str; 4] = ["(@_@)", "/|V|\\", " /_\\ ", " / \\ "];
const PEER: [&str; 4] = ["( o.)", "/|V|>", " /_\\ ", " / \\ "];

fn rows(pose: Pose) -> ([&'static str; 4], bool) {
    match pose {
        Pose::Stand => (STAND, true),
        Pose::Walk(frame) => (WALK[usize::from(frame % 4)], false),
        Pose::Climb(frame) => (CLIMB[usize::from(frame % 2)], true),
        Pose::Fall => (FALL, false),
        Pose::Dazed => (DAZED, false),
        Pose::Peer => (PEER, false),
    }
}

fn mirror(c: char) -> char {
    match c {
        '/' => '\\',
        '\\' => '/',
        '<' => '>',
        '>' => '<',
        '(' => ')',
        ')' => '(',
        other => other,
    }
}

/// What a sprite cell is, for styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Head,
    Ribbon,
    Body,
}

/// One painted sprite cell, relative to the anchor: `dx` in −2..=2,
/// `dy` in −4..=−1 (rows above the floor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SpriteCell {
    pub dx: i32,
    pub dy: i32,
    pub glyph: char,
    pub part: Part,
}

/// The visible cells of `pose` (spaces are transparent). Frontal poses
/// take `face`; profile poses keep their drawn face.
pub(super) fn cells(pose: Pose, facing: Facing, face: Face) -> Vec<SpriteCell> {
    let (rows, frontal) = rows(pose);
    let mut out = Vec::with_capacity(16);
    for (row, text) in rows.iter().enumerate() {
        let dy = row as i32 - HEIGHT;
        let mut glyphs: Vec<char> = text.chars().collect();
        if row == 0 && frontal {
            let [a, b, c] = face.glyphs();
            if let Some(slot) = glyphs.get_mut(1..4) {
                slot.copy_from_slice(&[a, b, c]);
            }
        }
        if facing == Facing::Left {
            glyphs.reverse();
            for glyph in &mut glyphs {
                *glyph = mirror(*glyph);
            }
        }
        for (col, glyph) in glyphs.into_iter().enumerate() {
            if glyph == ' ' {
                continue;
            }
            let part = match (row, glyph) {
                (0, _) => Part::Head,
                (1, 'V') => Part::Ribbon,
                _ => Part::Body,
            };
            out.push(SpriteCell {
                dx: col as i32 - WIDTH / 2,
                dy,
                glyph,
                part,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(pose: Pose, facing: Facing, face: Face) -> Vec<String> {
        let mut grid = vec![vec![' '; WIDTH as usize]; HEIGHT as usize];
        for cell in cells(pose, facing, face) {
            grid[(cell.dy + HEIGHT) as usize][(cell.dx + WIDTH / 2) as usize] = cell.glyph;
        }
        grid.into_iter()
            .map(|row| row.into_iter().collect())
            .collect()
    }

    #[test]
    fn frontal_faces_and_mirroring() {
        assert_eq!(
            picture(Pose::Stand, Facing::Right, Face::Surprised),
            ["(o_o)", "/|V|\\", " /_\\ ", " / \\ "]
        );
        assert_eq!(
            picture(Pose::Walk(0), Facing::Left, Face::Vacant),
            ["(_. )", "<|V|\\", " /_\\ ", " / \\ "]
        );
    }

    #[test]
    fn every_pose_fits_the_box_and_is_ascii() {
        let poses = [
            Pose::Stand,
            Pose::Walk(0),
            Pose::Walk(1),
            Pose::Walk(2),
            Pose::Walk(3),
            Pose::Climb(0),
            Pose::Climb(1),
            Pose::Fall,
            Pose::Dazed,
            Pose::Peer,
        ];
        for pose in poses {
            for facing in [Facing::Left, Facing::Right] {
                for cell in cells(pose, facing, Face::Blink) {
                    assert!((-2..=2).contains(&cell.dx), "{pose:?}");
                    assert!((-4..=-1).contains(&cell.dy), "{pose:?}");
                    assert!(cell.glyph.is_ascii_graphic(), "{pose:?}");
                }
            }
        }
    }
}
