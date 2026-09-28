//! Osaka's sprites: pure ASCII, 5 wide × 4 tall, defined facing right
//! and mirrored for left. The anchor is bottom-centre: she stands *on*
//! the floor row, so the sprite occupies the four rows above it.

/// Sprite width in cells (the anchor is the middle column).
pub(super) const WIDTH: i32 = 5;
/// Sprite height in cells, all above the floor row.
pub(super) const HEIGHT: i32 = 4;

/// Which way she faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Facing {
    Left,
    Right,
}

/// A body pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Pose {
    Stand,
    /// Walk cycle frame 0–3.
    Walk(u8),
    /// Climb frame 0–1 (hands alternate), with the pole `pole` cells to
    /// the side she faces (0 = in front of her).
    Climb {
        frame: u8,
        pole: i8,
    },
    Fall,
    Dazed,
    /// Peering over the edge she faces.
    Peer,
    /// Pulling a line on her box row `row` (0 = top) on the side she
    /// faces: bracing, or heaving back.
    Pull {
        heaving: bool,
        row: u8,
    },
    /// Sitting on the floor hugging her knees.
    Sit,
    /// Lying on her back, knees up, hands behind her head (frame 0–1
    /// swings a foot).
    LieBack(u8),
    /// Lying on her stomach, chin propped, feet kicking (frame 0–1).
    LieFront(u8),
    /// Jumping jacks (frame 0: in, 1: out).
    Jack(u8),
    /// Touching her toes (frame 0: up, 1: down).
    ToeTouch(u8),
    /// A big stretch, arms up.
    Stretch,
    /// Gazing up at something.
    Gaze,
    /// Standing side-on, looking the way she faces.
    Side,
}

/// Her face, drawn into the head of frontal poses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Face {
    Vacant,
    Blink,
    Surprised,
    Pleased,
    /// Big eyes, open smile.
    Happy,
    /// Looking up, a small "o".
    Curious,
}

impl Face {
    pub fn glyphs(self) -> [char; 3] {
        match self {
            Self::Vacant => ['.', '_', '.'],
            Self::Blink => ['-', '_', '-'],
            Self::Surprised => ['o', '_', 'o'],
            Self::Pleased => ['^', '_', '^'],
            Self::Happy => ['^', 'o', '^'],
            Self::Curious => ['\'', 'o', '\''],
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
const SIT: [&str; 4] = ["     ", "(._.)", "<(V)>", " d b "];
const LIE_BACK: [[&str; 4]; 2] = [
    ["     ", "     ", "   /\\", "o=V=^"],
    ["     ", "     ", "  /\\ ", "o=V=^"],
];
const LIE_FRONT: [[&str; 4]; 2] = [
    ["     ", "     ", "    /", "o_V_/"],
    ["     ", "     ", "   | ", "o_V_|"],
];
const JACK: [[&str; 4]; 2] = [
    ["(._.)", "/|V|\\", " /_\\ ", " | | "],
    ["(._.)", "\\|V|/", " /_\\ ", "/   \\"],
];
const TOE_TOUCH: [[&str; 4]; 2] = [
    ["( ._)", " |V| ", " /_\\ ", " / \\ "],
    ["     ", "  __o", " /V|\\", " / \\ "],
];
const STRETCH: [&str; 4] = ["(._.)", "\\|V|/", " /_\\ ", " / \\ "];
const GAZE: [&str; 4] = ["( 'o)", " |V| ", " /_\\ ", " / \\ "];
const SIDE: [&str; 4] = ["( ._)", " |V| ", " /_\\ ", " / \\ "];
const PULL: [[&str; 4]; 2] = [
    ["( ._)", "\\|V|=", " /_\\ ", " / \\ "],
    ["(._ )", "\\|V|=", " /_\\ ", "/  \\ "],
];

fn rows(pose: Pose) -> ([&'static str; 4], bool) {
    match pose {
        Pose::Stand => (STAND, true),
        Pose::Walk(frame) => (WALK[usize::from(frame % 4)], false),
        Pose::Climb { frame, .. } => (CLIMB[usize::from(frame % 2)], true),
        Pose::Fall => (FALL, false),
        Pose::Dazed => (DAZED, false),
        Pose::Peer => (PEER, false),
        Pose::Pull { heaving, .. } => (PULL[usize::from(heaving)], false),
        Pose::Sit => (SIT, true),
        Pose::LieBack(frame) => (LIE_BACK[usize::from(frame % 2)], false),
        Pose::LieFront(frame) => (LIE_FRONT[usize::from(frame % 2)], false),
        Pose::Jack(frame) => (JACK[usize::from(frame % 2)], true),
        Pose::ToeTouch(frame) => (TOE_TOUCH[usize::from(frame % 2)], false),
        Pose::Stretch => (STRETCH, true),
        Pose::Gaze => (GAZE, false),
        Pose::Side => (SIDE, false),
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

/// The visible cells of `pose`. Body spaces are transparent (the UI
/// shows between her legs), but her head is solid: its three middle
/// cells always exist, so the goodbye beat can draw a frontal face over
/// a profile. Frontal poses take `face`; profile poses keep their own.
pub(super) fn cells(pose: Pose, facing: Facing, face: Face) -> Vec<SpriteCell> {
    let (rows, frontal) = rows(pose);
    // Her head is the row with her parentheses (lower when she sits).
    let head = rows.iter().position(|row| row.contains('('));
    let mut out = Vec::with_capacity(16);
    for (row, text) in rows.iter().enumerate() {
        let dy = row as i32 - HEIGHT;
        let mut glyphs: Vec<char> = text.chars().collect();
        let is_head = head == Some(row);
        if is_head
            && frontal
            && let Some(slot) = glyphs.get_mut(1..4)
        {
            slot.copy_from_slice(&face.glyphs());
        }
        // Spaces are transparent, except inside her head.
        let inside: Vec<bool> = {
            let open = glyphs.iter().position(|&c| c == '(');
            let close = glyphs.iter().rposition(|&c| c == ')');
            (0..glyphs.len())
                .map(|i| is_head && open.is_some_and(|o| i > o) && close.is_some_and(|c| i < c))
                .collect()
        };
        let mut glyphs: Vec<(char, bool)> = glyphs.into_iter().zip(inside).collect();
        if facing == Facing::Left {
            glyphs.reverse();
            for (glyph, _) in &mut glyphs {
                *glyph = mirror(*glyph);
            }
        }
        for (col, (glyph, solid)) in glyphs.into_iter().enumerate() {
            if glyph == ' ' && !solid {
                continue;
            }
            let part = match glyph {
                _ if is_head => Part::Head,
                'V' => Part::Ribbon,
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

    /// Every head has all three face cells, so the dissolve's startled
    /// and smiling faces are whole even over a profile (a real-terminal
    /// run showed `( _o)` before heads were solid).
    #[test]
    fn every_head_has_three_face_cells() {
        for pose in [Pose::Walk(0), Pose::Peer, Pose::Stand] {
            for facing in [Facing::Left, Facing::Right] {
                let face: Vec<i32> = cells(pose, facing, Face::Vacant)
                    .iter()
                    .filter(|c| c.part == Part::Head && (-1..=1).contains(&c.dx))
                    .map(|c| c.dx)
                    .collect();
                assert_eq!(face, [-1, 0, 1], "{pose:?} {facing:?}");
            }
        }
    }

    #[test]
    fn every_pose_fits_the_box_and_is_ascii() {
        let poses = [
            Pose::Stand,
            Pose::Walk(0),
            Pose::Walk(1),
            Pose::Walk(2),
            Pose::Walk(3),
            Pose::Climb { frame: 0, pole: 2 },
            Pose::Climb { frame: 1, pole: 0 },
            Pose::Fall,
            Pose::Dazed,
            Pose::Peer,
            Pose::Pull {
                heaving: false,
                row: 1,
            },
            Pose::Pull {
                heaving: true,
                row: 2,
            },
            Pose::Sit,
            Pose::LieBack(0),
            Pose::LieBack(1),
            Pose::LieFront(0),
            Pose::LieFront(1),
            Pose::Jack(0),
            Pose::Jack(1),
            Pose::ToeTouch(0),
            Pose::ToeTouch(1),
            Pose::Stretch,
            Pose::Gaze,
            Pose::Side,
        ];
        for pose in poses {
            for facing in [Facing::Left, Facing::Right] {
                for cell in cells(pose, facing, Face::Blink) {
                    assert!((-2..=2).contains(&cell.dx), "{pose:?}");
                    assert!((-4..=-1).contains(&cell.dy), "{pose:?}");
                    let head_gap = cell.part == Part::Head && cell.glyph == ' ';
                    assert!(cell.glyph.is_ascii_graphic() || head_gap, "{pose:?}");
                }
            }
        }
    }
}
