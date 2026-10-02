//! The rules of her home: how her pieces ought to stand, as a table.
//!
//! A rule is broken or not, judged on where her pieces are laid out
//! ([`super::room::Home::layout`]), never on what text closets this
//! frame; a satisfied rule never moves anything. A rule applies only to
//! pieces laid out and out of their boxes (a parcel isn't furniture
//! yet). Each has a grievance: what she says while using a piece it
//! involves, while it's broken.

use super::room::{self, Extent, Furniture, Home, Role, Shown, Strip, Use};

/// How some of her pieces ought to stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rule {
    /// She can watch `screen` from `seat` (see [`room::faces`]).
    Faces { seat: Furniture, screen: Furniture },
    /// `a` stands on one strip with one of `b`, at most `gap` cells
    /// between them.
    Near {
        a: Furniture,
        b: &'static [Furniture],
        gap: i32,
    },
    /// The piece stands within a cell of one of its strip's walls.
    AgainstWall(Furniture),
    /// `a` and `b` aren't in one room (a room is a strip, for now).
    Apart { a: Furniture, b: Furniture },
    /// A piece she hasn't settled doesn't spoil its room: without it the
    /// room is something (not a den), and it's something else with it
    /// (a bed in a living room; a TV or a fridge in a bedroom).
    Belongs,
}

/// A rule, the uses of a piece it involves that she feels it on, and
/// what she says then.
#[derive(Debug)]
pub(super) struct RuleRow {
    pub rule: Rule,
    // Read once she feels rules (phase 4, step 4).
    #[allow(dead_code)]
    pub felt_on: &'static [Use],
    // Said once she feels rules (phase 4, step 4).
    #[allow(dead_code)]
    pub grievance: &'static str,
}

/// Every use she makes of a piece, once it's out of its box and in
/// shape: [`Rule::Belongs`] is felt on any.
pub(super) const ANY_USE: &[Use] = &[
    Use::Lounge,
    Use::Nap,
    Use::Sleep,
    Use::Homework,
    Use::Watch,
    Use::Read,
    Use::Snack,
    Use::Pet,
];

/// The rules, in the order they're judged.
pub(super) const RULES: [RuleRow; 6] = [
    RuleRow {
        rule: Rule::Faces {
            seat: Furniture::Sofa,
            screen: Furniture::Tv,
        },
        felt_on: &[Use::Lounge, Use::Nap],
        grievance: "Can't see the telly...",
    },
    RuleRow {
        rule: Rule::Near {
            a: Furniture::Lamp,
            b: &[Furniture::Bed, Furniture::Desk],
            gap: 3,
        },
        felt_on: &[Use::Sleep, Use::Homework],
        grievance: "Too dark in here...",
    },
    RuleRow {
        rule: Rule::AgainstWall(Furniture::Fridge),
        felt_on: &[Use::Snack],
        grievance: "This wants a wall...",
    },
    RuleRow {
        rule: Rule::AgainstWall(Furniture::Bookshelf),
        felt_on: &[Use::Read],
        grievance: "Wobbly... needs a wall.",
    },
    RuleRow {
        rule: Rule::Apart {
            a: Furniture::Bed,
            b: Furniture::Tv,
        },
        felt_on: &[Use::Sleep],
        grievance: "Too noisy to sleep...",
    },
    RuleRow {
        rule: Rule::Belongs,
        felt_on: ANY_USE,
        grievance: "Hm... not in here.",
    },
];

/// How far from a wall a piece against it may stand, in cells.
const WALL_GAP: i32 = 1;

/// A rule broken this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Broken {
    /// Its row of [`RULES`].
    pub row: usize,
    /// The pieces moving would mend it, in the rule's order (for
    /// [`Rule::Apart`], one she hasn't settled first).
    pub pieces: Vec<Furniture>,
    /// Every piece it involves (using one, she may feel it).
    // Read once she feels rules (phase 4, step 4).
    #[allow(dead_code)]
    pub involved: Vec<Furniture>,
}

impl Broken {
    /// Its row of the table.
    pub fn rule(&self) -> Option<&'static RuleRow> {
        RULES.get(self.row)
    }

    /// A few words for the stage: the rule, and the pieces it would move.
    pub fn label(&self) -> String {
        let kind = match self.rule().map(|r| r.rule) {
            Some(Rule::Faces { .. }) => "faces",
            Some(Rule::Near { .. }) => "near",
            Some(Rule::AgainstWall(_)) => "wall",
            Some(Rule::Apart { .. }) => "apart",
            Some(Rule::Belongs) => "belongs",
            None => "?",
        };
        let pieces: Vec<&str> = self.pieces.iter().map(|p| p.spec().name).collect();
        format!("{kind}({})", pieces.join(","))
    }
}

/// Every rule broken by her pieces as `layout` has them on `strips`,
/// with what `home` knows of them (whether she has settled each), in
/// [`RULES`] order (and [`Rule::Belongs`] piece by piece in layout
/// order).
pub(super) fn broken(layout: &[Shown], strips: &[(Strip, Extent)], home: &Home) -> Vec<Broken> {
    let here = |item: Furniture| {
        layout
            .iter()
            .find(|s| s.item == item && s.scrap.is_none() && !s.boxed && s.strip.is_some())
    };
    let settled = |item: Furniture| {
        home.props
            .iter()
            .find(|p| p.item == item)
            .is_none_or(|p| p.settled)
    };
    let mut out: Vec<Broken> = Vec::new();
    for (row, line) in RULES.iter().enumerate() {
        let mut push = |pieces: Vec<Furniture>, involved: Vec<Furniture>| {
            out.push(Broken {
                row,
                pieces,
                involved,
            });
        };
        match line.rule {
            Rule::Faces { seat, screen } => {
                if let (Some(a), Some(b)) = (here(seat), here(screen))
                    && !room::faces(a, b)
                {
                    push(vec![seat, screen], vec![seat, screen]);
                }
            }
            Rule::Near { a, b, gap } => {
                let Some(at) = here(a) else { continue };
                let partners: Vec<&Shown> = b.iter().filter_map(|&item| here(item)).collect();
                if !partners.is_empty()
                    && !partners
                        .iter()
                        .any(|p| p.strip == at.strip && between(at, p) <= gap)
                {
                    let mut involved = vec![a];
                    involved.extend(partners.iter().map(|p| p.item));
                    push(vec![a], involved);
                }
            }
            Rule::AgainstWall(item) => {
                if let Some(at) = here(item)
                    && let Some(&(_, e)) = strips.iter().find(|(s, _)| Some(*s) == at.strip)
                    && !against_wall(at, e)
                {
                    push(vec![item], vec![item]);
                }
            }
            Rule::Apart { a, b } => {
                if let (Some(x), Some(y)) = (here(a), here(b))
                    && x.strip == y.strip
                {
                    let mut pieces = vec![a, b];
                    // Unsettled first; otherwise in the rule's order.
                    pieces.sort_by_key(|&p| settled(p));
                    push(pieces, vec![a, b]);
                }
            }
            Rule::Belongs => {
                for s in layout {
                    let Some(strip) = s.strip else { continue };
                    if s.scrap.is_some() || s.boxed || settled(s.item) {
                        continue;
                    }
                    let without = room::role_without(layout, strip, s.item);
                    if without != Role::Den && without != room::role_of(layout, strip) {
                        push(vec![s.item], vec![s.item]);
                    }
                }
            }
        }
    }
    out
}

/// The cells between two pieces along a floor (0 when they touch).
fn between(a: &Shown, b: &Shown) -> i32 {
    let (ra, rb) = (a.rect(), b.rect());
    (i32::from(rb.x) - i32::from(ra.right())).max(i32::from(ra.x) - i32::from(rb.right()))
}

/// Whether `at` stands within [`WALL_GAP`] of one of `e`'s walls.
fn against_wall(at: &Shown, e: Extent) -> bool {
    let (cols, _) = at.size();
    at.left - e.from <= WALL_GAP || e.to - (at.left + i32::from(cols)) <= WALL_GAP
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::room::{Anchor, Nook, Prop, Side, strips};
    use super::super::sprite::Facing;
    use super::*;
    use Furniture::*;
    use tuirealm::ratatui::buffer::Buffer;
    use tuirealm::ratatui::layout::Rect;

    /// A piece anchored `offset` cells from `side`'s wall of `nook`.
    fn at(item: Furniture, nook: Nook, side: Side, offset: u16, facing: Facing) -> Prop {
        Prop {
            anchor: Some(Anchor { side, offset }),
            ..Prop::new(item, nook, 0, facing)
        }
    }

    fn unsettled(prop: Prop) -> Prop {
        Prop {
            settled: false,
            ..prop
        }
    }

    fn boxed(prop: Prop) -> Prop {
        Prop {
            boxed: true,
            ..prop
        }
    }

    /// The Users pane above the Playlist pane, each `width` wide and
    /// eight tall.
    fn panes(width: u16) -> [(Nook, Rect); 2] {
        [
            (Nook::Users, Rect::new(0, 0, width, 8)),
            (Nook::Playlist, Rect::new(0, 8, width, 8)),
        ]
    }

    /// The rules `props` break laid out on [`panes`] `width` wide, as
    /// `(rule's row, the pieces it would move)`.
    fn judge(width: u16, props: &[Prop]) -> Vec<(usize, Vec<Furniture>)> {
        let mut home = Home::default();
        for &p in props {
            assert!(home.add(p));
        }
        let nooks = panes(width);
        let laid = home.layout(&nooks);
        broken(&laid, &strips(&nooks), &home)
            .into_iter()
            .map(|b| (b.row, b.pieces))
            .collect()
    }

    fn row(rule: Rule) -> usize {
        RULES.iter().position(|r| r.rule == rule).unwrap()
    }

    const FACES: Rule = Rule::Faces {
        seat: Sofa,
        screen: Tv,
    };
    const APART: Rule = Rule::Apart { a: Bed, b: Tv };

    #[test]
    fn a_sofa_turned_toward_the_tv_faces_it() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        // The TV against the right wall of a 40-wide pane (columns
        // 33..39); the sofa 4 cells left of it.
        let sofa = |facing| at(Sofa, Nook::Users, Side::Right, 6 + 4, facing);
        assert_eq!(judge(40, &[tv, sofa(Facing::Right)]), []);
        assert_eq!(
            judge(40, &[tv, sofa(Facing::Left)]),
            [(row(FACES), vec![Sofa, Tv])],
            "turned away"
        );
        // Too close, too far, on another strip.
        let near = at(Sofa, Nook::Users, Side::Right, 6 + 1, Facing::Right);
        let far = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        let other = at(Sofa, Nook::Playlist, Side::Right, 10, Facing::Right);
        for sofa in [near, far, other] {
            assert_eq!(
                judge(40, &[tv, sofa]),
                [(row(FACES), vec![Sofa, Tv])],
                "{sofa:?}"
            );
        }
        // A rule needs every piece it names, out of its box.
        assert_eq!(judge(40, &[sofa(Facing::Left)]), []);
        assert_eq!(judge(40, &[boxed(tv), sofa(Facing::Left)]), []);
        // Nor does a strip too small to lay them out judge them.
        assert_eq!(judge(14, &[tv, sofa(Facing::Left)]), []);
    }

    #[test]
    fn the_lamp_stands_by_the_bed_or_the_desk() {
        let near = row(Rule::Near {
            a: Lamp,
            b: &[Bed, Desk],
            gap: 3,
        });
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let lamp = |offset| at(Lamp, Nook::Users, Side::Left, offset, Facing::Right);
        // The bed's columns 1..11; the lamp 0 and 3 cells past it.
        assert_eq!(judge(40, &[bed, lamp(10)]), []);
        assert_eq!(judge(40, &[bed, lamp(13)]), []);
        assert_eq!(judge(40, &[bed, lamp(14)]), [(near, vec![Lamp])]);
        // A desk will do as well, on another strip; a lamp alone is no
        // matter.
        let desk = at(Desk, Nook::Playlist, Side::Right, 0, Facing::Left);
        let by_desk = at(Lamp, Nook::Playlist, Side::Right, 9, Facing::Right);
        assert_eq!(judge(40, &[bed, desk, by_desk]), []);
        assert_eq!(judge(40, &[lamp(14)]), []);
        // Each partner it could stand by is involved.
        let mut home = Home::default();
        for p in [bed, desk, lamp(30)] {
            assert!(home.add(p));
        }
        let nooks = panes(60);
        let laid = home.layout(&nooks);
        let got = broken(&laid, &strips(&nooks), &home);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].involved, [Lamp, Bed, Desk]);
    }

    #[test]
    fn the_fridge_and_the_bookshelf_want_a_wall() {
        for item in [Fridge, Bookshelf] {
            let rule = row(Rule::AgainstWall(item));
            for (side, offset, ok) in [
                (Side::Left, 0, true),
                (Side::Left, 1, true),
                (Side::Left, 2, false),
                (Side::Right, 1, true),
                (Side::Right, 5, false),
            ] {
                let want = if ok { vec![] } else { vec![(rule, vec![item])] };
                assert_eq!(
                    judge(40, &[at(item, Nook::Users, side, offset, Facing::Right)]),
                    want,
                    "{item:?} {side:?} {offset}"
                );
            }
        }
        // Pushed off its wall by a piece packed against it.
        // (Of two the same distance from a wall, the newer is nearer it.)
        let fridge = at(Fridge, Nook::Users, Side::Left, 0, Facing::Right);
        let sofa = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        assert_eq!(
            judge(40, &[fridge, sofa]),
            [(row(Rule::AgainstWall(Fridge)), vec![Fridge])]
        );
    }

    #[test]
    fn the_bed_and_the_tv_are_apart() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let elsewhere = at(Tv, Nook::Playlist, Side::Right, 0, Facing::Left);
        assert_eq!(judge(40, &[bed, tv]), [(row(APART), vec![Bed, Tv])]);
        assert_eq!(judge(40, &[bed, elsewhere]), []);
        // The one she hasn't settled first.
        assert_eq!(
            judge(40, &[bed, unsettled(tv)]),
            [(row(APART), vec![Tv, Bed]), (row(Rule::Belongs), vec![Tv]),]
        );
        assert_eq!(judge(40, &[boxed(bed), tv]), []);
    }

    #[test]
    fn a_piece_she_has_not_settled_belongs_where_it_does_not_spoil_the_room() {
        let belongs = row(Rule::Belongs);
        let sofa = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = at(Tv, Nook::Users, Side::Left, 14, Facing::Left);
        let bed = at(Bed, Nook::Users, Side::Right, 0, Facing::Left);
        let in_bedroom = |item| at(item, Nook::Users, Side::Left, 0, Facing::Right);
        // A bed in the living room: settled, it's only too noisy.
        assert_eq!(judge(60, &[sofa, tv, bed]), [(row(APART), vec![Bed, Tv])]);
        assert_eq!(
            judge(60, &[sofa, tv, unsettled(bed)]),
            [(row(APART), vec![Bed, Tv]), (belongs, vec![Bed])]
        );
        // A fridge in a bedroom spoils it; a desk doesn't.
        assert_eq!(
            judge(60, &[bed, unsettled(in_bedroom(Fridge))]),
            [(belongs, vec![Fridge])]
        );
        assert_eq!(judge(60, &[bed, unsettled(in_bedroom(Desk))]), []);
        // A TV that makes a living room of a sofa's room completes it.
        assert_eq!(judge(60, &[sofa, unsettled(tv)]), []);
        // Alone on its strip, or still boxed, it spoils nothing.
        assert_eq!(judge(60, &[unsettled(bed)]), []);
        assert_eq!(judge(60, &[sofa, tv, boxed(unsettled(bed))]), []);
        // Only her pieces on the same strip make the room.
        let away = at(Bed, Nook::Playlist, Side::Left, 0, Facing::Right);
        assert_eq!(judge(60, &[sofa, tv, unsettled(away)]), []);
    }

    /// Belongs is felt on every use of a piece once it's out of its box.
    #[test]
    fn belongs_is_felt_on_every_use() {
        let all = [
            Use::Lounge,
            Use::Nap,
            Use::Sleep,
            Use::Homework,
            Use::Watch,
            Use::Unpack,
            Use::Read,
            Use::Snack,
            Use::Pet,
            Use::Crumple,
        ];
        for what in all {
            // Exhaustive: a new use is placed here, then in ANY_USE.
            let real = match what {
                Use::Lounge
                | Use::Nap
                | Use::Sleep
                | Use::Homework
                | Use::Watch
                | Use::Read
                | Use::Snack
                | Use::Pet => true,
                Use::Unpack | Use::Crumple => false,
            };
            assert_eq!(ANY_USE.contains(&what), real, "{what:?}");
        }
        for item in Furniture::ALL {
            for what in item.spec().uses {
                assert!(ANY_USE.contains(what), "{item:?} {what:?}");
            }
        }
    }

    use proptest::prelude::*;

    /// Some of her pieces, each anchored on one of the two strips, either
    /// way round, settled or not, boxed or not.
    fn pieces() -> impl Strategy<Value = Vec<Prop>> {
        proptest::sample::subsequence(Furniture::ALL.to_vec(), 1..=6).prop_flat_map(|items| {
            let n = items.len();
            (
                Just(items),
                proptest::collection::vec(
                    (
                        any::<bool>(),
                        any::<bool>(),
                        0u16..30,
                        any::<bool>(),
                        any::<bool>(),
                        proptest::bool::weighted(0.15),
                    ),
                    n,
                ),
            )
                .prop_map(|(items, places)| {
                    items
                        .into_iter()
                        .zip(places)
                        .map(|(item, (lower, right, offset, left, settled, boxed))| {
                            let nook = if lower { Nook::Playlist } else { Nook::Users };
                            let side = if right { Side::Right } else { Side::Left };
                            let facing = if left { Facing::Left } else { Facing::Right };
                            Prop {
                                settled,
                                boxed,
                                ..at(item, nook, side, offset, facing)
                            }
                        })
                        .collect()
                })
        })
    }

    /// [`panes`] `width` wide, with text cells at `text` along the row
    /// above each floor.
    fn frame(width: u16, text: &[u16]) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 16));
        for (_, rect) in panes(width) {
            let w = usize::from(width);
            let rows: Vec<String> = std::iter::once(format!("┌{}┐", "─".repeat(w - 2)))
                .chain((0..6).map(|_| format!("│{}│", " ".repeat(w - 2))))
                .chain(std::iter::once(format!("└{}┘", "─".repeat(w - 2))))
                .collect();
            for (dy, line) in rows.iter().enumerate() {
                buf.set_string(
                    rect.x,
                    rect.y + dy as u16,
                    line,
                    tuirealm::ratatui::style::Style::default(),
                );
            }
            for &x in text {
                if x > 0 && x + 1 < width {
                    buf[(x, rect.bottom() - 2)].set_symbol("x");
                }
            }
        }
        buf
    }

    fn judged(home: &mut Home, nooks: &[(Nook, Rect)]) -> Vec<Broken> {
        let laid = home.layout(nooks);
        broken(&laid, &strips(nooks), home)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// A resize and back breaks and mends nothing; text over a piece
        /// (which closets it) neither; and what's broken names only
        /// pieces laid out, out of their boxes.
        #[test]
        fn rules_hold_across_a_resize_and_text(
            props in pieces(),
            wide in 30u16..90,
            narrow in 16u16..90,
            text in proptest::collection::vec(1u16..90, 0..6),
        ) {
            let mut home = Home::default();
            for p in props {
                prop_assert!(home.add(p));
            }
            let (big, small) = (panes(wide), panes(narrow));
            let clean = frame(wide, &[]);
            let _ = home.project(&clean, &big, &|_, _| false);
            let before = home.clone();
            let first = judged(&mut home, &big);
            let laid = home.layout(&big);
            for b in &first {
                for piece in b.involved.iter().chain(&b.pieces) {
                    prop_assert!(
                        laid.iter().any(|s| s.item == *piece && !s.boxed),
                        "{:?} in {:?}", piece, b
                    );
                }
                prop_assert!(b.pieces.iter().all(|p| b.involved.contains(p)), "{:?}", b);
            }
            // A resize, and back (where it moved nothing off its strip).
            let _ = home.project(&frame(narrow, &[]), &small, &|_, _| false);
            let _ = judged(&mut home, &small);
            if home == before {
                let _ = home.project(&clean, &big, &|_, _| false);
                prop_assert_eq!(&judged(&mut home, &big), &first);
            }
            // Text closets what it covers; the rules don't see it.
            let mut home = before;
            let _ = home.project(&frame(wide, &text), &big, &|_, _| false);
            prop_assert_eq!(&judged(&mut home, &big), &first);
        }
    }
}
