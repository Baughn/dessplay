# Her front door, side-on, with its parcel flap (approved B; wired)

Last updated: 2026-10-10 (wired by the door batch, steps 7-9; the sheet below is the round-1 review, kept as it was
except where marked "as built")

> **As built (2026-10-10).** B was approved and is wired. The art code was committed in `8eaa25c1` (so
> `worktree.diff` and "against `1e44b050`" below are history: never re-apply the diff), made production by the door
> batch's step 7 (`art.rs`, `graphics.rs`, `sprite.rs`, `art/wall-door.svg`) and drawn in play by steps 8 (the
> door, her steps through it, the Away cue) and 9 (parcels through the flap). Where this sheet and the code differ,
> [the door batch's design](../design.md) and design.md (*Away at school*, *Getting furniture*) are the rule:
> - **SVG ids.** `wd-beyond` and `wd-day` are gone. The doorway, the hedge and the flap's hole are the unfilled
>   paths `wd-doorway`, `wd-hedge` and `wd-flap-hole`, filled per sky by `wall_door_body(door, sky)` with the
>   gradients `wd-sky-{night,dawn,day,dusk,evening}` (derived from the window's sky stops; `wd-sky-day` is the old
>   `wd-day`) and the hedge tinted by `art::hedge(sky)`.
> - **Rust.** `art::render_wall_door(door, sky, cols, facing, line, width, height)` (it was `(door, facing, line,
>   ..)`): the 6-column frame drawn and cropped to `cols` columns at the wall. Looks are built by
>   `Look::wall_door(door, sky, crop)` (sky is `Day` for a state that shows none of it). `WallDoor::Plate(u8)` is the
>   flap's plate alone, drawn over a sliding parcel. `lean` is production (`art::lean`), not sheet-only; the clip at
>   the wall's line is `graphics::Cut`.
> - **Her beats** (osaka.rs `wall_beat`): 6-7 `Shut { away: true }`, 8 `Ajar`, 9 `Open`, so **the slippers go at
>   beat 8**, as the door opens, not at 10. She steps through at walking pace (`4 × WALK_MS` a column), not in
>   thirds: beat 2 is `d = 1 ..= 4`, beat 10 is `d = 3 ..= 0`, her last step in side-on (`Side`) through beat 12;
>   with a shift she carries her shopping (`Carry`) from her first step in. The Away cue shows in every gap (a
>   dash's and work's too). The doorway follows the window's sky.
> - **ASCII.** The flap's open rows (`WALL_DOOR[1]`) show at any angle; the plate is drawn only while the flap is
>   up (angle above 0). Her cells are dropped at the wall's column and beyond, and she's hidden from `d = 4`.
> - **The flap's beats** are built as tabled below; a delivery waits while her door's act runs or her box (or her
>   walk) meets the space, and comes through another wall's `draw_flap` only when her door's strip can't take it.

This is the model sheet for her external door, seen side-on in a wall at the screen's edge, with a parcel flap
in it. It's the art half of the door-space fix (`scratchpad/door/brief.md` §5.3; the user: "The door can stay drawn
while she's out; there's really no other way to know she *is* out. But that might work better with a side-on
drawing of the door", and "parcels [come] through a flap in the door"). It was drawn in a throwaway worktree on top
of `1e44b050` and committed there with git. `worktree.diff` in this directory is its code part (`dessplay/` only,
against `1e44b050`):
- a new `art/wall-door.svg`;
- `WallDoor` and `render_wall_door` in `art.rs`;
- `sprite::WALL_DOOR` (the ASCII rows);
- two lints;
- the ignored `wall_door_sheet` test.

Nothing outside tests builds any of it (`#[cfg_attr(not(test), allow(dead_code))]`). No behaviour or golden moves.
The door in space (`Look::Door`, `art/door.svg`) is untouched.

## The main question: does a 1-column edge-on door read?

**No.** Band 1 puts the two candidates side by side.
- **A, edge-on:** a slab in the wall column alone, with a lintel cap, a threshold and the knob standing out into
  the room. At 1× it reads as a pink pillar or a post with a knob. Standing alone for 4.5 hours, nothing says
  "door".
- **B, turned a little toward the viewer:** two columns. The front post covers the wall's line (column `to`). The
  leaf's face shows in `to-1`, foreshortened, with its back post, a panel, a knob and the flap. At 1× it reads as a
  door at once, in both walls.

**I recommend B, and the rest of the sheet draws only B.** That's the brief's 2-column fallback, but the leaf's
*face* is in `to-1`, not just its edge.

## Review sheets (this directory)

The sheets are drawn over `#1e2127` with the `#1d1714` outline, as the game composites them. Each door stands at a
screen edge: the wall's line is drawn in the border grey and the door paints over it, as `draw_flap` does today.
There's a strip of floor too.
- `wall-door-1x.png`: true on-screen size (9 × 19 px cells).
- `wall-door-1x-nn3x.png`: the same pixels at 3×, nearest-neighbour. **Judge readability here.**
- `wall-door-3x.png`: a native 3× render. Judge the drawing here.

Bands, top to bottom (numbered in dim grey):

1. **Shut.** A at a right wall, then a left wall; B at a right wall, then a left wall.
2. **Going out** (right wall). She walks up, then stands at her spot facing the shut door. The door goes ajar (the
   leaf turning in on its back hinge, behind her), then open (the leaf flat against the back, face-on, the doorway
   showing daylight and a hedge). She steps into the doorway, half through, then nearly gone. While she's in the
   doorway she's clipped at the wall's line and the front post is drawn over her.
3. **Closing behind her**: open, ajar, shut. Then **her door while she's out**: at a right wall and at a left wall,
   each with her house slippers left on the floor before it, toes to the room, and a little card hung on the knob.
4. **Coming home** (left wall). The door goes ajar, then open. She peeks in at the doorway, then is half in. She
   stands at her spot facing the room (face-on, like today's door), the door goes ajar behind her, and it shuts as
   she walks off.
5. **A small parcel** (her clock's) **through the flap** (right wall).
   - The flap, shut.
   - The parcel nosing out, the flap riding up on its corner.
   - Half out.
   - Out, the flap falling back.
   - Settling.
   - The flap shut, the parcel at rest at the space's inner edge.
6. **A large parcel** (a bookshelf's, 5 columns) at a left wall: the same frames.
7. **Clearance.**
   - A bed whose near end is at `to-7` (`to-6` is the cell of air), with the door open and her at her spot.
   - At a left wall, a sofa likewise, with her door while she's out.
   - Her door's space (`to-6 ..= to-1`) is bracketed under the floor.

To regenerate:

```sh
HOUSEGUEST_WALL_DOOR=/dir cargo test -p dessplay --lib wall_door_sheet -- --ignored
```

## Geometry (settled)

Right wall shown; a left wall mirrors it. `to` is the wall's own column (the pane's border), `f` the floor row.
Her door's space is `to-6 ..= to-1`, as in the brief.

| What | Columns | Rows |
|---|---|---|
| Door, shut (B) | `to-1 ..= to`: the face and back post in `to-1`, the front post over the wall's line in `to` (painting the right half of `to` too) | from the lower half of `f-4` down to the floor line, with the threshold on the line (the image's bottom is the middle of row `f`, as hers is) |
| Door, shut, with the Away cue | `to-3 ..= to` (the slippers stand in `to-3 ..= to-2`) | the slippers sit on the floor line |
| Door, ajar | `to-3 ..= to` (the leaf turned into the room, hung on the back post) | as shut |
| Door, open | `to-3 ..= to`: the leaf face-on over `to-3 ..= to-1`, the doorway in `to-1 ..= to` | as shut |
| The flap | the leaf's lower half, rows `f-2 ..= f-1` (the old flap's rows), hinged along its top | — |
| The flap swung up | up to `to-5` for a large parcel (fully up, 90°, it just reaches `to-5`), inside the space | `f-2 ..= f-1` |
| Her spot | box `to-5 ..= to-1`, facing the wall to go out, the room on coming in | `f-4 ..= f-1` |
| Her in the doorway | box `to-3 ..= to+1`, then `to-1 ..= to+3`, **clipped at the wall's line** (the middle of column `to`); the front post over her | — |

The lint `art::tests::the_wall_door_keeps_to_its_columns` pins these column claims for both walls.

**Frame.** The brief proposed a 40 × 160 canvas (2 × 4 cells, her units). I used **120 × 168: 6 × 4 cells at a
piece's scale** (20 × 42 units per cell, as `props.svg`). That's because:
- the open leaf, the ajar leaf and the flap reach to `to-3` (the flap to `to-5`);
- one frame for every state means one placement rule;
- a piece's units make the parcel and the flap share a scale.

`render_wall_door(door, facing, line, 6·w, 4·h + h/2)` is placed with its left column at `to-5` (right wall, facing
Right) or at `to` (left wall, facing Left), its top at row `f-4`, like a standing piece. The parts are authored with
the wall's line at x = 70, and `wall_door_scene` shifts them 40 units right.

**Height.** The door is about her height (her head reaches its lintel). The frame is 4 rows, and the space is 4 rows
tall with the poster and clock hanging above it (brief Q2), so it can't be taller.

## What was added

### SVG parts (`art/wall-door.svg`)

| id | what |
|---|---|
| `wd-a-shut` | **A** (rejected, the sheet's comparison only): the slab edge-on in the wall column, its lintel cap, threshold and knob. |
| `wd-frame` | B's frame: the back post (the hinge side, nearer the room's middle), a lintel slanting a little (the turn), the front post over the wall's line, a wooden threshold. Drawn last, over the leaf. |
| `wd-post` | The front post and the threshold alone, drawn over her while she's in the doorway. |
| `wd-beyond` | Outside, through the doorway: a sky-to-pale-gold daylight gradient over a green hedge. |
| `wd-shut` | The leaf, shut: its face foreshortened, a top panel, the knob by its free (front) edge. |
| `wd-flap-shut` | The flap, shut: the leaf's lower half as a hatch, a lit edge along its hinge, a brass pull near its foot. |
| `wd-flap-hole` | The flap's hole when it's open: daylight. |
| `wd-ajar` | The leaf turned about 50° into the room on the back hinge, foreshortened, its free edge nearer (taller). |
| `wd-open` | The leaf face-on, flat against the back over `to-3 ..= to-1`: a top panel, the flap below with its pull, the knob at the far end. |
| `wd-slippers` | Her house slippers, a pair side by side on the floor, toes to the room (rose uppers, cream soles). |
| `wd-tag` | A small cream card on a string from the knob, with two red lines on it ("out"). |

The flap's plate when swung up is drawn by `art.rs` (`flap_plate(degrees)`). It's the hatch turned on its top hinge
in the picture's plane, with its edge as a darker strip and the pull near its free edge.

### Rust

- **`art::WallDoor`**:
  - `Shut { flap: u8, away: bool }`: `flap` is the angle the flap is swung up, 0 when shut; `away` adds the
    slippers and the card.
  - `Ajar`.
  - `Open`.
  - `Post`: the front post alone, drawn over her in the doorway.
- **`art::render_wall_door(door, facing, line, width, height)`.** Facing Right is the right wall.
- **`sprite::WALL_DOOR`**: the ASCII rows (below).
- **Lints:**
  - `art::tests::the_wall_door_keeps_to_its_columns`: every state renders in both walls, inside its columns;
  - `sprite::tests::the_wall_door_is_ascii_and_keeps_to_its_columns`.
- **`wall_door_sheet`** (ignored) writes the three PNGs. It has two sheet-only helpers that the builder will want in
  `art.rs`:
  - clipping an image at the wall's line;
  - `lean(item, lead)`, the flap's angle as it rides on a parcel. While the parcel's leading top corner is within
    the flap's reach (78 units from the hinge at (57.5, 82)), the angle is the one that rests the flap on that
    corner. After that, it's the angle that rests the free edge on the lid. A parcel's lid is at
    `168 − 64·scale`, with `scale = min(cols·20/100, 1)`.
- Two glyphs, `A` and `B`, were added to the sheet's bitmap font.

### ASCII rows (`sprite::WALL_DOOR`)

Each state is rows `f-4 ..= f-1` of columns `to-3 ..= to` at a right wall. The last column is painted over the
wall's border glyph, as `draw_flap` does today: only over a plain vertical line, skipping protected cells, and
recording `Frozen` cells for the rain. Spaces are left alone. For a left wall, mirror each row with `sprite::mirror`;
that needs `[`/`]` added to it. The rows are pure ASCII. ASCII gets no review and has no Away cue.

```
shut   flap open   ajar   open
  /|     /|        \.|   [].|
  o|     o|        |.|   [].|
  #|    /.|        o.|   o].|
  #|     .|        |.|   [].|
```

- **Shut:** the door is two columns: the lintel `/`, the knob `o`, and the flap `#` in its lower half.
- **Flap open:** the hole is `.` (daylight), and the plate `/` is swung up into `to-2`.
- **Ajar:** the leaf turning in at `to-2` (`\`, `|`, with the knob `o`), the doorway's daylight `.` in `to-1`.
- **Open:** the leaf face-on `[]` over `to-3 ..= to-2`, its knob `o` at the far end, the doorway `.` in `to-1`.
- **Her over it.** At her spot, her glyphs in `to-1` win, so she's drawn over the door as in line art. In the
  doorway, drop her cells at column `to` and beyond (ASCII's clip).

## Wiring notes for the builder (after approval)

### The DOOR beats, for her external door

The beats are `osaka.rs` `DOOR`, 13 of them. A new look beside `Look::Door` draws `render_wall_door`, and her door's
spot is the space's (`door_place`, brief §5.5). The door in space keeps `DOOR` and `door.svg` as they are. Her box in
the table is relative to her spot (box `to-5 ..= to-1`); `d` is columns toward the wall.

| # | Beat (ms) | Door | Her |
|---|---|---|---|
| — | `Job::Leave` walk | `Shut` (the door shows as she arrives) | walks to her spot (`Walk`), faces the wall |
| 0 | Closed + her (600) | `Shut` | at her spot, side-on facing the wall (`Side`) |
| 1 | Ajar + her (300) | `Ajar` | as 0 |
| 2 | Open + her (700) | `Open` | steps through in thirds: at her spot (`Side`); `d = 2` (`Walk(2)`); `d = 4` (`Walk(0)`). Clipped at the wall's line, `Post` over her while `d > 0` |
| 3 | Open, her gone (400) | `Open` | — |
| 4 | Ajar (250) | `Ajar` | — |
| 5 | Closed (350) | `Shut` | — |
| 6 | the gap (`u64::MAX` for her routine) | **`Shut { away: true }`** stays drawn. Today's door in space draws nothing here; her external door is the Away cue | — |
| 7 | Closed, there (400) | `Shut { away: true }` | — |
| 8 | Ajar, there (250) | `Ajar` | — |
| 9 | Open, there (350) | `Open` | — |
| 10 | Open + her, there (600) | `Open` | steps in in thirds, facing the room: `d = 4` (`Walk(0)`, a peek); `d = 2` (`Walk(2)`); at her spot (`Stand`, face-on, as today's door). Clipped and `Post` over her while `d > 0` |
| 11 | Ajar + her (300) | `Ajar` | at her spot, side-on facing the room (`Side`) |
| 12 | Closed + her (400) | `Shut` | as 11; then she walks off (`Walk`) |

- **The Away cue's span.** `away` is true from the gap (6) until she's back in (beat 10), including a cold start in
  school hours and `paint_empty`'s Away door. The slippers vanish as she steps into them. *As built: `away` holds
  through beats 6-7 only; the slippers go at beat 8, as the door opens (C2).*
- **Dashes and work by door** use the same beats with their own gaps. During a short gap the cue may show or not;
  I'd show it, so that "her door shut with slippers before it" always means she's out.
- **The clip is new.** Her image is cut at the wall's line, the middle of column `to`. Today her image is always
  drawn whole in her box. Line art needs a per-pixel cut at `to`'s middle; ASCII drops her cells at `x ≥ to`. If
  the clip is unwanted, beat 2 can stay at her spot and she vanishes as today (`her: false` at beat 3), but the
  half-through frames are what make "going through the door" read.
- **Draw order.** The leaf (ajar and open) is part of the door layer drawn *before* her. Its hinge is on the back
  post, so it's never over her. `Post` is a second door layer drawn *after* her, only while `d > 0`.

### The flap's beats (deliveries)

Today a delivery stands the boxed piece at its anchor at once, with `draw_flap` showing `╱`/`╲` in the wall for
`FLAP_MS` (800 ms). With her door, a parcel comes through the door's flap:

| Age (ms) | Door | The parcel |
|---|---|---|
| 0–150 | `Shut { flap: lean(item, -5) }` | its leading edge 2½ columns past the wall's line (a column past the back post), clipped at the line |
| 150–350 | `Shut { flap: lean(item, half) }` | half out (`half = -3 - cols`, in half columns) |
| 350–500 | `Shut { flap: 10 }` | out, just clear of the doorway |
| 500–650 | `Shut { flap: 4 }` | halfway to rest |
| 650– | `Shut { flap: 0 }` | at rest: its prop's own spot |

Notes for the builder:
- **Draw order.** The flap's plate is drawn after the parcel, so it rides on it. On the sheet, that's a second draw
  of just the plate.
- **The parcel at an offset.** `render_parcel` today draws only at its prop's footprint. The slide needs a parcel at
  any x (a transient offset from its prop spot), clipped at the wall's line.
- **`doorstep` changes.** With a door space, a parcel always comes through her door's wall and its flap. That
  overrides brief Q1's recommendation (A, another wall), since the user asked for B. The parcel then rests at the
  narrowed extent's `Anchor { side, offset: 0 }`, its trailing edge at `to-7`. When the space yields (no door
  space), today's wall flap (`draw_flap`'s glyphs) stays as the fallback.
- **ASCII:** the flap's open row is `WALL_DOOR[1]` for the 650 ms. The parcel's own ASCII stands at its spot, as
  today.

## Open choices for the user

1. **B over A.** The 1-column edge-on door (A) reads as a pink post. The turned door (B) takes `to-1` as well as the
   wall column, and reads as a door. Is two columns all right?
2. **The flap is the door's lower half** (rows `f-2 ..= f-1`, the old flap's rows). It's hinged at its top and rides
   up on the parcel as the parcel slides out. A large parcel pushes it nearly level, 4 columns into the room (still
   inside the space). The alternative is a smaller letterbox flap that the parcel squeezes through, cartoon-style.
3. **Where a delivered parcel comes to rest.** The sheet slides it out across the space to the space's inner edge,
   where pieces stand (trailing edge at `to-7`). The alternative is that it stops in the space beside the door, a
   transient exception to "no piece in the space" until she unpacks it. That would block her going-out spot until
   she does.
4. **The Away cue.** Her slippers on the floor before the door, plus a card on the knob. Both, one, or neither?
5. **Outside.** The doorway always shows daylight on the sheet. When wired, it could follow the window's `Sky`
   (dawn, day, dusk, evening, night). Her school run is all daylight, but errands and dashes may not be.
6. **The clip.** Is it worth cutting her image at the wall's line so she visibly walks through the doorway? The
   alternative is that she vanishes at her spot, as with the door in space.

## What ran

In the worktree:
- `cargo fmt`;
- `cargo clippy --workspace --all-targets -- -D warnings`: clean;
- `cargo test -p dessplay --lib houseguest`: 671 passed, 33 ignored (the art and sprite lints included);
- the ignored `wall_door_sheet`, which wrote the three PNGs.
