# Houseguest 5b art: the wall clock and the window (for review, not wired in)

Last updated: 2026-10-04

This is the model sheet for phase 5b's D7 (`phase5b-design.md`, "The window and the wall clock"). It needs
the user's approval before anything is wired in. The art was drawn in a throwaway worktree against master
`5fc9310` and committed there with git (`bba7efc`). `worktree.diff` in this directory is that commit:
SVG groups in `art/props.svg`, a little procedural code in `art.rs` and two tests. It adds no `Furniture`
variant, no `Spec`, no ASCII and no `Look`. `gen.py` generated the SVG groups and splices them into
`props.svg` (`python3 gen.py dessplay/src/ui/houseguest/art/props.svg`).

## Review sheets (this directory)

All three are drawn over `#1e2127`, with the outline in `#1d1714`. Hung pieces are rendered at exactly
their footprint (`9·cols × 19·rows` px), the way the game draws a piece that isn't on the floor line.

- `clock-window-1x.png`: true on-screen size (9 × 19 px cells).
- `clock-window-1x-nn3x.png`: the same pixels at 3×, nearest-neighbour. Use it to judge what reads at 1×.
- `clock-window-3x.png`: a native 3× render. Use it to judge the drawing.

Bands, top to bottom:

1. **The 4 × 2 window** in its five skies: night, dawn, day, dusk, evening. Then night again, asked for
   facing left, which draws the same.
2. **The 5 × 3 window**, laid out the same way.
3. **The 3 × 2 dial**: all 48 faces. The columns are the hours (12, 1, …, 11) and the rows are the
   quarters (:00, :15, :30, :45).
4. **The 4 × 2 dial**, laid out the same way.
5. **Facings.** Each clock at 3:00 three times: facing right; asked for facing left (normalised, so
   identical); and naively mirrored the way `render_piece`'s mirror group would draw it. The mirrored one
   reads 9:00 and is underlined red. Then the window at dusk facing right and asked for facing left.
6. **Context.** Each strip has the poster and both new pieces hung over the sofa (hang 4: their bottom
   row is floor − 5), with her in `Pose::Gaze` / `Face::Curious` under the window, facing it:
   - 16:00 with the day sky;
   - 23:00 with the night sky;
   - the 4 × 2 clock and the 5 × 3 window at 18:45, dusk.

   The clocks in the context strips are asked to face left, and draw unmirrored.

To regenerate:

```sh
HOUSEGUEST_CLOCK=/dir cargo test --release -p dessplay --lib clock_sheet -- --ignored
```

## Footprints: recommended 3 × 2 clock and 4 × 2 window

**Clock: 3 × 2 (60 × 84 units).** The face is 26 px across at 1×, and it tells the time at a glance on
the 1× sheet:
- the minute hand is 1 px wide and 8 px long; the hour hand is about 1.6 px wide and 5 px long;
- there are bold bars at 12, 3, 6 and 9;
- in the context strips, 4:00, 11:00 and 6:45 read without effort.

The 4 × 2 clock (36 px) is nicer at 3×, but it is as big as the poster and the window. On the wall it
reads as the room's centrepiece instead of a clock, and it costs a wall cell (poster 4 + clock 3 +
window 4 = 11 cells, against 12). If the user's terminal turns out to be HiDPI (18 × 38 cells), 3 × 2 is
comfortable. If it's 1× and the user finds it fiddly, 4 × 2 is a drop-in: same groups, prefix `clock4`.

**Window: 4 × 2 (80 × 84 units).** All five skies are obvious at 1× from colour alone:
- night is navy with a moon;
- dawn is pastel bands;
- day is sky blue with a sun and a cloud;
- dusk is purple over pink over orange;
- evening is a lighter blue with a violet band and one star.

The 5 × 3 window has more room for the moon and the skyline, but it needs `hang + rows` = 7 clear rows.
That is exactly `home_screen`'s 9-high pane, so any shorter pane puts it in the closet (map, room-art
§3/trap 8). It also towers over the poster. I don't think the extra sky earns that.

## Design choices

### Clock
- **The body (`clock`):** a round red-rimmed wall clock: a dark outline disc, a `#d9655b` rim with one
  hard shade (`#b84e47`) on its lower half, a thin dark inner line, and a cream face (`#fbf6ea`).
  - The rim red is the poster's title-band red, so the two read as one room.
  - The face has bold dark bars at 12/3/6/9, faint dots (`#b3a48e`) at the other hours (they show only
    from 2× up), and a white glint arc on the glass, upper left.
- **The hands:** separate groups (`clock-hand-hour`, `clock-hand-minute`), each drawn pointing at 12 from
  the face centre (30, 42.89). The dial rotates them about that point. The red `clock-pin` caps both.
- **Pixel geometry:** the face centre sits on a pixel centre at 1× (px 13.5, 19.5). So the 1-px minute
  hand is crisp at :00, :15, :30 and :45, and so are the 12/3/6/9 bars. The 4 × 2 variant centres on a
  pixel corner instead, for its 2-px hands.
- **Hour-hand creep:** the hour hand advances 7.5° a quarter, as a real clock's does. That shows from 2×
  up, but not on a 5-px hand at 1×. The minute hand carries the quarter at 1×.
- **Hands close together** (6:30, 2:45, 9:15, 12:00) read like any real clock's: the thick short hand
  over the thin long one. At 1× 6:30 is the weakest face, nearly one hand pointing down.
- **The ink floor** (w·h/6) holds in every state; the new lint checks it.

### Window
- **The frame (`window`):** a cream (`#f3ead8`, the poster's paper) two-pane sliding window, as in a
  Japanese flat.
  - The sashes overlap in the middle, so the centre bar is wider than the sides, with a small brass latch.
  - A white reveal line runs along the top inside edge.
  - A sill is wider than the frame, with a shaded front edge.
  - Every edge sits on a whole pixel at 9 × 19 (`gen.py`'s `Grid`: x px → x·20/9 units, y px →
    (y − 0.1·rows)/0.45 units). Outlines are filled 1-px bands rather than strokes, so they are crisp
    at 1×.
- **The skies (`window-sky-<phase>`):** flat bands, no gradients, each group clipped to `window-glass`.
  Each has a low skyline (flats, plus one gabled house in the left pane) in that phase's silhouette
  colour:
  - **night:** navy `#2c3f72` (clearly lighter and bluer than the terminal), a crescent moon, four stars
    (one twinkling), one lit window;
  - **dawn:** pale blue over pink over pale yellow, with a pale sun just clearing the roofs on the left;
  - **day:** sky blue, an outlined sun upper right, a cloud upper left, a bird, green-grey roofs;
  - **dusk:** purple over rose over orange, with a big yellow sun half down behind the roofs on the
    right. It is purple-dominant on purpose, so it doesn't repeat the poster's peach-and-orange sunset
    beside it;
  - **evening:** deep blue with a violet afterglow band, one first star, and three lit windows (people
    home; at night only one is still up).
- **Clipping (verified):** usvg honours `clip-path` on a `<g>` inside `<defs>` reached through `<use>`.
  The moon and sun clip cleanly to the glass. So the window needs **no procedural branch**: the sky is a
  static state, and `state_parts` can return `&["window-sky-dusk", "window"]` and so on.

### Mirroring
- Neither piece is ever mirrored. The sheet's `dial_scene` and `window_scene` take a `Facing` and ignore
  it, as defence in depth behind D7's `Spec.symmetric` normalisation in `prop_layer`/`piece_look`.
- The lint `every_dial_and_sky_renders_inside_its_footprint_unmirrored` asserts three things:
  - Left and Right render pixel-identical for all 96 quarter-hours of a day (both clocks) and all five
    skies (both windows);
  - the ink floor holds;
  - the 48 faces of a 12-hour day are 48 distinct images.
- The lint names `CLOCK4` and `window5` too. Whoever drops the losing variants must trim it, or it won't
  compile.

### What ran

In the worktree, all on `bba7efc`:
- `cargo clippy --release -p dessplay --all-targets -- -D warnings`: clean.
- `PROPTEST_CASES=32 cargo nextest run -p dessplay`: 1541 passed, 19 skipped.
- `clock_sheet`, the ignored test that writes the sheets.

## SVG groups (recommended pieces; paste into `art/props.svg` inside `<defs>`, after `poster`)

Add to the header comment's frame list: "clock 60 × 84 (3 × 2), window 80 × 84 (4 × 2); both hang on
the wall and are never mirrored". The 4 × 2 clock (`clock4*`) and the 5 × 3 window (`window5*`) are in
`worktree.diff`. Drop whichever loses.

```svg
  <!-- A round wall clock, 3 x 2 cells (60 x 84). The dial rotates
       clock-hand-hour and clock-hand-minute about the face centre
       (30, 42.89); it is never mirrored. -->
  <g id="clock">
    <circle cx="30" cy="42.89" r="28.89" fill="currentColor"/>
    <circle cx="30" cy="42.89" r="26.67" fill="#d9655b"/>
    <path d="M 55.06 52.01 A 26.67 26.67 0 0 1 4.94 52.01 L 30 42.89 Z" fill="#b84e47"/>
    <circle cx="30" cy="42.89" r="22.89" fill="currentColor"/>
    <circle cx="30" cy="42.89" r="21.33" fill="#fbf6ea"/>
    <rect x="28.89" y="21.78" width="2.22" height="4.44" fill="currentColor"/>
    <circle cx="39.67" cy="26.15" r="1.11" fill="#b3a48e"/>
    <circle cx="46.74" cy="33.22" r="1.11" fill="#b3a48e"/>
    <rect x="46.67" y="41.78" width="4.44" height="2.22" fill="currentColor"/>
    <circle cx="46.74" cy="52.56" r="1.11" fill="#b3a48e"/>
    <circle cx="39.67" cy="59.63" r="1.11" fill="#b3a48e"/>
    <rect x="28.89" y="59.56" width="2.22" height="4.44" fill="currentColor"/>
    <circle cx="20.33" cy="59.63" r="1.11" fill="#b3a48e"/>
    <circle cx="13.26" cy="52.56" r="1.11" fill="#b3a48e"/>
    <rect x="8.89" y="41.78" width="4.44" height="2.22" fill="currentColor"/>
    <circle cx="13.26" cy="33.22" r="1.11" fill="#b3a48e"/>
    <circle cx="20.33" cy="26.15" r="1.11" fill="#b3a48e"/>
    <path d="M 23.92 26.18 A 17.78 17.78 0 0 0 14.6 34" fill="none" stroke="#ffffff" stroke-width="1.78" stroke-linecap="round"/>
  </g>
  <g id="clock-hand-hour">
    <path d="M 30 45.56 L 30 31.78" fill="none" stroke="currentColor" stroke-width="3.56" stroke-linecap="round"/>
  </g>
  <g id="clock-hand-minute">
    <path d="M 30 46.44 L 30 25.11" fill="none" stroke="currentColor" stroke-width="2.22"/>
  </g>
  <g id="clock-pin">
    <circle cx="30" cy="42.89" r="2.22" fill="#d9655b" stroke="currentColor" stroke-width="1"/>
  </g>
```

The window: a `clipPath` for its glass, the frame, then the five sky groups (each carries its own
skyline polygon).

```svg
  <!-- A two-pane sliding window, 4 x 2 cells (80 x 84), hung on the
       wall: the sky groups (window-sky-*) go behind the frame and
       clip themselves to window-glass. Never mirrored. -->
  <clipPath id="window-glass">
    <rect x="13.33" y="15.11" width="53.33" height="46.67" fill="#000"/>
  </clipPath>
  <g id="window">
    <path d="M 4.44 6.22 H 75.56 V 70.67 H 4.44 Z M 13.33 15.11 H 66.67 V 61.78 H 13.33 Z" fill="currentColor" fill-rule="evenodd"/>
    <path d="M 6.67 8.44 H 73.33 V 68.44 H 6.67 Z M 11.11 12.89 H 68.89 V 64 H 11.11 Z" fill="#f3ead8" fill-rule="evenodd"/>
    <rect x="6.67" y="8.44" width="66.67" height="2.22" fill="#ffffff" fill-opacity="0.55"/>
    <rect x="35.56" y="15.11" width="8.89" height="46.67" fill="currentColor"/>
    <rect x="37.78" y="15.11" width="4.44" height="46.67" fill="#f3ead8"/>
    <rect x="37.78" y="36.22" width="4.44" height="4.44" fill="#c9a14a"/>
    <rect x="2.22" y="68.44" width="75.56" height="11.11" fill="currentColor"/>
    <rect x="4.44" y="70.67" width="71.11" height="6.67" fill="#f3ead8"/>
    <rect x="4.44" y="75.11" width="71.11" height="2.22" fill="#d8ccb4"/>
  </g>
  <g id="window-sky-night">
    <g clip-path="url(#window-glass)">
      <rect x="13.33" y="15.11" width="53.33" height="46.67" fill="#2c3f72"/>
      <circle cx="23.33" cy="29.11" r="5.87" fill="#f6e7a8"/>
      <circle cx="26.56" cy="27.35" r="4.99" fill="#2c3f72"/>
      <rect x="31.11" y="44" width="2.22" height="2.22" fill="#fff4c2"/>
      <rect x="51.11" y="24" width="2.22" height="2.22" fill="#fff4c2"/>
      <rect x="48.89" y="24" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="53.33" y="24" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="51.11" y="21.78" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="51.11" y="26.22" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="62.22" y="35.11" width="2.22" height="2.22" fill="#fff4c2"/>
      <rect x="53.33" y="44" width="2.22" height="2.22" fill="#fff4c2"/>
      <path d="M 13.33 61.78 L 13.33 57.33 L 17.78 57.33 L 17.78 55.11 L 24.44 50.67 L 31.11 55.11 L 31.11 55.11 L 44.44 55.11 L 44.44 50.67 L 51.11 50.67 L 51.11 55.11 L 55.56 55.11 L 55.56 48.44 L 62.22 48.44 L 62.22 57.33 L 66.67 57.33 L 66.67 61.78 Z" fill="#18223f"/>
      <rect x="55.56" y="50.67" width="2.22" height="2.22" fill="#f6d48a"/>
    </g>
  </g>
  <g id="window-sky-dawn">
    <g clip-path="url(#window-glass)">
      <rect x="13.33" y="15.11" width="53.33" height="46.67" fill="#a9c7ec"/>
      <rect x="13.33" y="35.11" width="53.33" height="26.67" fill="#f7bfcc"/>
      <rect x="13.33" y="46.22" width="53.33" height="15.56" fill="#ffe0a6"/>
      <circle cx="23.33" cy="52.89" r="5.33" fill="#fff6d6"/>
      <path d="M 13.33 61.78 L 13.33 57.33 L 17.78 57.33 L 17.78 55.11 L 24.44 50.67 L 31.11 55.11 L 31.11 55.11 L 44.44 55.11 L 44.44 50.67 L 51.11 50.67 L 51.11 55.11 L 55.56 55.11 L 55.56 48.44 L 62.22 48.44 L 62.22 57.33 L 66.67 57.33 L 66.67 61.78 Z" fill="#9a88b0"/>
    </g>
  </g>
  <g id="window-sky-day">
    <g clip-path="url(#window-glass)">
      <rect x="13.33" y="15.11" width="53.33" height="46.67" fill="#7cc4ec"/>
      <circle cx="57.33" cy="29.11" r="5.87" fill="#ffd84a" stroke="currentColor" stroke-width="1.11"/>
      <circle cx="18.85" cy="39.56" r="3.73" fill="#ffffff"/>
      <circle cx="24.08" cy="36.95" r="4.67" fill="#ffffff"/>
      <circle cx="28.56" cy="39.56" r="3.55" fill="#ffffff"/>
      <rect x="15.12" y="39.56" width="16.99" height="3.55" fill="#ffffff"/>
      <path d="M 47.78 41.82 Q 49.56 41.38 51.11 44.04 Q 52.67 41.38 54.44 41.82" fill="none" stroke="#33405a" stroke-width="1.56" stroke-linecap="round" stroke-linejoin="round"/>
      <path d="M 13.33 61.78 L 13.33 57.33 L 17.78 57.33 L 17.78 55.11 L 24.44 50.67 L 31.11 55.11 L 31.11 55.11 L 44.44 55.11 L 44.44 50.67 L 51.11 50.67 L 51.11 55.11 L 55.56 55.11 L 55.56 48.44 L 62.22 48.44 L 62.22 57.33 L 66.67 57.33 L 66.67 61.78 Z" fill="#5d8f7c"/>
    </g>
  </g>
  <g id="window-sky-dusk">
    <g clip-path="url(#window-glass)">
      <rect x="13.33" y="15.11" width="53.33" height="46.67" fill="#7a4f9a"/>
      <rect x="13.33" y="32.89" width="53.33" height="28.89" fill="#d0637e"/>
      <rect x="13.33" y="46.22" width="53.33" height="15.56" fill="#f2924a"/>
      <circle cx="54.44" cy="52.89" r="8" fill="#ffcf5a"/>
      <path d="M 13.33 61.78 L 13.33 57.33 L 17.78 57.33 L 17.78 55.11 L 24.44 50.67 L 31.11 55.11 L 31.11 55.11 L 44.44 55.11 L 44.44 50.67 L 51.11 50.67 L 51.11 55.11 L 55.56 55.11 L 55.56 48.44 L 62.22 48.44 L 62.22 57.33 L 66.67 57.33 L 66.67 61.78 Z" fill="#4a2c52"/>
    </g>
  </g>
  <g id="window-sky-evening">
    <g clip-path="url(#window-glass)">
      <rect x="13.33" y="15.11" width="53.33" height="46.67" fill="#36579a"/>
      <rect x="13.33" y="44" width="53.33" height="17.78" fill="#6a5ea8"/>
      <rect x="57.78" y="26.22" width="2.22" height="2.22" fill="#fff4c2"/>
      <rect x="55.56" y="26.22" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="60" y="26.22" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="57.78" y="24" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <rect x="57.78" y="28.44" width="2.22" height="2.22" fill="#fff4c2" fill-opacity="0.55"/>
      <path d="M 13.33 61.78 L 13.33 57.33 L 17.78 57.33 L 17.78 55.11 L 24.44 50.67 L 31.11 55.11 L 31.11 55.11 L 44.44 55.11 L 44.44 50.67 L 51.11 50.67 L 51.11 55.11 L 55.56 55.11 L 55.56 48.44 L 62.22 48.44 L 62.22 57.33 L 66.67 57.33 L 66.67 61.78 Z" fill="#232a52"/>
      <rect x="22.22" y="57.33" width="2.22" height="2.22" fill="#f6d48a"/>
      <rect x="46.67" y="52.89" width="2.22" height="2.22" fill="#f6d48a"/>
      <rect x="55.56" y="50.67" width="2.22" height="2.22" fill="#f6d48a"/>
    </g>
  </g>
```

## The procedural dial (in `worktree.diff`, `art.rs` after `render_tv`)

```rust
pub(super) struct Dial { pub hour: u8, pub quarter: u8 }      // 48 faces
impl Dial {
    pub(super) fn at(minute: u16) -> Self;                      // minute of day, floored to its quarter
    fn angles(self) -> (f32, f32) {                             // clockwise from 12
        let q = f32::from(self.quarter % 4);
        (f32::from(self.hour % 12) * 30.0 + q * 7.5, q * 90.0)
    }
}
struct ClockArt { prefix: &'static str, frame: (f32, f32), centre: (f32, f32) }
const CLOCK: ClockArt = ClockArt { prefix: "clock", frame: (60.0, 84.0), centre: (30.0, 42.888_89) };

fn dial_parts(art: &ClockArt, dial: Dial) -> String {
    // <use #clock/>, <use #clock-hand-hour rotate(h cx cy)/>,
    // <use #clock-hand-minute rotate(m cx cy)/>, <use #clock-pin/>
}
fn dial_scene(art: &ClockArt, dial: Dial, _facing: Facing, line: &str) -> String; // never mirrored
```

When it's wired in, `ClockArt` collapses to constants for the chosen footprint. The dial reaches the
cache as `PieceState::Dial(Dial)` (map seam 3a) or `Look::Clock(Dial)` (3b). Either way it needs a
procedural arm in `render_piece` (or `Look::render`), like `render_tv`, because `state_parts` returns
static ids. The sky needs no such arm. The sheet's `Sky` enum (`Night, Dawn, Day, Dusk, Evening`, with
`id()`) maps onto static `state_parts` arms.

## ASCII and inks (suggestions for the `Spec` rows; not drawn)

- Clock: `.-.` / `(o)`. Hand glyphs go in an override (`o` → one of `'` `>` `.` `<` for 12/3/6/9 by the
  minute hand, or by the hour hand; your call). Ink: the rim, `Rgb(217, 101, 91)`, 16-colour `LightRed`.
- Window: `.--.` / `|  |`, with the two inner cells as sky overrides (` o` day, `~~` dusk, `*.` night)
  and a per-cell ink from the overlay seam (map seam 4). Ink: the frame cream, `Rgb(243, 234, 216)`,
  16-colour `White`.
- Both are symmetric, so normalised facing keeps `(`/`)` from flipping.

## Open questions for the user

1. **Footprints.** Are 3 × 2 for the clock and 4 × 2 for the window right? The alternatives are 4 × 2 and
   5 × 3, both on the sheet.
2. **The clock's look.** I chose a red-rimmed kitchen clock (red to match the poster's band). A
   teal-rimmed one would match the sofa instead, and a plain dark "school clock" rim would tie in with
   "Time for school!" but has less character. Any preference?
3. **The skyline.** Keep the little town in the window (it grounds the view and carries the lit windows
   at night), or show plain sky? It's 2–6 px tall at 1×.
4. **The phase boundaries** aren't settled in D7. A suggestion, by game minute: dawn 05:00–07:00, day
   07:00–17:00, dusk 17:00–19:00, evening 19:00–21:00, night 21:00–05:00. A new ledger starts at 16:00,
   so dusk comes within the first session, as the map's O5 wants.
5. **Her LookOut spot.** Gaze is a profile pose that looks up the way she faces. Standing dead centre
   under a hung window, she looks past it. On the sheet her box is a little left of the window's middle,
   facing right, and that reads as looking out. The new `seat()` arm should do the same: offset toward
   the side she faces from. The sheet places her by eye, though, and is not a `seat()` spec. Her 5-wide
   box overlaps the sofa's last cell there, which is fine for a picture but not the seat's geometry.
6. **The 1× hour hand.** Its quarter-hour creep is invisible at 1× (fine at 2×+). Accept that, or quantise
   the hour hand to whole hours for 12 dials × 4 minute positions? The image count is the same: 48.
