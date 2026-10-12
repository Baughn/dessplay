# Houseguest decor: the gremlin figurine (approved variant A; wired)

Last updated: 2026-10-12 (wired by phase 6a, step 7; the sheet below is the round-1 review, kept as it was)

> **As built (2026-10-11, phase 6a step 7, `1db6402c`).** Variant A was approved and is wired: its `gremlin` group
> (the last section's, identical to `worktree.diff`'s) is in `art/props.svg` before the clock, and
> `Furniture::Gremlin` draws it (`art.rs`: `(Furniture::Gremlin, _) => &["gremlin"]`). Variants B, C and D are never
> wired, and of `worktree.diff` only the `gremlin` group was taken (never re-apply the diff: the `gremlin_sheet`
> test and the stand-in font's glyphs stay in the throwaway worktree). Where this sheet and the code differ, the
> code and design.md (*Furniture*, *Getting furniture*, *The rules of her home*) are the rule:
> - **The `Spec`** (`room.rs` `GREMLIN`): footprint 2 × 2, ASCII `oo` / `ww` (kept by the user's seventh round:
>   his turn doesn't show in ASCII), ink `Rgb(178, 154, 224)` / `LightMagenta`, beauty 1.0, small decor
>   (`shelf: true`), no uses (her gaze up at him is offered by where he stands, `Use::GazeUp`). Its pitch is "Aww, a
>   little gremlin!" (the user's second round), not the sheet's "Hehehe. He looks like trouble!".
> - **Where he stands**: on any strip small decor stands on, a floor too; he wants up on a shelf (`Rule::High`).
> - **In play**: real frames of him (his parcel, his carry up, turned round on his shelf) are in
>   [../shots/](../shots/README.md) (phase 6a step 9).

This is the model sheet for the decor piece the user asked for: "buying a potted plant, or a gremlin for the
mantelpiece". The proposal (§ *Structures from the UI's own lines*) puts it on a shelf: "a run of `─` above a
floor … decor can stand on it (the gremlin)". It was drawn in a throwaway worktree on top of master `60727649`, and
nothing was committed. `worktree.diff` in this directory is the whole code part (`dessplay/` only, against
`60727649`):
- four SVG groups in `art/props.svg` (the gremlin and three variants), and a line in its header comment;
- the ignored `gremlin_sheet` test in `art.rs`;
- two glyphs (`/` and `\`) in the sheet's stand-in bitmap font.

No `Furniture` variant was added. The sheet renders the SVG groups directly, so no behaviour or golden moves.
`gen.py` here generates the groups and splices them into `props.svg` (`python3 gen.py path/to/props.svg`).

## The design

He's an original imp, not a film gremlin or Mogwai. There's no fur and no big innocent eyes.
- **Body.** A small lavender-glazed ceramic figure: a jowly round head, a pot belly with a paler patch, and a pointed
  tail curling off his hip.
- **Ears.** Big leaf-shaped ears out wide to the frame's edges, pink inside.
- **Face.** Yellow eyes with both pupils slid to one side (sly), brows pinched in, and a toothy grin from ear to ear.
- **Pose.** He's rubbing his hands together under his chin: up to something.
- **Plinth.** A little wooden plinth, so he reads as an ornament and not a creature sitting on the shelf.

**Palette.** Violet isn't used anywhere else in the room, so he stands apart from the green plant, the red clock, the
teal sofa and the pink door. The other colours are borrowed from `props.svg`: the plinth's wood is the TV cabinet's
`#8a5f3c`, the mouth is Kamineko's `#7a1f24` and the ear pink is hers too. The outline is `currentColor` throughout,
with strokes 2 for the silhouette and 1–1.8 for detail, as the other props have.

**At 1×** he's 18 × 38 px. What reads is the shape: a violet head with wide pink-tipped ears, two yellow eyes, a white
grin, a pot body and a wooden plinth. The hands, the pupils' glance and the brows only show at 3× (HiDPI and up).

## Frame

**40 × 84 units: 2 × 2 cells**, at 20 × 42 units per cell. He stands like a floor piece, on a shelf instead of the
floor:
- the plinth's base is on the frame's bottom edge (y = 84);
- he's rendered standing, `w·2 × (h·2 + h/2)`, with his top at `h·(shelf − 2)`;
- so the base sits on the shelf's `─`, at the middle of the shelf row.

Two cells is enough. The ears reach the frame's edges (x 1 to 39). The figure uses y 30 to 84 and leaves the top 7 px
of the frame empty. A 3-wide variant with long goblin ears is on the sheet for comparison (variant D below).

**Pixel snap.** Drawn standing at a 9 × 19 cell, `rasterize` is width-limited at scale 0.45, with dy = 9.2 px (the
poster's case). Whole pixels fall on x = k·20/9 and on y = 84 − k·20/9. The plinth's rects sit on those lines, so its
edges are crisp at 1× (its outline is exactly 1 px). The curved parts are anti-aliased, as in every other piece.

## SVG group (paste into `art/props.svg` inside `<defs>`, before the clock)

Add "gremlin 40 × 84 (2 × 2; it stands on a shelf, its plinth on the frame's bottom edge)" to the header comment's
frame list.

```svg
  <!-- A gremlin figurine for the shelf, 2 x 2 cells (40 x 84): a
       lavender-glazed imp on a wooden plinth, ears out wide, yellow
       eyes slid sideways, brows pinched, grinning ear to ear and
       rubbing his hands; a pointed tail off his hip. The plinth's
       edges sit on whole pixels at a 9 x 19 cell standing on its line
       (x in 20/9 steps, y in 20/9 steps up from 84). -->
  <g id="gremlin">
    <rect x="4.44" y="75.11" width="31.11" height="8.89" rx="1.2" fill="currentColor"/>
    <rect x="6.67" y="77.33" width="26.67" height="4.44" fill="#8a5f3c"/>
    <rect x="6.67" y="77.33" width="26.67" height="2.22" fill="#b07f52"/>
    <path d="M 26 72 C 31.5 73.4 35.6 70.6 35.4 65.6" fill="none" stroke="currentColor" stroke-width="4.2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M 26 72 C 31.5 73.4 35.6 70.6 35.4 65.6" fill="none" stroke="#9179c4" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M 35.4 61.4 L 38.2 66.6 L 32.8 66.4 Z" fill="#9179c4" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>
    <ellipse cx="14.6" cy="73.51" rx="4.2" ry="2.5" fill="#9179c4" stroke="currentColor" stroke-width="1.6"/>
    <ellipse cx="25.4" cy="73.51" rx="4.2" ry="2.5" fill="#9179c4" stroke="currentColor" stroke-width="1.6"/>
    <path d="M 12.6 74.6 C 10.4 66 12.4 54 20 53.6 C 27.6 54 29.6 66 27.4 74.6 Z" fill="#b29ae0" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/>
    <ellipse cx="20" cy="69.8" rx="4.4" ry="4.2" fill="#d8cbf0"/>
    <path d="M 13.4 61.2 C 12.4 64.4 14.6 66 17.8 65" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M 13.4 61.2 C 12.4 64.4 14.6 66 17.8 65" fill="none" stroke="#b29ae0" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M 26.6 61.2 C 27.6 64.4 25.4 66 22.2 65" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M 26.6 61.2 C 27.6 64.4 25.4 66 22.2 65" fill="none" stroke="#b29ae0" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>
    <circle cx="18.4" cy="64.4" r="2.3" fill="#b29ae0" stroke="currentColor" stroke-width="0.9"/>
    <circle cx="21.6" cy="64.4" r="2.3" fill="#b29ae0" stroke="currentColor" stroke-width="0.9"/>
    <path d="M 10.6 50 C 6.2 49.4 2.4 45 1 37.6 C 4.8 38.2 9 40 12 42.6 Z" fill="#b29ae0" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>
    <path d="M 29.4 50 C 33.8 49.4 37.6 45 39 37.6 C 35.2 38.2 31 40 28 42.6 Z" fill="#b29ae0" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>
    <path d="M 9.8 47.6 C 6.6 47 4 44 2.9 40.2 C 5.8 41 8.4 42.4 10.6 44.4 Z" fill="#e7a6a6"/>
    <path d="M 30.2 47.6 C 33.4 47 36 44 37.1 40.2 C 34.2 41 31.6 42.4 29.4 44.4 Z" fill="#e7a6a6"/>
    <path d="M 18 37 C 17.4 32.6 20 29.6 24 30 C 21.6 31.6 21.2 34 22.2 37 Z" fill="#b29ae0" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/>
    <path d="M 20 35.6 C 27.2 35.6 31.2 40.2 31.2 46.6 C 31.2 52.8 26.6 57.4 20 57.4 C 13.4 57.4 8.8 52.8 8.8 46.6 C 8.8 40.2 12.8 35.6 20 35.6 Z" fill="#b29ae0" stroke="currentColor" stroke-width="2"/>
    <ellipse cx="15.0" cy="45.2" rx="3.8" ry="3.6" fill="#f5d04a" stroke="currentColor" stroke-width="1"/>
    <circle cx="16.3" cy="45.8" r="1.7" fill="currentColor"/>
    <ellipse cx="25.0" cy="45.2" rx="3.8" ry="3.6" fill="#f5d04a" stroke="currentColor" stroke-width="1"/>
    <circle cx="26.3" cy="45.8" r="1.7" fill="currentColor"/>
    <path d="M 11.8 40.6 L 18.8 42.8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>
    <path d="M 28.2 40.6 L 21.2 42.8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>
    <path d="M 11.6 49.4 Q 20 51.8 28.4 49.4 C 27.4 54.4 24 56 20 56 C 16 56 12.6 54.4 11.6 49.4 Z" fill="#7a1f24" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>
    <path d="M 12.6 50.2 Q 20 52.4 27.4 50.2 L 26.4 52.8 Q 20 54.4 13.6 52.8 Z" fill="#ffffff"/>
    <path d="M 16.6 51.4 L 16.8 53.6 M 23.4 51.4 L 23.2 53.6" fill="none" stroke="#7a1f24" stroke-width="0.9"/>
  </g>
```

## Suggested `Spec` (`room.rs`), when it's wired

```rust
const GREMLIN: Spec = Spec {
    name: "gremlin",
    pitch: line!("Hehehe. He looks like trouble!"),
    footprint: (2, 2),
    ascii: &["oo", "ww"],
    ink: (Color::Rgb(178, 154, 224), Color::LightMagenta),
    uses: &[],
    offers: &[Offer::Decor],
    sit: None,
    comfort: 1.0,
    // Not on the floor and not on the wall: on a shelf she built
    // (phase 6 structures). Placement needs the shelf strip first.
    hang: None,
    beauty: 1.0,
    symmetric: false,
};
```

- **`hang: None` is a placeholder.** He stands on a shelf strip, which is neither floor nor wall. That placement
  belongs to the structures step (a shelf is "a new strip: decor can stand on it"). I haven't invented a field for it.
- **Mirroring.** `symmetric: false`: facing left mirrors him, which flips his tail and glance. Nothing in him reads wrong
  mirrored (band 1 shows both facings).
- **Ink.** His body glaze `#b29ae0`, the largest flat colour, is `Rgb(178,154,224)`; the 16-colour ink is
  `LightMagenta`.
- **`parts()` arm** (`art.rs`): `(Furniture::Gremlin, _) => &["gremlin"]`. `drawn_facing` handles the mirror.
- **Other edits**, as for the plant and poster: the `Furniture` enum, `ALL`, `spec()`, `RoomKind::of` (`Living`), and
  the `CATALOGUE` / shopping-channel stock gated on shelves existing (proposal: "the gremlin waits for shelves").

### ASCII (2 × 2)

```
oo     eyes
ww     the toothy grin
──     (the shelf)
```

- `oo` / `ww` is mirror-safe (`room::mirror` maps each row to itself) and uses no `< > { }`. Band 1 shows it in his ink,
  drawn in the sheet's stand-in 5 × 7 font, beside an alternative.
- **Alternative: `/\` over `oo`**, the ears over the eyes. It's also mirror-safe: `/\` reversed and swapped is `/\`.
  Read as text, though, it's a roof over two eyes rather than a grin.
- **Also considered:** `^^`/`ww` (no eyes), `vv` for teeth (reads as a chevron), and `"` ears (too faint).

## Variants (on the sheet, band 1; also in the worktree's `props.svg`)

- **A, `gremlin` (recommended).** As above.
- **B, `gremlin-tall`.** The same imp lifted 2 px onto a two-step pedestal: a base 14 px wide and a top step 12 px
  wide, both snapped. It reads more as a trophy or ornament, at 2 px taller. The plinth is
  replaced by:

  ```svg
    <rect x="4.44" y="77.33" width="31.11" height="6.67" rx="1.2" fill="currentColor"/>
    <rect x="6.67" y="79.56" width="26.67" height="2.22" fill="#8a5f3c"/>
    <rect x="6.67" y="70.67" width="26.67" height="8.89" rx="1" fill="currentColor"/>
    <rect x="8.89" y="72.89" width="22.22" height="4.44" fill="#8a5f3c"/>
    <rect x="8.89" y="72.89" width="22.22" height="2.22" fill="#b07f52"/>
  ```

  The figure is wrapped in `<g transform="translate(0 -4.44)">`.
- **C, `gremlin-jade`.** The same drawing in a jade glaze: body `#7fc2a8`, shade `#5e9e86`, belly `#bfe3d4`. It reads as
  well as the lavender, but it's a green next to the plant's greens.
- **D, `gremlin-wide` (3 × 2 cells, 60 × 84).** The figure is shifted 4 px right (`translate(8.89 0)`, kept on the pixel
  grid), with long goblin ears out to the frame's edges:

  ```svg
    <path d="M 10.6 50.4 C 3 50.2 -4 46 -7.8 38.4 C -1.6 38.2 5 40 12 42.6 Z" .../>   (and mirrored about x = 20)
    <path d="M 9.8 48 C 3.6 47.8 -1.8 45 -5.2 40.4 C 0 40.6 5 42 10.6 44.4 Z" fill="#e7a6a6"/>
  ```

  At 1× the long ears are the most "gremlin" silhouette on the sheet: the 2-wide ears are only about 3 px each side.
  But it costs a third column, and the 2-wide one already reads. The figure is half a pixel off the frame's centre,
  because 27 px can't centre a 14 px plinth.

## Review sheets (this directory)

All are drawn with the `#1d1714` outline. **The outline is the same on light and dark backgrounds**: the guest has no
light-mode outline, and in truecolor the client paints its own dark background anyway. The light bands are a precaution for
any case where the terminal's own background shows through; that case was not tested.
- `gremlin-1x.png`: true on-screen size (9 × 19 px cells).
- `gremlin-1x-nn3x.png`: the same pixels at 3×, nearest-neighbour. **Judge readability here.**
- `gremlin-3x.png`: a native 3× render. Judge the drawing here.

Bands, top to bottom. Each is drawn on a dark terminal (`#1e2127`) and then again on a light one (`#f4f1ea`); the
numbers are at the left.
1. **On a shelf run** (a `─` line, drawn like the floor):
   - A facing right, then A facing left (mirrored);
   - B, the tall pedestal;
   - C, jade;
   - D, 3 wide with long ears;
   - then the ASCII candidates on a shelf in his ink: `oo`/`ww`, and `/\`/`oo`.
2. **A pane's back wall.** The plant stands on the floor. She stands under a shelf hung just over her head (at the
   floor − 5, so she can pass under it), with the gremlin on it. The clock (at 16:00) and the poster hang beside it at
   hang 4. This shows the scale: he stands about a third of her height.

To regenerate:

```sh
HOUSEGUEST_GREMLIN=/dir cargo test -p dessplay --lib gremlin_sheet -- --ignored
```

## What I tried and changed

I made the drafts with `resvg` from `gen.py`. They used the same geometry as `rasterize` (`xMidYMax meet`, so centred
and bottom-aligned), and each was judged at 1× ×8 nearest-neighbour, then again from the cargo test's output.
- **v1.** The ears were upright triangles, with a tuft of three spikes on top. At 1× it read as a cat with a crown.
  - The ears were turned out sideways and a little up (goblin ears, not cat ears).
  - The tuft became a single curl.
- **v2.** The hands-clasped oval on the chest, together with the pale belly and a curled tail, read as a mug or teapot
  with a handle at 1×.
  - The oval was replaced by short arms with the hands meeting, the "rubbing his hands" scheme. The arms' outline was
    thinned (0.4 units a side), so they turn into soft shading at 1× and not a dark smear.
  - The tail ends in a point.
  - A no-arms trial with the old curled tail was dropped: it was cleaner at 1×, but it looked like a teapot.
- **v3.** Slanted half-lid eyes, for a sly look, ate the yellow at 1×.
  - The eyes went back to big round ones (rx 3.8), with thinner pinched brows. A thin-brow trial and a no-brow trial
    were indistinguishable at 1×, so I kept the brows, which add character at 3×.
- **v4.** At 1× the head floated above the body, with a dark gap row between them.
  - The body was tucked up under the head, so only the chin's outline separates them.
  - The body was lightened to `#b29ae0` so the outline reads against `#1e2127`.
- **The plinth was snapped to the pixel grid**, for a crisp 1-px outline and a 1-px lit lip. B's pedestal top step was
  first off centre by a pixel, which was fixed before the final render.

No separate art-director critique was run; this is round 1.

## Open choices for the user

1. **2 wide (A) or 3 wide with long ears (D)?** I recommend A. D's ears are more unmistakably "gremlin" at 1×, but it
   takes a third column on the shelf.
2. **Plinth (A) or pedestal (B)?** The pedestal makes him more of an ornament, but taller.
3. **Glaze:** lavender (A) or jade (C)? Or another colour.
4. **ASCII:** `oo`/`ww` (eyes over a grin) or `/\`/`oo` (ears over eyes).
5. **His size.** The figure itself is about 30 px tall at 1×, about a third of her height, so as an ornament he's big.
   2 × 2 cells is the smallest frame where the face still reads. Smaller would need a 2 × 1 frame (9 px tall), which
   would be a blob.

## What ran

In the worktree:
- `cargo fmt`;
- `cargo clippy -p dessplay --lib --tests -- -D warnings`: clean;
- `cargo test -p dessplay --lib houseguest::art`: 13 passed, 12 ignored;
- the ignored `gremlin_sheet`, which wrote the three PNGs into this directory.

## The user's choice (2026-10-10)

**Approved: variant A** (lavender glaze, flat wooden plinth, 2 × 2 cells),
and the ASCII form `oo` / `ww`. Variants B, C and D are not to be wired;
their groups in `worktree.diff` stay out of `props.svg`.
