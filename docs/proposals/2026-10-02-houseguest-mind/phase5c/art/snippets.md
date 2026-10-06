# Houseguest 5c art: stillness (for review, not wired in)

Last updated: 2026-10-05 (round 2 approved, with its fix)

This is the model sheet for phase 5c's new line art (`phase5c-design.md`, "Round-1 amendments" → "Steps,
renumbered", step 0; D5, D6, D7). **Approved by the user** (round 2, with the doze-book fix below); step 9 wires it. It was drawn in a
throwaway worktree on top of master `33b91ee` and committed there with git: round 1 `2cf21bb`, round 2 `5740425`,
the fix on top of it (see the branch). `worktree.diff` in this directory is the code part of all three (`dessplay/` only,
against `33b91ee`): SVG parts in `art/osaka.svg` and `art/props.svg`, rigs and eight `Pose` variants in
`art.rs`/`sprite.rs`, a paper-desk heap in `scrap.rs`, a `Channel::Programme` and the `stillness_sheet` test.
Nothing constructs any of it outside tests: every new variant carries `#[cfg_attr(not(test), allow(dead_code))]`,
`MAKES` and the window's `hang` are untouched, and no behaviour or golden moves.

## Review status

**Round 1 (2026-10-05), approved as drawn:** row 2 the paper desk with her kneeling in seiza; row 3 reading a
torn strip; row 4 cross-legged at the TV; row 7 the sitting doze ("looks great"); rows 8–9 looking up in place
and stirring; row 10 the TV programme cards. These are unchanged in round 2.

**Round 2 redraws** (the user's notes):
1. Floor homework: "Homework would generally be in front of her, not directly under her head." The paper is
   now out in front of her face; the doze matches. New: "A version where she's on her back holding the book
   above her head to read it would also work; she can doze with it on her face." Added as `LieRead`.
2. Window: "I think it has to be lower. The window being drawn behind the sofa would look fine from a design
   perspective, unlike other furniture." The sheet hangs it at 1 (its sill at her chest) beside a sofa, partly
   behind it, and she leans on the sill, chin in her hands.
3. Cloud-watching: "Looks fine, but can only really happen underneath a window, or in a solarium (which
   remains todo)." Shown lying under the low window.

**Round 2 approved (2026-10-05) with one fix,** now made: "when she has the book on her face while sleeping,
the book needs to rotate ninety degrees; the side should be towards the viewer. That would cover her eyes,
which is largely why one uses books this way." `LieRead(2)` now lays the book over her face turned side-on
(`book-cover`: its near cover toward the viewer, spine along the top, page edges along the bottom), covering
her eyes. Everything else in round 2 is approved as drawn.

## Review sheets (this directory)

All three are drawn over `#1e2127`, outline `#1d1714`, as the game composites them (a piece's back layer,
her box, terminal text for bubbles). Bubbles are terminal text in the game; the sheet draws them in a
stand-in 5 × 7 bitmap font, since the renderer has no fonts.

- `stillness-1x.png`: true on-screen size (9 × 19 px cells).
- `stillness-1x-nn3x.png`: the same pixels at 3×, nearest-neighbour. **Judge readability here.**
- `stillness-3x.png`: a native 3× render. Judge the drawing here.

Bands, top to bottom (numbered in dim grey at the left of each band):

1. **Floor homework** *(redrawn)*: `FloorHomework(0)`, `(1)` (writing: the pencil moves), `(2)` (dozed off,
   `zzz`), then `(0)` facing left. **Reading on her back** *(new)*: `LieRead(0)`, `(1)` (a page turns),
   `(2)` (dozed off with the book open on her face, turned side-on over her eyes, `zzz`).
2. **The paper desk** *(approved)*. The cube alone; then homework at it kneeling: writing ×2, nodding off,
   asleep on it (`zzz`); then the cross-legged alternative (not chosen): writing, asleep.
3. **Reading a torn strip** *(approved)*. `ReadStrip(0)`, `(1)`, `(0)` facing left; today's `Read(0)`.
4. **Cross-legged before the TV** *(approved)*. `CrossLegged` by the penguin and cooking cards; today's
   `Sit` by the shopping channel for reference.
5. **The window, hung low** *(redrawn)*: hung 1 (its bottom row the floor − 2), beside the sofa and partly
   behind it (the window drawn first), her leaning on the sill facing left (Curious); again alone, facing
   right (Vacant, a `~` musing); `UnderSill` sitting in front of it (Curious, `~`); `SitDoze(1)` there
   (`zzz`), the settle-in.
6. **Cloud-watching** *(redrawn)*: `LieBack(0)`, Curious, lying under the low window with "That cloud's a
   bun."; the doze (`LieBack(0)`, Blink, `zzz`) on the bare floor for contrast.
7. **The sitting doze** *(approved)*. `Sit` for reference, `SitDoze(0)`, `SitDoze(1)`, `SitDoze(1)` facing left.
8. **Looking up in place** *(approved)*, the chat to her left: `!` with Surprised then `?` with Curious, for
   `Sit`, `LieBack(0)`, `CrossLegged` and `UnderSill`. Bubbles sit where `bubble_spot` would put them.
9. **The same on the sofa** *(approved)* (`Lounge`); the dozes stirring instead (`LieBack(0)`, `Nap`, `Mm?`).
10. **Programme cards** *(approved)*: news, weather, penguins, cooking; then today's colour bars and sunrise.

To regenerate:

```sh
HOUSEGUEST_STILLNESS=/dir cargo test -p dessplay --lib stillness_sheet -- --ignored
```

## What was added

### SVG parts

`art/osaka.svg`:

| id | what |
|---|---|
| `paper` | A sheet lying flat, seen a little from above (a white parallelogram with two blue rule lines), centred on (0, 0), underside at y = 2. On the paper desk's top. |
| `paper-floor` | The same, 40 wide, for floor homework: lying out in front of her. |
| `pencil` | A yellow pencil with a pink eraser, grip at (0, 0), point down. 1 px wide at 1×. |
| `book-side`, `book-side-turn` | An open book seen from the side: teal covers as a tent (ridge up) over white pages, a yellow title stripe; the turn has a page lifting off the ridge. Centred on (0, 0), 34 × 13. Held over her face lying on her back, to read. |
| `book-cover` | The same book left open on her face as she dozes, turned side-on: the teal near cover with a darker spine band along the top (the tent's ridge), a title stripe, the white page edges along the bottom where it rests. Centred on (0, 0), 35 × 22; lying over her face it covers her eyes. |
| `strip` | A strip of torn text: a ragged dark band of shredded glyph blocks in two text colours (grey `#c9d1d9`, blue `#79c0ff`), its far end curling down. Drawn as the makeshift furniture draws text. Centred on (0, 0), about 38 × 13 units. |
| `p-crossed-legs` | Cross-legged legs in profile, in her own unbobbed frame (floor at y = 132): both thighs out forward, the shins crossed under the knees, the far loafer tucked behind and the near one peeking out in front. Like `p-seated-legs`. |

`art/props.svg` (each clipped to the TV's glass, 26..84 × 72..118):

| id | what |
|---|---|
| `tv-news` | A blue studio: an anchor (dark suit, white shirt) behind a pale desk, a yellow inset with a red roof, a red ticker with white text bars. |
| `tv-weather` | Blue sea, a green island arc (the map), a sun with rays, a white cloud with two rain strokes. |
| `tv-penguin` | Pale sky over a dark-blue sea, a white ice floe, a penguin (black, white belly, orange beak and feet) and a grey chick. |
| `tv-cooking` | A tiled kitchen wall, a wooden counter, a red ramen bowl (yellow noodles, a naruto slice, chopsticks), three steam curls. |

### Rust

- **`Rig` gains two fields.**
  - `ground: Option<(&str, x, y, angle)>`: a part placed in **canvas** coordinates (not bobbed, turned or
    scaled with her), drawn behind her so her hands go over it. `hold` lives in her own turned frame, so a
    paper on the floor would have rotated with her lying body.
  - `crossed: bool`: draws `p-crossed-legs` in front of her skirt, unrotated by `lean` (her legs stay on
    the floor as she leans).
  - `pulling`, `climbing` and `standing` (the full literals) set both to `None`/`false`.
- **Rigs** (`art.rs`). Arm angles are solved (a forward-kinematics script, not by eye) so the hands land
  on the paper, the book, the sill, the chin and the strip:
  - `Rig::floor_homework(frame, face)` *(redrawn)*: `turn` 90 at 0.74 like `LieFront`, her body pushed
    back in her box (`shift` −38), chest a little up (`lean` −18), head up looking ahead (`tilt` −20), feet
    up behind; the `paper-floor` lies out in front of her face (`ground`), `hold` pencil in her near hand
    at its near edge. **Her chibi arms reach only to under her chin**, so the paper's near edge is there and
    the rest (about two thirds) lies out past her face. Frame 2: flatter, head down on her arms, eyes shut
    (Blink is forced), the paper still out in front.
  - `Rig::lie_read(frame, face)` *(new)*: `LieBack`'s body (on her back, knees up); both arms up holding an
    open `book-side` by its near end just over her face, tipped open toward her eyes; frame 1 swaps in
    `book-side-turn`. **Her hands reach only to her face's height** (her head is big), so the book is held
    close over her face rather than at arm's length. Frame 2: arms down, the book lying open on her face
    turned side-on (`book-cover`), covering her eyes.
  - `Rig::paper_desk(frame, kneel, face)` *(approved)*: kneeling in seiza or cross-legged (`crossed`);
    `ground` paper on the cube's top (`PAPER_DESK_TOP` = 110 canvas units). Frames 0–1 write, 2 nods,
    3 asleep on her arms on the cube; drawn facing the cube, her box centred a column past its end.
  - `Rig::reading_strip(frame, face)` *(approved)*: `reading()` with the strip over her knees.
  - `Rig::cross_legged(face)` *(approved)*: `crossed`, upright, hands in her lap.
  - `Rig::sill_lean(face)` *(redrawn)*: standing in profile facing the window, leaning in a little (`lean`
    8, `shift` 4), elbows on the sill at her chest (canvas y 80) and chin cupped in both hands, head up a
    touch (`tilt` −6), the near knee eased. Drawn for a window hung 1.
  - `Rig::under_sill(face)`: `Sit`'s floor seat (knees up), chin cupped in both hands, head up 16°.
  - `Rig::sit_doze(frame)` *(approved)*: `Sit`, eyes shut, head sinking onto her knees.
- **Poses** (`sprite.rs`), each with ASCII rows, in `sprite::ALL` (47 → 64) and in `art.rs`'s `poses()`:
  `FloorHomework(0..=2)`, `PaperDesk(0..=3)` (kneeling), `ReadStrip(0..=1)`, `CrossLegged`, `SillLean`,
  `UnderSill`, `SitDoze(0..=1)`, `LieRead(0..=2)`. `Rig::for_pose` maps each to its rig. The lints pass:
  `every_pose_is_drawn_its_own_way` (no new exemption), `every_pose_renders_something_inside_the_box`,
  `every_pose_has_a_head`, `every_pose_fits_the_box_and_is_ascii`.
- **The paper desk** (`scrap.rs`): `Heap` gains `block` (a box of shreds; the old heaps draw exactly as
  before) and `DESK` is one block, 80 × 84, its top 48 units up. `footprint(Desk)` = (4, 2),
  `heaps(Desk)` = `DESK`, and `cell` draws every cell of a Desk. **`MAKES` is not touched.**
- **Programme cards**: `Programme { News, Weather, Penguins, Cooking }`, `Channel::Programme(Programme)`;
  `tv_scene` draws the card's part. `Prop::framed` holds it; `screen_glyphs` gives ASCII pairs (news `o=`,
  weather `*c`, penguins `i~`, cooking `~u`).
- **`stillness_sheet`** (ignored) writes the three PNGs. The low window is drawn there only (a hung piece
  with its top row at floor − 3); `WINDOW.hang` stays `Some(4)`.

### ASCII rows (`sprite.rs`; facing right, mirrored for left)

All profile rows (their faces baked in), so `frontal` is false for each. Each pose is four rows of five
cells; `|` marks the box edges here.

```
FloorHomework(0)  FloorHomework(1)  FloorHomework(2)
|     |           |     |           |     |
|     |           |     |           |     |
|\    |           |\    |           |     |
|\_Vo=|           |\_Vo-|           |__Vo=|

LieRead(0)  LieRead(1)  LieRead(2)
|     |     |     |     |     |
|     |     |     |     |     |
|[] /\|     |/] /\|     |^  /\|
|o=V=^|     |o=V=^|     |o=V=^|

PaperDesk(0)  PaperDesk(1)  PaperDesk(2)  PaperDesk(3)
|     |       |     |       |     |       |     |
|     |       |     |       |     |       |     |
|( ._)|       |( ._)|       |( -_)|       | (-_)|
|_|V|=|       |_|V|-|       |_|V|=|       |_/V\=|

ReadStrip(0)  ReadStrip(1)  CrossLegged  SillLean  UnderSill
|     |       |     |       |     |      |( 'o)|   |     |
|( ._)|       |( ._)|       |( ._)|      | |V/ |   |( 'o)|
|==V| |       |~=V| |       | |V| |      | /_\ |   | |V/ |
| d b |       | d b |       |_/x\_|      | / \ |   | d b |

SitDoze(0)  SitDoze(1)
|     |     |     |
|(-_-)|     |     |
|<(V)>|     |(-_-)|
| d b |     |<dVb>|
```

- The floor paper is `=` (the pen `-` on frame 1, as at the desk) in the column past her head. ASCII has
  no column to spare for a paper further out; the line art carries "in front".
- `LieRead` holds the book `[]` over her head (`/]` turning a page); dozing, the tent `^` lies over it.
  Her head stays the lone `o`, so `head()` and the bubbles find it.
- `SillLean`'s near arm goes up to her chin (`/`), as `UnderSill`'s does seated.

## Wiring notes for step 9 (after approval)

| Sheet item | Pose | Rig | Notes |
|---|---|---|---|
| 1. Floor homework | `FloorHomework(0/1)` writing at `USE_FRAME_MS`; `FloorHomework(2)` the doze | `floor_homework` | `ground` paper and `hold` pencil are in the rig. M6: the nod-off point by mood moves to frame 2 (`Dots` then `Zzz`). "My back..." is a Stand + line after it, not art. The paper takes the column in front of her box's head end: the spot needs no extra room (it's inside her box). |
| 1b. Reading on her back | `LieRead(0/1)` reading (page turns at the Read cadence), `LieRead(2)` dozed off | `lie_read` | **Serves two wants:** floor reading (a Use(Read) or Idle method where no bookshelf stands, beside `use/borrow`'s strip) and **floor homework with a book** (a second method of floor homework: reading the set text). Its doze is a doze for the settle-in and stir rules (`Mm?` on chat), like `LieBack`. |
| 2. Paper desk | `PaperDesk(0/1)` write, `(2)` nod (`Dots`), `(3)` asleep (`Zzz`), like `HOMEWORK` | `paper_desk(frame, true, …)` | The made desk's seat: her box centred one column past the cube's end, facing it (the real desk's `sit: Some((7, true))` rule; for the 4-wide cube, column 4). `Shown::seat`'s makeshift arm needs a per-item seat (today one `scrap::SEAT` = 3, which would put her *in* it). Draw the cube's `Part::Back` behind her. |
| Cube | — | `scrap::DESK` | Add `Furniture::Desk` to `MAKES`; `footprint`/`heaps`/`cell` arms exist. `cell`'s `loose` test (`2..cols-2`) is empty at 4 wide: use `1..cols-1` for the desk. |
| 3. Reading a strip | `ReadStrip(0/1)`, as `Read` | `reading_strip` | A static part in two fixed text colours; a procedural `hold` from the torn glyphs is possible later. |
| 4. TV from the floor | `CrossLegged` as the Watch host away from a sofa (osaka.rs:7189 host rule) | `cross_legged` | Seat unchanged. |
| 5. The window hung low | `SillLean` for LookOut's long daydream hold; `UnderSill` and `SitDoze` the settle-ins in front of it | `sill_lean`, `under_sill`, `sit_doze` | See "The low window" below. |
| 6. Cloud-watching | `LieBack(0)` held (period 0) with `Face::Curious`, one static sky bubble | (existing) | **Gated to lying under a window** (a solarium later): offered only from a spot under a window piece's columns (the window's settle-in or a method of LieBack/Daydreams bound to `look_out_spots`), never on the bare floor. Elsewhere a LieBack stays the doze. ASCII differs by bubble only. |
| 7. Sitting doze | `SitDoze(0)` then `(1)`, a second settle branch from `Sit` | `sit_doze` | Blink is forced by the rig. |
| 8–9. Looking up in place | the running act's pose, facing the chat, `Face::Surprised` + `!` then `Face::Curious` + `?` | (existing) | Lying back, keep the facing that puts her head nearer the chat (don't flip `LieBack`/`Nap`/`LieRead`). |
| 10. Programme cards | `Channel::Programme(p)` as the held picture when there's no screenshot | `tv_scene` | `CHANNELS` (the render lint's list) can gain the four when wired. |

Remove the `allow(dead_code)` attributes as each variant gets constructed.

### The low window (wiring changes for D6)

- **Hang.** `WINDOW.hang`: `Some(4)` → **`Some(1)`**. Its bottom row is then the floor − 2, its top row the
  floor − 3, and its sill top sits at her chest (canvas y ≈ 81: `160 − (hang + ½) · 42.2 − 15.6`). Hang 2 puts
  the sill above her eyes; hang 0 at her hips. The clock and poster stay at 4.
- **`hung_pieces_clear_every_standing_piece`** (room.rs) asserts every hung piece hangs at least as high as
  the tallest standing piece (the TV, 4 rows). It needs a window exception: the window may hang under that
  height, because it **may overlap a sofa but no other standing piece**. Restate it as: every hung piece
  clears every standing piece it can share columns with; the window shares columns only with the sofa.
- **Placement.** Today the wall lane is packed on its own ("a hung piece may hang over a standing one",
  room.rs `Lane`), so nothing stops a hung piece's columns overlapping a standing one. For the low window,
  placement (packing the wall lane, a delivery's spot, and her moving it) must keep its columns clear of
  every standing piece except a sofa, both ways round: a standing piece (not a sofa) can't be put under it
  either. Make the rule one predicate (say `Furniture::may_overlap(standing)` → only `Window`/`Sofa`) used by
  both lanes, so the two directions can't drift. Its `needs()` rows (3) still fit any pane that fits the sofa.
- **Draw order.** The window is wall decor: it draws **before** standing furniture, so a sofa covers its
  lower corner (as on the sheet), and before her. Check that wall-lane pieces draw first in both line art
  and ASCII (in ASCII the sofa's cells win where they overlap).
- **Her spots.** `look_out_spots` (room.rs) gives the window's middle and its sides. For the lean her face
  is over the glass: on the sheet her box is centred on the window's first column facing into it (box left
  = window left − 2 facing right; window left + 2 facing left). A spot under the sofa's part is blocked by
  the sofa (`clear_of`), so a window half behind a sofa is leaned at from its free side.
- **Settling in** from the lean: sitting in front of it (`UnderSill`), then dozing there (`SitDoze`). At hang 1
  she sits *in front of* the lower window (her head over the sill), not under it. Cloud-watching lies under
  it, her head well below the sill.
- **As built (step 8c's review, rendered at the code's spots in both facings):** the lean matches the sheet
  either way. The settle-ins keep the lean's spot (settling in is in place, no step), so `UnderSill` and
  `SitDoze` sit a column further out than panel 5 facing right (her face at the window's first column, not
  its second) and, facing left, beside its end rather than under its last columns. Cloud-watching, turned
  end to end, has her head under the glass in both. Accepted (decisions.md, "Her stillness ships"); an
  offset of their own would need a clearance check against a sofa covering the window's other corner.

## Open choices for the user

1. **Floor homework (redrawn):** the paper now lies out in front of her face. Her chibi arms reach only to
   under her chin, so the near edge of the paper is there and the rest lies out past her face; anything
   further would need longer arms than her other poses have.
2. **Reading on her back (new):** likewise her hands reach only to her face's height, so she holds the book
   close over her face by its near end, not at arm's length. The doze leaves it lying open on her face, side-on, covering her eyes (the approved fix).
3. **The window at hang 1:** she leans on the sill with her chin in her hands; the sofa covers the window's
   lower corner. Sitting in front of it, her head is over the sill (she's beside it, not under it).
4. **Cloud-watching** only under a window, as you said; the bare-floor LieBack stays the doze.

## What ran

In the worktree, on the fix commit:
- `cargo fmt`; `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test -p dessplay --lib houseguest`: 571 passed, 16 ignored (the art and sprite lints included).
- `PROPTEST_CASES=32 cargo nextest run --workspace`: 2182 passed, 35 skipped.
- `stillness_sheet`, the ignored test that writes the sheets.
