# Phase 6a (shelves and the gremlin): implementation design

**Working design, 2026-10-10; amended the same day twice: with the user's answers, and after four critiques (their files
are in `phase6/`: `critic-mechanics.md`, `critic-feasibility.md`, `critic-tests.md`, `critic-character.md`; every
finding's disposition closes this file); with step 1's as-built notes (D1, step 1); and with step 2a's review (D1, D2:
only a finished shelf holds a piece the record puts on it, never a parcel; the Tests field order); and with step 2b's
review (D3: the log; D4: where the write runs out of the house, the keep-out for her door and her pulls; Gates).** Brief: docs/plan.md, Phase 38, "Phase 6 — structures (brief, 2026-10-10)";
plan.md wins over everything here, but for the one place this design departs from the brief's wording, said in D4.
Code maps at `60727649`: [phase6/map-terrain.md](phase6/map-terrain.md) and [phase6/map-life.md](phase6/map-life.md);
this design overrides them where it says so. Proposal: [../2026-10-02-houseguest-mind.md](../2026-10-02-houseguest-mind.md),
*Structures from the UI's own lines*, *Rooms from contents*, *Rules, not an objective*. The gremlin's art is approved
(variant A: [phase6/art/snippets.md](phase6/art/snippets.md), its last section; of `worktree.diff` only the `gremlin`
group is wired, in step 7). Paths are relative to `dessplay/src/ui/houseguest/` unless noted. Lines were checked at
`4c5c7151`; the critics re-checked them at `ad7f4af7` (`dessplay/` unchanged).

**Geometry words**, used throughout.
- A **pane strip** is today's `Strip::Bottom(nook)`, with its raw `Extent { from, to, floor: f, rows }` (room.rs:741;
  interior rows `f-rows ..= f-1`). `WIDTH` = 5, `HEIGHT` = 4 (sprite.rs:9-11), `SPACE` = 6 (room.rs:823).
- Her box standing on `f` is rows `f-4 ..= f-1`.
- The **wall lane** is where hung pieces hang: the poster and the clock (hang 4, 2 rows tall: room.rs:310, 325) take
  rows `f-6 ..= f-5`; the window (hang 1) rows `f-3 ..= f-2`.
- A shelf's **rise** is how many rows above its pane's floor it hangs (its row `y = f - rise`). Its **host** is the pane
  strip it hangs in.
- A **high** shelf hangs above the wall lane (rise ≥ `HIGH_RISE` = 7). A **low** one hangs just over her head (rise
  `LOW_RISE` = 5), in the wall lane's rows, so the poster and the clock pack beside it. Its **block** is a low shelf's
  run's columns and its margin's (as built in step 4: a poster flush against the run would cover its margin and hide
  it) across the wall lane's rows `f-6 ..= f-5`, where no hung piece may hang.
- A shelf's **run** is `SHELF_WIDTH` = 10 cells drawn `┬────────┬` (brackets at both ends). A half-built one is the
  bracket at its wall's end and the half of the run nearer that wall (`┬────`, 5 cells). Its **margin** is the 2 cells
  past each end of the run, on its row.
- **Small decor** is what may stand on a shelf (`Spec.shelf`: the plant, the clock, the gremlin).

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
- **On a shelf**: a new **gremlin** figurine and small decor (the plant, the clock); never furniture.
- **6a's payoff**: decor on a shelf counts toward the room's beauty, and she sometimes stands under it and gazes up (a
  still act, inside 5c's band).
- Every 6a step keeps the golden hashes and terrain snapshots unchanged in a home with no structure.

Second round, the user's answers to this design's questions (2026-10-10):
- **Shelf height: "Highest that fits."** High (above the wall decor, never meeting the poster or the clock, with room for
  her to stand on it in 6b) where the pane allows; otherwise low (just over her head), the poster and the clock packing
  beside it. Both are built (D3, D3a; steps 4 and 5a).
- **The plant and the clock go up only by a repair** that finds the shelf the best place for them; a clock on a shelf is
  a mantel clock, same art. The gremlin always wants the shelf (`High`).
- **Lines**, all four: the kit's pitch `"A shelf! Up high!"`, the gremlin's `"Aww, a little gremlin!"`, the grievance
  `"It wants a shelf..."`, and `"Up it goes..."` at a build stage's start.
- **The gremlin's art: variant A** (lavender glaze, flat wooden plinth, 2 × 2 cells), ASCII `oo` / `ww`; variants B, C
  and D are never wired. Step 7 doesn't wait on anything.

Third round (2026-10-10, after the critiques):
- **The user's terminal is 191×46.** Step 3 measures the bundled layout there too (which pane takes a high shelf, which a
  low one, with typical content); step 9's review shots are at 191×46; the stage scenes run on a frame where a shelf
  fits, as the `LookOut` precedent does (tests.rs:2371-2384).
- **Reaching up: "a little hop."** She hops at the moment a shelf's stage goes up, or a piece goes up onto a high shelf
  (D7c).
- **Brackets**: the shelf is drawn `┬────────┬`, with 2 blank cells past each end, rows with no text preferred; a
  half-built shelf shows one bracket and half the run. For 6b: the brackets' stubs never count as climbable poles.
- **All four extras are in 6a** (step 8, each budgeted): the gremlin's tiny life (he turns round while she's asleep or
  out, and she notices: `"?"`, `"Did you move...?"`); a gaze-up line pool (`"Hi, little guy."`, `"You stay up there."`,
  `"Behave, okay?"`, `"He's watchin' me..."`) and more gazing for a day or two after something new goes up; pleased
  beats (`Act::Admire`'s `"hehe"` after each build stage and after setting a piece on the shelf, and a wished kit's
  pitch `"A shelf for the gremlin!"`); the flat-pack gag (the kit's unpack ending on `"Which way's up?"`).

## Decided unless the user objects

Each is recorded in decisions.md when it lands.
- **Two heights, high first.** High: rise from `HIGH_RISE` = 7 (the first row above every hung piece, derived from the
  `Spec` table), with `HEIGHT` clear rows over the run when it goes up; hung pieces never meet it. Low: rise
  `LOW_RISE` = `HEIGHT` + 1 = 5, with `LOW_ABOVE` = 2 clear rows over it (the gremlin's height and the clock's; the
  plant, 3 tall, stands on a low shelf only where a third row is clear, else `Extent::holds` keeps it off). So a low
  shelf needs a pane 9 rows tall (7 inside), a high one 13 (11 inside). Chosen once, when it goes up, saved as its rise.
- **A low shelf's block belongs to the shelf**: hung pieces keep out of it, as the window keeps clear of what stands
  (`hung_clear`, generalised: D3a). A low shelf goes up only where it meets no hung piece as laid out, so it never
  pushes one; later deliveries and moves hang beside it. The block is geometry only (D3a).
- **6b's standing** needs `HEIGHT` clear rows over a shelf: every high shelf has them when it goes up; a low one only
  in a pane 11 rows or taller. A low shelf in a shorter pane is for decor only, and 6b's ladder trigger must name a
  shelf she could stand on.
- **Off the sides by `max(WIDTH, SPACE)` = 6 columns**, margin included, from both walls (the run itself `EDGE` = 8
  in), purely geometric: no door space (6 columns against its wall) can ever meet a run or its margin, whatever her
  door does, so the door chooser never picks a wall whose space
  would, by construction, and a shelf's layout reads no door.
- **One shelf a pane at most; in 6a one shelf in all** (one kit, one of each piece).
- **Its ink is its host's border's**: each frame, the `fg` of the host pane's bottom border cell under the run's middle,
  as `wall_glyph` takes the wall's (mod.rs:5157-5167). Her image keys already hold border colours, so no new cache
  key; it follows the user's theme and the pane's focus as the borders do.
- **Where it goes is chosen on the frame** when she unpacks the kit, then saved. It's never chosen again: a frame that
  can't hold it (text or a protected pane over it or its margin, its pane hidden, too narrow or short, the chat come
  to meet it) hides it and what stands on it for that frame, all at once, no rain, back when the frame holds it (as
  text closets a piece today; no hysteresis, as pieces have none). Rejected: choosing it text-blind like her door
  (text fills most panes from the top, so such a shelf would mostly be hidden).
- **Two build stages** after unpacking: the half nearer its wall, then the rest. She puts the first up straight after
  unpacking (chained, as a crumple chains to a use), so the box never just vanishes; the second on a later game day.
  A stage is a 6 s act standing beside where it goes, arms up (`Pose::Stretch`, `Face::Curious`, no `"nnn~"`), its
  cells appearing at the end with the hop.
- **The hop is for high shelves**: a stage of a high shelf, and a piece set down onto a high shelf, end in one beat of
  `Pose::Jack(1)` (arms up, feet apart, lifted: art.rs:247-261; ASCII `JACK[1]`, sprite.rs:424), about 400 ms, then
  `Act::Admire` (step 8). A low shelf is within her reach: `Pose::Stretch` only. Existing frames, no new art. A bigger
  hop (her image a row up) would be code, not art (a `Rig` with a larger `bob`): not done unless the user asks.
- **Reaching a shelf is arms up**: a lift from or a set-down onto a shelf draws `Pose::Stretch` (`Face::Curious`),
  never `Pose::ToeTouch`.
- **Building is not a home act**: the per-day pacing bounds it, not her mood's 0–3 cap (else a lazy streak stalls it).
- **She can't stand on a shelf in 6a**, structurally: shelf cells are neither ledge nor pole of hers (D5). Pieces still
  stand on it, but only on cells `read_ground` wrote this frame.
- **The gremlin stands anywhere small decor stands**, a floor too, and **wants up high** (a rule, `High`). It's sold
  only on a frame where a shelf stands or could go up; `High` is felt only then, only on the gremlin's room's floor,
  facing it. That felt rule is what the wishlist reads.
- **The gaze up is a use**, `Use::GazeUp`, of the gremlin or the plant standing on a shelf (not the clock: her clock
  glance already looks at it), offered only on the shelf's room's floor, from beside the piece facing it, 20–45 s:
  `"hehe"` for its first 2–3 s, then `Pose::Gaze`, `Face::Curious`.
- **The kit is sold last** (after furniture and decor) unless wished, and only on a frame where a shelf could go up now;
  a wish that can't be sold this frame doesn't block the next catalogue piece.
- **Older builds** (today's, pre-shelf): they ignore `structures` and drop it when they save; a piece on a shelf loses
  its anchor and goes to its kind's room pane from `rooms` (the Living pane for the plant, the clock, the gremlin),
  at its share, and is dropped if no Living piece stands; the gremlin and the kit are skipped as unknown kinds. Step
  2a records what today's reader actually does with a hand-written 6a record.

## Design

### D1. Strips: a shelf variant, one admission rule, no zips

```rust
pub(super) enum Strip {
    Bottom(Nook),
    /// The shelf she put up in `nook`'s pane (one a pane at most).
    Shelf(Nook),
}
impl Strip {
    pub fn nook(self) -> Nook;                 // total: every strip has a pane
    pub fn room(self) -> Strip;                // Strip::Bottom(self.nook()): a shelf is part of the room below
    pub fn takes(self, item: Furniture, boxed: bool) -> bool; // a floor takes anything; a shelf, `Spec.shelf` kinds out of their box
}
```

- **The seven irrefutable `let Strip::Bottom(nook) = ..`** (ledger.rs:205, 365, 553; rules.rs:433; mod.rs:1624, 4341;
  room.rs:2321) all need only the pane: each becomes `.nook()`. rules.rs:433 and mod.rs:1624 print the strip
  (`Users shelf`); `Summary.owns` (ledger.rs:464-473) prints a shelf piece's strip as `Users shelf` too.
- **No zips.** `raw_strips` (room.rs:805) becomes `pane_strips(nooks) -> Vec<PaneStrip>` with `PaneStrip { nook, rect,
  strip, raw }`; `choose` (room.rs:1279) and `doorstep` (room.rs:2429) read `rect` from it instead of zipping
  `plan.nooks` with strips by position. Shelves never enter `pane_strips`.
- **Floors-only paths take a pane by type.** `move_off(&mut self, from: Nook, ..)` (room.rs:2196) iterates `pane_strips`
  for targets; `project_with` (room.rs:2159) moves only `Strip::Bottom` strips off (a shelf whose plan is gone hides its
  pieces, never moves them). `doorstep`, `spot`, `stage_arrange` (mod.rs:4329) and the door's chooser iterate
  `pane_strips`.
- **One admission rule**, `Strip::takes` (the kind and whether it's boxed: a shelf takes no parcel; as built in step 3's
  review fixes), read in two places, and by the ledger read:
  1. `Home::on` (room.rs:1839), which every layout reads (`lay_strip`, room.rs:2019): a piece a strip doesn't take is
     never laid out there, so every search that judges a candidate by its layout refuses it with no filter of its own
     (the repair's `evaluate`, rules.rs:605; `spot`'s `find(item)?`; `admits`).
  2. `StripPlan::takes(item)`, called by every per-piece enumeration of plans before it makes candidates (cost, and the
     closet's agreement with the search): `rules::search`'s `Places` loop (rules.rs:930), `placements` (room.rs:978)
     and `on_own_strip`/`packed_on`.
  `Spec.shelf` lands with it (step 1: twelve rows, true for the plant and the clock), so `takes` is never a
  placeholder. The ledger read (`held_up`) never puts a piece on a shelf that doesn't take it (or that the record lacks
  or hasn't finished) either: it lands on that pane's floor at its share, unanchored (D2).
- **Lanes depend on the strip.** On a shelf everything stands: `lift(item, boxed, strip)` (room.rs:697) is 0 there;
  `Prop::lane`/`needs` (room.rs:708-722), `Shown::lift`/`lane` (room.rs:1448-1462) pass their strip. Every place that
  lanes a piece for a candidate strip uses the candidate's lane: `search`'s `plan.along(old.lane())` (rules.rs:935),
  and `evaluate`'s `e` and `pin` (rules.rs:626, 634). (`holds` there keeps its footprint-only test: see step 1.)
- **Where she stands to reach a piece.** `StripPlan` gains `host: Extent` (a pane plan's own `raw`). `stand(prop, plan,
  left)` (room.rs:2684) sets a new `Shown::stood: i32`, the floor she stands on to reach it (`host.floor`). `middle_of`
  (mod.rs:4266, read by `reach`, `set_down_refused`'s way back at 4079 and `Judged.home` at 4316), `Shown::seat`
  (room.rs:1534) and `room_for` (room.rs:2738) read `stood`; for a pane piece it equals `floor`, so nothing moves.
  `StripPlan::pin`'s share (`at`) is of `host`.
- **Rooms are floors.** `Home::rooms` (room.rs:1663), `role_among` (room.rs:616), `has_room`/`becomes_den`
  (rules.rs:575-597), `Before.roles` (rules.rs:548), `evaluate`'s tier and `role_without`/`role_of(laid, to.strip)`
  (rules.rs:753-770), and `spoils`/`newly_spoilt` (rules.rs:689-690) key on `strip.room()`, iterating pane plans only:
  a shelf is never a room (a decor-only shelf would read as a den), and an empty shelf is no "strip with nothing on it"
  tier. `Faces`, `Near`, `Apart`, `AgainstWall` and `Belongs` never involve small decor, so they stay on `strip`.
- `DoorWall` (room.rs:827) deserializes through `#[serde(try_from = ..)]` refusing a shelf strip. In code only the
  chooser makes one, from `pane_strips`; making `DoorWall.strip` a `Nook` in memory would touch every `d.strip == strip`
  comparison for a state no code path builds, so the guard is the deserializer.

### D2. The structure model and the ledger

```rust
/// Something she has put up with the UI's own lines (phase 6). 6b adds the ladder, 6c the partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum Structure { Shelf(Shelf) }

/// A shelf on `nook`'s back wall: `SHELF_WIDTH` cells from `anchor` (its near end, counted from that wall of the
/// pane's raw strip), `rise` rows above the pane's floor; `built` of `SHELF_STAGES` (2) stages up; `fresh`, the day
/// something last went up on it (step 8).
pub(super) struct Shelf { pub nook: Nook, pub anchor: Anchor, pub rise: u16, pub built: u8, pub fresh: Option<Dated> }
```

- `Home { props, door, structures: Vec<Structure> }` (room.rs:1631). `Home::owns(ShelfKit)` is true while the kit is a
  prop **or** a shelf stands. `Home::shelf(nook)`, `Home::shelves()` read it. (The mend basis hash, mod.rs:4441, changes
  value with the new field even when empty; it's only compared for equality and isn't traced.)
- **Ledger** (ledger.rs, by the pattern of `door` and `seen`): `Saved.structures` with `skip_serializing_if =
  "Vec::is_empty"`, declared before `door`; `Raw.structures: Option<serde_json::Value>`, read **before** the props (a
  shelf piece's anchor is checked against it), entry by entry: an entry that isn't a structure this build knows (a later
  build's ladder, garbage) is skipped; so is a second shelf on one pane, a second shelf at all (6a), `built` above
  `SHELF_STAGES`, or a rise under `LOW_RISE`. `paced` lands in step 5c, the same way.
- **Pieces on shelves.** `SavedAnchor.strip` is a `Strip`, so a shelf piece writes `{"Shelf":"Users"}`. On read, a
  piece stays on its shelf only if the record has that shelf **finished** (`Shelf::finished`: all `SHELF_STAGES` up;
  only a finished shelf is a strip, D6), the shelf `take`s its kind, and it's out of its box. Any other (a shelf the
  record lacks or hasn't finished, one that doesn't take it, a parcel) keeps only its pane and its `at`: it lands on
  that pane's floor at its share, unanchored. (Step 2a's review: a piece kept on an unfinished shelf would have no plan
  until it was finished, hidden yet owned, out of every rule's and the closet's sight; and a parcel only ever comes in
  on a floor, where she unpacks it. Nothing in production writes either; the reader refuses both.) A shelf too high for
  the frame (rise 40) is kept with what's on it: that's D3's hiding, back when a frame holds it.
- **A kit is never an unboxed prop, by one accessor** (step 5b): `Prop.boxed` and `Shown.boxed` become private to
  room.rs, read through `boxed()` = `self.boxed || self.item.is_kit()` (read by `lane`, `lift`, `needs`, `uses`, beauty,
  `piece_look`, and every caller outside room.rs), set only by constructors; `Home::unbox` returns false for a kit.
  So no path, scratch (`doorstep`'s unboxed probe, room.rs:2448-2462), stage (`give`, `spot`) or record, can show or
  judge an unboxed kit; it's always its box.
- `an_unknown_strip_falls_back_to_the_rooms_pane` (ledger.rs:859) is changed to feed `{"Ledge":3}` so it keeps testing
  an unknown strip.
- `Summary` (ledger.rs:391) gains `structures: Vec<String>` (`"Users shelf, 2 of 2 up"`), skipped when empty, so
  dump.rs's JSON golden holds.

### D3. Where a shelf goes, and whether it stands this frame

**Geometry** (pure; read by `Home::extents`, D6): `Shelf::run(host: Extent) -> Option<Run>` with `Run { from, to, y }`:
`from` from the anchor (`Side::Left`: `host.from + offset`; `Side::Right`: `host.to - offset - SHELF_WIDTH`), `to = from
+ SHELF_WIDTH`, `y = host.floor - rise`. `None` (it can't hang this frame) unless `from >= host.from + EDGE` and `to <=
host.to - EDGE`, with `EDGE` = `max(WIDTH, SPACE)` + 2 (the margin) = 8, and `LOW_RISE <= rise` and `host.rows - rise >=
LOW_ABOVE` (inside the pane with room for decor). No door input. A built-1 shelf's run is the bracket and half nearer its
wall; built 0 draws nothing (D3a still blocks for it).

**Choosing the spot** (`Home::shelf_spot(buf, plan, keep_off, kit_at) -> Option<Shelf>`, room.rs, no RNG):
- **Highest that fits**: every pane is tried high before any is tried low.
- **High**: rise from `HIGH_RISE` up to `host.rows - HEIGHT`, the run, its margin and the `HEIGHT` rows over the run free.
  **Low**: rise `LOW_RISE` only, the run, its margin and the `LOW_ABOVE` rows over it free, and its block meeting no hung
  piece of the host as laid out (`laid_and_shifted(plan.nooks)`, the host's `Lane::Wall` pieces).
- Within a height, candidates in this order: panes (the gremlin's pane if it stands on a floor, her living room (her
  sofa's pane, out of its box: the user's eighth round, 2026-10-12), the pane she unpacks in (`kit_at`), then
  `plan.nooks` order; never a pane meeting `plan.chat`); then **rows with no text** before rows with
  text elsewhere in the pane's interior (the user's preference: a shelf among a table's text reads as the UI's own
  rule); then a high run that **touches no hung piece** (columns meeting a poster or clock right below it, at rise 7)
  before one that does; then the lowest rise; then lefts ordered by the distance of the run's middle from the gremlin's
  middle if it's in this pane, else the pane's middle, the left first of two as near.
- One pass per pane builds a column-wise "free cells upward" count and a per-row "has text" flag, so the search is
  linear in the pane's cells (~1.5k a pane). It's computed only when asked (D7b: the channel's queue resolving to the
  kit or the gremlin; a boxed kit shown), never every paint.
- Saved with `anchor` from the nearer wall, its rise (which says high or low for good) and `built: 0`. A pane that grows
  later keeps a low shelf low; one that shrinks below the geometry hides it.

**Per frame** a shelf stands when its geometry holds (else `Home::extents` has no plan for it) and every cell of its
built run and margin passes D4's test. Otherwise it's hidden that frame, with what stands on it (they stand only on
written cells: D6), never moved, never re-hung. Logged at info once per change of what her home shows, as the
closet is (`note_closet`; CLAUDE.md: a user-visible state change; step 2b's review), from the arm that keeps the
cells.

`HIGH_RISE` = `max(hang + rows)` over the hung kinds + 1 = 7, `LOW_RISE` = `HEIGHT` + 1 = 5; both land in step 2a with
the test that computes them (from the `Spec` rows, `sprite::HEIGHT` and the tallest standing piece).

### D3a. A low shelf in the wall lane

- **The block**: for a shelf with rise under `HIGH_RISE`, its run's columns and its margin's (step 4) across the wall
  lane's rows from the lane's top (`f - (HIGH_RISE - 1)`) down to its own row (`f-6 ..= f-5` at `LOW_RISE`):
  `Shelf::block(host)`. It exists whenever the shelf's geometry holds, from `built: 0`,
  whatever the frame's cells: layout stays pure geometry (room.rs:1884-1886).
- **Hung pieces keep out of it.** `hung_clear` (room.rs:2074) today moves only the window clear of what stands, to the
  nearest place between its neighbours that meets none, keeping its order, else leaves it out alone. It takes the
  host's blocks as obstacles for **every** hung piece by the same search: a poster whose anchor puts it over a low shelf
  hangs beside it; on a wall too crowded for that, it's in the closet alone. Its `shifts` say so, so the repair search's
  pushed-hung refusal (rules.rs:605) and the yield rule (`plan_strip`, room.rs:1721) see it. With no block it is
  exactly today's `hung_clear` (step 4 proves it against a frozen copy). "Too crowded" is judged between its
  neighbours' places on the wall alone (as packing has them), even a neighbour about to be moved past the block itself
  (so a wall with room once the order is kept can still leave one out; the rule as written, kept).
- **What makes way is judged where it will hang.** A delivery (`Home::admits`) and hung pieces moving off a pane that's
  gone (`Home::move_off`) judge a piece that packing puts on a block where the layout hangs it beside the block, and
  refuse if it's left out (step 4's review: the block has no stand-in among what shows, as what stands has for her
  window, which is still judged where packing puts it, as before 6a). Only a block ever moves a poster or the clock,
  so with no low shelf both are exactly as before.
- **The host's layout reads its shelf**: `Home::lays_from` and `StripLaid` (room.rs:891, 1852) record the host's shelves
  beside its pieces and door, and `relaid` (room.rs:1969) lays the pane out again when they differ. A shelf strip's own
  layout reads its `Shelf` record and `plan.host`; `StripLaid` records both, so neither strip is ever reused stale.
- The window (rows `f-3 ..= f-2`) never meets either height; decor on a low shelf stands in rows `f-8 ..= f-6`, beside
  any hung piece.

### D4. `read_ground`: one write, before every read, undone by every arm that doesn't own it

```rust
/// What `read_ground` wrote into the frame this paint: each cell, with the real cell it covers.
#[must_use = "an arm keeps it (into its painted list) or undoes it"]
pub(super) struct Written { cells: Vec<Frozen> }      // structures.rs (new module)
impl Written {
    pub fn none() -> Self;
    pub fn contains(&self, x: i32, y: i32) -> bool;
    pub fn real(&self, buf: &Buffer, x: u16, y: u16) -> Option<Cell>;   // what the app drew there
    pub fn keep(self, painted: &mut Vec<Frozen>);   // structure cells first: the bottom layer
    pub fn undo(self, buf: &mut Buffer);            // each written cell back to its `under`
}
pub(super) fn read_ground(buf: &mut Buffer, home: &Home, plan: Plan, keep_off: &KeepOff) -> Written;
```

- **The per-cell test** (shared by `read_ground` and `shelf_spot`; tested against an independent oracle): on screen;
  blank (`symbol().trim().is_empty()`), not a wide glyph's trailer (`width(cell(x-1)) < 2`), not untouchable; outside
  every `keep_off.protected` rect (the **gated** view's), outside `plan.chat`; not a cell of her layer (the layer's
  **pre-validate** cells, `visit.layer.cells()`, layer.rs:291: moved text, sources, torn cells); not in a made piece's
  cover; not in the cover of a laid piece other than those on this shelf. A written cell is `Frozen { glyph: '┬' or
  '─', ink: <the host border's fg>, under: <the real cell>, face: None, burst: false }`.
- **Where it runs: once, at the top of `paint_state`** (mod.rs:2499), after `observe`, `gate` and `whole_glyphs`
  (2501-2504) and before `errand_progress`/`nudge_due` (2506-2507), in every state but `Leaving`, and only at or above
  `MIN_WIDTH`×`MIN_HEIGHT` (mod.rs:248). In `Absent` only when the paint could begin a visit (visits on and a poke due:
  `nudge_due` → `send` is the one way in within a paint), so the commonest frames write and undo nothing (step 2b's
  review). It can precede the visit's `layer.validate` (2747) because validate reads only
  the layer's own cells (layer.rs:516-560, `fits` 301-327), which `read_ground` never writes. This departs from the
  brief's wording ("right after `layer.validate`") for that reason; plan.md's record says so when step 2b lands, and
  `TextLayer::validate`'s doc ("call before anything of hers is painted", layer.rs:513-515) gains the exception.
- **Owned or undone, every frame, whatever the state became.** The state can change after the write within the paint
  (`errand_progress` → `leave`, mod.rs:3403; the arrival's `call_off` → `rain_home`, 2585/1858; `out_by_door` → `leave`,
  2359; the `vanish` returns at 2580, 2613, 2907; `send` from `Absent` beginning a visit). So the match's arms, not the
  state at the top, decide: `Visiting` and `Away` `keep` it into their painted list (structure cells first:
  `visit.painted` at mod.rs:3187, `empty.painted` at 5769); `Arriving` that becomes a visit keeps it; `Absent`,
  `Leaving`, a failed arrival and every `vanish` return `undo` it before they read `buf`. `Written` is `#[must_use]` and
  moved into exactly one of the two. So "the goodbye's frame has no structures" holds whatever state the frame began in,
  and a visit begun by `send` from `Absent` has its shelf on its first frame.
- **Made structural:** the production terrain reads take `&Written`; `Terrain::read` without it becomes `#[cfg(test)]`
  (its six production callers are all in mod.rs: 2515, 2649, 3446, 4120, 5716, 5727). The goodbye's read at 2649 passes
  `&Written::none()` (it has undone the cells), the only production call that does, with a comment saying why.
  `nudge_due`/`send`, `furnish` and `paint_empty` take `&Written`.
- **Keep-out.** The whole placed run and margin of every shelf whose geometry holds, written or not (an unbuilt half,
  a run hidden by text), joins `base` (mod.rs:2799: no pulled or swapped text, no made piece lands there), the made
  keep, the text reads that offer her pulls (so none is offered across a shelf, to be refused once she's there), and
  her fallback door's spots (below). Bubbles already avoid non-blank cells (`bubble_spot`, mod.rs:3897-3903). Not `Visit::solid` (mod.rs:1020): she
  walks in front of a shelf as of a border.
- **Rains.** Kept cells go first into the painted lists, so they're the bottom layer the dissolve's stacking expects
  (`first_at`, dissolve.rs:106-110: the first cell at a place holds the real UI's `under`). The focused-pane rain
  (mod.rs:2724-2745) rains a shelf out of a focused pane (and D3 keeps it out while focused), `rain_home` takes it with
  her empty home, and the goodbye's dissolve owns it.
- **Every burst cell's `under` reads through `Written::real`**: `draw_art`'s closure (mod.rs:5499, her box and doorway),
  the pieces in her image (5601), line-art `unders_of` (5345), `draw_front` (5805, shared with the visit at 3138), and
  ASCII `draw` (3871). Other cells keep the cell they were drawn over (the dissolve stacks them on purpose). This is 6a
  work, not a 6b guard: her box can cross a shelf's row in 6a, standing on a text `─` in the host pane or climbing a
  text `│` (the long-visit alphabet has both, tests.rs:708). Then `Dissolve::unchanged` (dissolve.rs:220) compares her
  burst cells with the goodbye's frame, which has no shelf, and they match.
- **Her fallback door never stands over a shelf**: `Ground` (door.rs:141) gains `structure(x, y)` (from the terrain's
  `STRUCTURE` flag; door.rs's two test impls at 532, 581 answer false), and `on_floor` (door.rs:353-367) refuses a box
  meeting one, as it refuses a wall glyph. That reads only this frame's cells, so `door_place` also refuses a box meeting
  the keep-out (step 2b's review): a spot chosen while a shelf is hidden would otherwise meet it when it shows, and the
  door would hop. The terrain's check stays as a second one (redundant in production: written cells lie in the
  keep-out).
- **The rule's exception, documented:** decisions.md *Houseguest reads the frame she stands on (2026-10-01)* gains an
  amendment and design.md:2619-2622 a clause: her structures are the one thing of hers written before the reads,
  because they're terrain: once a paint, before anything reads the frame, and undone by every frame that doesn't show
  her home; every burst `under` she records is the real frame's. The comments at mod.rs:2596-2603, 2643-2646 and
  2744-2746 say the same.

### D5. The terrain: lines, but neither ledge nor pole of hers (6a)

- `Terrain::read_guarded(buf, solid, guarded, graphics, written)` (terrain.rs:229): a `lines` flag `STRUCTURE` (16) on
  written cells; `ledges.push(!skip && floor && !structure)` and `poles.push(!skip && pole(symbol) && !structure)`
  (terrain.rs:263-264), `LEDGE` likewise. So a shelf is never a platform (`add_platforms`, terrain.rs:318), never a
  `Ground::ledge`, and its brackets (`┬`, which `pole` would take) give no link (`find_links`, terrain.rs:518). It stays
  `STROKE` and `quiet`: she passes in front of it and her line-art image redraws it (`┬` is in `strokes`,
  graphics.rs:333) as a border.
- That keeps her off a shelf in 6a: `settle`'s nearest-platform fallback (osaka.rs:10230), arrivals, `find_rest`, doors
  in space and `nearest_clear` pick among platforms, and there is none there. 6b lifts the ledge half for a shelf her
  ladder reaches; the brackets stay no pole (the user's note).
- With nothing written the flag is never set: the terrain snapshots and every terrain read are unchanged.

### D6. Shelves as strips

- `Home::extents(nooks)` (room.rs:1711) and every layout path built on raw strips (`pin_anchors` 1867, `laid_out` 1894,
  `strips_laid_out` 1901, `laid_and_shifted` 1914, `strips_laid` 1929) take their strips from one `Home::raw_extents`:
  the pane strips in `nooks` order, **then** each **finished** shelf whose geometry holds, as `Strip::Shelf(nook)` with
  `Extent { from, to, floor: y, rows: host.rows - rise }`, no space, `host` its pane's raw extent. Panes first, so every
  test indexing `extents(..)[i]` still reads a pane.
- **Pieces on a shelf stand only on written cells.** `project_with` and `fits` (room.rs:2159, 2839) take `&Written`, and
  a piece on `Strip::Shelf` shows only where every cell of its floor row is a written cell this frame. The app's own
  `────` rule across a hidden run never holds one up. The layout stays pure: rules judge "up there" while it's hidden.
- **Moves onto a shelf** come only from the repair search, which already enumerates every plan (rules.rs:930) and now
  meets a shelf's plan for small decor. On a shelf, its candidate lefts are the one centring the piece on the run (a
  lone ornament stands mid-shelf) and the packed places beside pieces already there. `fits_now` (rules.rs:837) judges it
  on the frame, the shelf written. The pocket carry stands her on `stood`; `Act::Lift`/`Act::SetDown` on a shelf draw
  `Pose::Stretch`.
- **InChat skips shelf pieces**: a shelf the chat comes to meet is hidden (D4's test), so its pieces aren't judged in
  the chat (InChat's judge and `closetable`, room.rs:957, skip `Strip::Shelf`); a repair never carries them down for it.
- **`move_off` never touches a shelf** (D1).
- **The closet** (`stranded`/`placements`, room.rs:946-1100) considers a shelf only for pieces it takes.
- `clock_on` (mod.rs:213) reads the pane strip of `clock.strip.room()`, so her glance at a mantel clock is taken from its
  room's floor.

### D7. The kit

#### D7a. Choosing a spot (step 5a): D3's `shelf_spot`, pure, with its oracle and cost test.

#### D7b. The kind, the channel, delivery, unpacking (step 5b)

- **The kind.** `Furniture::ShelfKit` (room.rs:20), appended to `ALL` (13). `Spec` (`name` "shelf kit", its pitch,
  footprint (5, 2) so its parcel shows whole, `offers: &[]`, `beauty: 0.0`, `uses: &[]`, an ASCII flat pack filling the
  footprint for the lint at room.rs:2884) plus `Spec.kit: Option<Kit>` (`Kit::Shelf`); `Furniture::is_kit()`.
  `legacy()` false; `RoomKind::of` → `Living` (ledger.rs:575); `scrap` arm; `art::parts` gives it nothing unboxed (it's
  never unboxed: D2's accessor), and the art lints that render unboxed pieces (`every_prop_renders_inside_its_footprint`,
  art.rs:4325; the review sheets at art.rs:3406, 4384) iterate `ALL` less kits, saying why;
  `every_parcel_and_channel_renders_inside_its_box` (art.rs:3357) and `lean_rests_the_flap_on_the_parcel` (4236) include
  it (its parcel).
- **Frozen kinds for old tests.** `#[cfg(test)] Furniture::PRE_SHELF: [Self; 12]` (today's `ALL`) and
  `CATALOGUE[..10]`: every existing strategy, proptest body and kind lint is pinned to them (step 5b's list, from a grep
  of `Furniture::ALL` and `CATALOGUE` in tests and strategies: rules.rs:1459, 2798; room.rs:3237, 4519, 5264;
  tests.rs:5656-5658, 7789, 7810; mind.rs:1708; rules.rs:1447; brain.rs:995; tests/credit.rs:129). Their seeds keep
  meaning what they meant. The kit and the gremlin get new proptest functions sharing those bodies.
- **The stage**: `give` and `wishlist()` (mod.rs:1486, 1508) skip kits (`give(ShelfKit)` puts up a finished shelf where
  `shelf_spot` says, as `Scene::Shelf` does); `send_parcel` may send the kit.
- **The channel** (`advert`, mod.rs:194): `advert(ledger, shop_now, needs, wish, shelf_room)`. A three-way `next` by
  tier (furniture, decor, kits; the tier from `Spec.kit` and `decor()`): the wished kit first if it can be sold now;
  else, beauty pressing, decor then furniture, else furniture then decor (as today); kits last. `CATALOGUE` (mod.rs:169)
  appends `ShelfKit`. The kit (and, step 7, the gremlin) is sold only when `shelf_room`: `shelf_spot` finds a spot on
  this frame, computed only when the queue resolves to one of them. A wish that can't be sold now doesn't block the next
  piece. A boxed kit counts for `advert`'s `idle` (mod.rs:196); the residual risk (the frame changes for good between
  purchase and unpacking) is accepted: the box waits visibly, logged at debug once per change.
- **Delivery**: as any parcel, through her door's flap (`doorstep`, pane strips only). The doorstep's `seats` closure
  (mod.rs:4125) judges the kit's box with an ungated `Unpack` seat (she could open it), so the kit's delivery never
  waits on a shelf spot.
- **Unpacking**: the existing `Unpack` (4–6 s, osaka.rs:1787), but the kit's unpack seat in `seats`/`spots_for`
  (mod.rs:4537-4600) is offered only while `shelf_spot` finds a spot (a flag computed in the paint, as `cat` is, only
  while a boxed kit is shown). **The kit is the one parcel exempt from "unpacked at once"** (design.md *Getting
  furniture*; `every_parcel_on_her_doorstep_gets_unpacked`, tests.rs:5632, stays pinned to the pre-shelf catalogue):
  she can't open a shelf she can't put up. `HomeEvent::Unpacked(ShelfKit)` in `record` (mod.rs:3688) sets `visit.raise =
  Some(kit)` instead of `unbox`; `raise` persists on `Visit` until the next `furnish` (`record` runs twice a paint,
  2756 and 3189), which chooses the spot on its frame and calls `Home::raise(shelf)`: the prop goes, the shelf comes
  (`built: 0`), logged at info. With no spot by then, or the visit ending first, the box stays shut (debug) and she
  unpacks it again once a spot exists.

#### D7c. Putting it up (step 5c)

- `Want::PutUp` (brain.rs:520, `ALL`; `row(12.0, [(Nesting, 1.0)])`, `own_sake`, between Arrange's 6 and Unpack's 40),
  method `put-up/stage` (mind.rs) binding `Job::PutUp(PutUp { x, y, side })` (scenes.rs:236) at `Chances.put_up`
  (osaka.rs:29): beside the run's next half, on its room's floor, facing it, where she may stay. Offered only while the
  next half's cells (and margin) pass D4's test on the frame, and `paced` allows.
- **Chained after unpacking**: `Unpacked(ShelfKit)` sets `Osaka.then_put_up`; her next decision takes `PutUp` in the
  Continuation bucket if it's offered (as a crumple chains to `Act::Admire` and a use, osaka.rs:4826-4832), else drops
  the chain silently. The flat-pack line (step 8) plays before she walks.
- `Act::PutUp { since, until }` (6 s, `Moves::Still`, `Act::SetDown`'s ActProps): `Pose::Stretch`, `Face::Curious`, no
  `"nnn~"`; `"Up it goes..."` at its start. At its end `HomeEvent::PutUp`, which `record` commits after re-checking the
  half's cells on that frame (not committed if they fail): `built += 1`, `paced` updated, saved at once. On a high
  shelf the commit frame starts the hop: one beat of `Pose::Jack(1)`, about 400 ms (an `Act::Hop { until }`,
  `Moves::Still`, no line), then (step 8) `Act::Admire`. Cut short, nothing is committed.
- **Set-downs onto a high shelf hop too**: `Act::SetDown`'s commit onto a high shelf is followed by the same `Act::Hop`.
- **Pacing.** `Ledger.paced: Option<Paced { on: Dated, stages: u8 }>` (skipped when `None`). `Dated` is today's `Said`
  (mod.rs:3593), moved to ledger.rs and made serializable: the game day (`Osaka::day`, osaka.rs:4365), or with no clock
  reaching her the visit. `PutUp` is offered only while `paced` isn't today's with `STAGES_A_DAY` (1) done. A stage is
  never offered on the unpacking day if one was done (it can't have been: one kit).
- **Credit and lints**: `credit_path(PutUp, "put-up/stage")` (osaka.rs:7647; tests/credit.rs:19), the cue lint
  (`Scene::PutUp`), `all_lists_every_want` (brain.rs:1144), `every_want_has_a_sane_row` (brain.rs:1210), the act lints,
  the exhaustive `Act` matches (osaka.rs:1003, 1229, 1282, 3516, 4864, 5569, 8078, 8132, 11247, …: the compiler finds them).

### D8. The gaze up: `Use::GazeUp`

- `Use::GazeUp` (room.rs:410, `ALL` 12): not `inside`, not `asks_room` (as `LookOut`), `lingers`; not in `ANY_USE`'s
  felt-on lists (rules.rs:61): a piece on a shelf is in no chat, no door space, and set there by her. `Shown::uses()`
  (room.rs:1437) gives `[GazeUp]` for a real, unboxed gremlin or plant standing on a shelf (the clock excluded), else
  what it gave before. Its spots (`spots_for`, mod.rs:4570): beside the piece, her box's near edge one or two columns
  past the piece's, still under the run, facing it (`Pose::Gaze` is in profile); at `stood`; offered only while she
  stands on the shelf's room floor (no climb for it). `seats_of`'s `restful` check applies.
- `Want::Use(Use::GazeUp)`: `row(5.0, [(Daydreams, 0.4)])` (under `Gaze`'s 6/0.5, brain.rs:716); step 8 adds the
  freshness factor. **Amended by step 8's retune** (the user's sixth round): base 8, above `Gaze`'s 6, with
  `Factor::Fresh(BOOST)`; measured, not argued (see Step 8, "As built": base 5 gave her 0.05 gazes up a game day). Its script (script.rs; `LookOut`'s arms are at 469, 500, 539, 584, 619, 643, 685, 724, 758, 845,
  1458, 2479, 2517, 2558, the or-pattern ones to read): a prelude of `Act::Admire`'s look (`Face::Pleased`, `"hehe"`,
  2–3 s), then `Posed::Still(Pose::Gaze)`, `Face::Curious`; 20–45 s (osaka.rs:1787 table), so 5c's settling-in applies.
  A use of her things, it eases *beauty* in a pretty room as every use does (osaka.rs:7475-7583).
- Lookups by kind (`Guest::cue`'s `_` arm, mod.rs:1541-1548; `Scene::furniture()`, stage.rs:296; brain.rs:995) find no
  piece for `GazeUp`: `Scene::GazeUp` sets up its own shelf with the plant on it.

### D9. The gremlin, `High`, and the wishlist (step 7)

- `Furniture::Gremlin`, appended to `ALL` (14): the approved variant A as `snippets.md`'s *Suggested `Spec`* has it:
  footprint (2, 2), ASCII `oo` / `ww`, ink `(Rgb(178, 154, 224), LightMagenta)`, `symmetric: false`, `offers: &[Decor]`,
  `beauty: 1.0`, `hang: None`, `shelf: true`; pitch `"Aww, a little gremlin!"` (the user's choice, not the sheet's
  suggestion, which is over 24 characters). Art: only the `gremlin` group from `worktree.diff` into `art/props.svg`;
  `art::parts` gains `(Furniture::Gremlin, _) => &["gremlin"]`. `RoomKind::of` → `Living`; ledger kind-list tests
  (ledger.rs:1345) extended. `CATALOGUE` gets it after the poster; it's sold only when a shelf stands or `shelf_room`
  holds (D7b), so the story it starts can finish.
- `Rule::High(Furniture)` (rules.rs:19): broken while the piece is laid out, out of its box, on a strip that isn't a
  shelf. A `RULES` row appended (rules.rs:74; indices of the others unchanged): `High(Gremlin)`, felt on sight, grievance
  `"It wants a shelf..."`; `kind(row)` (rules.rs:191) gains its label. `search` gives `High` only shelf strips as places
  (a floor can't mend it), so an unmendable `High` costs no search.
- **Felt only where it means something**: `on_sight` (mod.rs:3642) feels `High` only while she stands on the gremlin's
  room's floor and a shelf stands or `shelf_room` holds, and (the user's seventh round) never while her shelf kit is on
  order, boxed or going up; she says it turned to face it, on the InChat terms (her mood
  would mend, once a game day). With a shelf up it's said only on a visit she'll mend (`to_mend`'s cap, osaka.rs:9639),
  so the carry follows the line.
- With no shelf, no move mends it: `Arrange` is offered (`may_arrange`, osaka.rs:8444) and its `lift` finds none
  (mind.rs:689), so it doesn't bind; whims are keyed by name and an unbound want draws nothing (brain.rs:883-920).
  Step 7 checks in the census's want table that this costs her no slot of `choose`'s four; if it does, `may_arrange`
  also asks for a repair (the class, DoorClear included).
- **The wishlist**: `Rule::wants(self) -> Option<Furniture>` (`High(_)` → `ShelfKit`; 6b and 6c add theirs). `wish` =
  for the first rule she has felt this visit (`Osaka.felt`, in the order felt) that is still broken and whose kit she
  doesn't own, that kit. Per visit, as felt rules are; she feels it again on sight early in the next. Computed where
  `advert` is (mod.rs:2955, 3008).

### D10. Beauty from a shelf

`beauty_at` (mod.rs:233): her room is the pane strip whose floor she stands on (`pane_strips`), and its beauty sums what
shows, unboxed and real, whose `strip.room()` is that strip: a shelf's decor counts for the room below, a hidden
shelf's doesn't (it isn't shown).

### D11. The stage

`Scene::PutUp` (stage.rs:28): gives her a shelf where `shelf_spot` puts it (`built: 0`), clears today's pacing, cues
`PutUp`. `Scene::Shelf`: a finished shelf with the plant on it (the gremlin from step 7; as built in 5a, the clock where
the frame hides the plant, as on a 9-row pane's low shelf: Step 5a, and the fifth round's open question). `Scene::GazeUp`: its own shelf
and plant, then the use. `Scene::ALL` (stage.rs:170) grows from 58 to 61.

`every_scene_has_a_spot_in_the_stage_room` (tests.rs:2355) loops `(100, 30)` and `(80, 24)`. At 100×30 two bundled
panes are 9 rows (The List and the Playlist, `[Add New]` on its first inside row): a low shelf by geometry, if
`stage_ui`'s content leaves its rows blank. Step 3 checks which; the three scenes follow the `LookOut` precedent
(tests.rs:2371-2384): where no pane takes a shelf on that frame, their note is `Err`, asserted, with the frame
asserted to be one that can't (`80×24` always: no pane is 9 rows). Each gets an early "visibly happened" observable in
the `happened` match (tests.rs:2454): `Pose::Stretch` for PutUp, `Pose::Gaze` for GazeUp. A test beside it runs the
three on a frame where a shelf fits (`rooms(100, 30)`: Users 13 rows), so they're exercised whatever the stage room
holds.

## Tests

The door batch's rules hold: written first, run, **confirmed failing for the stated reason** (a test that can't be red
today is a **guard**, proven by a reviewer's mutant); deterministic; every houseguest test loops `for graphics in
[false, true]`; panes filled with text where terrain matters; pinned case counts through `proptest_cases(N)`; a proptest
failure committed to its regressions file and pinned as a unit test. **No existing proptest's strategy or value space
changes**, and **appending to a strategy's domain is a strategy change**: old strategies are pinned to
`Furniture::PRE_SHELF` and `CATALOGUE[..10]` (D7b); new kinds and shelves get new functions sharing the bodies, with
their own strategies. Each step's exit greps `Furniture::ALL` and `CATALOGUE` in strategies and proptest bodies. **Oracles
are independent**: geometry is computed from the `Shelf` record and the nook rect alone, never from `extents`,
`Written` or the shared per-cell function.

By step:
- **1.** `takes` unit table; `a_garbled_record_never_puts_her_door_on_a_shelf` (red within the step: the variant first,
  then the `try_from`; the rest of the record reads intact); one unit test per floors-only site (`move_off`,
  `project_with`, `doorstep`, `spot`, the door's chooser) with a shelf the only free run (they land on a step-3 shelf
  plan, so they're written in step 3 and listed here for the class).
- **2a.** Ledger: `a_shelf_round_trips`; garbage, a later build's structure, a second shelf, `built` =
  `SHELF_STAGES + 1`, rise `LOW_RISE - 1`, a kit both as a prop and a shelf, each skipped; a piece on a shelf the record
  lacks lands on its pane. **Before the reader changes**, a hand-written 6a record (a shelf, the plant on it, a boxed
  kit, `paced`) is read by today's `from_json` and the outcome recorded; that reader is frozen as the model
  `as_a_pre_shelf_build_reads` beside `as_an_older_build_reads`. Goldens: `the_record_with_a_shelf_as_written`
  (character for character, pinning field order: `seen`, `structures`, `door`, then 5c's `paced`, then `clock_sent`), and
  `the_record_as_written` and dump.rs's unchanged. `rises_clear_every_hung_and_standing_piece`.
- **2b.**
  - `read_ground_takes_only_blank_free_cells` (proptest): half the shelves valid by construction for their frame, the
    rest any geometry; text, wide glyphs, protected and focused rects, a chat, layer and made cells, laid pieces. Every
    written cell blank and no trailer, its margin blank, outside protected, chat, layer, made and other laid covers;
    inside `host.from + EDGE .. host.to - EDGE` on row `host.floor - rise`; each shelf all or nothing; `built: 0` writes
    nothing, `built: 1` exactly the bracket and the half nearer its wall, `built: 2` `┬────────┬`; with no structures
    the buffer byte-identical. Non-vacuity: written-shelf counts over the run.
  - `a_shelf_is_no_floor_of_hers`: a finished shelf with ≥ 4 on-screen rows over it, both modes: terrain with and
    without the written cells has the same platforms, links and `Ground::ledge`; no pole at a bracket.
  - **The goodbye, frame by frame** (`her_shelf_rains_out_with_her_goodbye`, both modes, a finished shelf with the plant):
    for `t < RAIN_FROM_MS` every shelf cell shows its frozen glyph; for `SETTLE_BY_MS ≤ t < DURATION_MS` every shelf cell
    equals `real`; in no `Leaving` frame does a settled shelf cell show the glyph again. Red against "Leaving writes".
  - `an_errand_goodbye_rains_her_shelf_out` and `a_called_off_arrival_rains_her_shelf_out`: every frozen shelf cell is
    painted at least once after `RAIN_FROM_MS`, never settled on the first frame. Red against the write keyed on the
    state at the top.
  - **Direct `under` tests**: `draw_art`, `draw_props` (both modes), `draw_front` and ASCII `draw` with a hand-made
    `Written` (a `#[cfg(test)]` constructor) whose cell lies in her box or a piece's footprint: every burst `Frozen`
    there has `under ==` the real cell. Mutant: the closure reads `buf`.
  - `she_waves_goodbye_under_her_shelf`: a smoke test (her standing on a text `─` under it), not the guard.
  - `her_fallback_door_never_stands_over_her_shelf`.
  - Text, focus, resize, both modes, visiting **and away** (`paint_empty`): text over the run or its margin hides the
    whole shelf at once and what's on it, back when it goes; a cursor over one cell toggles all ten (asserted: the
    decided behaviour); a focused pane rains it out and keeps it out; too narrow hides it, a resize and back restores it;
    hiding its pane hides it and moves nothing.
- **3.**
  - `only_small_decor_stands_on_a_shelf` (new proptest over every kind anchored on random strips, shelves included):
    `laid_and_shifted` lays none on a strip that doesn't take it; `search` proposes none; `placements` agrees with the
    search (a new `stranded_iff_no_room_takes_it_with_a_shelf`); `from_json` of such a record lays it on its pane.
  - `a_piece_stands_on_its_shelf_only_where_it_was_written`: a host row of the app's `────` under a hidden run holds no
    piece up.
  - The floors-only unit tests (step 1's list).
  - `a_plant_in_the_chat_goes_up_on_her_shelf`: an industrious mood forced, its precondition asserted (`search` returns
    exactly the shelf move, centred), carried up, set down arms up (`Pose::Stretch`, never `ToeTouch`), shown, mended.
  - `every_repair_mends_and_breaks_nothing_with_a_shelf` (new function: high shelves, small decor, furniture breaking
    InChat/DoorClear).
  - Siblings: a clock on a shelf gives `ClockOn` at the host's floor (and a fed run where she reads the time from it); a
    decor-only shelf adds no room and moving the plant onto it changes no role; a shelf piece's seat, reach and
    `middle_of` are at `host.floor`; text over the shelf in a running visit lowers the beauty she feels.
  - **Long visits with shelves** (`long_visits_with_a_shelf_never_touch_what_is_protected` on `rooms_frame`, and a
    `chat_over` variant; `her_days_with_a_shelf_never_touch_what_is_protected`, tests/away.rs): `long_visit_of` gains a
    `structures` argument and an `owned` form placing small decor on a strip; `proptest_cases(16)`, 60 000 ms spans,
    each measured in its commit (target ≈ 3 s at 32 cases). Per frame, visiting and away: `most` counts the frame's
    shelf cells; each shelf all or nothing; no shelf cell protected, in the chat, a layer cell, in a made cover or in
    another piece's cover, or within `SPACE` columns of either wall; every shown piece on a shelf is small decor on a
    finished shelf, its floor row the independently computed run (ASCII: `┬`/`─` there; line art: in the piece's image);
    her feet never on a shelf cell; at written positions every burst `under` equals `real` and the first frozen cell at
    each place has `under == real`; the final `end == real`. A size step drops the host pane and restores it: the
    record unchanged, pieces back at their anchors. Non-vacuity companion (both modes): frames with a high shelf
    written, a low one written, a piece on a shelf, a shelf hidden by text, and one coming back.
- **4.** `no_hung_piece_meets_a_low_shelf` (new room.rs proptest: heights up to 20, the clock in the hung set, a shelf
  of either height, a low one at `LOW_RISE` or a row higher; each asserted to occur; the closet's placements out of a
  chat checked too); `a_high_shelf_changes_no_hung_place`; with no shelf, `hung_clear`
  equals a frozen `#[cfg(test)]` copy of today's over `decorated()` (`PRE_SHELF`) homes, `shifts` listing only the
  window (deleted in step 9); `a_move_relaid_strip_by_strip_is_laid_out_as_a_whole_with_a_shelf` (new function; red with
  `StripLaid` not recording the shelf); `a_poster_delivered_hangs_beside_a_low_shelf`;
  `every_repair_mends_and_breaks_nothing_with_a_low_shelf` (rises 5, 6 and 7). **A sanctioned exception to the strategy
  freeze**: step 4 widened step 3's shelved long visits (both) and `her_days_with_a_shelf_never_touch_what_is_protected`
  from rises `7..=10` to `5..=10`, as step 3's notes asked (they were high-only until the block existed); they are 6a's
  own strategies, no pre-6a one changes, and no committed regression seed touches a shelf. Their homes hang nothing, so
  their hung-piece check is a guard; step 9 may add a long visit of its own whose kinds hang (poster, clock) beside a
  low shelf.
- **5a.** `a_shelf_hangs_as_high_as_fits`: a brute-force oracle over the frame (every pane, rise and left, written
  without the shared per-cell function): `shelf_spot(..).is_some() == oracle.any()`, and the result is the oracle's first
  in D3's order. Generator: panes exactly 9 and 13 tall among others, text, wide glyphs, standing, hung and made pieces,
  layer cells, protected and focused rects, doors, chats; counts of high, low and none each non-zero.
  `shelf_spot_is_cheap` (cells visited ≤ k × pane cells) and an ignored timing test beside `repair_search_is_cheap`
  (rules.rs:2295): `read_ground` plus `shelf_spot` on a 400-wide dense three-pane frame.
- **5b.** `the_kit_is_sold_last_and_at_the_usual_cadence`; `no_kit_is_sold_where_no_pane_takes_a_shelf` (geometry: the
  bundled layout at 80×24; text: a tall pane whose rows are full); `a_kit_is_never_unboxed` (every path: stage, doorstep
  probe, record); `a_delivered_kit_is_unpacked_iff_a_shelf_can_go_up` (new property: its unpack seat offered iff
  `shelf_spot` finds a spot; when offered, the other promises of `every_parcel_on_her_doorstep_gets_unpacked` hold);
  `a_kit_with_nowhere_to_go_stays_boxed`.
- **5c.** Pacing, deterministic, on offers not choices: `Ledger::new_at(.., tue(15, 50))` with the kit boxed
  (precondition asserted); after stage 1, `PutUp` absent from her offers for the rest of Tuesday, across a restart
  (`Guest::restore(Ledger::from_json(&guest.ledger.to_json()))`); offered again across 23:59 → 00:00; the unfed case, one
  a visit. Red against `paced` not saved, or reset by a restart. `unpacking_a_kit_puts_its_first_half_up`; `a_stage_is_
  never_committed_over_text`; `she_hops_up_to_a_high_shelf` (the commit frame followed by `Pose::Jack(1)`; none for a
  low shelf).
- **6.** A property over the shelf long visits: every `GazeUp` has its piece on a shelf, her feet at `host.floor`,
  beside it, facing it; `she_gazes_up_at_what_is_on_her_shelf` (a seed with its precondition asserted: offered),
  opening on `"hehe"`.
- **7.** `the_gremlin_wants_a_shelf_and_gets_one` (end to end over game days, on a home lacking a furniture piece and a
  decor piece, so "first" is visible); `a_felt_high_rule_puts_the_kit_first`; `a_rule_felt_last_visit_is_no_wish_this_
  one`; `no_gremlin_is_sold_where_no_shelf_can_go`; `no_gremlin_grievance_where_no_shelf_can_go` (every pane full of
  text, a fed day, both modes: the line never said, `Arrange` never offered for it); `the_grievance_is_said_in_its_room_
  facing_it`; `every_repair_mends_and_breaks_nothing_with_a_gremlin`; `rules_hold` with a gremlin (new functions).
- **8.** One test per extra (step 8).
- **9.** Look: a fixed-frame `Scene::Shelf` test in ASCII pinning cells and ink (`┬────────┬`, the plant on it), and a
  line-art check that the brackets and run reach the image keyed with the host border's colour.

**Gates** (every step): `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` (no `dead_code`: each item
has a production reader in its own step, or is `#[cfg(test)]` until it does), `PROPTEST_CASES=32 cargo nextest run
--workspace --all-targets`. The CPU perf test is `dessplay-rendezvous/tests/perf.rs::houseguest_visit_cpu_is_negligible`
(no shelf in it); steps 2b and 5a carry their own cost tests instead: 2b's is deterministic (out of the house,
`read_ground` writes, and lays out, nothing unless the paint could bring her in, as no frame then shows a shelf;
`out_of_the_house_her_shelf_is_written_only_when_she_could_come`), and the timing of `read_ground` with a shelf on a
dense frame is 5a's ignored timing test.

## Steps

Each step: one implementer (tests first and confirmed failing, implement, the gate, commit with `jj commit`), two
reviewers, a fixer. Strictly sequential, each compiling and passing the gate alone. Sizes are changed lines, tests
included. **No push of `master` mid-phase**; no protocol change, so `stable` stays put. **Goldens**: every step is
predicted byte-identical (no golden scene has a structure, kit or gremlin; `read_ground` writes nothing; the new ledger
fields are skipped when empty; new wants and uses are offered only with a shelf; new guards test for a shelf before
drawing a whim); any move is investigated with the trace diff (testing-strategy.md:776-819), never re-recorded on faith.

| Step | What | Lines |
|---|---|---|
| 0 | This design, the critiques, the approved sheet (docs) | — |
| 1 | Strips: `Strip::Shelf`, `takes`, `Spec.shelf`, rooms are floors, `pane_strips`, `stood` (refactor) | ~750 |
| 2a | Structures and the ledger; `Written` threaded as `none()`; `Terrain::read` test-only; rises | ~850 |
| 2b | `read_ground`: the write, keep/undo, the terrain flag, rains, unders, keep-out, door fallback | ~1150 |
| 3 | Shelves as strips: written-only floors, small decor, repairs onto them, beauty, the measurement | ~1300 |
| 4 | The low shelf: the wall lane hangs beside its block | ~800 |
| 5a | `shelf_spot`: highest that fits, its oracle, its cost | ~650 |
| 5b | The kit: kind, frozen kinds, the `boxed()` accessor, channel, delivery, unpack gate, `raise` | ~1200 |
| 5c | Putting it up: want, job, act, chaining, pacing, the hop | ~1050 |
| 6 | The gaze up | ~650 |
| 7 | The gremlin, `High`, the wishlist | ~1050 |
| 8 | The four extras | ~350 |
| 9 | Census, deep runs, review shots, docs | ~450 |

### Step 1. Strips (refactor, ~750 lines)

**Goal.** D1 with no shelf anywhere. **Sites.** room.rs:629 (enum, `nook`/`room`/`takes`), the `Spec.shelf` column,
697-722 and 1448-1462 (lanes by strip), 805 (`pane_strips`), 837-868 (`StripPlan.host`, `takes`, `pin` of `host`),
1272-1321 (`choose` without the zip), 1663-1686 (`rooms`), 1839 (`on` admits), 1867-1990 (one `Home::raw_extents`,
panes only for now), 2159-2296 (`move_off(from: Nook)`), 2297-2390 (`spot`), 2419-2560 (`doorstep` without the zip),
2684 (`stand` → `stood`), 978-1100 (closet: `takes`), 1534, 2738; rules.rs:433, 548-597, 626, 634, 689-690, 753-770,
930-935; ledger.rs:205, 365, 553, 859, `DoorWall` try_from; mod.rs:213-245, 1624, 4266, 4339-4341. **Exit.** Every golden
byte-identical, snapshots unchanged. **Traps.** `Strip` is a ledger key: serde names unchanged for `Bottom`.
`furnished()` stays per strip (layout order); only rooms gather (the door chooser's "her pieces' strip" key reads
`furnished()` mapped through `Strip::room`). **As built.** `rules::search`'s `e.holds((cols, rows))` keeps today's
footprint-only test on purpose (only `along` takes the candidate lane): adding the hang there would move the
`examined` counts, and on a shelf it is exact anyway, since lift there is 0. An older record's share is of the
`host` both ways (`StripPlan::pin` writes it, `RawStrip::anchor_at`, read by `Home::pin_anchors`, reads it back).

### Step 2a. Structures and the ledger, threaded (~850 lines)

**Goal.** D2's model and ledger, `Written` threaded everywhere as `none()` (every production terrain read takes it; the
`under` sites read through `real`, a no-op), `HIGH_RISE`/`LOW_RISE`/`LOW_ABOVE`/`EDGE`/`SHELF_WIDTH` with the ledger's
reader of them. Behaviour-free by construction. **Sites.** New structures.rs (types, constants, `Written`); room.rs:1631;
ledger.rs (fields, `Raw`, structures before props, `Summary`, module doc); terrain.rs:229 (signature; `read` →
`#[cfg(test)]`); mod.rs 2515, 2649, 3411-3446, 4003/4120, 5345, 5499, 5601, 5716, 5727, 5805, 3871. **Tests.** Step
2a's list. **Exit.** Goldens, snapshots, ledger and dump goldens unchanged. **Docs.** design.md *Her record* (2641).
**As built.** `Shelf` has no `fresh` yet (its `Dated` is 5c's; step 8 adds it, skipped when `None`), and
`the_record_with_a_shelf_as_written` pins `structures` between `seen` and `door` with no `paced` (5c adds it to the
golden). The kit-as-a-prop-and-a-shelf case is a guard until 5b (`ShelfKit` is an unknown kind here). Today's reader
(`d1f53e7d`) was run on the hand-written record, as the *Older builds* bullet predicts: the plant on the shelf goes to
the Living pane at its share, unanchored, or is dropped with no Living pane on record; the kit is skipped. The
production read is `Terrain::read_with(.., &Written)`; `Terrain::read` is `#[cfg(test)]`. Only `Written::none`/`real`
and `Home::shelf` exist yet (the rest has no production reader before 2b); the constants without one are
`#[cfg(test)]`. Review fixes: the reader keeps a piece on a shelf only if the shelf is finished and the piece unboxed
(D2); `only_the_first_sound_shelf_is_read` (proptest, its own strategy), a later build's extra shelf field (`fresh`)
read past, `houseguest_shows_her_shelf` (dump.rs), and a paint-level test restoring the hand-written record
(`a_restored_piece_on_a_shelf_is_never_shown`, tests/away.rs, visiting and away).

### Step 2b. `read_ground` (~1150 lines)

**Goal.** D4, D5, D3's geometry and per-frame test; hand-placed shelves are drawn, rain, hide and are no floor of hers.
Not yet strips. **Sites.** structures.rs (`read_ground`, the per-cell test, keep/undo, the ink); terrain.rs:229-264 (the
flag on ledges and poles); door.rs:141, 353-367, 532, 581 (`structure`); mod.rs:2499-2507 (the write), the arms' keep and
undo (2580, 2585, 2613, 2643-2650, 2907, 3187, 5769), 2724-2747 (comments), 2799 (`base`, keep-out); layer.rs:513-515
(doc). **Tests.** Step 2b's list. **Exit.** Goldens unchanged; `read_ground` with no structures is one `is_empty`.
**Docs.** decisions.md: the amendment to *reads the frame she stands on*, and *Her structures are the UI's own lines*
(why line glyphs on brackets, why written before the reads, why undone by every frame not showing her home, why neither
ledge nor pole in 6a; rejected: written just before `Terrain::read`, which `fits` would miss; re-writing in the goodbye;
a write keyed on the state at the top). design.md:2619-2622. plan.md: the departure from the brief's wording.
**Traps.** Pre-validate layer cells, never `solid` alone. Structure `Frozen`s first and `burst: false`.

### Step 3. Shelves as strips (~1300 lines)

**Goal.** D6, D10, the floors-only tests: a finished shelf is a strip; small decor may stand on it (only on written
cells), by a repair; it counts for beauty. **The measurement** (an ignored `real_frame` test, its output in the commit
and plan.md): for the bundled layout at 100×30, 80×24, 191×46 and heights 32, 35, 40, 50 at width 100, with typical
content (3–5 users, ~10 playlist rows, ~20 franchises; `chatty_ui` or a fuller fixture), which pane takes a high shelf
and which a low one; and which of the stage room's 9-row panes does (D11). **Sites.** room.rs (`raw_extents` adds
finished shelves; `fits`/`project_with` take `&Written`; the shelf's centred lefts); rules.rs:905-1050; mod.rs:213-245,
4266, the lift/set-down pose on a shelf (osaka.rs:11246-11249); tests.rs long-visit additions. **Exit.** Goldens
unchanged; `repair_search_is_cheap` under its bound with a shelf on each pane. **Docs.** design.md *Strips and anchors*
(1850), *Rooms from contents* (1957), beauty (1571). **Traps.** The clock changes lane on a shelf. InChat skips shelf
pieces. **As built.** `raw_extents` adds each shelf `Shelf::finished()` (the reader's predicate) whose `Shelf::run`
holds. `fits`, `project_with`, `Home::frame`, `move_off`, `spot`, `doorstep`, `admits` and `rules::Frame` take the
paint's frame with what was written into it (`Drawn`; made pieces too, via `tend_made`/`made_stands`/`builds`); a piece on a shelf stands only on written
cells, and no other piece stands on one. `closetable` takes floors only (so InChat and the closet skip shelf pieces);
a shelf takes no parcel, by the one admission rule itself (`Strip::takes(item, boxed)`, read by `Home::on`,
`StripPlan::takes` in `search` and `placements`, and the ledger's `held_up`: the boxed question of step 2a's review).
The layout's text checks take a `structures::Drawn` (the paint's buffer with what was written into it, made only by
`Written::over`), never a buffer and a `Written` apart; a set-down is `Home::set_down`. `rules::search` weighs,
on a shelf, only `shelf_lefts` (centred, and packed beside each piece there) and `here`. `scenes::Lift.up` and
`Judged.up` carry "on a shelf" to the pose (`Pose::Stretch`, `Face::Curious` for a lift off or a set-down onto one).
The measurement is in plan.md (6a step 3's measurement): at 100×30 the bundled List pane is 9 rows and Users and the
Playlist 8 (not "List and the Playlist 9", as D11 says): only List takes a shelf there (low), the stage room likewise.
The repair properties' oracle (`mends`) now judges "newly broken" with the closet as it was, as `evaluate` does and
as the door batch decided (decisions.md, "The closet and a repair": a piece leaving the closet for a repair is no rule
newly broken); the new shelf property found the old oracle's disagreement on its first run, and the case, with no
shelf, is pinned as `a_repair_may_let_a_piece_out_of_the_closet`. For a piece on a shelf, `mends` checks it stands on
the run worked out from the record and the pane's rect (`stands_on_its_shelf`), not through `room::fits`. The
measurement's List figures were re-taken with twenty franchises in The List (`fuller_ui`).

### Step 4. The low shelf (~800 lines)

**Goal.** D3a. **Sites.** structures.rs (the block); room.rs:891, 1839-1860, 1969 (`StripLaid` records shelves),
2019-2131 (`hung_clear` with blocks for every hung piece), 1721. **Exit.** Goldens unchanged; the frozen-`hung_clear`
identity property; `repair_search_is_cheap` with a low shelf on each pane. **Docs.** design.md *Wall pieces* (1925);
decisions.md *A shelf hangs as high as fits* (the user's rule; why two heights; why the block, not shelves packed into
the wall lane as pieces; why a low shelf never pushes a hung piece going up; what 6b inherits). **Traps.** A block is an
input of the **pane** strip's layout. **As built.** The block (`Shelf::block`, structures.rs) spans the margin's
columns too: D4 shows a shelf only with its margin clear of every other laid cover, so a poster flush against the run
would hide it every frame. `StripPlan.block` carries it (a pane's floor under a low shelf; `None` on a shelf), read by
`hung_clear` (via `Home::block_on`) and by the closet (`first_clear`'s keep and `packed_on`'s refusal: a hung piece in
the chat is never placed on a block the layout would move it off). Only her window looks for a place to lean when moved
aside; every other hung piece goes to the nearest place. `StripLaid.shelf` records the pane's shelf; `relaid` reuses
`was` only while every strip's recorded shelf is this home's, else it works the strips out again from `was`'s pane
floors (`Home::with_shelves`, shared with `raw_extents`) and lays them all out (never taken within a search). After
review: a delivery and a hung move judge what packing puts on the block where the layout hangs it (D3a).

### Step 5a. `shelf_spot` (~650 lines)

**Goal.** D3's choice, pure, with its oracle and cost tests, and `Scene::Shelf` (D11), its first production reader
(the stage gives her a finished shelf where `shelf_spot` puts it), so nothing is dead code before 5b. **Sites.**
room.rs (`shelf_spot`, the free-upward pass); stage.rs, mod.rs:1517-1545 (the scene); the `every_scene_…` exception.
**As built.** `Home::shelf_spot(buf, plan, keep_off, kit_at)` (and `shelf_spot_visiting`, which counts what it reads)
returns `None` while she has a structure (one in all, in 6a). Each pane's wall is read once, lazily (a private `Wall`):
for each cell where a run or its margin could lie, the cells upward from it that pass `structures::free` in one
background (the test run in the cell's own background, the chain broken where the background changes), and for each
row whether the pane's interior has text; a place then asks its run and margin's cells in its pane's background, read
off the floor under the run's middle as `read_ground` reads it. The words, as built: a row **has text** if any cell of
the pane's interior on it has a non-blank symbol; a high run **touches** a hung piece of its pane (as laid out) whose
top row is the run's row + 1 and whose columns meet the run's (not its margin's); the distance is twice the run's
middle's from the pane's (`|2·from + 10 − (host.from + host.to)|`, the pane's walls); the anchor is `Extent::pin`'s (from the nearer wall, the
left of two as near). The gremlin's pane (first in order) is step 7's. The low block rule (`Shelf::block` against the
pane's hung pieces) is redundant with the cells today: the poster and the clock fill both of the block's rows, so one
meeting a block meets the run's row or its margin too, which the per-cell test refuses first (the property's
companion counts no frame where the block alone refused; dropping either check alone survives every test, both
together don't); it stands for a hung kind that doesn't. `Scene::Shelf`
(`Scene::ALL` 59) is set up at the top of the paint, before `read_ground` writes (not in `furnish`, as `Arrange` is),
so the cue's frame shows it and `stage::direct` reports what stands on it (the flag is taken only on a frame with room
for her, so a cue painted too small waits with its arrival); with a shelf already hers, it's finished where it hangs.
On it goes the first small decor that **shows there on this frame**, centred, tried as the frame will take it (mod.rs
`shelf_decor`: the shelf written into a copy of the frame by `read_ground`, her pieces projected over it by
`project_with` with what `furnish` keeps them off): the plant; where it doesn't show (on a low shelf with only
`LOW_ABOVE` rows, every 9-row pane, the stage room's List at 100×30 among them, or a word where its top would be)
the clock, a mantel clock there. Where none shows (her shelf can't hang this frame, or the frame hides it), nothing
of hers moves and nothing is given her; the note says so. A piece she has is moved up, unboxed, its facing kept. The
shelf is put up and saved even if the arrival it was cued with comes to nothing. At 80×24 no pane takes one (`Err`,
the frame asserted to have no pane 9 rows tall). Cost: what's kept off (her layer, protected rects, what she made,
her pieces' covers) is mapped onto each wall once, so the per-cell test asks only the frame and the chat pane. Timing
(`her_shelf_is_quick`, ignored, release): `shelf_spot` 0.15 ms on a blank 400-wide three-pane frame, 0.28 ms on a
dense one, 0.15 ms on a crowded one (a layer of 399 cells, a protected rect, what she made, five pieces; 0.63 ms
before the keep-off was mapped); `read_ground` with a finished shelf and the plant, written and undone, under
0.01 ms.

### Step 5b. The kit (~1200 lines)

**Goal.** D7b, `Scene::PutUp`'s setup. User-visible from here (a kit that goes up as `built: 0`, put up in 5c; no push
mid-phase). **Sites.** room.rs:20-140 (kind, `ALL`, `PRE_SHELF`, `Spec.kit`, `is_kit`), the `boxed` accessor (room.rs and
every outside reader), `Home::raise`; mod.rs:166-208 (`CATALOGUE`, `advert`), 1486-1510 (stage), 2955/3008, 3673-3720,
4003ff, 4125, 4537-4600; art.rs lints; scrap.rs; the frozen-kind list (D7b). **Exit.** Goldens predicted unchanged,
checked by trace diff. **CHANGELOG.** "Added: the shopping channel sells Osaka a shelf kit for her wall." **Docs.**
design.md *Getting furniture* (1787: the kit, tiers, `shelf_room`, the one parcel not unpacked at once). **Traps.**
`Unpacked(ShelfKit)` never reaches `unbox`.
**As built.** The boxed bit is a newtype, `room::Boxed(bool)` with its `bool` private to room.rs, in `pub boxed: Boxed`
fields of `Prop` and `Shown`, read everywhere through `Prop::boxed()`/`Shown::boxed()` (= the bit `|| item.is_kit()`):
the same guarantee as a private field for reads (no code outside room.rs reads the raw bit as a bool, so no reader can
see an unboxed kit), without breaking every struct literal and `..base` update outside room.rs (some sixty, mostly
tests). The raw bit is still compared by the derived `PartialEq`/`Hash` and printed by `Debug`, so the bit is
normalized where a kit comes in: `Home::add` and `Ledger::from_json` set it to what the accessor reads (a record's
kit written `"boxed":false` reads in boxed and writes back as read), and a write of `false.into()` to a kit is
harmless, as the accessor still says boxed. The frozen-kind sites D7b lists that still read `Furniture::ALL`
(brain.rs and osaka.rs test strategies, mind.rs, tests/credit.rs: `ALL.find(|f| f.spec().uses.contains(..))`) are
immune because the kit has no `uses` and comes last in `ALL`, so every `find` returns what it did; step 7's gremlin
has a use (`GazeUp`, step 6) and must re-check them. `advert(ledger, shop_now, needs, wish,
shelf_room: &dyn Fn() -> bool)`: the wish is `None` from production until step 7 computes it (its test passes it);
`shelf_room` is asked only for a kit at the head of what's left. The paint asks `shelf_spot` once (mod.rs `kit_room`,
right after `furnish`, before her layer is painted, what her visit keeps off as of the last frame) only while her kit
shows or the queue (asked with room assumed) comes to a kit, and hands the answer to `seats` (`kit_room`, read by
`spots_for`) and to `advert`. `furnish` takes `visit.raise` first, before laying out the frame: `Home::raise(shelf)`
puts the shelf up `built: 0` (written from the next frame on: structures change after `read_ground` in that paint,
harmless as built 0 writes nothing and `kept_out` is geometry). The stage's `give(ShelfKit)` puts up a finished,
bare shelf (`Guest.shelving: Option<Shelving>`, `Bare`, beside `Scene::Shelf`'s `Decor`), and the stage's shelf
replaces a boxed kit. `Scene::PutUp` is step 5c's (it needs `Want::PutUp`). Old strategies pin to
`Furniture::PRE_SHELF` (room.rs, `#[cfg(test)]`) and tests.rs `PRE_SHELF_CATALOGUE` (= `CATALOGUE[..10]`, asserted).

### Step 5c. Putting it up (~1050 lines)

**Goal.** D7c. **Sites.** brain.rs (`Want::PutUp`); mind.rs (`put-up/stage`); scenes.rs:236 (`Job::PutUp`); osaka.rs:29
(`Chances.put_up`), 406 (`HomeEvent::PutUp`), the `Act::PutUp` and `Act::Hop` arms, 1787, 4826-4832 (the chain), 7647;
`Said` → ledger `Dated`; ledger.rs (`paced`); mod.rs:3673-3720; stage.rs. **Exit.** Goldens unchanged; `visit_census`
unchanged; `shelf_census` (below). **CHANGELOG.** "Added: Osaka puts her shelf up on the wall, a bit a day, with a
little hop." **Docs.** design.md new *Structures* bullet (unpacking, two stages a day apart, pacing, the hop, where it
hangs, hidden when the frame can't hold it); decisions.md *She puts up a shelf, a stage a game day* (why per game day,
why not a home act, why chained after unpacking, why chosen on the frame, why never re-hung).
**As built.** One predicate offers and commits a stage: `structures::next_stage(drawn, home, plan, keep_off) ->
Option<Stage { nook, to, high, half: Run, floor }>`, true where the shelf at `built + 1` would be written whole on the
frame (its cells and margin `free`, or written by her already this paint: the half up and shown), which settles 2b's
open wording ("the next half's segment ±2") as "what the next paint would write"; `read_ground` and it share
`structures::showing`. The paint (`put_up_stage`, mod.rs, right after `kit_room`) takes the stage her act put up
(`HomeEvent::PutUp { nook, to }` → `Visit.put_up`, as a set-down is: `record` runs in the tick too, with no frame)
only if `next_stage` names it, then `Home::put_up`, `Ledger::built_today(today)` and `Osaka::put_up_done(high, now)`
(nesting eased, `Via::PutUp`; on a high shelf `Act::Hop`, 400 ms, `Pose::Jack(1)`, only while she still stands back
from it); then `Visit.stage` for this frame, gated by `Ledger::may_build(today)` (`STAGES_A_DAY` = 1, `Paced { on:
Dated, stages }`, `Dated` the old `Said`). `Chances.put_up: Option<scenes::PutUp { nook, to, high, x, y, side }>`
is resolved beside `arranging` on her terrain (`put_up_at`: her middle `WIDTH/2 + 1` left of the half or `WIDTH/2`
right of it, the nearer first, on the pane's floor where she may stay, facing it). The act ends on `Act::Stand`
(`AFTER_CARRY_MS`) so the paint judges it before she decides. Her kit's unpack ends on the same stand with
`Osaka.then_put_up` set; `choose_next` takes it once (after her routine, owed beats and a move under way, before a
piece she made) and goes to `chances.put_up` in the Continuation bucket ("put-up/stage"). A set-down onto a high
shelf hops too (`set_down_done(piece, high, now)`). `Scene::PutUp` is `Shelving::PutUp`: her shelf (`shelf_spot`'s,
or hers) at `built: 0` unless something stands on it, `paced` cleared, staged at the top of the paint as
`Scene::Shelf` is; `direct` pursues `chances.put_up`. **The shelf census** (`shelf_census`, ignored, release, 3 s):
32 of 32 seeds finished, stage 1 Monday 16:01–16:28 straight after opening, stage 2 first thing Tuesday (07:01–07:36:
`PutUp` at 12, for its own sake, is her first choice on waking), 0.63 game days from delivery (sd 0.01). **Review
fixes.** `Shelf::high()` holds the high rule (the block, `Stage.high`, and `Home::up_high(strip)` for a set-down's
hop: a shelf strip only); `scenes::PutUp` has no `high` (the commit's `Stage.high` decides the hop). She hops only
while `Osaka.standing_back` (set as the post-act stand of `Act::PutUp`/`Act::SetDown` starts, cleared by `set`).
`Osaka.put_up_waits` holds the stage she put up until the paint answers (`put_up_done`, or `put_up_refused(toward)`:
credit cleared, a `Loss::Heading` glance at the half, `structures::stage_half`); meanwhile `mind::put_up` (via
`Ctx.put_up_waits`) and the chain don't offer it, so a late paint can't send her back to it. `put_up_stage` reads
no frame when nothing's to take and nothing's due; the offer is logged once per change at the chances site
(`note_stage`, with "due, but nowhere to stand"). `Scene::PutUp` on a finished shelf with something on it changes
nothing. `Dated`'s kind switch is accepted (unfed is test-only; decisions.md). `visit_census` re-run, both modes,
against the parent: identical but for wall time.

### Step 6. The gaze up (~650 lines)

**Goal.** D8, `Scene::GazeUp`. **Sites.** room.rs:410-500, 1437; mod.rs:4570; script.rs; osaka.rs:1787; brain.rs;
rules.rs:61; stage.rs. **Exit.** Goldens unchanged; the band on the shelf rooms (Census). **CHANGELOG.** "Added: Osaka
sometimes stands by her shelf and gazes up at what's on it." **Docs.** design.md *Using her furniture* (1969).
**As built.** `Use::GazeUp` (`ALL` 12; not `inside`, not `asks_room`, `lingers`; out of `ANY_USE`, its lint saying
why). Which kinds: `Furniture::gazed_up_at` (wildcard-free: the plant today; step 7 decides the gremlin's row).
`Shown::uses()` gives `[GazeUp]` for a real, unboxed piece of such a kind `on_shelf()`, before its spec's uses, so no
`Spec.uses` names it and every `Furniture::ALL.find(|f| f.spec().uses.contains(&what))` site finds nothing for it
(`Guest::cue` has its own `Scene::GazeUp` arm; the test-only `unwrap_or(Sofa)` sites never draw it; tests/credit.rs
runs it in its own `shelf_room`). Spots: `Shown::gaze_up_spots(under)` = `[left, right, left - 1, right + 1]` of
`beside()` (her box one column past the piece's, then two), **kept only where under its run** (D8's "still under the
run": review fix; the run from `structures::hung_runs(home, nooks)`, threaded through `seats`/`seats_of`/`spots_for`),
at `stood`, facing it (the `Watch`/`Read` seat arm). With the plant centred on a 10-cell run, `left - 1` is the
margin, never a spot. mod.rs `spots(piece, what, runs)` is the one list of a use's spots (`spots_for` offers seats
from it; `seats_her` judges a seat against it). `room_for` never asks GazeUp (not `asks_room`): its arm returns
`true`. **The floor gate is at choice time**, in `mind::place` (`platform_at(seat) == Some(c.here)`), not in
the frame's seats: `stage::direct` picks the seat before it places her. `Want::Use(GazeUp)` appended last to
`Want::ALL` (31), `row(5.0, [(Daydreams, 0.4)])`, no factors (not a fun source, not in the chat; step 8 retuned it
to base 8 with `Factor::Fresh`: see Step 8). `ScriptId::GazeUp`
(`ALL` 28): `Still(Pose::Stand)`, `Face::Pleased`, `Bubble::Hehe` for `GAZE_UP_PLEASED_MS` (2500), then
`Still(Pose::Gaze)`, `Face::Curious`, the rest; `Common`, `Chat::Look`, `OnChat::LooksUp`. `use_duration` 20–45 s.
No settling arm (**kept by the user, 2026-10-11**: D8's "5c's settling-in applies" is the linger alone; a GazeUp → Sit chain as Gaze's would grow `settle_chains_ms` and the band's guard).
`census_group` "spacing out" (as `LookOut`); the census names it "gaze up". `Scene::GazeUp` (`ALL` 61) is
`Shelving::Gazed` (review fix: `shelf_decor` takes a kind filter, `Furniture::gazed_up_at`; where none shows, her
home is left unchanged, no shelf and no clock saved) with `furniture()` = `GazeUp` through `direct`'s seat arm (notes
`stage::NOTHING_UP` when nothing she gazes up at shows on a shelf, `stage::NO_ROOM_BESIDE` when it does but no spot
under its run is restful). **Deviation (D11):** the stage room at 100×30 is `Err` too: its one shelf is List's low
one, with 2 clear rows (the plant needs 3), so nothing she gazes up at goes up; `every_scene_has_a_spot_in_the_stage_room` asserts `Err` where no pane has `LOW_RISE` + the plant's rows
inside (both stage sizes), and the `Pose::Gaze` observable is in `tests::gaze_up::the_stage_gazes_up_where_her_plant_goes_up`
on `rooms_frame` 100×30 (the `happened` arm for it panics, unreachable, until step 7 flips `no_plant`);
`every_script_can_be_cued` cues `GazeUp` there too. Step 7's gremlin (2 tall) makes the stage room `Ok` with only its
`gazed_up_at` row (`Shelving::Gazed` never tries the clock). **Census:** no band room has a shelf, so the band can't
move; `visit_census` re-run both modes against 5c's. Review fix: tests/census.rs `Room.shelf` (her Users shelf and plant
up after her pieces) and `shelf_room()` (the resident's screen at 100×30, a sofa, a TV and her window, her plant up
high), printed beside the band (`printed_rooms`, `CENSUS_ROOMS=shelf`), in `visit_census` and `day_rooms` (printed; its look-outs not judged). Measured
(line art, 17 min, 20×4 seeds): moving 6.4 % lazy, 11.1–12.3 % ordinary and dreamy, 14.3–15.7 % industrious, set-offs
0.56–1.41 a minute: every mood inside its band. She gazes up rarely: 1 visit in 16 (`visit_census`), 0.05 a game day
(`day_census`), against 7.76 look-outs a game day there with or without the shelf (that screen leaves her little else).
GazeUp's weight is an open question for the user. Step 9's rooms (191×46, a low shelf, the gremlin) stay step 9's.
**Resize (review fix, the class):** a resize that moves the piece she's using (any use but unpacking) or lifting
from where she is startles her off it (`lost_seat`), as a piece gone does (mod.rs `seats_her`, `lifts_from`); judged
only on a resized frame, so a parcel nudging a piece she's on stays as it was (goldens byte-identical).

### Step 7. The gremlin, `High`, the wishlist (~1050 lines)

**Goal.** D9. **Sites.** room.rs (kind, `Spec`), art/props.svg (`gremlin` group only), art.rs:1373; mod.rs:169, 194,
2955/3008, 3642; rules.rs:19, 74, 191, 930 (`High`'s places), `Rule::wants`; ledger.rs:575 and kind-list tests;
stage.rs. **Exit.** Goldens unchanged. **CHANGELOG.** "Added: a gremlin figurine for Osaka's shelf; it wants to be up
high, and she'll get a shelf for it." **Docs.** design.md *The rules of her home* (2101), *Getting furniture* (the
wishlist, the gremlin's gate); decisions.md *The gremlin wants up high* (why a rule, why felt only in its room and only
when a shelf can be, the wishlist as felt rules naming kits; rejected: a shelf-only gremlin).
**As built.** `Furniture::Gremlin` (`ALL` 14; `Spec.shelf`, decor, `uses: &[]`: his gaze up comes from
`Furniture::gazed_up_at`, now Plant and Gremlin, so every `ALL.find(uses.contains(..))` site is unchanged) and
`Furniture::wants_up` (wildcard-free, the one list of kinds that want up: the sale gate, `shelf_spot`'s pane order, and a
lint that it equals the `High` rows of `RULES`). `Rule::High(Furniture)` (row 8, `felt_on: &[]`) and `Rule::wants`.
`rules::search` returns at once for `High` with no finished shelf (nothing laid out) and weighs only shelf strips for it.
`CATALOGUE` `[.., Poster, Gremlin, ShelfKit]` (`CATALOGUE[..10]` unchanged). mod.rs `wish(felt, broken, home)` (pure);
`wanting_up_here(broken, shown, nooks, her)` (each broken `High` whose piece's room floor she stands on, with an
`osaka::AloudAt { x, floor, from, to }`), read by `on_sight` and by `kit_room`, which now also asks `shelf_spot` while
she stands in such a room unfelt with no shelf, and while the queue (with the wish) comes to the gremlin.
**Deviation:** "a shelf stands" is two things: the gremlin is **sold** while she has a shelf at all (up or going up) or
one could go up; the rule is **felt** only while a shelf is finished or one could go up (not while one is going up: the
step's own test found the line, said as she walked into his room to put up the first half, dropping the chain).
`Osaka.owed_aloud: Vec<(usize, Option<AloudAt>)>`: a located line is said only while she stands on that floor, turned
to face `x`. `shelf_spot`: the gremlin's pane first (unboxed on a floor), then `kit_at`, then `plan`'s; `Wall::best`
takes the middle (twice the column) its `away` measures from. The stage room at 100×30 now plays `Scene::GazeUp`
(`Shelving::Gazed` puts the gremlin on List's low shelf). The gremlin's parcel is the box scaled into 2 cells (0.4 in
line art; ASCII ` _`/`|_`): flagged for step 9's review shots.
**Review fixes (2026-10-12).** "Up" for feeling `High` is a finished shelf **shown on this frame**
(`Written::holds_up`: its pane among what was written), not merely finished: a shelf the frame hides for good had her
say the line every game day with no repair possible; `rules::search`'s early return reads the same. `Osaka.owed_aloud`
is rows again (`owe_aloud(row)`): where a located line is said is the frame's when she says it (`Chances::wants_up`,
each broken `High` with its room's floor and his middle column, read by `on_sight`, `kit_room` and `aloud_owed`), so a
resize or his move since she felt it can't send her to a stale floor or column. Her kit's chain (`then_put_up`) is taken
by the first decision that reaches it (a reflex before it leaves it set; `settle_for_night` lets it go), and `say_aloud`
waits while it's set or her heading is `PutUp` (the test found a line said on landing from the hop to the stage cut the
trip too). `kit_room` memoises `shelf_spot` on `Visit.shelf_asked` (a hash of what it reads, and the frame compared
whole). The wish is logged as it changes (debug) and a purchase that was the wish at info. D9's census check that a felt
`High` with no shelf costs no slot of `choose`'s four is **an argument plus a test** instead of a census room
(`a_felt_high_with_no_shelf_takes_none_of_her_offers`: no repair, `Arrange` binds nothing, every want binds as with
`may_arrange` false). `no_gremlin_grievance_where_no_shelf_can_go` runs a game **hour**, not a fed day (cost; never felt
implies `may_arrange` is false for it, so `Arrange` is never offered for it), and now pins that `shelf_spot` is asked only
as what it reads changes. The end-to-end is one test a mode. The gremlin census's home now lacks the bookshelf and the
poster too, so "kit sold first" measures the wish. Once the kit is hers, the rule was felt and said with it on order
or boxed (a shelf could go up), not while the shelf is going up; the user chose none of the three (seventh round:
built in the fix after step 8, below; the chain-drop that first forced the third is fixed apart from it).

### Step 8. The four extras (~350 lines)

Each reuses what's there; each has one test, red first.
1. **The gremlin's tiny life (~120).** Once a game day, at the first moment she falls asleep for the night or goes out
   by her door, a draw keyed on (master seed, game day), 1 in 20, turns a gremlin standing on a shelf round (its saved
   `facing`, written at once; `symmetric: false` mirrors him for free) and sets `Guest.turned` (in memory, as the day's
   other carried state). Her next gaze up at him opens on `"?"`, then `"Did you move...?"` instead of `"hehe"`, and
   clears it. Test: a fed night, the draw forced by seed; the facing flips while she's asleep, never while she's awake
   and in sight; the next gaze up's lines.
2. **The gaze-up pool and freshness (~120).** `PoolId::Shelf` (mind.rs:1180, as `Sky` for the window's sill) with
   `"Hi, little guy."`, `"You stay up there."`, `"Behave, okay?"`, `"He's watchin' me..."`, at most one a gaze after the
   opening, cooled as pools are; for the gremlin only (the plant gets the opening alone). `Shelf.fresh` is set to today
   when a stage completes or a piece is set on the shelf; a `Factor` on `GazeUp` (×2 for that game day and the next).
   Test: the boost on the day after, gone the day after that; one pool line at most a gaze.
3. **Pleased beats (~80).** `Act::Admire` (`"hehe"`, 1.2 s) after each committed stage (after the hop, on a high shelf)
   and after a set-down on a shelf (~40); a wished kit's pitch `"A shelf for the gremlin!"` (24: `line!` checks it) when
   `advert`'s item came from the wish (`Play.wished`, script.rs:204) (~40). Tests: the beat follows each; the wished
   pitch only when wished.
4. **The flat-pack gag (~30).** The kit's unpack ends on `"Which way's up?"` in place of `"ooh"` (a per-item bubble in
   `UNPACK`'s last key, script.rs:2225-2238). Test: the kit's unpack's last bubble.

**CHANGELOG.** "Added: Osaka's gremlin has a life of his own." **Docs.** design.md *Using her furniture*, *Getting
furniture* lines.
**As built (2026-10-12).** (1) `brain::gremlin_turns(master, day)` (a pure hash, `TURN_SALT`, one day in
`TURNS_ONE_IN` = 20, pinned) drawn by `Guest::gremlin_turns(now)` once a game day (`Guest.gremlin_drawn`, per process;
fed only): from `Guest::advance` on the tick she falls asleep for the night (`Guest.asleep`, the last tick's: an
edge, so midnight passing in her sleep draws nothing; review of the build, red as first built), and at the top of
`out_by_door` (both of its callers). It turns an unboxed gremlin on a shelf strip (`Facing::turned`), marks the record unsaved and sets
`Guest.turned`, which reaches her as `Chances.turned`; her gaze up at him (seat on `Real(Gremlin)`) then plays
`GAZE_UP`'s branch 1 (`script::gaze_up_branch`: `"?"` to `GAZE_UP_PUZZLED_MS` 1000, `DID_YOU_MOVE` to
`GAZE_UP_NOTICED_MS` 3500, inside the shortest use) and pushes `HomeEvent::Noticed`, which `record` (new `turned`
argument) clears. A restart may draw the day again (a second turn, still hers to notice). Errands and goodbyes are
not "out by her door" here: she waves, in sight. (2) `PoolId::Shelf` (id 10, unbudgeted; `ALL` 12) and
`mind::SHELF` (n 1, d 2: one gaze in two), said through the sill's `Session` (`Osaka.sill`, `left: 1`) from
`lean_on_the_sill` for a gaze up at the gremlin, `GAZE_UP_LINE_GAP` (2–8 s, whims) after its branch's opening
(`script::gaze_up_opening_ms`); `sill_on` picks `SHELF` while the act is a gaze up. `Shelf.fresh: Option<Dated>`
(serde default, skipped when `None`, read leniently by `fresh_or_none`: garbage keeps the shelf), set by
`Home::put_up(nook, to, today)` (the stage's commit) and `Home::freshen(strip, today)` (`furnish`'s set-down onto a
shelf; `furnish` takes `today`); `Shelf::fresh_on(today)` (`Dated::within_a_day_of`: the day or the next, a day and a
visit never meet) → `Chances.fresh` → `brain::factor(.., fresh)`'s new `Factor::Fresh(BOOST)` on GazeUp's row.
**Retune (the user's sixth round):** GazeUp base 5 → 8 (`day_census`'s shelf room: 1.19 a game day, 50 in 42; base
5 0.05, base 10 2.83, base 15 5.48; look-outs 7.76 a day at 5 and 8); `day_census` now asserts 0.7–1.5 there. Fed
afternoon (line art, 17 min, 20×4) on the shelf room: lazy 6.5 %, ordinary 11.6–12.3 %, dreamy 11.0–11.5 %,
industrious 14.7–16.2 %, set-offs 0.57–1.41 a minute: every mood inside its band; `visit_census` there: 3 visits in 16
gaze up (was 1). (3) `room::Height { Floor, Low, High }` and `Home::height(strip)` (replacing `up_high`);
`Osaka::pleased_with(height, now)` (replacing `hop_if`): only while `standing_back`; low → `Act::Admire`
(`ADMIRE_MS` 1200) at once, high → `Act::Hop`, whose end now sets `Act::Admire` (Hop is only ever this). Both
`put_up_done` and `set_down_done(piece, height, now)` call it; a set-down's "There!" is said only when no beat is
(on a floor, or moved on): both at once would hide the "hehe". `Chances.wished` (this frame's wish) → `Play.wished`
(set at a use's start when `bought == wished`) → `script::pitch`: a wished shelf kit pitches `SHELF_FOR_HIM`.
(4) `UNPACK` branch 1 (`script::unpack_branch(item)`: kits) ends on `WHICH_WAY_UP`.
**Tests** (red first, each for its stated reason): osaka.rs `her_shelf_kit_unpacks_to_which_way_s_up`,
`a_wished_kit_is_pitched_for_her_gremlin`, `up_at_her_gremlin_she_says_a_line_at_most`,
`she_notices_her_gremlin_turned_round`, `she_is_pleased_with_each_stage_she_puts_up`; put_up.rs
`she_is_pleased_with_each_stage_of_her_shelf` (both stages, high and low, fresh); shelf.rs's two set-down tests
(the beat, fresh); gaze_up.rs `something_new_up_there_draws_her_eye_for_a_day_or_two`,
`she_notices_her_gremlin_turned_round_on_her_shelf`; gremlin.rs `her_gremlin_turns_round_while_she_sleeps_or_is_out`
(asleep on a school night, out to school; a turning and a still home each), `his_turning_is_drawn_as_she_goes_not_at_midnight`
(asleep through midnight, turned only as she goes out to school), `a_rule_felt_last_visit_is_no_wish_this_one`
(`Chances.wished`). Guards: brain.rs `the_gremlin_turns_by_the_home_and_day`, the fresh leg of
`a_boost_raises_its_want_only_in_its_time`; room.rs `what_goes_up_on_her_shelf_is_fresh`; ledger.rs's fresh round
trip and lenient read. Goldens and terrain snapshots byte-identical.
**Review fixes (2026-10-12).** *When and for which day (the class):* the asleep edge and `out_by_door`'s call are
replaced by one predicate, `Osaka::out_of_sight(now) -> Option<u64>`: asleep for the night **with her lamp off**
(`prop == LampOff`: not during `LAMP_ON_MS`, blinking, in sight), for the day **her night began** (`Night.first`'s
day; a stale or missing night: today), so back in bed after her midnight snack, a night errand or a new visit it's
the same night's day; or out by a door of hers to **school or work** (`leaving` or `Shift::Out`, and `hidden`),
today. Errands, goodbyes and the stage's way out are not it. `Guest::unseen_turns(now)` draws for that day each tick
(a level check, safe now the day is the spell's), before `gone_out` in `advance` and at the top of `out_by_door`
(the paint's caller); `Guest.asleep` is gone; `Guest::gremlin_turns(day)` never draws a day before one drawn
(`gremlin_drawn >= day`) and turns him only on a **finished** shelf of hers (`home.shelf(nook)` finished), out of his
box. *Noticed once shown:* `Osaka.notice_at` (the opening's end, set in `lean_on_the_sill` for the noticed branch,
woken for, cleared by `set`) pushes `HomeEvent::Noticed`, not the gaze's start. *Handled:* `record` clears `turned` on
a `SetDown` of the gremlin. *Census:* `Act::Admire { until, home }`; the shelf's beats (`pleased_with`, the hop's end)
are `home: true` and count as "home"; a pull's and a made piece's stay "mischief" (goldens record `act_name`, so
unchanged). *Accepted:* in ASCII his turn is invisible (`oo`/`ww` mirror to themselves; approved art, not changed;
an open question for the user). CHANGELOG entry moved under a new `## 2026-10-12`. *Tests:* gremlin.rs rewritten
around `Going` (Asleep, School, **Work**) and `Stands` (Up, Floor, HalfUp, Boxed), with an oracle for "gone" of the
test's own: `her_gremlin_turns_round_while_she_sleeps_or_is_out` (work added; written at once checked through
`ledger_to_save`, every earlier frame's taken), `he_turns_only_up_on_her_finished_shelf`,
`his_turning_is_drawn_as_she_goes_not_at_midnight` (a fridge and a snack night, both modes),
`he_turns_once_a_day_however_often_she_goes` (school, home, asleep), `he_never_turns_at_a_goodbye_or_unfed`;
osaka.rs `a_gaze_up_cut_before_she_asks_notices_nothing`, `a_fresh_shelf_doubles_her_gaze_up_as_she_weighs_it`
(the `factored` record), `she_notices_her_gremlin_turned_round` (noticed at the opening's end),
`she_is_pleased_with_each_stage_she_puts_up` (census group), the kit test's oracle by name; gaze_up.rs's noticed test
turns him through `Guest::gremlin_turns` (a round trip); shelf.rs's low-shelf test reuses `admires_within`; brain.rs
`her_gaze_up_weighs_as_measured` (a pin; the ignored `day_census` is the measure); ledger.rs
`a_day_of_hers_is_within_a_day_of_the_same_kind` and the new property `a_fresh_shelf_reads_back_as_written`
(existing strategies untouched). Red: each against its mutant (scratchpad `s8fix/red.log`). Goldens byte-identical.

**Fix after step 8 (2026-10-12, the user's seventh round): "It wants a shelf..." stops once the kit is ordered.** One
test, mod.rs `wants_coming(ledger, key)`: the kit the rule wants (`Rule::wants`) is coming, by
`Ledger::kit_coming(item)` (on order, or `Home::kit_coming`: in its box, or what it puts up going up). Read in three
places: `on_sight` feels no rule whose kit is coming (for `High`, on top of `shelf_room`); `kit_room`'s `unfelt` (the
ask of `shelf_spot` for her to feel it) skips one, so nothing asks while the kit is on order; and
`let_be_what_is_coming(ledger, osaka)`, a level check at the end of every `record` and after `furnish` each paint,
`Osaka::forget`s each felt rule whose kit is coming (the `Felt` and its owed row), so a line felt before the order is
never said, `to_mend`/`grieved` drop it (her Nesting no longer rises for it meanwhile), and `wish` reads nothing; however
the kit comes (bought, delivered, the stage's gift). Unchanged, and why: the sale (`advert` sells nothing while
anything is on order or boxed) and the kit's Unpack seat (`kit_room`'s `kit_shown` asks whatever she feels) never
depended on `High` being felt; `rules::search` keys on a shown finished shelf already; `aloud_owed` needs no gate (a
forgotten rule isn't felt). A lint in `rules::high::the_gremlin_wants_up_high`: every rule that wants a kit has
`felt_on` empty, so `on_sight` is the one way it's felt. Tests (gremlin.rs, both modes): `no_gremlin_grievance_with_her_kit_on_order`
(an hour in his room: never felt, said or set about; `shelf_asks == 0`), `no_gremlin_grievance_with_her_kit_boxed`
(never, and she opens the kit and puts its first half up), `a_line_owed_as_she_buys_her_kit_goes_unsaid` (felt and
owed, then `Bought(ShelfKit)`: forgotten, `to_mend` empty, no wish, unsaid; the same visit unbought says it); the
end-to-end now asserts, every step from the order till the shelf is finished, no new line and nothing felt. All red
before the fix. Census (`shelf_census`, its gremlin half): his line on 3.34 game days a run before, 31 of them in all
said with her shelf coming; after, 3.16 a run and none (the census now asserts none); up 32/32, kit first 32/32, mean
3.95 → 3.97 game days to him up (sd 0.82, max 7.64). Goldens byte-identical.

### Step 9. Census, deep runs, review shots, docs (~450 lines)

Census and band (below) in plan.md's record; the deep run clean (each failure pinned and fixed first); **review shots**
at 191×46 from an ignored test on `Graphics::shot` (as the door batch's `door_shots`), into
`phase6/shots/` with a README: the kit unpacked, each stage with the hop, the half shelf, the carry up, the gaze up, a
high shelf in the user's layout and a low one, the gremlin turned; the frozen `hung_clear` copy deleted; design.md
read through; CLAUDE.md's index line for this design; plan.md's record; `snippets.md`'s header marked wired.
**Traps.** `shelf_spot`'s low block check and its laid-covers check each stand in for the other with today's hung kinds
(the poster and the clock fill both block rows), so the block rule has no test of its own (the property's companion
counts `blocked` 0): when a hung kind that doesn't fill both rows comes (or in a hung long visit of this step's), add a
case where the block alone refuses, and assert `blocked > 0` in `shelves_hang_high_low_and_nowhere`.

**As built (2026-10-12).** The look tests are guards (a mutant writing the shelf in `Color::Reset` turns both red):
`tests/shelf.rs` `her_shelf_looks_as_drawn_in_ascii` (the stage's shelf on the client's own layout at 191×46, whose
borders have the theme's ink: the run's ten cells and their fg, the plant's cells and ink, from the record and the
pane's rect alone) and `her_shelf_reaches_her_images_in_its_pane_s_ink` (the gremlin at the run's left end and the
plant at its right: both brackets and the run in the images' redrawn lines, in the border's rgb), read through a new
`#[cfg(test)] Graphics::take_lines` (the line cells each painted image redraws, on screen). The census rooms (Census,
below) are `wide_room`/`wide_shelf_room` (the design's "`shelf_room()` at 191×46": step 6 had taken the name for the
resident's 100×30 room, kept) and `low_shelf_room`, through `Room.shelf: Shelved` (`No`, `Users`: step 6's, and
`Staged(more)`: put up where the stage's `Scene::GazeUp` puts one on the room's frame, on a guest of its own, then
`more` beside it); a gate test `the_shelf_rooms_put_her_shelf_up` holds them to their docs. The low room owns her
window too, so her look-outs compete. The bare 191×46 twin was added when the shelf room read out of the band: it
reads the same (plan.md's record). `shelf_shots` (tests/shelf_shots.rs) runs in release with `cargo test` (past
nextest's 60 s kill); away.rs's `film`/`Shot` gained `her` and `contact_sheet_round` (a crop size and centre). The
frozen `hung_clear_before_6a` and `hung_clear_is_as_it_was_with_no_shelf` are deleted. Not done: the block rule's own
case (no hung kind leaves a block row free; the trap stands), and a hung long visit. Goldens byte-identical.

**Review fixes (2026-10-12).** The census's staged shelves are staged as in play: `Shelved::Staged(more)` puts `more`
(her gremlin) on her living room's floor (her sofa's pane) before the stage's `Scene::GazeUp` works the shelf out, so
`shelf_spot` puts it in his pane, then moves him up; both step 9 rooms are staged so (as first built, the 191×46 shelf
went up in The List, the first pane in order, with no gremlin waiting: a room that doesn't happen; its 0.17 gazes up a
game day are now 0.76). The gate test pins the pane (her sofa's) and shares one oracle for the run
(`tests/shelf.rs` `run_from_record`; structures.rs's own oracles stay its own, with the half-run cases). The look tests
recolour the host border's cell under the run's middle (Magenta) so they pin "its pane's border ink" and not the theme's
colour, pin the gremlin in ASCII beside the plant, and check the composed pixels in line art. `shelf_shots` shoots a
stage's hop on the first frame its whole built run shows (stage 2's tile had shown the half shelf). Added a shelved long
visit with hung pieces (`long_visits_with_a_shelf_and_hung_pieces_never_touch_what_is_protected`, poster and clock given
after the shelf is up) and its companion: a low shelf with no block at all now turns it red (the earlier shelved visits
stay green); `hung_clear`'s own read of the block alone does not (another site lays the block too), so the trap's
`shelf_spot` half stands. The perf look (`gremlin::perf_look_at_his_room`, ignored) measured what the notes flagged:
cheap (plan.md's record).

## Census, band and stillness

- The band rooms (`furnished_room`, `resident_room`, the stage) have no shelf, so their numbers can't move; steps 5c and 6
  confirm it.
- New ignored rooms in tests/census.rs, chosen to be rooms that occur (step 3's measurement): `shelf_room()` (a high
  shelf on Users, the user's 191×46 if it takes one there, else `rooms(100, 30)`; the plant and the gremlin on it from
  step 7) and `low_shelf_room()` (`home_screen`'s 9-row panes, a low shelf, the poster hung beside it). Through
  `visit_census` both modes and the band's per-mood share and set-off cap (tests/band.rs) at the census seeds, with the
  window's look-outs a game day (the user's 2–6), since GazeUp's *daydreams* term competes with `LookOut`. Out of the
  band: a finding for the user (GazeUp's weight), not a band retune.
- `shelf_census`: game days from a kit's delivery to a finished shelf (expected 1–2) and from a gremlin's delivery to it
  on the shelf; her `"It wants a shelf..."` lines in between; over 32 seeds; in plan.md.

## Deep runs (step 9)

`PROPTEST_CASES=256 cargo nextest run --release --profile deep -p dessplay houseguest` (read nextest's own exit code);
`CENSUS_BAND_SEEDS=<n> cargo nextest run --release --profile band --run-ignored all -p dessplay -E
'test(/houseguest::tests::band::/)'`; `cargo nextest run --workspace --all-targets`.

## Docs that change (by step)

| Step | design.md | decisions.md |
|---|---|---|
| 2a | *Her record* 2641 | — |
| 2b | 2619-2622 (structures before the reads) | amendment to 2263; *Her structures are the UI's own lines* |
| 3 | *Strips and anchors* 1850; *Rooms from contents* 1957; beauty 1571 | (in 2b's entry: small decor only, written-only floors, rooms are floors) |
| 4 | *Wall pieces* 1925 | *A shelf hangs as high as fits* |
| 5b | *Getting furniture* 1787 | (in 5c's entry) |
| 5c | new *Structures* bullet | *She puts up a shelf, a stage a game day* |
| 6 | *Using her furniture* 1969 | a line under 5c's |
| 7 | *The rules of her home* 2101; *Getting furniture* | *The gremlin wants up high* |
| 8 | *Using her furniture*; *Getting furniture* | a line under 7's |
| 9 | read-through; CLAUDE.md line; plan.md record (incl. the brief-wording departure) | — |

## Not in 6a

The ladder, the pole check in `settle` (osaka.rs:10169) and `Clamber`, standing on a shelf (6b: lift D5's ledge half for
a shelf her ladder reaches and only one with `HEIGHT` clear rows over it; the brackets stay no pole); partitions, rooms as
regions, `Connected` (6c); a second shelf or kit; posters on shelves; re-hanging a shelf the frame can't hold, or raising
a low shelf when its pane grows; standing on the kit's box to reach (a natural 6b prequel); the cells of a stage growing
under her hands (the hop marks the moment instead).

## Questions for the user

Step 9's open questions (2026-10-12) are answered: the eighth round, below. Fourth round (2026-10-10, the user):
- **High means standable**, as designed: a high shelf keeps `HEIGHT` clear rows over its run. Step 3 measures how often
  each height fits at 191×46 with real content; revisit only if the measurement surprises.
- **The hop is for high shelves only** (D7c as designed): a low shelf is in her reach, so she stretches up.
- **Built as a workflow** per group of steps, as the door batch was (implement → correctness review → tests review with
  mutants → fixer; a 256-case deep run after each workflow).

Fifth round (2026-10-11, during the build; the hand-off notes have the detail):
- **A styled blank hides the shelf, as text does** (the user): a blank cell in reverse video, or with a background unlike
  its pane's (a list's selection bar past its entry's text), counts as occupied, so the shelf and what stands on it are
  hidden that frame. Built in step 3 (`structures::free` takes the pane's background, read off the host border's
  cell under the run's middle, as the ink is; the step-2b oracle's `expected` likewise).
- **Decided by the coordinator** (build agents' open questions, reversible): a piece the record puts on an unfinished
  shelf lands on that pane's floor at its share (2a's choice (a)); a half-built shelf is shown while its built half and 2
  cells past each end of that half are blank and free; a makeshift piece a shelf's geometry comes to meet falls apart and
  the shelf shows, as with her door's space; a shelf shown or hidden is logged at info, as the closet is.
- **Kept by the user** (step 4's build call, 2026-10-11): a low shelf's **block spans its margin's
  columns too** (the run ± 2), not only the run's as "Decided unless the user objects" and D3a first said. Why: D4
  shows a shelf only while its margin is clear of every other laid cover, so a poster hung flush against the run would
  hide the shelf every frame. The cost: a low shelf keeps 14 columns of the wall lane, not 10.
- **Kept by the user** (step 5a's build call, 2026-10-11; in `Guest::stage_shelf`/`shelf_decor`):
  the stage's `Scene::Shelf` puts **her clock (a mantel clock) on the shelf where the frame hides the plant**, as on a
  low shelf in a 9-row pane (`LOW_ABOVE` = 2 rows over the run; the plant is 3 tall): the stage room at 100×30 is that
  case. D11 said "a finished shelf with the plant on it". Alternatives: leave the shelf bare there (the note then
  `Err`), or require a third clear row for a low shelf (D3 would then refuse every 9-row pane).

Sixth round (2026-10-11, the user, on step 5c's and step 6's build calls):
- **The put-up pace is kept**: stage 1 straight after opening, stage 2 first thing the next morning (0.63 game days
  from delivery in the shelf census; the design expected 1–2). She is keen, and finishes as soon as she may.
- **She gazes up about once a game day** with her plant up (built: 0.05 a game day, 1 visit in 16 in `shelf_room`).
  Step 8 retunes `Want::Use(GazeUp)` to that, measured in `day_census` on `shelf_room`, before its freshness boost.
- **No settling chain after the gaze up**: the mood linger alone, as built.
- **A parcel nudging a piece she's on** (since before 6a): she moves with the piece. A fix of its own after 6a, since
  it moves the goldens of homes with no structure.

Seventh round (2026-10-12, the user, on step 7's and step 8's build calls):
- **"It wants a shelf..." stops once the kit is ordered**: she feels and says it for the gremlin on a floor only while
  no shelf kit of hers is on order, boxed or going up (a finished shelf shown, or room for one with no kit coming).
  Built in a fix before step 9 (as built, she said it with the kit on order and boxed too).
- **Kept as built**: a piece set on her shelf gets the pleased "hehe" in place of "There!"; her lines up at the
  gremlin come one gaze in two.
- **The ASCII gremlin stays `oo`/`ww`**: his turn doesn't show in ASCII (dev and tests only).
- **His parcel's look** (the 5×2 box scaled into his 2×2): judged on step 9's review shots.

Eighth round (2026-10-12, the user, on step 9's findings; built after step 9, plan.md's record has the numbers):
- **His parcel stays the tiny box** (the 5×2 parcel scaled into his 2×2 footprint): no change.
- **A shelf with no gremlin waiting prefers her living room**: D3's pane order within a height is now her gremlin's
  pane while he waits on a floor, then her living room (the pane her sofa stands in, out of its box), then the pane she
  opens the kit in, then the panes in order; high before low over every pane still comes first. "Her living room" is
  her sofa's pane, not the rooms-from-contents `Role::Living` (screen and seat, no bed: missing while a bed shares the
  room or before her TV comes) nor the record's `RoomKind::Living` (an older build's shim); the sofa picks the pane,
  the run stays mid-wall. At the user's 191×46 with no gremlin waiting she now gazes up 0.81 times a game day (was 0.17,
  her shelf in The List).
- **Her gaze up over her living room is kept** at its rate (1.93 a game day in the low census room, where she lives
  under it): more where she spends more of her day on its floor.
- **The user's 191×46 out of the stillness band: left for now.** Not 6a's (the shelf moves no cell); the user has a fix
  in mind for a later session.
- **`Visit.shelf_asked` holds a hash, not a frame** (the coordinator's call, no behaviour change): `kit_room`'s memo
  keys on `room::ShelfAsk::basis`, a hash of everything `shelf_spot` reads (her home, the frame, its panes, chat and
  screen, what's kept off, the pane she unpacks in), 8 bytes where a 411 KiB frame was held at 191×46.
- (Review shots: her pleased beat after setting her gremlin on her shelf is the set-down's, not "stage 2"'s:
  `carry-up.png` relabelled; `gaze-up.png`'s shelf, with no gremlin waiting, now in her living room.)

## Critique dispositions

M = mechanics, F = feasibility, T = tests, C = character (`phase6/critic-*.md`). A = accepted, D = amended differently,
R = rejected.

| Finding | | Disposition |
|---|---|---|
| M1 major | A | `Written` kept or undone by the arm that ends the frame (D4); errand and called-off-arrival tests (2b). |
| M2 major | A | Structure cells first in painted lists; "every burst cell" through `real`; two-part long-visit check (D4, 3). |
| M3 major | A | Claim dropped; `under` fix is 6a work; fallback door refuses structure cells via `Ground::structure` (D4). |
| M4 major | D | Coordinator's choice: `EDGE` margin (≥ `SPACE`) makes door and shelf disjoint by geometry; no door input (D3). |
| M5 major | A | Shelf pieces stand only on written cells; `fits`/`project_with` take `&Written` (D6). |
| M6 major | A | `PRE_SHELF` frozen kinds; kits out of real-piece lints; appending a domain is a strategy change (D7b, Tests). |
| M7 minor | A | InChat and `closetable` skip shelf pieces (D6). |
| M8 minor | A | `evaluate`'s `e`/`pin` lane by the candidate strip (D1). |
| M9 minor | A | `middle_of` reads `stood` (D1). |
| M10 minor | A | `Scene::GazeUp` sets up its own shelf; GazeUp out of `ANY_USE` (D8). |
| M11 minor | A | Older-build claim reworded; structures read before props (Decided, D2). |
| M12 minor | A | One rule: rise ≥ `LOW_RISE` and `LOW_ABOVE` rows; `paced` in 5c (D2, D3). |
| M13 minor | A | PutUp offered and committed only with the half's cells clear; keep-out takes the whole placed run (D4, D7c). |
| M14 minor | A | Doorstep judges the kit ungated; `raise` persists to the next `furnish`; visit ending: box stays shut (D7b). |
| M15 minor | A | Three-way `next`; `shelf_room` only when queued; `High` searches shelves only (D7b, D9). |
| M16 minor | A | `PutUp` want/job/act/chance/event; `Use::GazeUp` (D7c, D8). |
| M17 minor | A | Tier, `role_without`, `spoils`/`newly_spoilt` key on `room()` (D1). |
| F1 blocker | A | As M6, `PRE_SHELF` and `CATALOGUE[..10]` with the site list (D7b). |
| F2 blocker | A | The kit is the one parcel exempt; its own property (D7b, 5b). |
| F3 major | A | `boxed()` accessor, private field, `unbox` refuses kits; debug-assert dropped (D2). |
| F4 major | A | Kits excluded from unboxed-render lints, included in parcel lints, with the reason (D7b). |
| F5 major | A | `EDGE` margin (D3). |
| F6 major | A | Step 5 split into 5a, 5b, 5c. |
| F7 major | A | Step 2 split into 2a (threading), 2b (behaviour); brief departure and `validate` doc noted (D4). |
| F8 major | A | Check scoped to written positions and first cells; ASCII `draw` (3871) added (D4, 3). |
| F9 major | D | Two 9-row panes noted; `LookOut` precedent; the pane is checked in step 3 (step 0 runs no code) (D11). |
| F10 major | A | Frozen `hung_clear` oracle, identity property (4). |
| F11 minor | A | `Spec.shelf` in step 1; rises in 2a; `paced` in 5c. |
| F12 minor | A | Block is geometry only (D3a). |
| F13 minor | A | Gated computation; `shelf_spot_is_cheap` and a timing test; perf test location corrected (D3, Tests). |
| F14 minor | A | As M17. |
| F15 minor | A | An unsellable wish doesn't block the next piece (D7b). |
| F16 minor | A | As M13. |
| F17 minor | A | No dead code per step; gate wording (Tests). |
| F18 minor | A | No write below the minimum size; failed arrival undoes (D4). |
| F19 minor | A | Ink from the host border's fg (Decided). |
| F20 minor | R | `DoorWall.strip: Nook` touches every comparison for a state no code builds; the deserializer guards (D1). |
| F21 minor | D | Refs fixed (2355; LookOut's script arms; `Summary.owns`); `Written` kept: dissolve's local is another module. |
| F22 minor | A | As M11. |
| T1 blocker | A | As M6, F1. |
| T2 blocker | A | Frame-by-frame goodbye; direct `under` tests; waves test demoted to smoke (2b). |
| T3 major | A | `most` counts shelf cells; per-frame structure checks (3). |
| T4 major | A | Independent oracles from the record and nook rect (Tests). |
| T5 major | A | Brute-force oracle, ordering, non-zero counts, wider generator (5a). |
| T6 major | A | Biased generator, geometry and stage clauses, `built: 0` (2b). |
| T7 major | A | `structures` argument, `rooms_frame` + `chat_over`, companion, pane drop/restore, 16 cases (3). |
| T8 major | A | New room.rs functions with own strategies (4). |
| T9 major | D | With the `EDGE` margin the property is "no shelf cell within `SPACE` of either wall", every frame (3). |
| T10 major | A | Deterministic pacing on offers, restart, midnight, unfed (5c). |
| T11 major | A | Homes lacking furniture and decor; per-visit wish pinned; two no-kit cases (5b, 7). |
| T12 major | A | Hand-written 6a record through today's reader; `as_a_pre_shelf_build_reads`; golden both ways (2a). |
| T13 major | A | Sibling-site tests (1, 3). |
| T14 major | A | Repair properties with shelves at 3, 4 and 7; mood forced (3). |
| T15 minor | A | Red within step 1 (Tests). |
| T16 minor | A | Boundary tests (2a). |
| T17 minor | A | Finished shelf, rows over it, `Ground::ledge`, poles (2b). |
| T18 minor | A | Line fixed; `Err` at 80×24; observables (D11). |
| T19 minor | A | Away repeats; whole-shelf toggle asserted, no rain, no hysteresis (Decided, 2b). |
| T20 minor | A | ASCII fixed-frame and line-art ink checks (9). |
| T21 minor | A | Operation-count test (5a). |
| T22 minor | A | GazeUp property (6). |
| C1 major | A | Measurement with content at several heights and 191×46; Users as the realistic host; census rooms that occur (3). |
| C2 major | A | Gremlin sold only when a shelf stands or can; `High` felt only then; tests (D9, 7). |
| C3 major | A | Arms up for shelf lifts and set-downs; the hop (the user's answer) for high shelves (Decided, D7c). |
| C4 major | A | 2-cell margin (the user's brackets answer); no-text rows preferred (D3). |
| C5 major | A | Beside it, facing it, opening `"hehe"`, then the pool (step 8) (D8). |
| C6 minor | A | (a): the first stage chained after unpacking (D7c). |
| C7 minor | D | `Face::Curious`, no `"nnn~"`; cells appear at the end with the hop rather than growing (the hop marks it). |
| C8 minor | A | Felt only in its room, facing it; said after a shelf is up only on a visit she'll mend (D9). |
| C9 minor | A | 20–45 s; only from the shelf's room floor; clock excluded; look-outs counted (D8, Census). |
| C10 minor | A | Untouching runs ordered first at a height (D3). |
| C11 minor | A | Centred left on a shelf (D6). |
| C12 minor | A | Shelf scenes on a frame that fits; review shots at 191×46 (D11, 9). |
| C13 minor | A | `Use::GazeUp` (D8). |
| C14 idea | D | `Act::Admire` after stages and shelf set-downs (user's extra 3); "There!" and a straight-into-gaze not added. |
| C15 idea | A | The user's extra 3: `"A shelf for the gremlin!"` (8). |
| C16 idea | A | The user's extra 4 (8). |
| C17 idea | A | The user's extra 2 (8). |
| C18 idea | A | The user's extra 1 (8). |
| C19 idea | A | The user's brackets answer; bracket stubs no pole (D5). |
