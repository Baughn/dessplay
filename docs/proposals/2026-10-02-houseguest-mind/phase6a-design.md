# Phase 6a (shelves and the gremlin): implementation design

**Working design, 2026-10-10, before critiques; amended the same day with the user's answers (*User decisions*,
second round).** Brief: docs/plan.md, Phase 38, "Phase 6 — structures (brief,
2026-10-10)" (plan.md wins over everything here). Code maps at `60727649`: [phase6/map-terrain.md](phase6/map-terrain.md)
and [phase6/map-life.md](phase6/map-life.md); this design overrides them where it says so. Proposal:
[../2026-10-02-houseguest-mind.md](../2026-10-02-houseguest-mind.md), *Structures from the UI's own lines*, *Rooms from
contents*, *Rules, not an objective*. The gremlin's art is **approved** (variant A: [phase6/art/snippets.md](phase6/art/snippets.md),
its last section; `worktree.diff` holds the drawing, of which only the `gremlin` group is wired, step 7). Paths are relative to `dessplay/src/ui/houseguest/` unless noted. Every
line cited was checked at `4c5c7151` (`dessplay/` unchanged since the maps).

**Geometry words**, used throughout. A **pane strip** is today's `Strip::Bottom(nook)` with its raw `Extent { from, to,
floor: f, rows }` (room.rs:741; interior rows `f-rows ..= f-1`). `WIDTH` = 5, `HEIGHT` = 4 (sprite.rs:9-11). Her box
standing on `f` is rows `f-4 ..= f-1`. The **wall lane** is where hung pieces hang: the poster and the clock (hang 4, 2
rows tall: room.rs:310, 325) take rows `f-6 ..= f-5`; the window (hang 1) rows `f-3 ..= f-2`. A shelf's **rise** is how
many rows above its pane's floor it hangs (its row `y = f - rise`); its **host** is the pane strip it hangs in. A **high**
shelf hangs above the wall lane (rise ≥ `HIGH_RISE` = 7); a **low** one just over her head (rise `LOW_RISE` = 5), in the
wall lane's rows, so the poster and the clock pack beside it. Its **block** is a low shelf's columns across the wall
lane's rows `f-6 ..= f-5`: where no hung piece may hang. **Small
decor** is what may stand on a shelf (`Spec.shelf`: the plant, the clock, the gremlin).

## User decisions

From plan.md (2026-10-10), overriding the proposal:
- **Split in three**; this is 6a, shelves and the gremlin. 6b (the stepladder, the pole check) and 6c (partitions, rooms
  as regions, `Connected`) are out of scope.
- **Pacing is per game day**: one build stage a game day, the counts saved in the ledger (a restart doesn't reset them).
- **Structures are bought kits**: the channel sells a shelf kit like any piece; it comes boxed through her door's flap;
  she unpacks it and puts it up in stages. A **wishlist** puts the kit a felt need wants first in the queue, at the
  channel's usual cadence.
- **A shelf hangs on the back wall, off the sides**: a `─` run at least a body-width from either side wall, so nothing
  reaches it until 6b's ladder.
- **Door-batch rules hold**: no structure in the chat pane or her door's space, and no shelf above her door's space.
- **On a shelf**: a new **gremlin** figurine (model sheet first) and small decor (the plant, the clock); never furniture.
- **6a's payoff**: decor on a shelf counts toward the room's beauty, and she sometimes stands under it and gazes up (a
  still act, inside 5c's band).
- Every 6a step keeps the golden hashes and terrain snapshots unchanged in a home with no structure.

The user's answers to this design's questions (2026-10-10, second round), overriding anything below that says otherwise:
- **Shelf height: "Highest that fits."** A shelf hangs above the wall decor (high: never meeting the poster or the
  clock, with room for her to stand on it in 6b) where the pane is tall enough; otherwise just over her head (low),
  with the poster and the clock packing beside it on the wall. Both are built (D3 and D3a; steps 4 and 5).
- **The plant and the clock go up only by a repair** that finds the shelf the best place for them; a clock on a shelf
  is a mantel clock, same art. The gremlin always wants the shelf (`High`).
- **Lines as proposed**, all four: `"A shelf! Up high!"`, `"Aww, a little gremlin!"`, `"It wants a shelf..."`, `"Up it
  goes..."`.
- **The gremlin's art: variant A** (lavender glaze, flat wooden plinth, 2 × 2 cells), ASCII `oo` / `ww`; variants B, C
  and D are never wired. Step 7 no longer waits on approval.

## Decided unless the user objects

Each is recorded in decisions.md when it lands.
- **Two heights, high first** (the user's "highest that fits"). High: rise `HIGH_RISE` = 7, the first row above every
  hung piece (derived from the `Spec` table, pinned by a test), with `HEIGHT` clear rows above it when it's put up;
  hung pieces and a high shelf never meet by construction. Low: rise `LOW_RISE` = `HEIGHT` + 1 = 5, the row just over
  her head, with `LOW_ABOVE` = 2 clear rows above it (the gremlin's height, the clock's too: the plant, 3 tall, stands
  on a low shelf only where a third row is clear, else `Extent::holds` keeps it off), so a low shelf needs a pane 9
  rows tall (7 inside: The List in the bundled layout at 100×30 qualifies by geometry), a high one 13 (11 inside). The
  height is chosen once, when it goes up, and saved as its rise (D3).
- **A low shelf's block is the wall's**: hung pieces (the poster, the clock) keep out of it as the window keeps clear
  of what stands (`hung_clear`, generalised: D3a), hanging at the nearest clear place in their order or, with none,
  alone in the closet. A low shelf is only ever put up where it meets no hung piece as laid out, so going up never
  pushes one; later deliveries and moves pack beside it. The window (rows `f-3 ..= f-2`) never meets either height.
- **6b's standing** needs `HEIGHT` clear rows over a shelf: every high shelf has them when it goes up; a low one only in
  a pane at least 11 rows tall (9 inside). A low shelf in a shorter pane is for decor only, and 6b's ladder trigger
  ("she wants up there") must name a shelf she could stand on.
- **`SHELF_WIDTH` = 10** (the gremlin, the plant and the clock side by side, and at least the 7 cells a platform needs
  for 6b); one shelf a pane at most, and in 6a one shelf in all (one of each piece: one kit).
- **One fixed ink** for structure glyphs (`SHELF_INK`, the bundled theme's unfocused-border grey, looked up in
  `style.css` in step 2 and pinned), because a stroke's colour is part of her image's cache key (map-terrain §5).
- **Where it goes is chosen on the frame** when she unpacks the kit (blank cells now, the pane she'd want), then saved
  (host, anchor from the nearer wall, rise). It's never chosen again: a frame that can't hold it (text or a protected
  pane over it, its pane hidden, too narrow or short, the chat or her door's space come to meet it) hides it and
  what stands on it for that frame, as the closet hides a piece. Rejected: choosing it text-blind like her door (text
  fills most panes from the top, so a shelf chosen over text would mostly be hidden).
- **Two build stages** after unpacking: the first puts up the half of the run nearer its wall, the second the rest. A
  stage is a 6 s act standing under it, arms up (`Pose::Stretch`, existing art). Unpacking is no stage (it's the
  existing `Unpack`), so a kit unpacked on a Tuesday afternoon is half up that day and done on Wednesday.
- **Building is not a home act**: the per-day pacing bounds it, not her mood's 0–3 cap (else a lazy streak stalls it).
- **She can't stand on a shelf in 6a**, structurally: shelf cells are no ledge of hers (D5). Pieces still stand on it,
  because `fits` reads the glyph in the frame, not the terrain.
- **The gremlin stands anywhere small decor stands**, on a floor too; it **wants up high** (a new rule, `High`), felt
  on sight. That felt rule is what the wishlist reads: with no shelf, the shelf kit goes first in the queue; with a
  shelf up, the rule's repair carries the gremlin up. Rejected: a gremlin that may only stand on a shelf (its parcel
  comes in on the floor, so it would need a carry anyway, and the story would lose its "It wants a shelf...").
- On a shelf every piece stands: the clock stands on it as a mantel clock, same art (the user's answer above). The plant
  and the clock go up only by a repair (InChat, DoorClear), rare in practice.
- **The gaze up is a use** of what stands on a shelf, `Use::Admire`, offered only for a piece on a shelf, from the
  floor under it, `Pose::Gaze` held (existing art). Using her things in a pretty room already eases *beauty*
  (design.md:1571-1577); no new need.
- **The kit is sold last** (after furniture and decor) unless wished, and only on a frame where a shelf could go up
  now (D7), so a boxed kit, which holds the channel up as any box does, never waits on a short or text-filled pane.
- **Where the lines go** (approved above): the kit's pitch, the gremlin's pitch (not the sheet's suggested `"Hehehe. He
  looks like trouble!"`, which is over 24 characters), the grievance, and the line at a build stage's start.
- **Older builds**: they ignore `structures` and drop it when they save; a piece on a shelf falls back to its room's
  pane (its `at` is written as its share along the host pane, so it lands under where the shelf was); the gremlin and
  the kit are skipped as unknown kinds.

## Design

### D1. Strips: a shelf variant, one admission rule, no zips

**Types** (room.rs:629):

```rust
pub(super) enum Strip {
    Bottom(Nook),
    /// The shelf she put up in `nook`'s pane (one a pane at most).
    Shelf(Nook),
}
impl Strip {
    /// The pane it's in (total: every strip has one).
    pub fn nook(self) -> Nook;
    /// The room it's in: its pane's floor. A shelf is part of the room below, never a room.
    pub fn room(self) -> Strip;        // Strip::Bottom(self.nook())
    /// Whether `item` may stand on it: a pane's floor takes anything; a shelf, small decor only.
    pub fn takes(self, item: Furniture) -> bool;
}
```

- **The seven irrefutable `let Strip::Bottom(nook) = ..`** (ledger.rs:205, 365, 553; rules.rs:433; mod.rs:1624, 4341;
  room.rs:2321) all need only the pane: each becomes `.nook()` (ledger.rs:205 then falls back to a shelf piece's own
  pane, which is right). rules.rs:433 and mod.rs:1624 print the strip (`Users shelf`), so the stage shows it.
- **No zips.** `raw_strips` (room.rs:805) becomes `pane_strips(nooks) -> Vec<PaneStrip>` with `PaneStrip { nook, rect,
  strip, raw }`; `choose` (room.rs:1279) and `doorstep` (room.rs:2429) read `rect` from it instead of zipping
  `plan.nooks` with the strips by position. Shelves never enter `pane_strips`: they come only from `Home::extents`
  (D6), so the misalignment class can't recur.
- **Floors-only paths take a pane by type.** `move_off(&mut self, from: Nook, ..)` (room.rs:2196) iterates
  `pane_strips` for targets; `project_with` (room.rs:2159) moves only `Strip::Bottom` strips off (a shelf whose plan is
  gone hides its pieces; it never moves them). `doorstep`, `spot` (stage gifts), `stage_arrange` (mod.rs:4329) and the
  door's chooser iterate `pane_strips`. No parcel, gift or door ever lands on a shelf.
- **One admission rule**, `Strip::takes`, read in exactly two places:
  1. `Home::on` (room.rs:1839), which every layout reads (`lay_strip` room.rs:2019): a piece a strip doesn't take is
     never laid out there. So every search that judges a candidate by the layout refuses it with no filter of its own:
     the repair's `evaluate` ("a piece left out", rules.rs:605), `spot`'s `find(item)?` (room.rs:2391), `admits`.
  2. `StripPlan::takes(item)`, which every per-piece enumeration of plans calls before generating candidates (a cost
     saving, and the closet's agreement with the search): `rules::search`'s `Places` loop (rules.rs:930), `placements`
     (room.rs:978) and its `on_own_strip`/`packed_on`.
  The ledger read refuses it too (D2). `Extent::holds` alone would admit a sofa on a 10-wide shelf, so geometry is no
  guard.
- **Lanes depend on the strip.** On a shelf everything stands: `lift(item, boxed, strip)` (room.rs:697) is 0 on a
  shelf; `Prop::lane`/`needs` (room.rs:708-722), `Shown::lift`/`lane` (room.rs:1448-1462) pass their strip. The
  search's `plan.along(old.lane())` (rules.rs:932) becomes the lane the piece would have on the candidate strip, and its
  `holds((cols, rows))` the rows it would need there.
- **Where she stands to reach a piece.** `StripPlan` gains `host: Extent` (the pane's raw extent; a pane plan's own
  `raw`). `stand(prop, plan, left)` (room.rs:2684) sets a new `Shown::stood: i32`, the floor she stands on to reach it
  (`host.floor`). `reach` (mod.rs:4273), `Shown::seat` (room.rs:1534 `y: self.floor`) and `room_for` read `stood`;
  for a pane piece it equals `floor`, so nothing moves. `StripPlan::pin`'s share (`at`) is of `host` (D2's older-build
  note).
- **Rooms are floors.** `Home::rooms` (room.rs:1663), `role_among` (room.rs:616), `has_room`/`becomes_den`
  (rules.rs:575-597) and `Before.roles` (rules.rs:548) key on `strip.room()`, iterating pane plans only, so a shelf is
  never a room of its own (a decor-only shelf would read as a den). `Faces`, `Near`, `Apart`, `AgainstWall` and
  `Belongs` never involve small decor (it offers only `Decor` and forbids nothing), so they stay on `strip`.
- `DoorWall` (room.rs:827) deserializes through `#[serde(try_from = ..)]` refusing a shelf strip, so a garbled record
  can't put her door on one; the chooser only ever makes `Bottom` walls.

### D2. The structure model and the ledger

```rust
/// Something she has put up with the UI's own lines (phase 6). 6b adds the ladder, 6c the partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum Structure { Shelf(Shelf) }

/// A shelf on `nook`'s back wall: `SHELF_WIDTH` cells from `anchor` (its near end, counted from that wall of the
/// pane's raw strip), `rise` rows above the pane's floor; `built` of `SHELF_STAGES` stages up.
pub(super) struct Shelf { pub nook: Nook, pub anchor: Anchor, pub rise: u16, pub built: u8 }
```

- `Home { props, door, structures: Vec<Structure> }` (room.rs:1631). `Hash`/`PartialEq` pick it up (the mend basis and
  `paint_empty`'s `changed`). `Home::owns(Furniture::ShelfKit)` is true while the kit is a prop **or** a shelf stands
  (one of each). `Home::shelf(nook)`, `Home::shelves()` read it.
- **Ledger** (ledger.rs, by the pattern of `door` and `seen`): `Saved.structures: Vec<Structure>` with
  `skip_serializing_if = "Vec::is_empty"`, declared before `door` (so `the_clock_sent_round_trips`, which asserts the
  JSON ends with `clock_sent`, holds); `Raw.structures: Option<serde_json::Value>`, read entry by entry: an entry that
  isn't a structure this build knows (a later build's ladder, garbage) is skipped; a second shelf on one pane, a
  `built` above `SHELF_STAGES` or a rise of 0 is skipped. Step 4 adds `paced` the same way (D7).
- **Pieces on shelves.** `SavedAnchor.strip` is a `Strip`, so a shelf piece writes `{"Shelf":"Users"}`. On read, a
  piece whose anchor names a shelf the record has no shelf for, or a shelf that doesn't `take` it (garbage), keeps
  only its pane (`strip.nook()`) and its `at`: it lands on that pane's floor where its share puts it.
- **A kit is never an unboxed prop**: `from_json` drops a `ShelfKit` prop recorded unboxed; `Home::add` debug-asserts it.
- `an_unknown_strip_falls_back_to_the_rooms_pane` (ledger.rs:859) feeds `{"Shelf":3}`, which still fails to parse (a
  shelf is `{"Shelf":"Users"}`), but it is renamed to feed `{"Ledge":3}` so it keeps testing an unknown strip.
- `Summary` (ledger.rs:391) gains `structures: Vec<String>` (`"Users shelf, 2 of 2 up"`), skipped when empty, so dump.rs's
  JSON golden holds.
- Module doc (ledger.rs:1-46) and design.md *Her record* (design.md:2641) say what's kept and what older builds lose.

### D3. Where a shelf goes, and whether it stands this frame

**Geometry** (pure; read by `Home::extents`, D6): `Shelf::run(host: Extent, door: Option<Space>) -> Option<Run>` with
`Run { from, to, y }`: `from` from the anchor (`Side::Left`: `host.from + offset`; `Side::Right`: `host.to - offset -
SHELF_WIDTH`), `to = from + SHELF_WIDTH`, `y = host.floor - rise`. `None` (it can't hang this frame) unless
`from >= host.from + WIDTH`, `to <= host.to - WIDTH`, `LOW_RISE <= rise <= host.rows` (inside the pane, never in
her box's rows or a standing piece's), and the run's columns
miss the columns of her door's space on this strip **wherever the space can exist, kept or yielding** (`space_of`,
room.rs:1232; the space is 6 wide, more than `WIDTH`, so the side rule alone wouldn't keep it out). A built-1 shelf's
run is the half nearer its wall.

**Choosing the spot** (`Home::shelf_spot(buf, plan, keep_off, kit_at) -> Option<Shelf>`, room.rs, no RNG):
- **Highest that fits, then her preference** (the user's rule): every pane is tried high before any is tried low, so a
  high shelf in another pane beats a low one in the gremlin's. Within a height, panes in order: the pane of the gremlin
  if it stands on a floor (it wants up there), the pane she unpacks the kit in (`kit_at`), then `plan.nooks` order.
  Never a pane meeting `plan.chat`.
- **High**: rise from `HIGH_RISE` up to `host.rows - HEIGHT` (lowest first: text fills panes from the top, and lower is
  nearer her), the run **and the `HEIGHT` rows above it** free. **Low**: rise `LOW_RISE` only, the run and the
  `LOW_ABOVE` rows above it free, and its block meeting no hung piece of the host as laid out
  (`laid_and_shifted(plan.nooks)`, the host's `Lane::Wall` pieces), so going up never pushes one.
- Per rise, lefts in `host.from + WIDTH ..= host.to - WIDTH - SHELF_WIDTH` ordered by the distance of the run's middle
  from the gremlin's middle if it's in this pane, else the pane's middle (the left first of two as near). The first
  spot whose run passes D3's geometry and whose cells pass D4's per-cell test on this frame wins. One pass per pane
  builds a column-wise "free cells upward" count, so the search is linear in the pane's cells (~1.5k a pane), cheap
  enough to ask every paint while a boxed kit waits.
- Saved with `anchor` from the nearer wall (as `Extent::pin` does), its rise (which says high or low for good) and
  `built: 0`. A pane that grows later keeps a low shelf low; one that shrinks hides it (below).

**Per frame** a shelf stands when D3's geometry holds (else `Home::extents` has no plan for it, D6) and every cell of its
(built) run passes D4's test. Otherwise it's hidden that frame, and what stands on it with it (their floor row isn't a
line, so `fits` (room.rs:2839) refuses them): never moved, never re-hung, back when the frame holds it. The change is
logged at debug once per change.

`HIGH_RISE` = `max(hang + rows)` over the hung kinds + 1 = 7, and `LOW_RISE` = `HEIGHT` + 1 = 5; a test computes both
(from the `Spec` rows and from `sprite::HEIGHT` and the tallest standing piece), so a new hung piece that reaches higher,
or a standing one taller than her, fails it. The ledger skips a shelf whose rise is under `LOW_RISE`.

### D3a. A low shelf in the wall lane

- **The block**: for a shelf with rise under `HIGH_RISE`, its run's columns across the wall lane's rows that its row and
  the rows above it reach (`f-6 ..= f-5` for `LOW_RISE`). A placed shelf has a block from `built: 0`, so nothing hangs
  where it is going up.
- **Hung pieces keep out of it.** `hung_clear` (room.rs:2074) today moves only the window clear of what stands, to the
  nearest place between its neighbours on the wall that meets none, keeping its order, else left out alone. It takes
  the host's blocks as obstacles for **every** hung piece (the poster, the clock, the window, though the window's rows
  never meet one), by the same search: a poster whose anchor puts it over the shelf hangs beside it, at the nearest
  clear place in its order; on a wall too crowded for that, it's in the closet alone (a wall never moves a room). Its
  `shifts` entries say so, as for the window, so the repair search's "a pushed hung piece" refusal (rules.rs:605) and
  the door batch's yield rule (`plan_strip`, room.rs:1721) see it.
- **The host's layout reads its shelf**, so it's an input of the pane strip's layout: `Home::lays_from` and
  `StripLaid` (room.rs:891, 1852) record the host's shelves beside its pieces and door, and `relaid` lays the pane out
  again when they differ (the door batch's no-stale-strip rule, D6).
- **Placements see it**: `fits` already refuses a hung piece over the shelf's glyphs (its body must be blank); the
  block makes that a layout fact, so a delivered or moved poster is laid out beside the shelf rather than laid over it
  and hidden. `doorstep`, `spot` and the repair search judge by the layout and need nothing more.
- **Her wall clock as a gift** (mod.rs:4156-4190) and a window delivery are laid out the same way.
- The decor on a low shelf stands in rows `f-8 ..= f-6`, above the window and beside any hung piece (the block covers
  `f-6`; rows above `f-6` are above the wall lane).

### D4. `read_ground`: one write, before every read

```rust
/// What `read_ground` wrote into the frame this paint: each cell, with the real cell it covers.
pub(super) struct Written { cells: Vec<Frozen> }      // structures.rs (new module)
impl Written {
    pub fn none() -> Self;
    pub fn contains(&self, x: i32, y: i32) -> bool;
    /// The frame as the app drew it at (x, y): what `read_ground` covered, else `buf`'s cell.
    pub fn real(&self, buf: &Buffer, x: u16, y: u16) -> Option<Cell>;
    pub fn frozen(&self) -> &[Frozen];
}
/// Write her structures into `buf` where each stands this frame (all of its cells or none of them).
pub(super) fn read_ground(buf: &mut Buffer, home: &Home, plan: Plan, keep_off: &KeepOff) -> Written;
```

- **The per-cell test** (shared with `shelf_spot`): on screen; blank (`symbol().trim().is_empty()`), not a wide glyph's
  trailer (`width(cell(x-1)) < 2`, as `fits` checks), not untouchable; outside every `keep_off.protected` rect (the
  **gated** view's, so a focused pane hides it), outside `plan.chat`; not a cell of her layer (`visit.layer.cells()`,
  layer.rs:291: moved text, its sources and torn cells), not in a made piece's cover, not in the cover of a laid piece
  other than those on this shelf. Each written cell becomes a `Frozen { glyph: '─', ink: SHELF_INK, under: <the real
  cell>, face: None, burst: false }`.
- **Where it runs: once, at the top of `paint_state`** (mod.rs:2499), after `observe`, `gate` and `whole_glyphs`
  (2501-2504) and before `errand_progress`/`nudge_due` (2506-2507), for `Arriving`, `Away` and `Visiting`; `Absent`
  shows no home and `Leaving` writes nothing (below). It can go before the visit's `layer.validate` (2747) because
  validate reads only the layer's own cells (layer.rs:516-560, `fits` 301-327), which `read_ground` never writes; its
  `keep_off` uses the layer before validation (a superset: conservative). So every read of the paint sees the same
  frame, which closes the map's three missed sites at once: the errand's read in `Guest::send` (mod.rs:3432), the
  delivery's lazy read in `furnish` (mod.rs:4120), and her empty home's `ground`/`terrain` reads in `paint_empty`
  (5716, 5727); and the arrival's read (2515) and `home.frame` (2534) both follow it.
- **Made structural:** the production terrain reads take `&Written` (D5). `Terrain::read` without it becomes
  `#[cfg(test)]` (the test helpers in scenes.rs, terrain.rs and mind.rs:1980), so a new production read site that
  skipped the structures wouldn't compile. `nudge_due`/`send`, `furnish` and `paint_empty` take `&Written`.
- **Keep-out.** Its cells join `base` (mod.rs:2799: no pulled or swapped text, no made piece lands on them) and the
  made pieces' keep (`tend_made` judges made pieces on `buf`, where the glyph already isn't blank). Bubbles already
  avoid every non-blank cell (`bubble_spot`, mod.rs:3897-3903). Not `Visit::solid` (mod.rs:1020): she walks in front of
  a shelf as of a border.
- **Rains.** Its frozen cells join the frame's painted list: `visit.painted` (mod.rs:3187) and `empty.painted`
  (mod.rs:5769). So the focused-pane rain (mod.rs:2724-2745) rains a shelf out of a focused pane (and D3 keeps it out
  while focused), `rain_home` takes it with her empty home, and the goodbye's dissolve owns it.
- **The goodbye.** `Leaving` writes nothing: the dissolve, built from `visit.painted`, paints the structure cells as
  frozen glyphs and rains them out with the rest; writing them again each goodbye frame would keep them up after their
  rain and break the final `end == real` (tests.rs:1526). What stops a shelf from costing her the wave
  (`Dissolve::unchanged`, dissolve.rs:220, compares each burst cell's `under` with the live frame) is fixed where
  `under` is taken: every `under` recorded for her rains reads through `Written::real`: `draw_art`'s `under` closure
  (mod.rs:5499), the prop unders in `draw_props` (`unders_of`, mod.rs:5345; mod.rs:5601), `paint_empty`'s (5805). So
  her image's under-cells are the real frame's (blank where a shelf glyph stood), the goodbye frame (no structures)
  matches them, and two frozen cells at one place never disagree about what to put back. In 6a her box never meets a
  shelf (even a low one hangs a row above her head), so this is a guard for 6b, tested as such.
- **The rule's exception, documented:** decisions.md *Houseguest reads the frame she stands on (2026-10-01)* gains an
  amendment and design.md:2619-2622 a clause: her structures are the one thing of hers written **before** the reads,
  because they're terrain: once a paint, by `read_ground`, before anything reads the frame; the goodbye is the one frame
  without them (its dissolve owns their cells); every `under` she records is the real frame's. The code comments at
  mod.rs:2596-2603, 2643-2646 and 2744-2746 say the same.

### D5. The terrain: lines, but no ledge of hers (6a)

- `Terrain::read_guarded(buf, solid, guarded, graphics, written)` (terrain.rs:229): a new `lines` flag `STRUCTURE`
  (16) on written cells, and `ledges.push(!skip && floor && !structure)` (terrain.rs:263) with `LEDGE` likewise. So a
  shelf is never a platform (`add_platforms`, terrain.rs:318), never a `Ground::ledge` (her face-on fallback door,
  door.rs:353-367, never stands on one) and gives no link (`find_links`, terrain.rs:518: its `─` is no pole). It's still
  `STROKE` and `quiet`, so she passes in front of it and her line-art image redraws it (graphics.rs:598-760), as a
  border.
- That's what keeps her off a shelf in 6a: `settle`'s "no landing: the nearest platform" (osaka.rs:10230), arrivals,
  `find_rest`, doors in space and `nearest_clear` all pick among platforms, and there is none there. 6b lifts the flag
  for a shelf her ladder reaches.
- With nothing written, the flag is never set: the terrain snapshots (snapshots/*default_layout_terrain_*) and every
  terrain read are unchanged.

### D6. Shelves as strips

- `Home::extents(nooks)` (room.rs:1711) and every layout path built on raw strips (`pin_anchors` 1867, `laid_out` 1894,
  `strips_laid_out` 1901, `laid_and_shifted` 1914, `strips_laid` 1929) take their strips from one
  `Home::raw_extents(nooks) -> Vec<(Strip, Extent, Extent /*host*/)>`: the pane strips in `nooks` order, **then** each
  **finished** shelf whose run hangs this frame (D3), as `Strip::Shelf(nook)` with `Extent { from, to, floor: y, rows:
  host.rows - rise }`, no space, `host` its pane's raw extent. Panes first, so every test indexing `extents(..)[i]` still
  reads a pane. An unfinished shelf is drawn (D4) but no strip.
- **No stale shelf plan** (the door batch's `relaid` rule, door/design.md D1): a shelf's plan reads its `Shelf` record,
  its host's extent and her door's wall on its host (D3's space columns). `strip_door(strip)` (room.rs:1860) for a shelf
  reads the door on `strip.room()`; `StripLaid` (room.rs:891) records the `Shelf` and `plan.host` beside `from` and
  `door`, and `relaid` (room.rs:1969) lays a shelf strip out again whenever any of them differs, so a move that changes
  any input of a shelf's layout can never reuse the old one.
- **Pieces on a shelf** are laid out by the same packing in anchor order, show where `fits` holds (their floor row is the
  shelf's glyphs, written before `furnish`), and are hidden with it. `Shown::cover` of a piece on a shelf is its body and
  the shelf cells under it: solid to text, like any piece's.
- **Moves onto a shelf** come only from the repair search, which already enumerates every plan (rules.rs:930) and now
  meets the shelf's plan for small decor. `fits_now` (rules.rs:837) judges it on the frame (the shelf drawn). The
  pocket carry needs no change but `reach` reading `stood`: she lifts and sets down standing on the floor under it.
- **`move_off` never touches a shelf** (D1): a shelf's pieces aren't moved to a floor when the shelf can't hang, nor are
  a floor's pieces moved onto a shelf when a pane goes.
- **The closet** (`stranded`/`placements`, room.rs:946-1100) considers a shelf only for pieces it takes, so it agrees with
  the search (`stranded_iff_no_room_takes_it`, room.rs:5917).
- `clock_on` (mod.rs:213) reads the room's floor: the pane strip of `clock.strip.room()`, so her glance at a clock on a
  shelf is taken from its room's floor.

### D7. The kit: sold, delivered, unpacked, put up

- **The kind.** `Furniture::ShelfKit` (room.rs:20, `ALL` 13), a `Spec` (`name` "shelf kit", the pitch above, footprint
  (5, 2) so its parcel shows whole, `offers: &[]`, `beauty: 0.0`, `uses: &[]`, an ASCII flat pack that fills the
  footprint for the lint at room.rs:2884, never drawn unboxed), and two new `Spec` fields: `kit: Option<Kit>` (`Kit::Shelf`)
  and `shelf: bool` (true for the plant, the clock, the gremlin). `legacy()` false; `RoomKind::of` → `Living`
  (ledger.rs:575); `scrap` and `art::parts` arms (boxed only: its parcel is `art/delivery.svg` scaled by footprint).
- **The channel** (`advert`, mod.rs:194): `advert(ledger, shop_now, needs, wish, shelf_room)`. The queue is the wished
  kit first, if any; else, beauty pressing, decor then furniture, else furniture then decor (as today); kits last.
  `CATALOGUE` (mod.rs:169) appends `ShelfKit` (its tier comes from `Spec.kit`, so the array's order within a tier is
  unchanged). A kit is sold only while `shelf_room`: `shelf_spot` finds a spot **on this frame** (geometry and blank
  cells), computed where `advert` is (mod.rs:2955, 3008, which have the frame), so a kit is never bought for panes full
  of text; a boxed kit counts for `advert`'s `idle` (mod.rs:196), so one that can't go up would stop the channel. The
  residual risk (the frame changes for good between purchase and unpacking: a resize, a layout change) is accepted: the
  box waits, visibly, and the channel with it, until a frame has a spot; logged at debug once per change. The cadence
  (`SHOP_EVERY`, every third visit) is unchanged; the wish only reorders.
- **Delivery**: as any parcel, through her door's flap (`doorstep`, pane strips only).
- **Unpacking**: the existing `Unpack` (4–6 s, osaka.rs:1787), but the kit's unpack seat is offered only on a frame where
  `shelf_spot` finds a spot (`seats`/`spots_for`, mod.rs:4537-4600, gain a flag as `cat` does), so she never unpacks a
  kit with nowhere to put it. `HomeEvent::Unpacked(ShelfKit)` in `record` (mod.rs:3688) sets `visit.raise = Some(kit)`
  instead of `unbox` (it needs the frame, as `SetDown` does); `furnish` (mod.rs:4003) chooses the spot on its frame and
  calls `Home::raise(shelf)`: the prop goes, `Structure::Shelf(.. built: 0)` comes, logged at info with its pane, rise
  and columns. With no spot by then (the frame changed in her 4–6 s), the box stays shut, logged at debug, and her
  unpack seat waits for a spot again.
- **Building**: `Want::Build` (brain.rs:520, `ALL`; def `row(12.0, [(Nesting, 1.0)])`, `own_sake`, between Arrange's 6
  and Unpack's 40; tuned in step 5's census), method `build/stage` (mind.rs) binding `Job::PutUp(PutUp { x, y, side })`
  (scenes.rs:236) at `Chances.build` (osaka.rs:29): the spot under the unfinished shelf's next half, on its pane's
  floor, where she may stay (`reach`'s test). `Act::PutUp { since, until }` (6 s, `Moves::Still`, the ActProps of
  `Act::SetDown`), `Pose::Stretch` held, `"Up it goes..."` at its start; at its end `HomeEvent::Built`, which `record`
  commits: `built += 1`, `paced` updated, saved at once (decisions.md:3299, "events at once"). Cut short, nothing is
  committed; she'll be offered it again.
- **Pacing.** `Ledger.paced: Option<Paced { on: Dated, stages: u8 }>` (skipped when `None`). `Dated` is today's `Said`
  (mod.rs:3593), moved to ledger.rs and made serializable: the game day (`Osaka::day`, osaka.rs:4365), or with no clock
  reaching her the visit. `Build` is offered only while `paced` isn't today's with `STAGES_A_DAY` (1) done. So it's per
  game day, survives a restart, and in the unfed tables (no clock) is per visit.
- **Credit and lints**: `credit_path(Build, "build/stage")` (osaka.rs:7647; tests/credit.rs:19), the cue lint
  (`every_want_can_be_cued`, tests.rs:10580: `Scene::Build`, D11), `all_lists_every_want` (brain.rs:1144),
  `every_want_has_a_sane_row` (brain.rs:1210), the act lints (ActProps).

### D8. The gaze up: `Use::Admire`

- `Use::Admire` (room.rs:410, `ALL` 12): not `inside`, not `asks_room` (like `LookOut`: offered only where she can get
  under it), `lingers`. `Shown::uses()` (room.rs:1437) gives `[Admire]` for a real, unboxed piece standing on a shelf,
  else what it gave before; its spots (`spots_for`, mod.rs:4570) are under its middle, then beside it, at `stood`,
  facing it; `seats_of`'s `restful` check (mod.rs:4547) applies.
- `Want::Use(Use::Admire)`: `row(5.0, [(Daydreams, 0.4)])` (a little under `Gaze`'s 6/0.5, brain.rs:716), no factors.
  Its script (script.rs, as `LookOut`'s arms: 525, 642, 744, 1453, 2465, 2512, 2546): `Posed::Still(Pose::Gaze)`, no
  line; duration 8–16 s (osaka.rs:1787 table). Being a use of her things, it eases *beauty* in a pretty room as every
  use does (osaka.rs:7475-7583), and the room is pretty because of what's on the shelf (D10).
- Within the band: a still use reached by a short walk on her own floor. Step 6 re-measures (Census).

### D9. The gremlin, `High`, and the wishlist (step 7)

- `Furniture::Gremlin` (`ALL` 14): the approved variant A, as phase6/art/snippets.md's *Suggested `Spec`* has it:
  footprint (2, 2), ASCII `oo` / `ww` (mirror-safe), ink `(Rgb(178, 154, 224), LightMagenta)`, `symmetric: false`,
  `offers: &[Decor]`, `beauty: 1.0`, `hang: None`, plus `shelf: true`; the pitch is `"Aww, a little gremlin!"` (the
  user's choice above, not the sheet's). Art: only the `gremlin` group from `worktree.diff` goes into `art/props.svg`
  (never `-tall`, `-jade` or `-wide`); `art::parts` (art.rs:1373) gains `(Furniture::Gremlin, _) => &["gremlin"]`.
  `CATALOGUE` gets it after the poster (decor tier). `RoomKind::of` → `Living`; ledger kind lists
  (`an_older_build_skips_the_clock_and_the_window`, ledger.rs:1345) extended.
- `Rule::High(Furniture)` (rules.rs:19): broken while the piece is laid out, out of its box, on a strip that isn't a
  shelf. A `RULES` row appended (rules.rs:74; row indices of the others unchanged): `High(Gremlin)`, felt on sight,
  grievance `"It wants a shelf..."`. `kind(row)` (rules.rs:191) gains its label. `on_sight` (mod.rs:3642) feels it as
  `InChat` (whenever she's in sight while it's broken), said on the same terms (when her mood would mend, once a game day).
  With a shelf up, the search's only mending move is onto a shelf (D6), carried as any repair; with none, no move mends
  it and it's what the wishlist reads. Then she is as with a DoorClear no move mends today: `Arrange` is offered while
  a felt rule is broken (`may_arrange`, osaka.rs:8444, from `to_mend` 9639, which doesn't ask for a repair) and its
  `lift` finds none (mind.rs:689), so it doesn't bind; *nesting* rises for the visit. Step 7 checks in the census's want
  table that this costs her no slot of `choose`'s four; if it does, `may_arrange` also asks for a repair (the class,
  DoorClear included).
- **The wishlist**: `Rule::wants(self) -> Option<Furniture>` (`High(_)` → `ShelfKit`; 6b and 6c add theirs). `wish` =
  for the first rule she has felt this visit (`Osaka.felt`, in the order felt) that is still broken and whose kit she
  doesn't own, that kit. Computed where `advert` is (mod.rs:2955, 3008).

### D10. Beauty from a shelf

`beauty_at` (mod.rs:233): her room is the pane strip whose floor she stands on (`pane_strips`), and its beauty sums
what shows, unboxed and real, whose `strip.room()` is that strip: a shelf's decor counts for the room below, a hidden
shelf's doesn't (it isn't shown). Nothing else changes: *beauty* rises in a plain room and is eased by rest and use in a
pretty one (design.md:1571).

### D11. The stage

`Scene::Build` (stage.rs:28): gives her a shelf where `shelf_spot` puts it (built 0), clears today's pacing, cues
`Build`. `Scene::Shelf`: a finished shelf with the plant on it (the gremlin from step 7). `Scene::Admire`: cues the use.
`Scene::ALL` (stage.rs:170) grows from 58 to 61. The stage room is the bundled layout at 100×30: no pane takes a high
shelf there, and only The List (9 rows, 7 inside) takes a low one by geometry, at rows 1–3 of the screen, where its
entries are drawn from the top. If the stage room's List leaves those rows blank in 10 columns `WIDTH` off its walls,
the three scenes run there with a low shelf (step 5 checks and says which). Otherwise
`every_scene_has_a_spot_in_the_stage_room` (tests.rs:1762) runs these three on the smallest bundled frame step 3
measures for a low shelf, and the example (`examples/houseguest.rs`) says so when the terminal is too short.

## Tests

The door batch's rules hold: written first, run, **confirmed failing for the stated reason** (a test that can't be red
today is a **guard**, proven by a reviewer's mutant); deterministic (seeded, game times via `tue`/`sat`/`home_at`);
every houseguest test loops `for graphics in [false, true]`; panes filled with text where terrain matters; pinned case
counts through `proptest_cases(N)`; a proptest failure committed to its regressions file and pinned as a unit test.
**No existing proptest's strategy changes** (their `cc` seeds would replay as other cases): shelves are new proptest
functions sharing the bodies.

- **Structures take only what's theirs** (`read_ground_takes_only_blank_free_cells`, structures.rs, proptest): random
  frames (text, wide glyphs, protected rects, a chat rect, a door wall), random shelves (any geometry, any `built`),
  random layer and made cells: every written cell was blank and not a trailer, outside every protected rect and the
  chat, outside her door's space columns, off every layer, made and laid cover; each shelf is all or nothing; with no
  structures the buffer is byte-identical and `Written` empty.
- **No furniture on a shelf, ever** (`only_small_decor_stands_on_a_shelf`, room.rs, proptest over homes with every kind
  anchored on random strips, shelves included): `laid_and_shifted` lays no piece on a strip that doesn't take it;
  `search` proposes no such move (rules.rs); `spot`, `doorstep` and `move_off` never target a shelf; `placements` agrees
  with the search (`stranded_iff_no_room_takes_it` runs with shelves); `from_json` of such a record lays it on its pane.
- **Where a shelf goes** (`a_shelf_hangs_as_high_as_fits`, proptest over panes of every height, text, doors, chats,
  hung pieces): every `shelf_spot` result is `WIDTH` off both walls, misses the door's space columns (computed by hand
  from the nook rect, never read from `space.rect`), not in the chat; either high (rise `>= HIGH_RISE`, `HEIGHT` free
  rows above at choice) or low (rise `LOW_RISE`, `LOW_ABOVE` free rows above, its block meeting no hung piece as laid
  out); and low only when no pane has a high spot on that frame. `HIGH_RISE` and `LOW_RISE` equal the values computed
  from the `Spec` rows and her height (`rises_clear_every_hung_and_standing_piece`).
- **The wall lane packs beside a low shelf** (`no_hung_piece_meets_a_low_shelf`, room.rs proptest beside
  `the_wall_lane_keeps_order_and_never_moves_the_room` 4164, `decorated()` pieces and a random low shelf): no laid
  hung piece meets a block; hung order is kept; a resize and back restores; a high shelf changes no hung piece's place
  (byte-identical layout to no shelf); a `relaid` home equals a fresh layout with a shelf on a touched pane
  (`a_move_relaid_strip_by_strip_is_laid_out_as_a_whole`, room.rs:4663, run with shelves). Plus
  `a_poster_delivered_hangs_beside_a_low_shelf` and `putting_up_a_low_shelf_never_pushes_a_hung_piece`.
- **She never stands on a shelf in 6a** (`a_shelf_is_no_floor_of_hers`): terrain with and without the written cells has
  the same platforms and links; in the long-visit host, her feet are never on a written cell.
- **Long visits with shelves keep every covering property**: `long_visits_with_a_shelf_never_touch_what_is_protected`
  (tests.rs) and `her_days_with_a_shelf_never_touch_what_is_protected` (tests/away.rs): `owned` plus a random shelf
  (pane, side, offset, rise, `built`, valid or not) and small decor on it; `long_visit_of`'s checks extended: "each real
  piece stands on a strip here" (tests.rs:1365) reads `home.extents`; "on lines, over blank" (tests.rs:1376) accepts a
  floor row that is a written cell; every frozen cell's `under` equals `real` (the guard for D4's `under`); the final
  `end == real` (tests.rs:1526) still holds, so the goodbye rains every shelf out.
- **The goodbye still waves beside a shelf** (`she_waves_goodbye_under_her_shelf`): a visit with a finished shelf and
  the plant on it, she under it; at the goodbye, `untouched` is true, the startled beat and the wave are drawn, the shelf
  rains out. Plus a dissolve unit test with a structure cell under her image (6b's case) built by hand: `unchanged`
  holds when unders come through `Written::real`, fails when taken from the written frame (the mutant).
- **Text, focus, resize** (both modes): text over the shelf hides it and what's on it, and back when it goes; a focused
  pane rains it out and keeps it out; a resize narrower than D3 allows hides it, and a resize and back restores it,
  pieces on it at their anchors; hiding its pane hides it, moves nothing.
- **Ledger**: `a_shelf_round_trips`, `an_older_build_reads_past_the_structures` (`as_an_older_build_reads`), garbage
  entries skipped one by one, a later build's structure skipped, a piece on a shelf the record lacks lands on its pane,
  an unboxed kit dropped, `paced` round trip; `the_record_as_written` (ledger.rs:773) and dump.rs's golden unchanged.
- **The kit** (step 5): `the_kit_is_sold_last_and_at_the_usual_cadence`; `no_kit_is_sold_where_no_pane_takes_a_shelf`;
  `the_kit_comes_through_her_door_and_goes_up_a_stage_a_day` (fed clock: unpacked Tue 16:00, half up that day, whole
  not before Wed's start, a restart between keeps the count); `a_kit_with_nowhere_to_go_stays_boxed`.
- **Moves onto a shelf** (step 3): `a_plant_in_the_chat_goes_up_on_her_shelf` (InChat, only the shelf free: carried
  there, set down from under it, shown, rule mended).
- **Beauty** (step 3): `beauty_counts_what_stands_on_her_shelf` (unit on `beauty_at`), and a hidden shelf's doesn't.
- **Admire** (step 6): `she_gazes_up_at_what_is_on_her_shelf` (cue and seeded), never at a piece on a floor.
- **The gremlin** (step 7): `the_gremlin_wants_a_shelf_and_gets_one` (end to end over game days: bought, felt, "It
  wants a shelf...", the kit wished first, delivered, put up over two days, the gremlin carried up, the rule mended);
  `a_felt_high_rule_puts_the_kit_first`; `an_unfelt_one_doesn't`.
- **Gates** (every step): `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `PROPTEST_CASES=32
  cargo nextest run --workspace --all-targets`. Steps 2 and 5 also run the perf tests (`--profile full --release`).

## Steps

Each step: one implementer (tests first and confirmed failing, implement, the gate, commit with `jj commit`), two
reviewers, a fixer. Strictly sequential. Sizes are changed lines, tests included. **No push of `master` mid-phase**; no
protocol change, so `stable` stays put.

### Step 0. This design, its critiques, the gremlin's sheet (docs)

Critiques (mechanics, tests, feasibility, character) beside this file in `phase6/`, resolved here. The gremlin's sheet
is done and approved (variant A: `phase6/art/`, `snippets.md`'s last section); it's committed with this design, and
nothing of `worktree.diff` is applied until step 7, which takes only its `gremlin` group.

### Step 1. Strips: `Strip::Shelf`, `takes`, rooms, no zips (refactor, ~650 lines)

**Goal.** D1 with no shelf anywhere: nothing a user or a golden can see moves.
**Sites.** room.rs:629 (enum, `nook`/`room`/`takes`), 697-722 and 1448-1462 (lanes by strip), 805 (`pane_strips`),
837-868 (`StripPlan.host`, `takes`, `pin` of `host`), 1272-1321 (`choose` without the zip), 1663-1686 (`rooms`), 1839
(`on` admits), 1867-1990 (raw strips through one `Home::raw_extents`, panes only for now), 2159-2296 (`move_off(from:
Nook)`, pane targets), 2297-2390 (`spot`), 2419-2560 (`doorstep` without the zip), 2684 (`stand` → `stood`), 978-1100
(closet: `takes`), `Shown::seat` 1534, `room_for` 2738; rules.rs:433, 548-597 (rooms by `room()`), 930 (`takes`, lane
per candidate); ledger.rs:205, 365, 553, 859 (rename), `DoorWall` try_from; mod.rs:213-245 (`clock_on`, `beauty_at` by
`room()`), 1624, 4273 (`reach` reads `stood`), 4339-4341 (`stage_arrange`).
**Tests first.** `takes` unit table; `a_garbled_record_never_puts_her_door_on_a_shelf` (red: today `{"Shelf":..}`
can't parse at all, so it's a guard until the variant exists; a mutant without the try_from proves it); the room.rs
proptests run unchanged.
**Exit.** The gate; **every golden byte-identical** and the terrain snapshots unchanged (any move: stop).
**Traps.** `Strip` derives `Hash` and is a ledger key: serde names unchanged for `Bottom`. `furnished()` stays per
strip (layout order); only `rooms()` gathers.

### Step 2. Structures: the model, the ledger, `read_ground`, the terrain flag (~1300 lines)

**Goal.** D2, D4, D5, and D3's geometry and per-frame test: a shelf in a home (tests only: nothing gives her one yet)
is drawn, rains, hides and is no floor of hers. Not yet a strip.
**Sites.** New structures.rs (`Structure`, `Shelf`, `Run`, `Written`, `read_ground`, the per-cell test, `HIGH_RISE`,
`SHELF_WIDTH`, `SHELF_INK`); room.rs:1631 (`Home.structures`, `owns`); ledger.rs (fields, `Raw`, `Summary`, module
doc); terrain.rs:229-263 (the flag; `read` → cfg(test)); mod.rs:2499-2507 (the one call), 2515 and 2534 (arrival),
2643-2650 (Leaving: its read at 2649 becomes `read_guarded(.., &Written::none())`, the only production call passing
`none()`, with a comment saying why), 2724-2747 (comments), 2799 (`base`), 3187 (`painted`), 3411-3446 (`nudge_due`/`send`
take `&Written`), 4003/4120 (`furnish`), 5345/5499/5601/5805 (unders), 5660-5769 (`paint_empty`); door.rs:532, 581
(test `Ground` impls unchanged: no new method).
**Tests first.** D4's property; the terrain equality (`a_shelf_is_no_floor_of_hers`); text/focus/resize hiding; the
dissolve `under` unit test; ledger tests; `rises_clear_every_hung_and_standing_piece`. Red: none of the API exists, so each is red by not
compiling; each is then run against a mutant (`read_ground` skipping one check) and must fail.
**Exit.** The gate; goldens byte-identical, snapshots unchanged; the ledger and dump goldens unchanged. Measure
`read_ground`'s cost with no structures (must be a no-op: one `is_empty`).
**Docs.** decisions.md: the amendment to *reads the frame she stands on* (D4's exception) and a new entry *Her
structures are the UI's own lines* (why line glyphs, why written before the reads, why the goodbye has none, why no
ledge of hers in 6a; rejected: written just before `Terrain::read`, which `fits` would miss; re-writing in the goodbye).
design.md:2619-2622 (the exception), *Her record* (2641: structures).
**Traps.** `read_ground` runs before `validate`: it must take the layer's **pre-validate** cells, never `solid` alone
(sources and torn cells aren't in `solid`). `Leaving` must not write. Frozen structure cells are `burst: false`.

### Step 3. Shelves as strips: small decor up there, moves, beauty (~1100 lines)

**Goal.** D6, D10, and the `Spec.shelf` field: a finished shelf is a strip; the plant and the clock may stand on it (by
a repair); what's on it counts for the room's beauty. Measure the smallest bundled-layout terminal whose panes take a
high shelf and a low one, by geometry (a `real_frame` test printing both per pane, ignored, its output recorded in the
commit and in plan.md). High shelves only, hand-placed in tests, until step 4.
**Sites.** room.rs (`Spec.shelf` for PLANT 296, CLOCK 325; `raw_extents` adds finished shelves; `lift` on a shelf);
rules.rs:905-1050 (candidates on shelves; `fits_now`); mod.rs:213-245, 4273; tests.rs:1365-1410 (`long_visit_of`'s
checks), the two new long-visit proptests.
**Tests first.** `only_small_decor_stands_on_a_shelf` (red on the step-1 tree only through a mutant of `on`'s admission:
it's the guard of D1's class); `a_plant_in_the_chat_goes_up_on_her_shelf` (red: no shelf plan); `beauty_counts_what_
stands_on_her_shelf` (red); the long-visit proptests with shelves (red on `long_visit_of`'s strip check until
updated); `a_shelf_hidden_hides_what_stands_on_it_and_moves_nothing`.
**Exit.** The gate; goldens byte-identical (no golden home has a shelf); `repair_search_is_cheap` (rules.rs:2295) under
its bound with a shelf on each pane.
**Docs.** design.md *Strips and anchors* (1850: a shelf is a strip for small decor, hidden with what's on it), *Rooms
from contents* (1957: a shelf is part of the room below), the beauty clause (1571).
**Traps.** The clock changes lane when it moves to a shelf (`Wall` → `Floor`): the search's extent and rows come from
the candidate strip. `fits_now`'s `room(at)` asks nothing of decor (no uses that ask room).

### Step 4. The low shelf: the wall lane packs beside it (~700 lines)

**Goal.** D3a: a shelf at `LOW_RISE` is a strip like a high one, and the poster, the clock (and in principle the window)
hang beside its block. Hand-placed in tests; nothing puts one up yet.
**Sites.** structures.rs (the block, `LOW_RISE`, `LOW_ABOVE`); room.rs:1839-1860 (`lays_from`, `strip_door`), 891 and
1969 (`StripLaid` records the host's shelves, `relaid`), 2019-2131 (`lay_strip`, `hung_clear` with blocks for every hung
piece), 1721 (`plan_strip`: the yield rule sees a hung piece the block left out); rules.rs:605 (`evaluate`'s pushed-hung
refusal, unchanged, now also met by blocks).
**Tests first.** `no_hung_piece_meets_a_low_shelf` (red: today a poster anchored there is laid over the shelf and hidden
by `fits`); `a_poster_delivered_hangs_beside_a_low_shelf` (red: the delivery lays it over the shelf, so `doorstep`
refuses the wall or it comes in hidden); the relaid property with shelves (red with `StripLaid` not recording the shelf:
proven by that mutant).
**Exit.** The gate; goldens byte-identical (no shelf anywhere: `hung_clear` with no blocks is today's); the room.rs
wall-lane proptests unchanged; `repair_search_is_cheap` under its bound with a low shelf on each pane.
**Docs.** design.md *Wall pieces* (1925): a low shelf's columns are no hung piece's; decisions.md *A shelf hangs as high
as fits* (the user's "highest that fits"; why two heights and not any; why the block and not packing shelves into the
wall lane as pieces; why a low shelf is put up only where it pushes no hung piece; what 6b inherits).
**Traps.** A block is a layout input of the **pane** strip (not of the shelf strip): forget it in `StripLaid` and a repair
judged on a stale wall passes. The window's own clearance rule (`may_overlap`, room.rs:99) is unchanged: blocks are
obstacles for it too, but its rows never meet one.

### Step 5. The kit: sold, delivered, unpacked, put up a stage a day (~1550 lines)

**Goal.** D7 and `Scene::Build`/`Scene::Shelf`. User-visible from here.
**Sites.** room.rs:20-140 (kind, `ALL`, `legacy`, `spec`, `Spec.kit`), `Home::raise`, `shelf_spot` (both heights, high
first: D3); mod.rs:166-208
(`CATALOGUE`, `advert`), 2955/3008 (`shelf_room`), 3673-3720 (`record`: `Unpacked` of a kit, `Built`), 4003ff
(`furnish`: raise), 4537-4600 (unpack seat gate), `Said` → ledger `Dated`; ledger.rs (`paced`); osaka.rs:29 (`Chances.build`),
406 (`HomeEvent::Built`), the `Act::PutUp` arms, 1787 (duration), 7647 (credit); brain.rs (`Want::Build`); mind.rs
(`build/stage`); scenes.rs:236 (`Job::PutUp`); stage.rs (scenes); art.rs `parts` (kit boxed only); scrap.rs arm.
**Tests first.** The four kit tests (Tests); `a_shelf_hangs_as_high_as_fits`; `every_want_can_be_cued` (red until
`Scene::Build`); the act lints. Check the stage room's List for a low shelf (D11) and say in the commit which frame the
three scenes run on.
**Exit.** The gate; goldens predicted byte-identical (no golden home exhausts the catalogue, `Want::Build` is never
offered without a shelf): checked with a trace diff (`HOUSEGUEST_GOLDEN_TRACE`, testing-strategy.md:776), and a move is
a finding, not a re-record. Census: `visit_census` both modes unchanged; the kit's own numbers from a new ignored
`shelf_census` (her days from a kit's delivery to a finished shelf, in `rooms(100, 30)`, over seeds).
**CHANGELOG.** "Added: the shopping channel sells Osaka a shelf kit; she puts the shelf up on a wall, a bit a day."
**Docs.** design.md *Getting furniture* (1787: the kit, its tier, the wishlist's place, `shelf_room`), a new
*Structures* bullet (the kit, unpacking, two stages a day apart, pacing, where it hangs: high where it fits, else low,
hidden when the frame can't hold it); decisions.md: *She puts up a shelf, a stage a game day* (why per game day, why not a home act, why chosen on
the frame, why never re-hung).
**Traps.** `Unpacked(ShelfKit)` must never reach `Home::unbox`. `her_home_fills_up_over_visits` (tests.rs:5574) and the
catalogue pins (tests.rs:5220, 5433, 5998, 11925) may count `CATALOGUE`'s length: update with the reason.
`send_parcel` (mod.rs:1490) walks `[Tv] + CATALOGUE`, so the stage can now send a kit.

### Step 6. The gaze up (~600 lines)

**Goal.** D8 and `Scene::Admire`.
**Sites.** room.rs:410-500 (`Use`), 1437 (`uses`); mod.rs:4570 (spots); script.rs (Admire's arms and keys);
osaka.rs:1787; brain.rs (`Want::Use(Admire)` row); stage.rs.
**Tests first.** `she_gazes_up_at_what_is_on_her_shelf`; the script/cue lints (`every_script_can_be_cued`,
tests.rs:10679).
**Exit.** The gate; goldens byte-identical (no shelf in any). Band (below) measured on the shelf room.
**CHANGELOG.** "Added: Osaka sometimes stands under her shelf and gazes up at what's on it."
**Docs.** design.md *Using her furniture* (1969); decisions.md one line under the step-5 entry.

### Step 7. The gremlin, `High`, the wishlist (art approved: variant A; ~1000 lines)

**Goal.** D9.
**Sites.** room.rs (kind, `Spec`), art/props.svg (group from the sheet), art.rs:1373 (`parts`); mod.rs:169 (`CATALOGUE`),
194 (`advert`'s `wish`), 2955/3008, 3642 (`on_sight`); rules.rs:19, 74, 191 (rule, row, label), `Rule::wants`;
ledger.rs:575 and the kind-list tests; stage.rs (`Scene::Shelf` puts it up).
**Tests first.** The three gremlin tests (Tests); `the_channel_sells_decor_when_her_room_feels_bare` (tests.rs:5220)
still passes.
**Exit.** The gate; goldens byte-identical (no golden home owns a gremlin; the rule is never broken without one);
`rules_hold` (rules.rs:1564) and `every_repair_mends_and_breaks_nothing` with a gremlin.
**CHANGELOG.** "Added: a gremlin figurine for Osaka's shelf; it wants to be up high, and she'll get a shelf for it."
**Docs.** design.md *The rules of her home* (2101: `High`, felt on sight, its line), *Getting furniture* (the wishlist);
decisions.md *The gremlin wants up high* (why a rule, why felt on sight, the wishlist as felt rules naming kits, the
rejected shelf-only gremlin).

### Step 8. Census, deep runs, docs (~250 lines)

Census and band (below) recorded in plan.md's Phase 38 record; the deep run clean (each failure pinned and fixed in
its own commit first); design.md read through (strips, structures, rules, beauty, record); CLAUDE.md's index line for
this design; plan.md's record (per step, deviations, the census); the gremlin's `snippets.md` header marked wired.

## Goldens

- Every step is predicted **byte-identical**: no golden scene has a structure, a kit or a gremlin; `read_ground` writes
  nothing and `Written` is empty; the ledger's new fields are skipped when empty; new wants and uses are offered only
  with a shelf; new guards check for a shelf before drawing a whim. Any move is investigated with the trace diff
  (testing-strategy.md:776-819), never re-recorded on faith.
- No new golden scene in 6a (the stage room takes a low shelf at best, and only if its List is blank there); the
  long-visit and day properties carry shelves instead.

## Census, band and stillness

- The band rooms (`furnished_room`, `resident_room`, the stage) have no shelf, so their numbers can't move; steps 5 and 6
  confirm it.
- New ignored measurements in tests/census.rs: `shelf_room()` (`rooms(100, 30)`, the furnished pieces, a finished high
  shelf on Users with the plant and the clock on it) and `low_shelf_room()` (`home_screen`'s 9-row panes, a low shelf
  with the clock on it and the poster hung beside it), run through `visit_census` both modes and the band's per-mood share and
  set-off cap (tests/band.rs) at the census seeds. Admire's walks are short and on her floor; if the shelf room's share
  or set-offs leave the band, that is a finding for the user (tune Admire's weight), not a retune of the band.
- `shelf_census`: game days from a kit's delivery to a finished shelf (expected 1–2), and from a gremlin's delivery to
  it standing on the shelf, over 32 seeds, recorded in plan.md.

## Deep runs (step 8)

`PROPTEST_CASES=256 cargo nextest run --release --profile deep -p dessplay houseguest` (read nextest's own exit code);
`CENSUS_BAND_SEEDS=<n> cargo nextest run --release --profile band --run-ignored all -p dessplay -E
'test(/houseguest::tests::band::/)'`; `cargo nextest run --workspace --all-targets`.

## Docs that change (by step)

| Step | design.md | decisions.md |
|---|---|---|
| 2 | 2619-2622 (structures before the reads); *Her record* 2641 | amendment to 2263 (*reads the frame she stands on*); new *Her structures are the UI's own lines* |
| 3 | *Strips and anchors* 1850; *Rooms from contents* 1957; beauty 1571 | (in step 2's entry: small decor only; rooms are floors) |
| 4 | *Wall pieces* 1925 (a low shelf's block: the poster and the clock hang beside it) | *A shelf hangs as high as fits* |
| 5 | *Getting furniture* 1787; new *Structures* bullet (both heights) | *She puts up a shelf, a stage a game day* |
| 6 | *Using her furniture* 1969 | a line under step 5's |
| 7 | *The rules of her home* 2101; *Getting furniture* (wishlist) | *The gremlin wants up high* |
| 8 | read-through; CLAUDE.md line; plan.md record | — |

## Not in 6a

The ladder, the pole check in `settle` (osaka.rs:10169) and `Clamber`, standing on a shelf (6b: lift D5's flag for a
shelf her ladder reaches, and only one with `HEIGHT` clear rows over it: every high shelf, a low one only in a pane 11
rows or taller); raising a low shelf when its pane grows; partitions, rooms as regions, `Connected` (6c); a second shelf or kit; posters on shelves;
re-hanging a shelf the frame can't hold.

## Questions for the user

None open. The first round's three (shelf height, the plant and the clock, the lines and the sheet) are answered in
*User decisions*, second round. One for 6b, not 6a: whether a low shelf in a pane too short to stand on should ever be
taken down and put up high when its pane grows (6a never re-hangs one).
