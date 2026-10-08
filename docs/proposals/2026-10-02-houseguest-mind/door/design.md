# The door batch: implementation design

**Working design, 2026-10-08, amended after four critiques (mechanics, tests, feasibility, character; their files
sit beside this one).** Brief: docs/plan.md, Phase 38, "Next: the door batch" (plan.md wins over everything here).
Investigation: [brief.md](brief.md) (partly stale). Code map: [map.md](map.md); its last section ("Gaps and
corrections") overrides its earlier sections, and this design overrides both. Art: [art/snippets.md](art/snippets.md)
(approved B; the art code is **already committed** in `8eaa25c1` as dead code: `art::WallDoor` art.rs:1121,
`render_wall_door` art.rs:1221, `flap_plate` art.rs:1146, `sprite::WALL_DOOR` sprite.rs:479, `art/wall-door.svg`;
`PARCEL` art.rs:1465 is already production; never re-apply `worktree.diff`). Paths are relative to
`dessplay/src/ui/houseguest/` unless noted. Every line below was checked against the tree at `3c4ed354` (`dessplay/`
unchanged since `057311b1`). The critique resolutions, one line per finding, close this file.

Geometry words, used throughout: for a strip's `Extent { from, to, floor: f, rows }` (room.rs:736), the **right
wall** is column `to`, the **left wall** is column `from - 1` (not symmetric: room.rs:795-813). Call the wall's
own column `w`. Right wall: the **space** is columns `w-6 ..= w-1`, her **spot** `x = w-3`. Left wall: the space
is `w+1 ..= w+6`, her spot `w+3`. The space's rect is those 6 columns × rows `f-4 ..= f` (her height plus the
floor row). `HEIGHT = 4`, `WIDTH = 5` (sprite.rs). A wall is an **edge wall** when its **nook rect** touches the
screen's edge on that side (`nook.right() == screen.right()` / `nook.x == screen.x`, the test `doorstep` makes at
room.rs:1529-1530, extracted into one helper): never the space rect, whose `x` is never the screen's.

## User decisions

From plan.md (2026-10-07) and the user's answers (2026-10-08):
- **A reserved space** for her external door at the screen's edge: floor against a wall, never over furniture; no
  piece stands in it, none hangs low enough to meet it (the window included); **poster and clock may hang above
  it**. The door's spot is computed from her home and the frame, never her feet. Outside the space (it yields, or
  no home): a **strict check that her box meets no piece's cover**, the same in both modes.
- **The door is sheet B:** side-on, two columns, turned slightly, set in the wall line, **the parcel flap in its
  lower half**. Parcels come through it and rest just past the space.
- **While she's out:** slippers before it and a card on the knob; the doorway's glimpse follows the window's sky;
  she visibly walks through, clipped at the wall line.
- **Work always through her door.** Every work exit and return is through the space, like school; work's
  screen-edge walk-out goes (settled fact 3; design.md:1762-1765 and :2233-2234 reconciled).
- **Older homes** whose furniture fills the space: she clears it herself through a felt rule ("Can't get to the
  door!") that phase 4's arranging answers; until then the space yields for that frame and the door stands at the
  nearest spot meeting no piece.
- **No furniture in the chat pane:** bought, makeshift, parcels, hung decor. Carried props are exempt. An older
  anchored piece in the chat is **shown where it stands** until she moves it out through the felt rule; if no
  strip takes it, it is **closeted per frame** (hidden while nothing fits, never deleted, no persisted flag;
  settled fact 1). **Makeshift pieces are never built in the chat.**
- **Her door is never in the chat pane;** only her **external** door (settled fact 4): doors in space (~1.5 s, in
  her box) and the errand's arrival at the accordion are unchanged.
- **No edge wall outside the chat:** an inner wall of one of her nooks (outside the chat); then the nearest floor
  spot outside the chat whose box meets no piece (face-on); with none, she goes out by a door in space where she
  stands and **no door is drawn while she's out**.
- **One shared placement check** (the space and the chat refused to every piece, both modes), the felt rule for
  both, tests first.

**The user's answers, second round (2026-10-08), overriding anything below that says otherwise:**
- **Her door stands over text.** While she's out (and in a work gap's beat 6), the shut door's own two columns are
  drawn over pane text: the pane loses its last column in those 4 rows meanwhile. Only a protected pane (a focused
  one in use, the status bar, …) hides her door. The slippers still show only on calm cells (cropped otherwise).
- **Walking pace through the doorway** (beats 2 and 10 at `4 × WALK_MS`), not the sheet's thirds.
- **The InChat line is `"People are talking here..."`.**

**Decided unless the user objects** (each recorded in decisions.md when it lands):
- The no-home door uses the same wall chooser, unsaved. Door logging: the spot at info as she goes out, a fallback
  or yield at debug with its reason.
- **A floor piece with no use (lamp, plant) in the space is felt as DoorClear when she comes home, dashes in, or
  comes back from work through the fallback door *because pieces fill the space*** (she bumps into it; felt rules
  die with the visit, so feeling it going out is useless). Settled fact 2. A fallback forced by a short room, the
  chat, a focused pane or no wall is never blamed on furniture. **InChat for a piece with no use** is felt the
  first time she's in sight in a visit while it's broken. Both are **said** only once she's quiet after her
  arrival line, only when her mood would mend, and at most once per game day per rule (D7). Lines: `"Can't get to
  the door!"` (the user's) and `"People are talking here..."` (the user's choice).
- The space is **kept by narrowing the floor extent** (never by `blocked`, which would closet: room.rs:1367); the
  chat is a **keep-out for new placements** plus the InChat rule, never in `project`'s showing pass. The one
  exception is the per-frame closet (D7), applied by the production frame call, not by `project`.
- **Text never moves her door, and never hides it** (the user's answer above). Neither the space nor the fallback
  is chosen against text; while it stands for long (she's out, or a work gap's beat 6) it is drawn over text. During a
  visit's short door beats and her walk-up the door is drawn over text **in passing**, as her own image is
  (`HIDDEN_MS`, tests.rs:549-556), so she never opens an invisible door. The Away cue's slipper columns are cropped
  (drawn only over calm cells); the shut door's own two columns stand over text. **The face-on fallback door
  stands over text too** (decided unless the user objects: the same cue, though 5 columns wide; the fallback is
  rare in the bundled layout except on short terminals).
- **The Away cue shows during short gaps too** (a dash's, work's): "her door shut with slippers before it" always
  means she's out (snippets' recommendation). At the face-on fallback there is no cue: the closed `Look::Door`
  stands while she's out, through a work gap's beat 6 too.
- **A parcel comes through her door first;** only when her door's strip can't take it (space yielding, no room)
  does it come through another wall with today's `draw_flap`, which survives only as that fallback. A delivery
  waits while her external door's act runs or her box meets the space.
- **The line latch is separate from `leaving`.** `set_off: Option<Routine>` latches her set-off line; `leaving`
  is still set only when her external door opens (as today, osaka.rs:5279), so nothing on her way to the door can
  end the visit (D6).
- **She gets up by the ordinary end of a use:** the cut ends the use and the walk starts from her seat, as after
  any nap; no `beside` step.
- **A Leave walk is never cut; the door re-checks.** A boundary (a dash's way out crossing 12:45) never interrupts
  the walk; at her spot `start_job` re-checks that her routine still has her out, and if not she decides instead,
  silently (Open choice 5 asks the user to confirm).
- The doorway sky gradients **and the hedge's tint** are derived from the window's sky stops (props.svg:444-505);
  the sheet is regenerated for the record, no separate approval round.

## Design

### D1. The space, `Home::extents`, and one laid bundle

**Types** (room.rs; `Room` is already taken by `admits`' enum at room.rs:1789, so the new names are distinct):

```rust
/// Her external door's wall: a side of one strip (saved).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct DoorWall { pub strip: Strip, pub side: Side }

/// A strip as this frame lays it out: `raw` between its walls; `floor`, what its floor lane packs on (raw less
/// her door's space, when kept); `space`, her door's reserved rect on this strip, if it can exist at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StripPlan { pub strip: Strip, pub raw: Extent, pub floor: Extent, pub space: Option<Space> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Space { pub side: Side, pub wall: i32, pub rect: Rect, pub kept: bool }

/// What one layout pass produced, with the plans it was laid on (never handed in separately).
pub(super) struct LaidOut { pub shown: Vec<Shown>, pub shifts: Vec<(Furniture, Option<u32>)>, pub plans: Vec<StripPlan> }

/// The frame's geometry every door and keep-out function reads.
#[derive(Clone, Copy)]
pub(super) struct Plan<'a> { pub nooks: &'a [(Nook, Rect)], pub chat: Rect, pub screen: Rect }
```

- `Home { props, door: Option<DoorWall> }` (room.rs:1120). The three `Home { props }` literals (room.rs:2335,
  :2837, :3141) get `..Default::default()`. `Hash`/`PartialEq` pick the field up (mend basis mod.rs:3581,
  `changed` in `paint_empty`): intended.
- `impl Home { pub(super) fn extents(&self, nooks: &[(Nook, Rect)]) -> Vec<StripPlan> }`: today's `strips`
  (room.rs:795) per nook, plus for `self.door`'s strip a `Space` **only when it can exist**: `raw.rows >= HEIGHT`
  and `raw.to - raw.from >= 6 + max(WIDTH, widest floor piece on the strip)`. Otherwise `space: None`: nothing to
  keep, judge, refuse or clear (a short terminal never makes her rearrange, M7/C5). When it exists, `rect` is
  computed from the raw extent and the side alone (rows `f-4 ..= f`, never underflowing since `f >= HEIGHT`),
  **independent of narrowing**, so DoorClear can judge the would-be space while it yields, and `kept =
  pack(&self.on(strip, Lane::Floor), narrowed).is_some()`; `floor = if kept { narrowed } else { raw }`, narrowed
  = `to -= 6` (Right) or `from += 6` (Left).
  - Needs no screen and no chat: narrowing reads only `self.door` and `nooks`.
  - `kept` must not depend on anchors (`pack`'s feasibility is anchor-independent: room.rs:831-864). Step 2 pins
    the property that matters with a proptest: `extents()[i].space.kept` is invariant under re-anchoring that
    strip's pieces.
- **Pinning is always on `raw`** (`pin_anchors` room.rs:1187, and the set-down pin in `furnish` mod.rs:3253-3262):
  `at` stays a share of the raw extent, the older builds' meaning, and `extents` is computed **after** pinning (so
  an older record whose pieces have only `at`, which `Home::on` room.rs:1176-1183 skips, is judged whole). Anchors
  are laid against `floor`: on the door's strip an `Anchor{Right,0}` stands flush with the space's inner edge.
- **One bundle, no stale plans:** `laid_and_shifted` (room.rs:1230), `laid_on` (1217) and `hung_shifts` (1225)
  take `nooks`, call `self.extents(nooks)` themselves after pinning, and return `LaidOut` (`pack` on `floor`,
  `pack_each` on `raw`). `broken`/`judge` take `&LaidOut`, never a separate plan slice, so a scratch home can't be
  judged on `before`'s plans.
- `strips` is renamed **`raw_strips`**, private to room.rs plus the two "which room is she in" readers that keep
  raw (`clock_on` mod.rs:211, `beauty_at` mod.rs:228, doc comment saying why). Every other caller reads plans:
  - room.rs: `layout` 1209, `pin_anchors` 1187 (raw), the three above, `hung_clear` 1281 (its `meets` at 1299-1304
    also refuses `space.rect`, kept or not, when a `Space` exists; only the window can meet it), `project` 1348,
    `move_off` 1378 (packs on the target's **`floor`** and refuses a target where a moved piece's cover meets the
    target's space, M6), `spot` 1445, `doorstep` 1517, `admits` 1617, `stand` 1682.
  - rules.rs: `broken` 183, `judge` 193, `against_wall` 324 (reads `floor`, so a fridge beside the space is still
    against the wall), `Before::new` 462 (`strips` at 465), `evaluate` 517 and `search` 734: **every** read of the
    target strip's extent (534, 548, 573, 595 and `search`'s 760, used for the `lefts` grid and the anchor offset at
    796-805) comes from a scratch home with the piece's strip already changed (any anchor, since `kept` is
    anchor-independent), computed once per candidate strip, never from `before` (F7). Judging after the move is
    `scratch.relaid(&before.strips)` (**amended, step 3p, 2026-10-08**): `before` keeps each strip laid out alone
    (`StripLaid`, its plan and layout) **with what it was laid out from**, her pieces on it by index and her door's
    wall if it's there (`Home::lays_from`, all a strip's layout reads); `relaid` takes a strip from `before` only when
    the scratch home's pieces and door there are identical, and lays out every other strip afresh. So `before`'s plan
    or layout is never used for a strip the move changed: a stale strip is unrepresentable, not guarded by a caller's
    list. A new input to a strip's layout (steps 5/6) goes into that record at `strip_laid`, or `relaid` would
    reuse a strip it changed. `laid_and_shifted(nooks)` is the same strip-by-strip layout, all of it fresh.
  - mod.rs: `furnish` 3206 (set-down pin on raw), `layout` 3269, `broken` caller 3366-3367, `stage_arrange` 3485
    (loop 3495).
  - Tests: rules.rs:1309 (`judged`), 1754, 2166; tests.rs:833 (compares a piece's floor to `plan.raw.floor`, equal
    to `floor.floor`), 1785 (reads `raw.rows`), 12076 (`breaks`).

### D2. Choosing `Home.door`, outside `project`, and the ledger

- `fn choose_wall(home: &Home, plan: Plan) -> Option<DoorWall>` (free fn, room.rs). A wall **qualifies by geometry
  only**: its strip is in `plan.nooks`, `raw.rows >= HEIGHT`, the strip is wide enough for a `Space` (D1), and its
  space rect misses `plan.chat`. Among qualifying walls the order is: **edge walls first** (the nook-rect helper
  above); then **her pieces' strips** (in `nooks` order, not `furnished()`'s prop order: room.rs:1164); then walls
  whose floor pieces **pack narrowed** (a tie-break only: a crowded edge wall of her own strip still wins, the
  space yields and DoorClear clears it, as the user asked for older homes); then `nooks` order, Right before Left.
  Inner walls are chosen only when no edge wall qualifies. None: `None`.
- `pub(super) fn wall(&self, plan: Plan) -> Option<DoorWall>`: `self.door`, else `choose_wall` unsaved. **Every**
  reader of "her door's wall" goes through it: `door_place` (D4), `Keep::of` (D3), `doorstep`/`admits` (judged on
  `with` with `with.door = with.wall(plan)`, so the first TV delivered to an empty home already rests past the
  space, M3) and D9's "her door's wall first".
- `pub(super) fn settle_door(&mut self, plan: Plan, moved: Option<Strip>) -> bool` (returns whether it changed the
  record): **saves** a wall only when (a) `self.door` is `None`, the home has a prop, and a wall qualifies; or (b)
  `move_off` actually moved the door strip's pieces this frame (`moved = Some(target)`): the door follows the room
  to `target` (the chooser restricted to it, falling back to the full order). It **never writes `None` over a saved
  wall** and never re-chooses for a resize, a pane hidden for a moment (`IdleView.nooks` drops empty panes,
  ui/app.rs:664-671: the door's strip simply has no plan that frame, so nothing narrows and nothing moves), or a
  short frame. A saved wall whose space comes to meet the chat (a custom layout, a pane drag) is **not** re-chosen:
  D4 refuses it per frame (`Fallback::Chat`), so going back restores it (F10).
- **Not in `project`.** `project` (room.rs:1348) keeps its signature and only reads `self.door`; unit tests that
  don't set `door` mean "no door" (F2). Production goes through one call, `Home::frame(&mut self, buf, plan,
  blocked) -> (Vec<Shown>, bool)`: `settle_door(plan, None)` → `project_with(buf, plan.nooks, plan.chat, blocked)`
  (the internal body; `project(buf, nooks, blocked)` is `project_with(.., Rect::default(), ..)`) → if `move_off`
  reported moving the door strip's pieces, `settle_door(plan, Some(target))` → the per-frame closet (D7, step 6).
  `move_off` receives the chat only through `project_with`, to refuse a target meeting it. Production callers of
  `project` all become `frame`: `paint_empty` (mod.rs:4704), `furnish` (3227, 3231, 3265, 3329, 3359),
  `stage_arrange` (3493, 3523), `place_gift` (3658), and the arrival's projection (D5). `admits`' scratch
  projection (room.rs:1576) stays `project_with` with `with.door` set from `with.wall(plan)`, unsaved. Each
  `frame`/`settle_door` change ORs into `self.unsaved` (as mod.rs:2123), the arrival's included (M16).
- **Ledger** (ledger.rs): `Saved` (568-600) gets `#[serde(skip_serializing_if = "Option::is_none")] door:
  Option<DoorWall>` declared **before** `clock_sent` (the test `the_clock_sent_round_trips` ledger.rs:1226 asserts
  the JSON ends with it); `Raw` (623-655) gets `#[serde(default)] door: Option<serde_json::Value>` read leniently
  (garbage reads as `None`); `VERSION` (46) stays 1. `Summary` (369) gains `door: Option<String>` (`"Users
  right"`) with `skip_serializing_if`, like the other optional fields, so `dessplay/src/dump.rs:500-552`'s
  expected JSON is unchanged for a home without one (and no `pub(super)` type leaks into the `pub` struct). The
  module doc (1-37) gains a paragraph: older builds drop the key on re-save and read the door strip's anchors
  against the raw wall, so those pieces stand up to 6 columns nearer the wall until a new build re-chooses;
  nothing is lost (M24). The next read re-chooses deterministically.

### D3. Placement keep-out (the chat and every space)

A pure helper, **one predicate** for every new placement, called once per candidate cover (never per cell):

```rust
/// What no piece may be put on this frame: the chat pane, and her door's space (kept or not, while it exists).
pub(super) struct Keep { chat: Rect, spaces: Vec<Rect> }
impl Keep { pub fn of(home: &Home, plan: Plan) -> Self; pub fn refuses(&self, cover: Rect) -> bool }
```

`Keep::of` builds its plans with `Home { door: home.wall(plan), .. }` so the no-home and not-yet-saved walls are
kept out too (M3).

Sites (all **new** placements; nothing here closets an existing piece):
- `admits` (room.rs:1617, via `doorstep`/`spot`): refuse a new piece whose `cover()` meets `Keep`, and refuse a
  delivery after which the door strip's space would yield (judged on `with` with its wall). `doorstep` and `spot`
  take `plan` instead of `nooks` (their ~12 test callers room.rs:2880-3087 build `Plan::bare(nooks)`, an empty
  chat and the nooks' bounding box as the screen; mod.rs:3322, 3352, 3644 pass the view's).
- `move_off` (D2): packs on the target's floor; refuses a target whose result meets `Keep`.
- Repairs: `rules::Frame` (rules.rs:404) gains `keep: &Keep`; `search`/`evaluate` call `keep.refuses(at.cover())`
  once per candidate (`Frame::free` rules.rs:428-441 stays the cell test for protected and text). Ten `Frame {`
  literals: rules.rs:1431, 1733, 1842, 1919, 2060, 2312, 2328 and mod.rs:3238, 3386, 3591. **Never widen
  `blocked`** (it is `project`'s closure, mod.rs:3220).
- Makeshift: `builds` (mod.rs:3922, `clear` at 3938-3943) refuses a piece whose cover meets `Keep`; `made_stands`
  (mod.rs:3837) applies the same test at **both** callers, `tend_made` (3798) and `furnish`'s delivery `seats`
  closure (3313, with the after-delivery home's keep), so a made piece the chat or a space comes to meet falls apart
  (the symmetric path). **The space half lands in step 3** with the door (T9); the chat half in step 5.
- `door_place` (D4) refuses the chat for every candidate.
- The mend throttle hash (mod.rs:3579-3581) adds `view.chat` (Home already carries `door`).

### D4. `DoorSpot` and `door_place` (new module `door.rs`)

```rust
/// Where her external door stands this frame. Only `door_place` makes one (fields private).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct DoorSpot { x: i32, y: i32, at: Set }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Set { Wall { side: Side, wall: i32 }, Floor(Fallback) }  // side-on in a wall | face-on fallback
/// Why the door isn't in its space (logged; `Yield` alone is "pieces fill it", D7's trigger).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fallback { Yield, Blocked, Short, Chat, Protected, Unmarked, NoWall }
impl DoorSpot {
    pub fn spot(self) -> (i32, i32);      // where she stands to go through
    pub fn out(self) -> Facing;           // toward the wall (Floor: toward the nearer screen edge)
    pub fn into_room(self) -> Facing;     // the inverse (C6)
    pub fn wall(self) -> Option<(Side, i32)>;
    pub fn bumped(self) -> bool;          // Set::Floor(Fallback::Yield)
    #[cfg(test)] pub fn at(x: i32, y: i32, at: Set) -> Self;   // for unit tests with no frame (F15)
}
/// What the caller's terrain says, text-blind: lines and protected cells only.
pub(super) trait Ground { fn platform(&self, x: i32, y: i32) -> bool; fn line(&self, x: i32, y: i32) -> Option<char>;
                          fn protected(&self, x: i32, y: i32) -> bool; }
pub(super) fn door_place(home: &Home, plan: Plan, obstacles: &[Rect], ground: &impl Ground,
                         prev: Option<DoorSpot>) -> Option<DoorSpot>
```

`obstacles` are every cover she mustn't stand in: the **laid** covers (`LaidOut.shown` before any hiding by text or
a focused pane, so a piece under a released pane counts), the visit's **made** covers (`visit.made`, mod.rs:493)
and the carried piece's `ghost` (mod.rs:3403). Every caller builds them with one helper (M4).

1. **The wall:** `home.wall(plan)` (saved, else chosen unsaved: the no-home door, or a frame where nothing
   qualified). Its plan comes from `Home { door: Some(wall), ..home.clone() }.extents(plan.nooks)`, so an unsaved
   wall has its `Space` too. No wall: `Fallback::NoWall`; no `Space`: `Short`.
2. **The space:** `Set::Wall` iff the plan's `space.kept` (else `Yield`), no obstacle meets `space.rect` (else
   `Blocked`: a scrap or the ghost), `space.rect` misses `plan.chat` (else `Chat`, unsaved, per frame), her spot
   `(w∓3, f)` is on a platform and the wall column holds a plain `│`/`┃` on rows `f-4 ..= f-1` with a stroke under
   it on `f` (else `Unmarked`), and no cell of the space or the wall column is `protected` by the caller's ground
   (else `Protected`). **Text never fails it.**
3. **The strict fallback** (`Set::Floor(why)`): if `prev` is a `Set::Floor` spot that still passes this check, it
   stays (M12); else the nearest spot to the space's spot (or `middle(size)` mod.rs:4600 with no wall), by L1 then
   y then x, with `platform` under her box's floor row, her box's cells not `protected`, `her_box(x, y)` meeting no
   obstacle, the box missing `plan.chat`, and (the user's answer, 2026-10-08, step 4a) no plain wall `│`/`┃` in
   her box above its floor row: it never straddles a pane's border. **Text-blind** (not `room::free`, which tests `rect()`, map B1; not
   `restful`, the root cause; never `furnish`): the spot moves only when pieces, the chat, a protected pane or the
   lines change, never with a pane's text (C7). Identical in both modes.
4. Else `None`.

- **Which ground, where** (M11, F11): the **drawn Away door** reads the `unkept` terrain (C2: it never hops while a
  focused pane is in use; hidden there instead); **arrivals** and the visit's **`Chances.door`** read the **gated**
  terrain, so she never comes out inside a focused pane nor walks into one (a focused space makes the visit's door
  fall back outside the pane, `Protected`, and evict never has to push her back). `Ground` is a thin view over a
  `Terrain` already built that frame (`visit.terrain`, the Away arm's `unkept` read at mod.rs:2107, the arrival's
  read): no new `Terrain::read` or `whole_glyphs` per visiting frame. **`platform_at` is not text-blind** (checked
  by the coordinator, terrain.rs:208-300): a platform needs her whole body `open` above the ledge, and in line art a
  **wide glyph** (CJK in a chat line or a title) is never open, so text splits platforms. So **`Ground::platform`
  never calls `platform_at`**: it is "a ledge glyph (`terrain::ledge`, redrawable in line art) under her box's floor
  row at `x-2 ..= x+2`, none of those cells protected"; `Ground` keeps its own per-cell ledge/protected grids, filled
  in `Terrain::read`'s existing loop (no second pass). Her own reachability (whether she can walk there) is the
  visit's business, not the door's (`go_to` falls back to a door in space, D6). The fallback search runs only when
  step 2 fails; step 3's exit runs the perf tests.
- `door_fits` (mod.rs:4607) and `door_spot` (mod.rs:4613) are **deleted**; nothing can place an external door from
  her feet. Stage `DashIn`/`DashForgot` (stage.rs:465-496) use `chances.door`.
- **Logging:** `door_place`'s callers log at debug on a `Set::Floor(why)` change, with `x, y, ?why`; `out_by_door`
  and the set-off at info with the spot.

### D5. `Out`, `Empty`, `How`, arrivals (C1, C2, C6)

- `struct Out` (mod.rs:700) loses its field: a unit marker (`Guest.out: Option<Out>` mod.rs:842 keeps meaning
  "out by her routine", read by census.rs:1920, :2184). `DoorAt` (mod.rs:691) is deleted.
- `Empty` (mod.rs:713) gains `door: Option<DoorDrawn>`, **derived, last frame's**: the `DoorSpot` and the cells it
  painted (so `door_rain` rains the wall column and slippers, not a 5×4 box). It is the source for every path with
  no `Buffer`: `closed_door()` (mod.rs:1849, now `-> Option<DoorSpot>` from the Away/Arriving `Empty`),
  `rain_home` (mod.rs:1365: origin from `empty.door`, its `door` param goes), `call_off` (1391), `door_of` (1422,
  deleted), `leave`'s Away arm (1308), `door_rain` (2805).
- `How::Return(Option<DoorAt>)` (mod.rs:669) becomes `How::Return`.
- `paint_empty` (mod.rs:4657): its `door: &mut Option<DoorAt>` param goes; it calls `door_place` (unkept ground,
  `prev = empty.door`'s spot) on its `frame` projection (4704) and stores the result in `empty.door`; the
  `standing` filter (4722-4727) then hides the door while any of its **drawn** cells are in gated `view.protected`
  (never for text: it stands over text, the user's answer). The image take-in (`terrain::image(..).with`, 4752-4755) goes: the door's image takes in no piece
  by construction. At a `Set::Floor` spot the closed face-on `Look::Door` stands (no cue, C17).
- `out_by_door` (mod.rs:1869): the rain origin is the visit's door spot (`chances.door`, else her x); no spot
  stored. The Away arm (mod.rs:2092-2124) stops passing `self.out...door`.
- **Arrivals** (mod.rs:2019-2080): before `Terrain::read` (2026), `frame` the home (`self.unsaved |= changed`,
  M16) and run `door_place` on the arrival's **gated** ground with `prev` = `empty.door`'s spot. `back_through_door`
  (osaka.rs:9111), `dash_in` (9138) and the stage's `dash_through` (9163) take the `DoorSpot`, build their act from
  it (`to: Through::Home(spot)` once step 4a lands, M21) and face `into_room()` (C6). A spot whose door cells or her
  box meet gated `view.protected`: the existing no-room path (2059-2077), she waits. **No spot at all** (no wall
  and no fallback): she comes in by a door in space at a calm spot outside the chat (`calm_elsewhere`
  osaka.rs:1393, its `clear` refusing `view.chat`), as `Through::Space`, gap 0, symmetric with going out (M22).
  Remember `visit.bumped = spot.bumped()` for D7's trigger.
- `absent` (mod.rs:1744) and the cold start get no special case: Away paints `door_place` every frame.

### D6. `Job::Leave`, the set-off latch, every exit through her door

**Types.**
- `scenes::Job::Leave { spot: DoorSpot, why: Leave }` with `enum Leave { School, Work }` (scenes.rs:235; `JobRef`
  315), so "which gap" is never derived from mixed state (M9): `school_from_work` (osaka.rs:3552) rewrites a work
  Leave's `why` to `School`. `spot()` is `DoorSpot::spot`. Every exhaustive site: `by_ref` (scenes.rs:277, 284),
  `JobRef::{spot 327, text_cells 342 (empty), side 353 (`out()`), box_row 367}`, `Heading::find` (mind.rs:1473,
  from `chances.door`), `Heading::row` (mind.rs:1540), `Bind::on` (mind.rs:226), `start_job` (osaka.rs:4636),
  `census_purpose` (osaka.rs:7065-7069: **"routine"**), `act_summary` (osaka.rs:2740), `JobRef` matches at
  osaka.rs:1138 and 7213. stage.rs:533 is a `Scene` match that builds jobs (no new arm needed unless a scene
  starts a Leave).
- `Act::Door { since, to: Through, gap }` (osaka.rs:676) with `enum Through { Space((i32, i32)), Home(DoorSpot) }`.
  The draw and the census tell her external door apart by type, not by `gap == u64::MAX`. Sites binding `to`:
  osaka.rs:2750 (`act_summary`), 4062 (the door tick and `there` teleport), 9205 (the errand retarget: `Space`),
  9377 (`evict`). Sites constructing it: 5283, 9079, 9091, 9118, 9139, 9167, 9215, 9426, 9473, 12047; tests 15650,
  15866, 15887, 16151.
- `Chances.door: Option<DoorSpot>` (osaka.rs:26, declared in step 3), filled at both production literals
  (mod.rs:2353 `offered`, 2403 `visit.chances`) from `door_place` on the visit's gated ground with the visit's
  obstacles and `prev = visit.door`.
- `set_off: Option<Routine>` on `Osaka` (the line latch): set at the first entry of `go_out`, cleared wherever
  `leaving` is cleared today (`errand` 9181, `place` 8611) and when the visit ends. **`leaving` is set only when
  her external door opens** (`start_job(Leave)` or the at-spot arm), as today; so a door in space taken on the way
  (`through_door`, gap 0) or `Act::Out` (4013-4030) can never end the visit early (M1). `gone_out` (5296) is
  unchanged: `leaving && hidden(now) && door(now).is_none()`, read on the face-on `DOOR` table (D8).
- **Stale spots (C5):** `take_in` (osaka.rs:2770) refreshes a `Through::Home` act's spot from `chances.door` when
  `Some` and not `Set::Floor(Protected)`, and converts it to `Through::Space(her feet)` when `None`, before the
  `there` beats (so a door at a stale spot can't be represented, M8). A focused pane over her door is never a
  reason to move it: the act keeps its spot and `evict` handles the focus (below), so a work gap's door doesn't hop
  to the fallback and back with focus. `start_job(Job::Leave { spot, why })` compares `spot` with `chances.door`: moved
  → `go_to` the fresh spot (silently, the latch); `None` → a door in space at her feet (`Through::Space`, gap as
  below). Then it re-checks the routine (decided above): `School` and her routine no longer has her out → she
  decides instead. Otherwise it opens `Act::Door { to: Through::Home(spot), gap }` facing `out()`, with `gap =
  u64::MAX` for `School`, `rng.range(SHIFT_MS)` for `Work` (F13), and sets `leaving` for `School`.

- **Step 4a review amendments (2026-10-08; override the bullets above and below where they differ):**
  `take_in` follows `chances.door_through` (the frame's door read on `door::Ungated` ground, the focused pane
  unprotected, from the opened door's spot), not `chances.door`, so no `Set::Floor(Protected)` special case: a
  face-on fallback covered by focus never moves the opened door either. The Leave walk re-reads `chances.door`
  every step (`door_moved`, osaka.rs walk tick): moved → `go_to` the fresh spot, `None` or unreachable →
  `out_where_clear`; `at_door` keeps only the routine re-check. Every door in space she goes out by for good
  (`go_out`'s `None`, the walk's moved/none, `go_to` false) goes through `out_where_clear`, whose step-off walk is
  `Then::Out(why)` (out at her feet on arrival, census "routine", no decision between). `set_off` is cleared in
  `choose_next` on any decision outside the Away slot (plus `errand`, `place`). `Osaka::on_her_way_out()` (latch,
  `leaving`, a Leave walk/heading, a `Then::Out` walk) gates `awake`, so nothing is delivered on her way out.
  `leave_by(why)` is the one mapping from `Leave` to gap and `leaving`/`returning` (`start_job`, `out_at_feet`).

**School** (`go_out` osaka.rs:5266, from `choose_next` 7371):
- First entry (`set_off.is_none()`): say `LATE`/`OFF`, set `set_off = Some(why)`, log info with the spot. Face the
  chat while saying it **only if she is already at her spot** (C14); otherwise she says it facing her way. **Re-entry**
  (a hop's landing, a walk come to nothing, a chat look) says nothing.
- Then: at the spot → `start_job`'s door arm directly; else `go_to(Want::Walk, Job::Leave { spot, why: School },
  here, terrain, at)` (osaka.rs:8026; `credit = None`, as `go_out` today) — it walks, hops, or takes a door in space
  between floors with no route, as every walk does (the user kept doors in space as her way of getting about).
  `go_to` returning `false` (the spot isn't on a platform of the visit's terrain, osaka.rs:8062): a door in space
  at her feet, gap `MAX`, `leaving` set (M25), but only once her box meets no obstacle; while it does (she's still
  in the piece), the reflex re-enters at her next decision. `Decision::reflex("routine/away")` throughout (tests/dash.rs:199).
  `chances.door == None`: today's door at her feet, `Through::Space((x, y))`, gap `MAX` (the user's "door in space
  where she stands"); the Away door stands wherever `door_place` puts it then, never at her feet.
- `cut` (osaka.rs:3491): the `to_job` arm (3507-3517) matches `Then::Job(Job::Use(..))` only, so a boundary never
  cuts a Leave walk **by construction** (T11).
- `evict` (osaka.rs:9334): the leaving arm (9345-9370) now meets only an opened door (she's through it or in its
  beats): its comment is rewritten to say so. A Leave walk is not `leaving`, so mid-walk eviction takes the
  ordinary arm: she moves out of the pane and the reflex re-enters with the latch (F21). The non-leaving `Act::Door`
  arm (9375-9387): a `Through::Home` act not yet `there` whose box meets the focus becomes `Through::Space(calm
  spot)` keeping its gap and `leaving` (M10); at `there`, a `Through::Space` external exit (`leaving` or a shift
  set) re-reads `chances.door` and comes in through it if `Some`.
- `errand` (osaka.rs:9181, retarget 9198ff): kept: an errand retargets a door not yet `there` to the accordion
  (`Through::Space`) and clears `leaving`/`set_off`; after the poke the reflex sets off again (and says its line
  again: a new set-off).
- `place` (osaka.rs:8611) clears `leaving`/`set_off` as today.

**Work** (settled fact 3; step 4b):
- `mind::work` (mind.rs:464-478) returns `Bind::Work` with **no payload** (the `Route::Around` search goes);
  `osaka.rs:7702` calls `go_to_work(chances.door, at, rng)`.
- `go_to_work(door: Option<DoorSpot>, at, rng)` (osaka.rs:9463): `shift = Going`, then `go_to(Want::Work,
  Job::Leave { spot, why: Work }, ..)` or, at the spot, `start_job`'s door arm; no door place: a door in space at
  her feet with the shift's gap (today's None arm). Silent (Open choice 4).
- `choose_next`'s shift arm (osaka.rs:7289-7301): `Some(Shift::Going)` with a `Job::Leave` walk or heading
  **resumes** it (a landing re-enters), else cuts as today.
- `Shift::Back` (osaka.rs:928-935) and its arms (4029, 7292, 9531, 9540) go: work never leaves by the edge. If
  `Shift` is left with one variant, keep the enum for its doc. `Act::Out`'s `shift` branch (osaka.rs:4024-4027)
  goes. `cut_shift` (osaka.rs:9520-9545): its `(Act::Door { gap > 0 }, _)` arm becomes the only shift share.
- `evict`'s `leaving_for_work` (osaka.rs:9408-9419) matches a `Walk { then: Job(Leave { why: Work, .. }) }` or such
  a heading (C4): the shift keeps its gap, never 0.
- The return: `Through::Home`'s `there` → `back_home` → `come_home` as today, facing `into_room()`; beats 10-12
  draw `Pose::Carry` while a shift is set, so her shopping comes in with her (C13). A work return through a
  `Set::Floor(Yield)` door sets `visit.bumped` at `there` (D7, M13).
- Stage `Scene::Work` (stage.rs:607-637) places her anywhere restful and calls `go_to_work(chances.door, ..)`.

- **Step 4b amendments (as built, and after its review, 2026-10-09; override the Work bullets above where they
  differ):** `Bind::Work` has no payload and `go_to_work(terrain, chances, at, rng)` reads `chances.door` (`plan`
  takes `chances`). `Shift` keeps two variants: `Going { gap }` (drawn at set-off) and `Out` (through the shift's
  door; only coming back out of that door is "home from work"). With no door or no way to it, work goes through
  `out_where_clear` like school (a step out of any piece first, `Then::Out(Work)`), never "today's None arm" at
  her feet. Every cut of a shift on her way goes through `Osaka::let_work_go` (cut, drop a Leave-Work heading and
  its hop), called by `errand` before anything else (walking, round the edge, a door in space, aloft) and by
  `choose_next`'s non-resume branch; `leave_by(Work)` without `Going` is a `warn!`, unreachable. "Her way to work"
  is one predicate, `work_way()` (a Leave-Work or `Then::Out(Work)` walk, or a Leave-Work heading); the resume
  reads `hopping && work_heading() || work_walk()`. `choose_next`'s old "deciding while `Out` → home from work"
  arm is gone (no path leaves a Door act with `Out` but its end, which runs `back_home`). M10 re-points a work
  door in space at `chances.door_through` only while `chances.door == chances.door_through` (no focused pane over
  her door; else she comes home where she went: re-pointing to the gated `chances.door` would be undone by
  `take_in`'s Home arm the next frame). Census purpose of a work Leave walk/heading/`Then::Out(Work)` is
  **"work"** (not "routine"; step 10 reads it in the work bucket). tests.rs `out_at_work` keeps a door in space
  at her feet in her sofa via chances with no door and no obstacles (its tests are about her box's image while
  she's out), not `go_to_work(chances.door, ..)`. `Osaka::bumped()` lives on Osaka (per visit), set by any
  Home return.

**Dash** (`dash_on` osaka.rs:5684): unchanged; out again reaches `go_out` through `choose_next` and walks.

### D7. Felt rules `DoorClear` and `InChat`, and the per-frame closet

- `Rule::DoorClear`, `Rule::InChat` (rules.rs:19), no payload; `kind` arms "door", "chat" (rules.rs:168).
  `RULES: [RuleRow; 8]` (rules.rs:63), the two rows **appended** (tests index rows 0 and 5: tests.rs:9891, 10206,
  10253, 10355, 10838; `broken_for` osaka.rs:10824 takes the first row by use), `felt_on: ANY_USE`.
- `broken(laid: &LaidOut, home, keep: &Keep)` and `judge` gain `keep` (sites: rules.rs:466, 573, 595, 1309, tests
  1035, 1112, 2336, mod.rs:3367, tests.rs:12076). Template: `Belongs` (rules.rs:262-276), one `Broken` per piece,
  `pieces = involved = [piece]`:
  - DoorClear: a laid piece on the door's strip whose `cover()` meets `space.rect` (**kept or not**; only where a
    `Space` exists, so never on a short room).
  - InChat: a laid strip piece whose `cover()` meets `keep.chat`, not stranded. This includes a window
    `hung_clear` shifted into the chat in an overlapping custom layout (M27): judged and repaired like any other.
- `Felt.on: Option<(Furniture, Use)>` (osaka.rs:1946): `None` for a felt-on-sight grievance; `just_set`
  (osaka.rs:8208-8212) becomes `.and_then(|f| f.on)`, so no "use-it" sit-back is invented for a lamp.
  `feel(key, on: Option<..>)` (osaka.rs:8121) loses its "stage only" doc.
- **The non-use trigger** (in `furnish`, after `visit.broken` is set, mod.rs:3368-3374): **feel** on the first frame
  she's in sight each visit: every DoorClear key if `visit.bumped` (a `Fallback::Yield` arrival, or set at `there`
  for a work return, M13); every InChat key. `How::Idle` arrivals and unfed clocks never feel a use-less piece in
  the space (accepted: the lamp waits for a return). **Say** the row's line only if a key of it was felt, only when
  her mood would mend (`to_mend` osaka.rs:8430 > 0), at most once per game day per row (an in-memory day on the
  guest; a restart may repeat it once), and only once she's quiet after her arrival line ("I'm home!" from
  `home_from`/`come_home`, osaka.rs:9487-9570): standing, for `GRIEVANCE_MS`, covered by `grumbling()` so a look-up
  doesn't step on it (the `grievance_from` pattern, osaka.rs:1655). Her lines never show two at once (C4, C6).
- **The per-frame closet** (applied in `Home::frame`, not `project`): `fn stranded(home, laid: &LaidOut, chat) ->
  Vec<usize>` (room.rs, pure). The pieces whose cover meets `chat` are taken in layout order and assigned
  **jointly**: each to its own strip at a left whose cover misses the chat, else to another room's lane where it
  packs with that room's pieces and the candidates already assigned there, on a stretch clear of the chat and the
  space. A piece no room takes is **stranded**: left out of `shown` (hidden, kept in `layout` and the record).
  Same in Visiting and Away; recomputed per frame (a pack per room). Stranded pieces aren't judged InChat. **The
  documented gap** (M20): a piece some room geometrically takes is shown in the chat until a repair moves it,
  which needs a mood that mends and a repair `evaluate` accepts; a Lazy Osaka lives with it (step 6 test 7 pins
  this).

### D8. The side-on door, wired (snippets "Wiring notes")

- **Look:** `Look::WallDoor { door: art::WallDoor, sky: art::Sky, cols: u8 }` (graphics.rs:65), size `(cols, 4)`,
  rendering the 6×4 frame (`render_wall_door`) cropped to the inked columns of the state (table, map art §1):
  `Shut{0,false}` 2, `Shut{0,true}`/`Ajar`/`Open` 4, `Shut{flap>0}` 6, `Post` 2 (`WallDoor` already derives
  `Hash`); `sky` threads `Sky` (art.rs:1385, `Sky::at` 1410) into `wd-beyond`, the hedge's tint and `wd-flap-hole`
  via per-sky gradients in `art/wall-door.svg` (unfed → `Day`, matching the window's `Plain`, art.rs:1269). Step 7
  keeps (and adds) `#[cfg_attr(not(test), allow(dead_code))]` on everything it introduces; step 8 removes them all
  (art.rs:1119, 1220; sprite.rs:478, and step 7's).
- **Layer clip and offset** (graphics.rs:136, `Key` 154, `key` 568, `compose` 735): `Layer`'s fields stay as
  they are (its literals in mod.rs: `Placement::layer` ~327, `Door::layer` 381, `prop_layer` 4094-4105). A new
  `pub(super) struct Cut { layer: Layer, clip: Option<(Side, i32)> /* the wall column */, dx: i8 /* half
  columns */ }` with `From<Layer>` (no clip, no offset), and `paint_cuts(buf, &[Cut], open)`; `paint_layers`
  becomes a wrapper mapping its layers to plain cuts, so every existing caller and cache key is unchanged. `Key`'s
  per-layer tuple gains the clip and offset (graphics.rs:154). `compose` zeroes alpha beyond `w*cw + (cw-t)/2`
  (Right: `>=`; Left: `< that + t`; `LineGeometry::for_cell` graphics.rs:179, not the sheet's formula) before
  `overlay`; `bounds()` (146) is **clipped at the wall** so her doorway box never claims the neighbouring pane's
  cells (else `key` refuses the whole image at an inner wall). **Trap:** goldens hash `figure.her()`
  (tests/golden.rs:56): keep `Placement`'s Debug (mod.rs:300) unchanged; carry her doorway offset and clip in
  `Figure` (mod.rs:409) beside it, not in `Placement`.
- **Beats.** The **timing and semantics stay on the face-on table**: `DOOR` (osaka.rs:1239), `door_beat` (its beat-6
  gap stretch, 1320-1325), `hidden`, `Osaka::door()` (9067) and `gone_out` (5296) are unchanged, so beat 6 is still
  `door: None` there and school still ends the visit (C1, F5, M23). Her external door alone stretches beats 2 and 10
  to walking pace (`4 × WALK_MS`, osaka.rs:386, she steps a column per `WALK_MS`, `d ∈ 0..=4`, `Walk(d % 4)`) by a
  `Through::Home` duration override in `door_beat`'s lookup, never by editing beat 6 (C11; Open choice 2). A
  **draw-only** `fn wall_beat(beat, elapsed_in_beat, shift: bool) -> WallBeat { door: Option<WallDoor>, her:
  Option<(Pose, d)> }` maps beats per the approved table (snippets.md:176-189): 0 `Shut`, 1 `Ajar`, 2 `Open` with
  her stepping through, 3 `Open`, 4 `Ajar`, 5 `Shut`, **6-7 `Shut{away: true}`, 8 `Ajar`, 9 `Open`** (the slippers
  go as the door opens, C2), 10 `Open` with her stepping in (`Carry` while a shift is set, C13), 11-12 `Ajar`/`Shut`
  with her held `Side` facing the room from beat 10's last step through 12, her one turn to the viewer being her
  arrival line's `Stand` (C12). `Post` after her while `d > 0`. In ASCII she is clipped at `d = 2` and hidden at
  `d >= 4` (C19). `first_due` (osaka.rs:2861) wakes at each step. Her pose during an external door comes from
  `wall_beat`, not the fixed `Pose::Stand` (osaka.rs:9857-9861).
- **When the door shows in a visit** (C3): `Shut` from her set-off (`set_off.is_some()`, or a `Job::Leave` walk or
  heading with `why: Work`), so its one appearance coincides with her line or her turn toward it; then the beats;
  after beat 12 it stays until her box no longer meets the space, then goes. So it never pops in or out beside
  her. A face-on `Set::Floor` door shows the same way (closed `Look::Door`).
- **Text** (C9): the door's image is drawn over text in passing during its beats 0-5 and 7-12 and its going, as her
  image is (`Hidden::check`'s `HIDDEN_MS` budget); during her walk-up it is drawn over text only within that budget
  (a long walk hides it over text until her beats start); during a work gap's beat 6 and while she's out
  it stands over text (the user's answer).
- **Draw paths:** visit line art `draw_art` (mod.rs:4482; layers 4581-4586: door → her (clipped) → `Post` →
  pieces in front), visit ASCII `draw` (mod.rs:3022: `wall_door_cells` from `sprite::WALL_DOOR`, her cells at
  `x >= w` (Right) / `x <= w` (Left) dropped), Away `draw_door` (mod.rs:4786). **Wall-column cells in every mode go
  through `draw_flap`'s rule** (mod.rs:4143: only over a plain `│`/`┃`, skip protected, record `Frozen { under:
  border }`); `put` (cells.rs:57) must not write them blindly. The wall door's ASCII uses its own
  `sprite::mirror_wall` (`[`↔`]`, then `mirror`); `mirror` (sprite.rs:453) is unchanged, since it also flips her
  sprites with `[]` rows (sprite.rs:349-350, 395-396) (F4).
- **The Away cue** (`Shut{away: true}`) in `paint_empty` and the visit's gap beats 6-7; its slipper columns only
  over calm cells (else cropped to the shut door's 2, which stand over text). No
  ASCII cue (snippets).
- `door_rain` (mod.rs:2805) filters `empty.painted` by `empty.door`'s drawn cells.

### D9. Parcels through the flap

- `doorstep` (room.rs:1517): her door's wall (`home.wall(plan)`) first when its space is kept (the parcel at the
  floor extent's `Anchor { side, offset: 0 }`, trailing edge `w-7`), `Flap { .., door: bool }` (room.rs:652), the
  flap's x always from the **raw** extent (the wall column; room.rs:1595-1598 today reads the extent it packs on,
  F8); then today's walls (sort 1533). Refusals per D3.
- **A delivery waits** (C10) while her external door's act runs, while she is out in a gap (the slippers stand in
  the space), or while her box meets the space, as the clock's delivery already waits on `visit.flap`
  (mod.rs:3345); both delivery sites (mod.rs:3322, 3352) check it.
- The slide (snippets "The flap's beats"): 0-150, 150-350, 350-500, 500-650, 650- ms: `Shut{flap: lean(..)}`,
  the parcel as `Look::Parcel(item, false)` at a half-column `dx`, clipped at the wall, and the plate
  (`WallDoor::Plate(u8)` from `wall_door_scene(&flap_plate(d), ..)`) drawn after it, **in one `paint_cuts` call**
  (it needs the clip and `dx`). `lean` (a closure in `wall_door_sheet`, art.rs:3615) moves into production art.rs
  (step 7); `PARCEL` is already there. The at-rest draw of that parcel is suppressed until 650 ms.
- Wakes: `visit.flap` (mod.rs:500) expiry (1684-1686) stays at `FLAP_MS` (mod.rs:4138, 800 ms, already past the
  slide's 650); the wake (1972-1975) adds each boundary. ASCII: `WALL_DOOR[1]` for 650 ms, the parcel at its spot
  at once.
- The slide's stillness exposure (5 changes in 650 ms against `USE_FRAME_MS` = 1400, osaka.rs:1640) is Open
  choice 3; step 9 builds the recommended answer and its test.
- `draw_flap` stays for the fallback walls only.

### D10. Stage

- *As built (step 4a):* `enum Leave { School, Stage }` (Work comes in 4b); `Scene::School` is `Leave::Stage`, and
  `returning` is set as its door opens (`start_job`), not at set-off, so a decision on her way (a landing) can't
  say "I'm home!" before she's out. `start_job` never reads `returning`.
- `Scene::School` (stage.rs:28; a test-and-stage cue, step 4a): she sets off for her door as at 08:15 but with
  **no `leaving` and no `set_off`**: `returning = Some(Routine::School)` (consumed by `back_home`, osaka.rs:9487)
  and a `Job::Leave` whose door opens with gap 5000 ms, so the visit never ends and she comes back through the
  coming-in beats with "I'm home!" (M14, F6). `Scene::ALL` (stage.rs:166) grows from 57 to 58;
  `every_scene_has_a_spot_in_the_stage_room` (tests.rs:1762) accepts it (she stays `Visiting`); tests.rs:9128,
  9187 and 9240 (wants/ids over `Scene::ALL`) are checked. The full Away state is reachable in `cargo run -p
  dessplay --example houseguest` with `t` (skip her clock: examples/houseguest.rs:276) to a school morning.
- `Scene::DashIn`/`DashForgot` (step 3) and `Scene::Work` (step 4b) use the space. `Scene::Parcel` goes through the
  flap (step 9).

## Tests

Rules for every step: written first, run, **confirmed failing for the stated reason** before the fix (a test that
can't be red today is labelled a **guard** and a reviewer proves it with a mutant); deterministic (seeded, game
times via `tue`/`sat`/`home_at`); every houseguest test loops `for graphics in [false, true]` (proptests draw it,
the unit regressions loop it); panes filled with text where terrain matters; never a bare `with_cases(N)`
(`proptest_cases(N)`, dessplay-core test_support.rs:43); a proptest failure is committed to its regressions file
**and** pinned as a unit test (the `her_days_case_pulling_text_as_school_begins` pattern, tests/away.rs:1257).

- **Never change an existing proptest's strategy** (T5): its `cc` seeds in `proptest-regressions/` (away.txt 3,
  tests.txt 26) would replay as different cases. New dimensions (`apart`, a cue, an overlapping chat) are **new
  proptest functions** sharing the body.
- **Non-vacuity** (T3): every door property counts the frames with a drawn `Set::Wall` door and asserts a floor
  (per unit test: at least one; per proptest: a deterministic companion test, or a final count over the run).
  Expected states per fixture are stated: on `home_screen` the chosen wall is **Users' left** (edge, col 0;
  `[U-L, P-R, U-R, P-L]` after the sort), the space cols 1-6, rows 12-16; on `wordy_home_screen` (tests.rs:5954,
  the user list at col 2, rows 9-14) that space is under text, so the Away door is **drawn over the text at its spot** on
  every frame, asserted as such (the slippers cropped).
- **Strict helper** (T8): the migrated `door_shows` (away.rs:63) counts a drawn cell under any `empty.shown` cover as
  a failure, not a skip; no text tolerance for the door's own columns (they stand over text); the slipper columns are only drawn on calm cells.
- **Honest geometry.** `view()` (tests.rs:20) puts the chat at `(0,0,30,20)`, over `nooks()`'s List box
  (tests.rs:62): every chat property would fail on the fixture. Step 1 fixes this before any production change.
- **Chat cases built on purpose** (T6): `chat_apart` and the bundled layout are disjoint, so no chat clause can fail
  there. Three constructed layouts, each asserting its precondition: (i) the space yields and the nearest calm floor
  to it is the chat pane's; (ii) every edge wall's space meets the chat (the chooser picks an inner wall); (iii) no
  wall outside the chat (no door drawn while out). Plus an **overlapping** variant of the long-visit property (a chat
  meeting part of a nook, as ui/app.rs:4003's grid overlap), the only geometry that reaches InChat, the closet and
  `hung_clear`'s chat refusal in a long run.
- **The reliable red for the class** is a new proptest (T4), `her_school_mornings_never_put_her_door_on_a_piece`
  (tests/away.rs, step 3): start Tue-Fri 08:12..=08:14; `owned` with at least one of Bed/Sofa/Desk plus an optional
  Lamp/Fridge/Window; `cue: Option<Scene>` from {Sleep, Lounge, Nap, Homework} fired once visiting; `apart: bool`;
  a 60-120 s span. Budget about 3 s at 32 cases, measured in the commit (T24). `her_days_never_touch_what_is_protected`
  stays the day-long guard.
- **Gates** (F9, T24): every step's exit is `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `PROPTEST_CASES=32 cargo nextest run --workspace --all-targets` (which includes
  `changelog::tests::embedded_changelog_parses`, dump.rs and other crates); the targeted `-E 'test(/houseguest::/)'`
  filter is for the edit loop only. Steps 3 and 8 also run the perf tests (`cargo nextest run --profile full
  --release`, perf.rs).

## Steps

Each step: one implementer (tests first and confirmed failing, implement, the gate above, commit with `jj commit`),
two reviewers, a fixer. Goldens re-recorded per behaviour step with the trace diff (below). CHANGELOG entry **in the
same commit** for user-visible steps. Sizes are changed lines, tests included. **Strictly sequential**: 1 → 2 → 3 →
4a → 4b → 5 → 6 → 7 → 8 → 9 → 10. (Step 7 touches only graphics.rs, art.rs, art/wall-door.svg and sprite.rs, but
parallel builders would share one jj working copy; if parallelism is wanted, it runs in a git worktree committing
with git, per the project memory, and is rebased before step 8.) **No push of `master` mid-batch** (`install.sh`
follows it; steps 2-7 leave intermediate looks): push after step 4b at the earliest, preferably after step 9 (F23).

### Step 1. Honest test fixtures (test code only; ~350 lines)

**Goal.** The harness stops putting the chat over a nook, so the chat rule can be tested; no production change.

- `view()` (tests.rs:20-32): `chat: Rect::new(0, 0, 30, 0)` (no area, same middle column 15, so `look`'s target at
  mod.rs:2893-2899 stays identical). It also feeds `Chances.chat` (mod.rs:2302) and through it `in_chat`/`elsewhere`
  (osaka.rs:353, 4107, 9272) for resident tests: check tests/calendar.rs:139, tests/dash.rs:1032, :1302 and every
  `chat:`/`.chat` use in tests/ still means what it says; a resident test that needs a real chat gets
  `chat_apart` or an explicit resident chat (T13). Doc: tests of the chat rule use `chat_apart`.
- New `fn chat_apart(w, h) -> (Buffer, IdleView)`: `rooms(w, h)` with `chat = nooks(w,h)[0].1`, `nooks =
  nooks(w,h)[1..]` (the `resident_view` tests.rs:6501 pattern), non-resident.
- New `fn chat_over(w, h)` (the overlapping variant: a chat meeting part of Users) and the three constructed chat
  layouts (Tests, "Chat cases"), each with a test of its precondition only.
- `long_visit_of` (tests.rs:759) takes `frame: impl Fn(u16, u16) -> (Buffer, IdleView)` and `cue: Option<Scene>`
  (fired once visiting; `None` everywhere today, for T4's proptest in step 3) (it builds `real` and the
  view inside its size loop and at its final dissolve, tests.rs:794-806, 965-971). Keep the Away `spanned`
  allowance (tests.rs:905-943) **for now** (step 3 removes it).
- New proptest functions sharing existing bodies (no strategy changes): `her_days_with_the_chat_apart_never_touch_
  what_is_protected` (tests/away.rs, `owned`'s pane index mapped onto the two remaining nooks) and
  `long_visits_with_the_chat_apart_never_touch_what_is_protected` (tests.rs).
- New `long_visit_on_the_real_layout` (tests.rs): one `long_visit_of` run on `real_frame(&mut real_ui(), 100, 30)`
  (tests.rs:990, built once per size), both modes, a fixed seed and school-morning start, so the bundled layout is
  gated.

**Tests first.** None red (fixture step). Exit: the gate; **every golden byte-identical** (run `golden` with
`HOUSEGUEST_GOLDEN_TRACE` before/after if any hash moves, and stop: it must not).
**Traps.** `nooks(` has 34 callers; change none of them. No existing proptest's strategy changes.

### Step 2. The space: `Home.door`, `extents`, the chooser, the ledger (~1300 lines)

**Goal.** D1 and D2. Pieces on her door's strip stand 6 columns along; nothing is drawn differently yet (the door
still stands at her feet). The parcel on the door's wall rests just past the space; its `draw_flap` stays at the
wall, its x from `raw` (a 6-column gap between flap and parcel until step 9: acceptable intermediate, not pushed).

**Sites.** room.rs:629-1700 (types `DoorWall`/`StripPlan`/`Space`/`LaidOut`/`Plan`, `extents`, `choose_wall`,
`wall`, `settle_door`, `frame`, `project_with`, `pin_anchors` (raw), `laid_and_shifted`/`laid_on`/`hung_shifts`
→ `LaidOut`, `hung_clear`, `move_off` (floor, report the target), `spot`/`doorstep` (`plan`), `admits`);
rules.rs:183-866 (`broken`, `judge`, `against_wall`, `Before::new`, `evaluate`, `search` per-candidate-strip floor)
and its tests 1309, 1754, 2166; mod.rs:3206-3420 (`furnish` → `frame`), 3485-3523 (`stage_arrange`), 3658
(`place_gift`), 4704 (`paint_empty`), 211/228 (keep raw); ledger.rs:46, 154-235, 369, 568-655; tests.rs:833, 1785,
12076. Remove the stale "(series), when short" in `Nook::List`'s doc (room.rs:400).

**Tests first** (room.rs `mod tests` 1863ff unless noted):
0. **The red test, on today's API:** `her_fridge_by_the_screen_edge_leaves_room_for_her_door` (tests.rs, both
   modes): `home_at(seed, tue(16,0), &[(Fridge, Users, 1000)], g)` (tests.rs:3865) on `rooms(100, 30)` (Users at
   `(50,0,50,13)`, its right wall col 99 the screen edge), paint until visiting; assert the chosen `DoorWall` is
   `Users Right` (once the API exists) and `shown_piece(&guest, Fridge)` (tests.rs:5335) has its right edge at
   `w - 7`. **Fails today:** the fridge stands at `w-4 ..= w-1`, in the space.
1. `no_floor_piece_or_window_meets_her_door_space` (proptest beside
   `the_wall_lane_keeps_order_and_never_moves_the_room` 3133, `proptest_cases(128)`; `decorated()` 2252 +
   `placed()` 2269; `users`/`two_panes` layouts with `door` drawn over **both sides of both strips**): whenever
   `space.kept`, no laid floor piece's cover and no window's rect meets the space rect **computed independently from
   the nook rect and side** (the geometry words), never read from `space.rect`; poster and clock may (T14).
2. `a_home_packing_both_ways_keeps_its_anchors_and_order` (extend `packing_keeps_order_and_a_resize_and_back_restores`
   2328, 256 cases): with a door set, layout order equals the raw order, anchors are unchanged, a resize and back
   restores the home (door included: `PartialEq`), and a strip that packs only raw lays out byte-identically to
   `door: None` (the yield). Guard.
3. `whether_the_space_is_kept_never_depends_on_anchors` (proptest): `extents()[i].space.kept` is unchanged when the
   strip's pieces are re-anchored at random (T15). Guard; a mutant computing `kept` from `laid_on`/`fits` fails it.
   **As built (step 2):** floor pieces only. Under the built yield rule (deviation 1 in door-notes.md: kept only
   when no hung piece is lost either) a hung piece's anchor can flip `kept`; pinned by
   `an_older_records_window_keeps_its_place_by_her_door` (its window at `Right 0` yields, at `Right 1` keeps).
4. `only_the_window_hangs_into_her_door_space` (replaces the redundant hang check, T16): for each hung item,
   `stand(..)`'s rect on an extent meets that extent's space rect (rows `f-4..=f`) **iff** it is the window (an
   off-by-one `f-5` fails it through the poster at hang 4).
5. `an_older_record_keeps_its_fridge_by_her_door` (tests/window.rs, after
   `an_old_window_loads_hung_low_drawn_and_in_reach` 307, built with `Ledger::from_json`) on **`rooms(100,30)`**
   (T1): no `door` key, a fridge at `Anchor{Right,0}` and a **share-only** piece (`anchor: None`, `at: 700`, T17)
   on Users, plus a variant whose pieces have **no anchors at all** (M2) and pack raw but not narrowed; after one
   `frame`: the `DoorWall` asserted explicitly (`Users Right`, M26), the fridge's right edge at `w-7`, its anchor
   unchanged, the share-only piece pinned on raw and laid at the left computed from the raw share against the
   narrowed floor, nothing closeted, `AgainstWall(Fridge)` not broken; the no-anchor variant lays every piece on
   raw (yield), nothing closeted, nothing moved; `to_json` round-trips with `door`. Both modes.
6. `the_door_wall_is_chosen_once` (room.rs): edge first; a **left-edge-only** layout (T18); her pieces' strip
   before another strip's edge only among edge walls; a **crowded edge strip is still chosen**, its space yields and
   (from step 6) DoorClear is broken (M5); a wall whose space meets the chat refused; an inner wall when no edge
   qualifies; a resize never re-chooses; **hiding the door's pane and showing it again** keeps the wall (M6, F14);
   `move_off` that moved the door strip's pieces re-chooses on the target; a short frame (rows < 4) saves nothing; a
   saved wall whose space comes to meet the chat stays saved (F10). **As built (step 2):** the DoorClear clause
   moves to step 6 (its to-do there); a hidden pane keeps the wall only while no other strip can take her pieces
   (when one can, they move and her door with them, and showing the pane keeps both there: an open question for
   the user against M6/F14); the move cases are `her_door_follows_her_pieces_not_the_best_wall`,
   `her_pieces_moving_with_her_door_stand_once_where_they_stay` and `her_pieces_never_move_under_the_chat`, the
   third key `her_door_goes_where_its_space_is_kept`.
7. `the_first_tv_rests_past_the_space` (M3): a fresh ledger's first delivery rests with its trailing edge at `w-7`
   on the frame it arrives, and no later frame moves it. Red as designed before `wall()`.
8. `an_older_record_migrates_without_closeting_anything` (moved here from step 6, T19): a proptest over records
   (including 1000-share pieces) on `home_screen`, `rooms` and `real_frame`: no piece hidden that was shown before
   the door. Re-run in step 6.
9. ledger: `a_door_wall_round_trips`, `an_older_build_reads_past_the_door` (as 1214), garbage `door` reads `None`;
   `the_clock_sent_round_trips` still passes; dump.rs's JSON test unchanged (no door) plus a case with one.
10. rules: `every_repair_mends_and_breaks_nothing` (2281) and `rules_hold_across_a_resize_and_text` (1319) run with
    a door set; `the_fridge_and_the_bookshelf_want_a_wall` (1118) with the fridge beside the space; a repair onto
    the door's strip lands at the left `search` chose (F7). **As built (step 2):** F7 is
    `a_move_onto_her_door_strip_is_weighed_with_the_piece_there` (the proptests can't show it: a stale plan only
    costs repairs).
11. `a_poster_is_delivered_where_it_fits_boxed_and_hung` (room.rs:2915) asserts `flap.x` is a wall column (F8).
- Migrate (T1): tests/window.rs:11-40 (`sofa_and_window_at`'s pinned columns), :307-380 (the old window at
  `Left,1`, the sofa/TV at `Left,0` on `home_screen`, where the door is Users' left wall), away.rs `HOME` (sofa at
  Users 300). Room/rules unit tests calling `project` keep `door: None` and mean it (F2).

**Exit.** The gate; `repair_search_is_cheap` (rules.rs:2038) still under its bound (per-candidate-strip `extents`:
measure); goldens re-recorded with the reason "pieces on her door's strip stand 6 columns along; the record saves
`door`" (every fed golden with a home moves: `finish` hashes `to_json`, golden.rs:88); `UNFED_*` move only by
layout/record, said per table. Band gate re-run (Census below).
**CHANGELOG** (F24): "Changed: furniture by the screen's edge stands a little further along, leaving room for
Osaka's door." **Docs:** design.md *Strips and anchors* (1813-1830): the space rule (brief §5.1's text, updated
for B); decisions.md: new entry "Her door has its own space at the screen's edge (2026-10-08)" with the post-mortem
(`restful` reused for the door), the rejected alternatives (`blocked` closets; a wall derived each frame hops with
deliveries; moving the room off its strip; the chooser inside `project`), and the older-build anchor note.
**Traps.** Pin on **raw**, always; `extents` after pinning. `furnished()` is prop order (room.rs:1164); the chooser
iterates `nooks`. Left wall is `from - 1`: write `to -= 6` / `from += 6`, never `wall - 6` for both. `settle_door`
never writes `None` over a saved wall. The edge test reads the nook rect. `doorstep` packs on `floor` but takes the
flap's x from `raw`.

### Step 3. `door_place`, the `Out` shape, the Away door at the space (~1500 lines)

**Goal.** D4 and D5, and the space half of D3's makeshift keep-out. The Away door stands at the space's spot,
face-on (`Look::Door`, its box `w-5..=w-1`) facing the wall, or at the strict fallback, or nowhere; arrivals come
out of it; nothing records her feet. The visit's own going-out door is still at her feet (step 4a walks her).

**Sites.** New `door.rs` (`DoorSpot`, `Set`, `Fallback`, `Ground`, `door_place`, the obstacles helper,
`DoorSpot::at`); mod.rs:645-731 (`State`, `How`, `DoorAt`, `Out`, `Empty`), 842, 1294-1440 (`leave`, `rain_home`,
`call_off`, `door_of`), 1744 (`absent`), 1792 (`school_out`), 1849-1910 (`closed_door`, `out_by_door`), 2019-2124
(arrivals, Away arm), the `Visit` struct (`door: Option<DoorSpot>`, the last spot placed, derived each frame like
`Empty.door`, the visit's `prev`), 2353/2403 (`Chances.door`, read by the stage's dash scenes and by `dash_through` only, until
step 4a), 2805 (`door_rain`), 3837/3922 (`made_stands`/`builds` call `Keep`, which lands here with `Keep::of` built from
`Rect::default()` as the chat until step 5 passes `view.chat`), 4600-4626 (delete
`door_fits`/`door_spot`; keep `middle`), 4657-4784 (`paint_empty`); osaka.rs:26 (`Chances.door`),
`back_through_door` 9111, `dash_in` 9138, `dash_through` 9163 (take `DoorSpot`); stage.rs:465-496.

**Tests first** (tests/away.rs unless noted; each loops both modes):
1. `out_of_her_bed_at_school_time_her_door_misses_it` + `_sofa_` + `_desk_` (T2): `home_at(seed, tue(8,12),
   &[(Bed|Sofa|Desk, Users, ..)], g)` on `rooms(100,30)`, `until_visiting`, cue Bed→`Scene::Sleep`,
   Sofa→`Lounge` and `Nap`, Desk→`Homework`; **precondition** at the last frame before 08:15: she is in `Act::Use`
   on that piece and her box meets its `cover()`. Run to `State::Away`. **Red first, on today's API:**
   `guest.closed_door()`'s box (or the Away image's door cells) meets an `empty.shown` cover; confirm it fails. Then
   add: the closed door's spot equals the space's spot **computed independently** from Users' nook rect and side,
   it is `Set::Wall`, and at 12:45 she comes out of it.
2. `her_school_mornings_never_put_her_door_on_a_piece` (the new proptest, Tests above; T4). Red today.
3. `her_door_stays_in_its_space_whatever_takes_her_home_away` (rename of :376): key, overlay, too small, visits
   off: it shows again at the same spot. Per screen: quiet → drawn; wordy → drawn over the text, slippers cropped, spot kept (T3).
4. `a_resize_moves_her_door_to_its_space_or_the_nearest_clear_spot` (rewrite of :466): on `wordy_rooms(100,30)`,
   the fallback's box meets no cover; the strict predicate checked directly.
5. `on_a_short_terminal_her_door_falls_back_clear_of_every_piece` (T7; H 18..=22 on `real_frame(&mut real_ui(),
   w, h)`, built once per size): precondition `space: None` (or `!kept`) for the door's strip on every frame; the
   door is `Set::Floor(Short)` meeting no cover, or absent and the arrival waits. Proptest over H and owned pieces
   (`proptest_cases(16)`).
6. `her_door_is_never_in_the_chat` (T6, T21, F10): on the three constructed chat layouts, each precondition
   asserted, plus a layout where the chat comes over the saved space; Away and arrivals. For the "red today" case
   `osaka.place` her on the chat pane's floor at 08:14:59 game time and assert it.
7. `a_focused_pane_never_moves_her_door` (extend :504/:578; the focus box from the drawn door cells, not a 5×4 box):
   in use, hidden; released, back at the space (C2); **`with_her_doors_pane_focused_she_neither_comes_out_nor_walks_
   into_it`** (M11): the arrival waits; the visit's `chances.door` is `Set::Floor(Protected)` outside the pane.
8. `her_fallback_door_doesnt_hop_with_pane_text` (M12, C7; wordy panes, H 20, both modes): text written into and
   out of its box; the spot is identical every frame, drawn over the text.
9. `with_no_home_she_comes_in_by_the_same_wall_each_time` (rewrite tests/dash.rs:900).
10. `a_scrap_in_her_door_space_makes_it_fall_back` (T9, M4): a made piece (and, separately, the ghost) in the
    space → `Fallback::Blocked`; `builds` never builds in the space; a made piece the space comes to meet falls
    apart.
11. `her_days_never_touch_what_is_protected` and its `_apart` sibling extended (assertions only, no strategy
    change): (a) in `State::Away`, the drawn door's cells (from `empty.door`) meet no `empty.shown` cover; (c)
    whenever the door is `Set::Wall`, no shown piece's rect meets the space bar poster and clock; plus no door meets
    `view.chat` on `apart` runs; non-vacuity counted. `long_visit_of` loses the Away `spanned` allowance
    (tests.rs:921-944) and takes its feet from `empty.door`.
12. tests/window.rs:207 rewritten: "a door forced onto her sofa stands in its space instead; its image takes in no
    piece" (forced by cueing her onto the sofa at 08:12, precondition asserted).
13. `with_no_door_anywhere_she_still_comes_home` (M22): layout (iii); the arrival comes in by a door in space
    outside the chat.
- Migrate: tests/away.rs helpers `closed`/`door_shows` (strict, T8)/`out_to_school`/`door_raining` (55, 63, 110,
  1277) to `DoorSpot`; `a_school_morning_out_through_her_door_and_home_again` (:125), `a_cold_start_in_school_hours`
  (:626) and :376 get per-screen expected states (T3); tests/dash.rs:670 and :1018 (`o.door.is_some()` →
  `guest.closed_door().is_some()`), :1171 (the stage dash now uses the space); tests/census.rs:1920 compiles
  unchanged.

**Exit.** The gate and the perf tests (F11). Plus `her_door_stands_over_text_and_keeps_its_spot` (text written into
the space: the door is drawn over it at the same spot, its slippers cropped; red against a build that hides it). Report in the commit how often
the space's drawn cells are calm on `real_frame` with `chatty_ui` (populated Users and Playlist) and on the wordy
fixture ("never", T3), for the record (the user chose the door over text). Goldens `school_morning`, `home_from_school`, `dash_home`
re-recorded ("her Away door and her return at the space"), the rest identical. **CHANGELOG:** "Fixed: while Osaka
is at school her door no longer stands over her bed, sofa or desk, or in the chat pane; it stands by the screen's
edge where there's room." **Docs:** design.md :2209-2221 ("stays standing, closed, where she left ... nearest floor
spot where she'd fit") → the space and the strict fallback; :1380-1382 ("stays within her box, so it never covers
anything") true only for doors in space; the paragraph 2390-2399 kept (C2). decisions.md: pointer from "Away at
school ends the visit at her door (2026-10-04)" (decisions.md:3334, "closed where she went out" at 3350): superseded
in *where*, not *whether*.
**Traps.** `frame` mutates the ledger (`changed` → `unsaved`), the arrival's too. The arrival's terrain is read
before `take_home`'s pieces are projected (mod.rs:2026): frame first. Away draw reads **unkept**; arrivals and
`Chances.door` read **gated**; the `standing` filter reads gated `view.protected`. `Empty.door` is derived; never
write it from anywhere but the paint. `closed_door()` is also reached by tests (tests.rs:818, 953).

**As built (step 3, 2026-10-08).** `door.rs` holds `DoorSpot`/`Set`/`Fallback`/`Ground`/`door_place`/`obstacles`/
`Keep`. `Ground` is four text-blind per-cell flags kept by `Terrain::read` (`ledge`, `wall` = plain `│`/`┃`,
`stroke`, `protected` = in a protected rect, untouchable or off screen) instead of `platform`/`line`; the Unmarked
test reads ledges whether protected or not, so a protected space gives `Protected`, not `Unmarked`. A saved wall
whose strip isn't on screen gives `NoWall` (only a strip with no space gives `Short`); on the bundled layout her
pieces leaving a short strip take her door's wall with them (step 2's `move_off`), so a short terminal gives
`Short` or `NoWall`. `back_through_door`/`dash_in`/`dash_through` keep `(spot, facing)` arguments (callers pass
`spot()` and `into_room()`): the no-door arrival has no `DoorSpot`. `visit.bumped` is not added yet (step 6, its
only reader). Keep's chat is `Rect::default()` at the step-3 call sites; furnish's delivery `seats` closure uses
the pre-delivery home's keep (step 5). Test 7's arrival with her door's pane focused does not wait: the fallback
stands outside the pane (`Protected`) and she comes in there. Test 13 runs on fixture (iv) and asserts what D5
gives there (no calm floor outside the chat: her coming home is called off); see door-notes.

**Review fixes (step 3, 2026-10-08).** An empty chat rect is no chat (`room::in_chat`, at `in_space`, `on_floor`,
`Keep::refuses`, `choose` and `move_off`; `plan_of` hands out `Rect::default()` for one). The visit's terrain is
read with `Terrain::read_guarded(buf, solid, view.protected, ..)`: her door's `protected` is the view's alone, so
text she moved never moves the visit's door. A `LayerOp::Make` whose piece the frame's `Keep` refuses is refused
before it's applied (a build planned before her door's space came there is never made to fall apart a frame
later). M22's door in space is reached with a protected floor row (the fallback refuses a protected cell under
her; a platform doesn't): `with_no_door_anywhere_she_comes_home_by_a_door_in_space_outside_the_chat`. Test 10's
third clause is `a_made_piece_her_door_space_comes_to_meet_falls_apart`. Open: a face-on fallback may straddle a
pane's `│` above the floor (case (i): (67, 26)), the user's call.

### Step 4a. `Job::Leave`: she walks to her door; the latch (~1000 lines)

**Goal.** D6's school, dash and Through parts: every routine exit and return is through her door's spot; the latch;
C3-C6. Work still leaves by `Route::Around` (`go_to_work(Option<Link>)` stays), and stays green.

**Sites.** scenes.rs:235-370; mind.rs:200-232, 1473ff, 1540; osaka.rs:676 (`Act::Door`), 1138, 2740/2750, 2770,
3491-3570 (`cut`'s `to_job`), 4013-4030 (`Act::Out`'s `leaving` arm, unchanged but re-read), 4062-4092, 4405-4411,
4636ff (`start_job`), 5266-5299 (`go_out`, `gone_out` comment), 7065-7082, 7213, 7371, 8026-8062, 8611, 9079-9230
(every `Act::Door` construction, `errand`), 9334-9426 (`evict`), 12047; the osaka.rs tests 15650, 15866, 15887,
16151 (via `DoorSpot::at`); mod.rs:2353, 2403; stage.rs (`Scene::School`, `Scene::ALL`), tests.rs:1762, 9128, 9187,
9240.

**Tests first** (osaka.rs unit tests unless noted; both modes where drawn):
1. `out_from_her_bed_she_walks_to_her_door` (tests/away.rs; step 3's test 1 extended, same cues and precondition):
   she gets up, says one OFF line, walks, and the visit's open door (`Through::Home`) meets no shown cover. **Red
   today:** the door opens at her seat.
2. `a_leave_walk_across_floors_never_ends_the_visit_before_her_door` (M1; a layout with no route, and one with an
   `Around` route; both modes): she is `Visiting` until her external door's beat 6. **Red** against a build that
   sets `leaving` at set-off (the mutant the design first proposed).
3. `her_line_as_she_sets_off_is_said_once` (two floors, `home_at`; guard): precondition that `go_out` was
   re-entered at least twice (a hop landing or a door in space); OFF said exactly once (T11).
4. `a_later_boundary_never_cuts_her_way_out` (T11): a direct `cut()` unit test on a constructed `Walk { then:
   Job(Leave) }` (guard; the structural `to_job` match makes it so), plus a dash's way out crossing 12:45 through
   the honest trigger: she reaches her spot after 12:45 and stays in, silently (decided; Open choice 5).
5. `a_resize_mid_walk_moves_where_she_goes_out` (M8; `rooms()`, shrink the width so the right wall moves;
   precondition old spot != new spot): the door opens at the new spot.
6. `her_door_follows_a_resize_mid_gap` (C5; `rooms()`, T20, precondition that the spot moves): she comes back out of
   the new spot; with `chances.door` `None` mid-gap the act becomes `Through::Space` at her feet.
7. `she_comes_in_facing_the_room` (C6): Return and Dash.
8. `evicted_on_her_way_out_she_walks_on` (F21): a focused pane not over her leaves the walk; one over her moves her
   and the reflex re-enters silently; an opened door evicted becomes `Through::Space` keeping its gap.
9. `a_school_scene_comes_back_after_its_gap` (D10; stage): visiting throughout, out through the space and in with
   "I'm home!".
10. Day-long property (b) (assertions on the step-1/3 functions): while visiting, an open `Through::Home` door meets
    no `visit.shown` cover.
- Migrate: tests/away.rs:125 (`spot == from` becomes the space spot), :1014 (`rained_out_on_her_way_to_school`: the
  door at the space), `her_routine_leaves_what_runs_its_course` (osaka.rs:12038).

**Exit.** The gate; goldens `school_morning`, `dash_home` and any the trace diff shows re-recorded ("her walk to her
door"). **CHANGELOG:** "Changed: Osaka gets up and walks to her door to leave for school." **Docs:** design.md
:2209-2212 (she walks to her door, getting up as after any use); decisions.md: the latch split (why `leaving` stays
at the door).
**Traps.** `census_purpose` checks `heading` first (osaka.rs:7064-7082): give Leave "routine" or the walk counts as
"to seat". `Decision::reflex("routine/away")` must stay the method name. The errand retarget (osaka.rs:9198ff) is
today's behaviour and stays. Never set `leaving` before her external door opens.

### Step 4b. Work through her door (~600 lines)

**Goal.** D6's work part: work goes out and comes home through her door.

**Sites.** mind.rs:464-478 (`mind::work`); osaka.rs:928-935 (`Shift::Back` removed), 3521-3526/3552
(`school_from_work` rewrites `why`), 4024-4029, 7289-7301, 7702-7704, 9408-9419, 9463-9545 (`go_to_work`,
`back_home`, `cut_shift`); stage.rs:607-637 (`Scene::Work`); tests.rs:3821/3838, away.rs:985/996.

**Tests first.**
1. `work_goes_out_by_her_door` (`sat(11,0)`, `tests.rs:3821 out_at_work` rewritten): `go_to_work` walks to the
   space; the door's gap is the shift's; she comes home out of it facing the room carrying her leeks through beats
   10-12 (C13). Rewrite `a_shift_by_the_edge_ends_at_home` (osaka.rs:13497) as
   `a_shift_through_her_door_ends_at_home`; `walking_out_to_work_as_school_begins_she_walks_on_to_school` (13561)
   asserts `gap == u64::MAX` at the door (M9); tests/away.rs:985 follows.
2. `evicted_on_her_way_to_work_she_keeps_her_shift` (C4): a focused pane over her mid-Leave-walk with `why: Work`:
   the door's gap is a shift's, never 0.
3. `evicted_at_her_door_on_her_way_to_work_she_comes_home_by_it` (M10), and `focusing_her_doors_pane_in_a_work_gap
   _never_moves_it` (the door stays at the space, hidden; no face-on door appears at the fallback).
4. `a_lamp_in_the_space_is_bumped_into_coming_home_from_work` (M13; asserts `visit.bumped` at `there` only for
   `Fallback::Yield`; the feeling itself lands in step 6).
- Migrate: tests.rs:3838 and away.rs:996 (`go_to_work(None, ..)` → `go_to_work(chances.door, ..)`).

**Exit.** The gate; goldens `weekend` and the `UNFED_*` tables that go to work, as the trace diff shows ("work
through her door"; golden `stage_room` cues no Work, golden.rs:209-219: let the diff decide, T23). **CHANGELOG:**
"Changed: Osaka leaves for work through her door too; she no longer walks off the screen's edge." **Docs:**
design.md:1762-1765 (work "out at a screen edge if her floor reaches one, else through her door") → "through her
door"; :2232-2234 reconciled ("Everything of hers that comes and goes by her routine **or work** uses her door").
decisions.md: "Work goes out by her door (2026-10-08)" (one external door reads clearly; the edge exit looked like
leaving through the wall beside it).
**Traps.** `Shift::Going` + a landing used to cut the shift (7296-7300): resume it. `school_from_work`'s door is now
at the space; it rewrites the Leave's `why`. Want for the heading: `Want::Work`.

### Step 5. Keep-out for new placements; makeshift never in the chat (~700 lines)

**Goal.** D3. No new piece is ever put in the chat or a space, in both modes.

**Sites.** room.rs `Keep`, `admits` 1617, `spot` 1445, `doorstep` 1517, `move_off`; rules.rs `Frame` 404/428 and its
ten literals, `search`/`evaluate` (one `keep.refuses` per candidate); mod.rs `builds` 3922 (called 2341),
`made_stands` 3837 at both callers (3798, 3313), `tend_made` 3791, `mend` 3553 (hash 3579-3581), `place_gift` 3631.

**Tests first.**
1. room.rs proptest `no_placement_meets_the_keep` (`chat_over`-style layouts with a chat meeting part of a nook, and
   a door): `doorstep`, `spot`, `move_off` and every `rules::search` repair put nothing whose cover meets the chat
   or a space; a delivery never makes the space yield. Red: `admits` checks neither.
2. `nothing_is_made_in_the_chat` (tests/window.rs beside `nothing_is_made_under_her_window` 389, which calls
   `builds(&real, visit, &[pull], &[])` directly): a pull on a chat floor builds nothing. Red: `clear` checks only
   protected and real pieces.
3. `a_made_piece_the_chat_comes_to_meet_falls_apart` (the symmetric path; both `made_stands` callers).
4. New property function `long_visits_with_the_chat_over_a_nook_never_touch_what_is_protected` (T5, T6; no made
   piece meets `view.chat`).
5. `a_parcel_is_never_left_on_a_doorstep_in_the_chat` (overlapping layout: a Users rect meeting the chat, as
   ui/app.rs:4003's grid overlap).

**Exit.** The gate; goldens re-recorded only where a make or delivery moved (expect `stage_room` makes,
`furnished`); `every_made_piece_is_used_or_let_go` unchanged. **CHANGELOG:** "Changed: Osaka no longer builds
furniture out of chat text in the chat pane." **Docs:** design.md makeshift and delivery paragraphs (:1784-1800)
and *Strips and anchors*: "no piece in the chat pane"; decisions.md: "No furniture in the chat pane (2026-10-08)"
(the user's quote; why not `blocked`).
**Traps.** `view.chat`, never `Chances.chat` (resident-only, osaka.rs:50-52). `IdleView::default()` has an empty
chat: a test built on it proves nothing.

### Step 6. Felt rules `DoorClear` and `InChat`; the per-frame closet (~1000 lines)

**Goal.** D7.

**Sites.** rules.rs:19-189, 193-277, 466-600, 1309; osaka.rs:1655 (the grievance pattern), 1944-1950, 8115-8130,
8208-8212, 8295-8320, 8430, 9487-9570 (arrival lines), 10824-10833; room.rs `stranded`, `Home::frame`; mod.rs
`furnish` 3366-3401 (trigger), `mend` 3553; the guest's per-row said-day.

**Tests first.** (Every test forces the mood; Lazy never mends, brain.rs:209 `home_acts`, T10.)
- **Carried from step 2:** step 2 test 6's DoorClear clause (a crowded edge strip is still chosen, its space
  yields, and DoorClear is broken, M5), deferred here; and decide how DoorClear blames a space that yields for a
  hung piece (a window the narrowed floor would leave nowhere), which the rule as specified (floor pieces with no
  use) can't name.
1. `a_lamp_in_her_door_space_is_felt_and_moved` (tests.rs, `rule_home` 9790 pattern, an older record with `door`
   set; Industrious): the door's strip holds pieces whose widths sum to **raw−5 ..= raw−3** (packs raw, not
   narrowed, kept once the 3-wide lamp leaves), the lamp at `Anchor{Right,0}` and its neighbour a settled no-use
   piece **outside** the would-be space (or the test asserts which piece moves), and a second strip with room for the
   lamp. Preconditions before the arrival: `!space.kept`, the lamp's DoorClear broken, the arrival's spot
   `Set::Floor(Yield)`. She comes home through it, feels DoorClear, says "Can't get to the door!" **after** "I'm
   home!", lifts the lamp elsewhere; the next frame `space.kept` and her door is `Set::Wall` at the space (face-on
   until step 8, F25). Red: no rule.
2. `a_sofa_in_her_door_space_is_felt_on_use` (ANY_USE path; Industrious).
3. `a_piece_in_the_chat_is_moved_out` (overlapping layout; Industrious): InChat felt, repaired outside the chat.
4. `a_piece_in_the_chat_with_nowhere_else_is_closeted_never_lost` (both modes; Visiting and Away): hidden while
   nothing fits, back when room appears; the record unchanged. Red: shown in the chat today.
5. `stranded_iff_no_room_takes_it` (T25, proptest over overlapping layouts): with one chat piece, stranded iff a
   brute-force search over strips and lefts finds no outside-the-chat placement; with several, `frame` never hides a
   piece that isn't stranded and every hidden piece has no placement given the others.
6. `on_a_short_terminal_she_never_feels_her_door_blocked` (M7, C5; H 18-22 on `real_frame`, both modes): nothing
   DoorClear-broken, no line, no repair.
7. `a_lazy_osaka_lives_with_it` (Lazy: the space keeps yielding, a chat piece keeps showing (M20), nothing lost, and
   no on-sight line is said).
8. `her_grievance_never_talks_over_her_hello` (C4): no frame shows two of her lines; the line shows whole after the
   arrival line; a `Protected` fallback arrival says nothing (no key felt).
9. `an_on_sight_line_is_said_once_a_day` (C6): a day-long run (Ordinary) says each row's line at most once per game
   day while still feeling it every visit.
10. `every_repair_mends_and_breaks_nothing` (rules.rs:2281): DoorClear and InChat keys mended, none broken;
    `repair_search_is_cheap` still green; step 2's test 8 re-run.

**Exit.** The gate; goldens move only where a rule is newly broken (expect none in the bundled fixtures: say so, or
explain each). **CHANGELOG:** "Added: Osaka moves furniture out of her door's way, or out of the chat pane, herself
(\"Can't get to the door!\")." **Docs:** design.md phase-4 rules list (the two rows, their trigger and when she says
it); decisions.md: the trigger (why on sight, why only when she'd mend, why once a day) and the per-frame closet
(why not persisted).
**Traps.** New rows **at the end**. DoorClear judges the **would-be** space (`space.rect`, kept or not, where a
`Space` exists), or it is never broken while yielding, the only time it matters. Evaluate on the scratch home's
`LaidOut`. `Felt.on` change ripples to `just_set` and the stage `feel`.

### Step 7. Art infrastructure (graphics.rs, art.rs, art/wall-door.svg, sprite.rs only; ~650 lines)

**Goal.** D8's looks and compositing, unwired: `Look::WallDoor` with crop and sky, `WallDoor::Plate(u8)`, `Cut`
(clip and `dx`) with `paint_cuts` and `paint_layers` as its wrapper, the cut's bounds clipped at the wall, `lean` in
production art, `wall_door_cells(state, side)` and `mirror_wall` in sprite.rs (`mirror` unchanged), per-sky
gradients for `wd-beyond`/`wd-flap-hole` and the hedge's tint. **No edit outside these four files**; every new item
unconstructed outside tests carries `#[cfg_attr(not(test), allow(dead_code))]` or clippy's `-D warnings` fails.

**Tests first.** `a_clipped_layer_paints_nothing_past_the_wall` (graphics.rs; both sides; red: no clip; this is
where pixels are asserted, T22); `the_wall_door_crops_to_its_inked_columns` (every state × both walls × every
`Sky`, against `the_wall_door_keeps_to_its_columns` art.rs:3869's table); `the_wall_door_is_ascii_and_keeps_to_its_
columns` (sprite.rs:780) extended to the left wall (red: `[`/`]` unmirrored); `her_sprites_mirror_as_before`
(guard: `mirror` unchanged); `lean_rests_the_flap_on_the_parcel` (monotone in lead; the sheet's values);
`the_hedge_darkens_with_the_sky`; `wall_door_sheet` (art.rs:3430, ignored) regenerated with the skies.

**Exit.** The gate; no golden moves (nothing wired); `image_census` unchanged. No CHANGELOG (invisible).
**Traps.** `Look` is in the cache `Key` (graphics.rs:154): keep the new look `Copy + Hash + Eq`. The wall line's
pixel column is `w*cw + (cw-t)/2` from `LineGeometry` (graphics.rs:179), not the sheet's `line_x`.

### Step 8. The side-on door, wired (~1300 lines)

**Goal.** D8: the external door is drawn side-on in the wall in every path; beats with her walking through at her
pace, clipped, `Post` over her; the Away cue; the doorway's sky; when it shows. Checked by eye with step 4a's
`Scene::School`.

**Sites.** Remove every `cfg_attr(not(test), allow(dead_code))` on the wall-door items (art.rs:1119, 1220;
sprite.rs:478; step 7's). mod.rs:300-470 (`Placement`, `Door`, `Figure`), 2437-2444, 3016-3115 (`draw`), 4482-4598
(`draw_art`), 4657-4871 (`paint_empty`, `draw_door`), 2805 (`door_rain`), 4059-4089 (`piece_state`'s sky source);
osaka.rs 1230-1360 (`wall_beat`, the `Through::Home` duration override for beats 2 and 10), 2861 (`first_due`),
5296 (`gone_out`: unchanged, its test 13588 run both modes), 9056-9075 (`hidden`, `door`: unchanged), 9857-9861
(pose); tests.rs:549-652 (`Hidden::check`, `open_flap` → `open_wall`), :818 (feet).

**Tests first.**
1. `her_door_shows_side_on_in_the_wall` (tests/away.rs; both modes, both walls via a left-edge layout): the drawn
   cells are the state's columns at `w`; the wall glyph is restored by the rain. Red: face-on `Look::Door`.
2. `going_out_she_is_clipped_at_the_wall` (both modes): her placed image's **cell bounds** never pass `w` in beats 2
   and 10 (pixels are step 7's test, T22); `Post` drawn after her while `d > 0`; she moves a column per `WALK_MS`.
3. `while_shes_out_her_slippers_stand_before_it` (line art: `Shut{away: true}` in Away, the cold start and gap beats
   6-7; `Ajar` at 8, `Open` at 9, no slippers from 8; never while she's in), plus `a_school_exit_still_ends_the_
   visit` (`gone_out` is `Some` at beat 6 while the cue is drawn, both modes; away.rs:125 both modes, F5).
4. `her_door_never_pops_beside_her` (C3): no frame in which the external door appears or goes while her box meets
   the space, except at her set-off line.
5. `the_doorway_shows_the_windows_sky` (`Sky::at(time_of_day)`, unfed → `Day`).
6. `Hidden::check` exempts only the door's wall-column cells (ASCII `|` etc. over `│`), its image cells over lines,
   and its image over text in passing during a visit's beats (C9); the slipper columns
   are checked (never over text); the door's own columns stand over text while she's out (the user's answer).
7. `she_turns_to_the_viewer_once_coming_in` (C12): `Side` held from beat 10's last step through 12; the next pose
   change is the arrival line's `Stand`.
8. `a_goodbye_while_she_is_out_shows_nothing_of_her` (tests.rs:3634) and tests/window.rs:33/240 move to the new look.

**Exit.** The gate and the perf tests; goldens `school_morning`, `home_from_school`, `dash_home`, `weekend` and
whatever else the trace diff shows re-recorded ("the side-on door: its cells, her steps, the Away cue");
`Placement`'s Debug unchanged (golden.rs:56), so scenes without her external door stay identical; `image_census`
(census.rs:1339) re-run, `CACHE_LIMIT` (graphics.rs:61) re-checked against the new looks (sky × state × crop, her
clipped poses). Stillness below. **CHANGELOG:** "Added: Osaka's front door is drawn side-on in the wall at the
screen's edge where there's room; she walks through it, and while she's out her slippers wait before it with a card
on the knob." **Docs:** design.md *Away at school* (the cue, the side-on door, the doorway's sky, when it shows);
decisions.md: "Her door is drawn side-on (2026-10-08)" (A read as a post; B approved; why the clip; why walking
pace). snippets.md's beats note ("the slippers vanish as she steps into them") corrected in step 10.
**Traps.** A non-edge (inner) wall: her doorway box reaches the neighbouring pane; clip `bounds()` or `key`
(graphics.rs:568-600) refuses the whole image and she vanishes. `put` over the wall column must go through
`draw_flap`'s rule. Text under the door for hours: the slippers' columns only over calm cells. `first_due` needs
each step or nothing repaints mid-beat. Never make `Osaka::door()` return the wall door for beat 6.

### Step 9. Parcels through her door's flap (~750 lines)

**Goal.** D9.

**Sites.** room.rs:652 (`Flap`), 1517-1611 (`doorstep`, walls 1529-1533, `Flap` 1595-1607); mod.rs:500, 1684-1686,
1972-1975, 3322/3345/3352 (delivery sites and their wait), 3998/4094-4105 (parcel look/layer), 4138-4180
(`FLAP_MS`, `draw_flap`), 2470-2477 (flap draw); tests.rs:642, 4971.

**Tests first.**
1. `a_parcel_comes_in_through_her_door` (rewrite of tests.rs:4971
   `a_parcel_comes_in_through_a_flap_at_the_screens_edge`): the parcel rests with its trailing edge at `w-7`;
   during 0-650 ms the door's flap is open and the parcel slides (cells never beyond the wall line); after,
   `Shut{flap: 0}`. Both modes. Red: `draw_flap` glyphs.
2. `a_parcel_with_her_door_space_yielding_comes_through_another_wall` (`draw_flap` fallback).
3. `no_parcel_slides_through_her_or_her_slippers` (C10): a delivery due during her door's act, a work gap, or with
   her in the space waits, then comes.
4. `a_parcel_keeps_her_still` (T12): `Scene::Watch` held over 40 s, a parcel due mid-watch, the drawn stillness judge
   (`judge`, stillness.rs:893) on the paints passes under Open choice 3's answer.
5. `every_parcel_on_her_doorstep_gets_unpacked` and `a_poster_is_delivered_where_it_fits_boxed_and_hung`
   (room.rs:2915, `flap.rows == (6, 8)`) still hold.
6. `the_slide_repaints_at_each_beat` (wakes at 150/350/500/650).

**Exit.** The gate; `stage_room` (Scene::Parcel at 255 s) and `UNFED_STAGE` re-recorded ("parcels through her
door"); `open_flap` callers (tests.rs:830, 2544, 2619, 5507, 5586, 5834, 6057, 6177; tests/clock.rs:561) updated.
**CHANGELOG:** "Added: parcels come in through the flap in Osaka's front door." **Docs:** design.md deliveries
(:1784-1790: "pushed in through a flap in a wall ... `╱`/`╲` for 800 ms" → through her door's flap, resting just
past the space; another wall's flap only when her door can't take it; the wait).
**Traps.** Door, parcel, plate (and her, if in the space) are **one** `paint_cuts` call: two images over the same
cells cut each other out. The plate swung up reaches `w-5`: inside the space, clear of the parcel at `≤ w-7`.

### Step 10. Census, docs, deep runs (docs and test runs; ~200 lines)

- Census (below) recorded in plan.md's Phase 38 record. Deep runs (below) clean, any failure pinned and fixed in
  its own commit first.
- design.md read through for leftovers of "where she left" / feet; brief §5.1's rule text is the final wording
  (`Strips and anchors`, *Away at school*, work, deliveries).
- CLAUDE.md: the door-batch line becomes "implemented", pointing at this design and plan.md's record; snippets.md
  header ("for review, not wired in", `worktree.diff` "against `1e44b050`") marked stale-fixed (committed in
  `8eaa25c1`, wired by steps 7-9), and its beats note on the slippers corrected (they go at beat 8).
- plan.md Phase 38: the batch's record (what landed per step, the census numbers, deviations).

## Goldens

- Which move: step 2 every fed golden with a home and the `UNFED_*` with a home (layout push + saved `door`);
  step 3 the three day scenes; step 4a the school/dash scenes; step 4b `weekend` and unfed work; step 5
  makes/deliveries; step 8 every scene with her external door; step 9 `stage_room` parcel. Steps 1 and 7 move nothing
  (asserted). Every other prediction is the trace diff's to make (T23). The insta snapshots (`snapshots/`,
  tests.rs:1051-1069) read terrain only; `osaka_at_home_seed_7` (no home) moves only if the no-home door shows in its
  95 s: re-accept with the reason.
- Procedure (docs/testing-strategy.md:776-819): run the old and the new revision with
  `HOUSEGUEST_GOLDEN_TRACE=<dir>` (old revision in a scratchpad `git worktree` with its own `CARGO_TARGET_DIR`),
  diff `<scene>-<seed>-<graphics>[-unfed].txt`; the commit says where each moved trace first differs and why, and
  that the rest are byte-identical. Never re-record a table without that diff.

## Census, band and stillness

- **What moves.** The walk to her door adds one set-off and a few seconds of walking (`Body::Walk`, purpose
  "routine") per school morning and per dash's way out, and replaces work's edge walk with a walk to the door
  (similar length). `Act::Door` counts as `Moves::Off(Body::Door)` whatever is drawn (osaka.rs:887), so the steps
  through the doorway don't change the census; door set-offs are counted at tests/census.rs:844-859
  (`if body == Body::Door { doors += n }`, T12, F18). tests/stillness.rs has no door handling: the drawn stillness
  tests (stillness.rs:989/1027) leave out acts that take her somewhere, and never see a parcel (an order arrives on
  a later visit, mod.rs:3321): step 9's test 4 covers the slide. The band and stillness gates run afternoons
  (`AFTERNOON` Tue 13:00, census.rs:1588) and **never see a school morning**; they move only through the 6-column
  push in `furnished_room()`/`resident_room()` (home_screen, 100×20: rows 7, so the space is kept, on Users' left
  wall).
- **Measure** after step 2 and again after steps 4a and 4b: `cargo nextest run -p dessplay --run-ignored all -E
  'test(/houseguest::tests::band::/)'`; `cargo test --release -p dessplay --lib day_census -- --ignored
  --nocapture` (census.rs:2310: set-offs and moving share for the 08:00-08:20 and dash rows, before/after, into
  plan.md's record); `visit_census` (census.rs:1107) both modes.
- **Bands that may move:** none of the band thresholds or set-off caps change in this batch. The day census's
  morning moving share and set-offs per minute rise (expected, recorded, not gated). If a gated band test fails
  after step 2 or 4, that is a finding for the user (the push changed her trips), not a retune.

## Deep runs (step 10)

- `PROPTEST_CASES=256 cargo nextest run --release --profile deep -p dessplay houseguest` (read nextest's own exit
  code); `her_days_never_touch_what_is_protected` and `her_school_mornings_never_put_her_door_on_a_piece` can carry
  more cases.
- `CENSUS_BAND_SEEDS=<n> cargo nextest run --release --profile band --run-ignored all -p dessplay
  -E 'test(/houseguest::tests::band::/)'`.
- `cargo nextest run --workspace --all-targets` (cross-crate breakage) and `changelog::tests::embedded_changelog_parses`.
- No protocol change: `stable` is not advanced.

## Docs that change (by step)

| Step | design.md | decisions.md |
|---|---|---|
| 2 | *Strips and anchors* (1813-1830): the space | "Her door has its own space at the screen's edge (2026-10-08)", with the older-build anchor note |
| 3 | 1380-1382 (doors in space only); *Away at school* 2209-2221 (space, strict text-blind fallback); 2390-2399 kept | pointer from 3334/3350: superseded in *where* |
| 4a | 2209-2212 (she gets up and walks to it) | the latch split |
| 4b | work 1762-1765; 2232-2234 | "Work goes out by her door (2026-10-08)" |
| 5 | deliveries/makeshift 1784-1800; *Strips and anchors*: no piece in the chat | "No furniture in the chat pane (2026-10-08)" |
| 6 | phase-4 rules: DoorClear, InChat, their trigger and line, the per-frame closet | why on sight, when said; why not persisted |
| 8 | *Away at school*: side-on, the cue, the sky, when it shows | "Her door is drawn side-on (2026-10-08)" |
| 9 | deliveries 1784-1790: through her door's flap, the wait | (with step 8's entry) |
| 10 | read-through; CLAUDE.md line; plan.md record; snippets.md header and beats note | — |

## Not in this batch

- Doors in space and `door_away` (mind.rs:440, picks an x with no check): unchanged (settled fact 4).
- The errand's arrival at the accordion: unchanged.
- The morning routine (kitchen, bathroom) that would end in `Job::Leave`: plan.md "Later".

## Open choices left

None blocks steps 1-2. Each has a recommendation that the steps build unless the user says otherwise; the step that
first depends on it is named.
1. **Answered (b): the door stands over text.** Was: **Text at the wall hides the Away cue** (C8; before step 3's commit, informed by its calm-cells report). The
   bundled stylesheet right-aligns playlist text into the pane's last interior column (`style.css:36-37`), which is
   the shut door's second column when her pieces are on Playlist; the cue would be hidden much of the time she's
   out. Options: (a) the chooser prefers, among edge walls, one whose space's inner column is calm; (b) while she's
   out, the shut door's own two columns may stand over text (slippers still cropped), as the flap already stands in
   the wall column. **Recommendation: (b)**: it keeps the chooser free of text (text never moves the door) and keeps
   the one sign the user asked for; it costs one column of the playlist's right-aligned counts in the bottom four
   rows while she's out.
2. **Answered: walking pace.** Was: **Walking pace through the doorway** (C11; step 8). The approved table steps her through in thirds (about three
   times her walking speed); the design slows beats 2 and 10 for her external door to `4 × WALK_MS` so she doesn't
   lurch after a calm walk. **Recommendation:** walking pace (the stillness memory: a change of pace draws the eye).
3. **The parcel's slide and stillness** (T12; step 9). Five changes in 650 ms during a held act would break the drawn
   stillness rule (five exemptions today). Options: a sixth exemption for the slide, or the delivery also waits
   until she is walking or between acts. **Recommendation:** the delivery waits (no new exemption; a parcel isn't
   urgent).
4. **A line as she sets off for work** (C21; step 4b). She walks to the same door as for school, silently.
   **Recommendation:** no line (the walk reads; one less voice).
5. **A dash's way out crossing 12:45** (T11; step 4a). **Recommendation (built):** she reaches her door, finds
   school is over, and stays in, silently.
6. **The doorway's sky and hedge** colours (derived from the window's sky stops) on the regenerated sheet (step 7);
   changing them is an SVG edit.
7. **Answered: `"People are talking here..."`.** Was: **The `"Not in the chat..."` line** (C20; step 6) is a placeholder; none of her lines mention panes. Suggested in
   her voice: `"Everyone reads here..."` or `"People are talking here..."`. "Can't get to the door!" is the user's.

## Critique resolutions

M = mechanics, T = tests, F = feasibility, C = character (their files beside this one).

- M1 accepted: `set_off` latch; `leaving` only when her external door opens (D6); test 4a.2.
- M2 accepted: pin on raw, `extents` after pinning, `LaidOut` bundle; step 2 test 5's no-anchor variant.
- M3 accepted: one `Home::wall(plan)` read by `door_place`, `Keep::of`, `doorstep`/`admits`, D9; step 2 test 7.
- M4 accepted: `obstacles` (laid, made, ghost) for the fallback and the space (`Fallback::Blocked`); step 3 test 10.
- M5 accepted: geometry-only qualification, packing as a tie-break among edge walls (D2); step 2 test 6.
- M6 accepted: `move_off` packs on floor and refuses Keep; re-choose only when it moved the door strip's pieces;
  never `None` over a saved wall.
- M7 accepted: `Space` exists only with rows and width; `Fallback::Short`; step 6 test 6.
- M8 accepted: `start_job(Leave)` re-routes on a moved spot; `take_in` converts to `Through::Space` on `None`; step
  4a test 5.
- M9 accepted, via its alternative: `Job::Leave { why }`; `school_from_work` rewrites it; gap from `why`.
- M10 accepted: evict maps `Home` → `Space(calm)` keeping the gap; the return re-reads `chances.door`; step 4b test 3.
- M11 accepted: arrivals and `Chances.door` read gated ground (arrivals wait); a set-off falls back outside the pane
  (`Protected`) rather than at her feet, so the overlap stays unrepresentable; an opened door keeps its spot
  (`take_in` ignores `Protected`) and evict handles it; step 3 test 7, step 4b test 3.
- M12 accepted: sticky `prev` and a text-blind fallback; step 3 test 8.
- M13 accepted: `visit.bumped` from `Fallback::Yield` only, set on arrivals and at `there`; idle arrivals documented.
- M14 accepted: `Scene::School` uses `returning`, no `leaving`/`set_off` (D10).
- M15 accepted: `StripPlan` (and `LaidOut`), the old enum untouched.
- M16 accepted: `doorstep`'s scratch projection uses `with.wall(plan)`; arrival's `unsaved |= changed`; tests keep
  `project` without a new argument (F2).
- M17 partly rejected: a saved wall whose space meets the chat is refused per frame (`Fallback::Chat`), not
  re-chosen and saved, so a pane drag and back restores it (F10's reasoning); step 2 test 6.
- M18 accepted: `keep.refuses(cover)` once per candidate; `Frame::free` stays the cell test for protected/text.
- M19 accepted: both `made_stands` callers, the delivery's with the after-delivery keep (D3, step 5).
- M20 accepted: joint assignment in `stranded`; the Lazy gap documented and pinned (step 6 test 7).
- M21 accepted: arrivals and `dash_through` build `Through::Home(spot)` (D5).
- M22 accepted: no spot → a door in space at a calm spot outside the chat; step 3 test 13.
- M23 accepted: `DOOR`/`door()`/`gone_out` unchanged; `wall_beat` draw-only (D8).
- M24 accepted: ledger doc and decisions.md note; `at` stays a raw share.
- M25 accepted: `go_to` false → a door in space at her feet, gap MAX.
- M26 accepted: steps 2 tests 0 and 5 assert the `DoorWall`.
- M27 accepted, second option: a window shifted into the chat is judged InChat (keeps the chat out of the showing
  pass).
- T1 accepted: test 5 on `rooms(100,30)`; `home_screen`'s wall is Users' left, recorded; window.rs/away.rs
  migrations listed.
- T2 accepted: Sleep/Lounge/Nap/Homework, 08:12, the in-piece precondition, red on today's API, an independent
  expected spot.
- T3 accepted: per-screen expected states and non-vacuity counters (Tests).
- T4 accepted: `her_school_mornings_never_put_her_door_on_a_piece` (step 3 test 2).
- T5 accepted: no strategy changes; new proptest functions (Tests, step 1).
- T6 accepted: three constructed chat layouts and an overlapping property (step 1 fixtures, step 3 test 6, step 5
  test 4).
- T7 accepted: `real_frame` at H 18-22 with the precondition (step 3 test 5).
- T8 accepted: strict `door_shows`.
- T9 accepted, both ways: `Set::Wall` refuses obstacles, and the space half of makeshift keep-out moves to step 3.
- T10 accepted: width window raw−5..=raw−3, the neighbour controlled, preconditions, moods forced.
- T11 accepted: guards labelled; re-entry precondition; `to_job` structural; the 12:45 trigger decided (Open choice
  5).
- T12 accepted: refs fixed; step 9 test 4; the exemption question is Open choice 3.
- T13 accepted: resident `view()` users listed; `long_visit_of` takes a frame provider.
- T14 accepted: an independent rect oracle, both sides of both strips.
- T15 accepted: replaced by the anchor-invariance of `kept`.
- T16 accepted: replaced by the iff-window geometry test.
- T17 accepted, adjusted: a share-only piece added; its expected left follows pin-on-raw (M2), not a narrowed share;
  the chooser now runs before `project` (F22), so no first-frame difference.
- T18 accepted: a left-edge-only layout; the edge test reads the nook rect (Geometry words).
- T19 accepted: moved to step 2 as a proptest, re-run in step 6.
- T20 accepted: `rooms()` with the moves-precondition.
- T21 accepted: `osaka.place` at 08:14:59 with the precondition.
- T22 accepted: cell bounds at frame level, pixels in step 7.
- T23 accepted: the `stage_room` Work claim dropped; the diff decides.
- T24 accepted: the workspace gate at every step; ~3 s budget per new long-visit proptest.
- T25 accepted: `stranded_iff_no_room_takes_it` (step 6 test 5).
- F1 accepted: `StripPlan`/`LaidOut`.
- F2 accepted, option (a): `settle_door`/`frame` at the production sites; `project` only reads `door`.
- F3 accepted: with (a), `project` keeps its signature (`project_with` internal); `doorstep`/`spot` take `plan`;
  every production caller listed in D2.
- F4 accepted: sequential (or git worktree); `mirror_wall`, `mirror` unchanged.
- F5 accepted: same as M23/C1; step 8 test 3's school-exit check.
- F6 accepted: `returning`, `Scene::ALL` 58, the stage tests listed (D10).
- F7 accepted: per-candidate-strip floors from a scratch home with the strip changed (D1).
- F8 accepted: the flap's x from `raw`; room.rs:2915 asserts it.
- F9 accepted: the workspace gate; `Summary.door: Option<String>` skipped when `None`.
- F10 accepted: `Fallback::Chat` per frame, unsaved.
- F11 accepted: `Ground` over terrains already built; fallback search only on failure; perf at step 3's exit.
- F12 accepted: steps 4a and 4b.
- F13 accepted: gap from `Leave`'s `why`; `Want::Walk` (credit `None`) for school, `Want::Work` for work.
- F14 accepted: same as M6; hidden pane and back keeps the wall.
- F15 accepted: `#[cfg(test)] DoorSpot::at`.
- F16 accepted: osaka.rs:26 in step 3; `dash_through` in step 3 only.
- F17 accepted: the `strips`/`broken` callers added (D1, D7).
- F18 accepted: census.rs:844-859 cited; stillness has no door handling, said.
- F19 accepted: `PARCEL` is production; only `lean` moves.
- F20 accepted: lines given; stage.rs:533 described correctly; `Act::Door` sites listed; 2390-2399; `paint_cuts`.
- F21 accepted: evict comment and step 4a test 8.
- F22 accepted: the order is settle → pin (raw) → `move_off` → re-settle.
- F23 accepted: no `master` push mid-batch.
- F24 accepted: step 2's CHANGELOG reworded.
- F25 accepted: "`Set::Wall` at the space (face-on until step 8)".
- C1 accepted in effect: `gone_out` keeps reading the face-on table's beat 6; drawing is `wall_beat`'s alone.
- C2 accepted: beats 6-7 `Shut{away}`, 8 `Ajar`, 9 `Open`; the slippers go at 8; snippets.md corrected in step 10.
- C3 accepted: shown from set-off until her box leaves the space; step 8 test 4.
- C4 accepted: said after the arrival line, when quiet, `GRIEVANCE_MS`, under `grumbling()`, only if felt.
- C5 accepted: `Space` only where it can exist; `Fallback::Short` never sets `bumped`.
- C6 accepted, both limits: said only when she'd mend and at most once a game day per row.
- C7 accepted: the fallback is text-blind and sticky.
- C8 → Open choice 1; **the user chose (b)**: the door stands over text.
- C9 accepted: the door drawn over text in passing in a visit's beats; hidden only while she's out or in a gap.
- C10 accepted: deliveries wait (D9); step 9 test 3.
- C11 accepted as the default, confirmed by Open choice 2 (it changes an approved table's timing).
- C12 accepted: `Side` held through beat 12.
- C13 accepted: `Carry` in beats 10-12 with a shift.
- C14 accepted: face the chat only when already at her spot.
- C15 rejected: with no route she travels between floors by a door in space as every walk does, then goes out by
  her own door; going out at her feet would open the door in the piece she may be standing in, and the user kept
  "a door in space where she stands" for the case with no door place at all.
- C16 accepted: the ordinary end of a use (Decided list).
- C17 accepted: the face-on fallback is the closed `Look::Door`, also through a work gap; CHANGELOG "where there's
  room".
- C18 accepted: the hedge tinted per `Sky` (step 7).
- C19 accepted: ASCII hides her at `d >= 4`.
- C20 → Open choice 7, with suggested wordings.
- C21 → Open choice 4, recommendation no line.
