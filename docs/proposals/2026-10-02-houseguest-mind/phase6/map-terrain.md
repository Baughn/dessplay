# Phase 6 code map: strips, rooms, terrain, climbing, line art

Last updated: 2026-10-10 (tree at `60727649`, after the door batch).

A read-only map for phase 6 (structures: shelf, partition, stepladder),
against the proposal's *Structures from the UI's own lines*, *Rooms from
contents*, *Rules, not an objective*, body item 6, and the migration and
testing rows 6. All paths are under `dessplay/src/ui/houseguest/` unless
given in full. "Guess" marks an inference not checked by running code.

## 0. The proposal's line numbers, now

| Proposal says | Now | Note |
|---|---|---|
| arrival read mod.rs:556 | mod.rs:2515 (`paint_state`, `State::Arriving`) | `Terrain::read`, before `home.frame` (2534) |
| visit read mod.rs:754 | mod.rs:2842 (`Terrain::read_guarded`) | after validate (2747), `furnish` (~2765), ops, `let_go` |
| errand read mod.rs:1014 | mod.rs:3446 (`Guest::send`) | runs from `nudge_due` at the top of `paint_state` (2508), before the state match |
| goodbye read mod.rs:585 | mod.rs:2649 (`State::Leaving`) | |
| (none: Away didn't exist) | mod.rs:5716 and 5727 (`paint_empty`) | her empty home while she's out: `ground` for her door (unkept), `terrain` for the view |
| (none) | mod.rs:4120 (`furnish`, the delivery's lazy `fresh` terrain) | read from `buf` mid-furnish, for a parcel's seats |
| settle's Climb arm osaka.rs:2308-2312 | osaka.rs:10169-10173 | |
| Climb conditions terrain.rs:511-532 | terrain.rs:549-570 (`find_links` from 518) | |
| strokes doc graphics.rs:181-183 | graphics.rs:307-309 (`strokes` at 309) | |
| `seats_of` mod.rs:1414-1436 | gone as the watch special case; `Faces` is room.rs:2705 `faces` | |

mind.rs:1980 (`two_floors`) and every `Terrain::read` in scenes.rs and
terrain.rs are test helpers.

## 1. Strips and anchors

| Item | File:line | Note |
|---|---|---|
| `Strip` | room.rs:629-633 | **only `Bottom(Nook)`**; no `Shelf`. Derives `Copy, Hash, Serialize, Deserialize` (saved in the ledger's `anchors` and `door`). |
| `Nook` | room.rs:400 | fixed enum: `List`, `Users`, `Playlist`. The chat pane is never a nook. |
| `Side`, `Anchor { side, offset }` | room.rs:636-648 | offset from the strip's wall on `side`; a resize keeps that distance. |
| `Flap` | room.rs:654-661 | delivery flap in a strip's wall (`door`: her door's flap). |
| `Prop { item, strip, anchor: Option, at, facing, boxed, settled }` | room.rs:664-680 | `Prop::new` (721-733) hard-codes `Strip::Bottom(nook)`. |
| `Lane { Floor, Wall }`, `lift`, `Prop::lane/needs` | room.rs:690-718 | a hung piece is `Wall` once unboxed; `needs` = footprint rows + hang. |
| `Extent { from, to, floor, rows }` | room.rs:741-799 | `holds`, `left(anchor)`, `share(at)`, `pin(left)`. `rows` is the pane's height − 2 (geometry, not text). |
| `raw_strips(nooks)` | room.rs:805-820 | **one strip per nook, from nook rects alone** (no `Home`). A shelf's extent would need the ledger, so either its signature changes or a second source is added. |
| `SPACE` = 6, `DoorWall { strip, side }` | room.rs:823, 827-830 | |
| `StripPlan { strip, raw, floor, space }`, `along(lane)`, `pin` | room.rs:837-868 | `floor` = raw less her door's kept space; `Wall` lane packs on `raw`. |
| `Space { side, wall, rect, kept }` | room.rs:875-880 | |
| `LaidOut`, `StripLaid` | room.rs:884-900 | `StripLaid` records what a strip's layout reads (`lays_from` + door) so `relaid` lays out only touched strips. |
| `Plan { nooks, chat, screen }` | room.rs:910-915 | the frame geometry door/keep code reads. |
| closet: `stranded`, `closetable`, `placements`, `on_own_strip`, `first_clear`, `packed_on` | room.rs:943-1100 | pieces in the chat no room takes; iterate `laid.plans` (every strip). |
| `at_edge`, `narrowed`, `space_of`, `space_rect` | room.rs:1207-1262 | `space_of` needs `rows >= HEIGHT` and width ≥ SPACE + widest. |
| `choose` / `choose_wall` | room.rs:1274-1321 | **`plan.nooks.iter().zip(raw_strips(plan.nooks))`**: index-aligned nook↔strip. Adding strips to `raw_strips` silently misaligns this zip (and `doorstep`'s, room.rs:2429). |
| `order_key(anchor, index)` | room.rs:1328-1334 | left-anchored by offset, then right-anchored far-first; ties: newer nearer its wall. |
| `pack(pieces, extent)` | room.rs:1342-1371 | anchor order, collisions pushed inward, never reordered; only the extent's two ends are walls. |
| `pack_each` | room.rs:1378-1400 | for the wall lane: each that fits beside those before. |
| `Shown` | room.rs:~1402-1420 | `strip: Option<Strip>` (None = makeshift); `cover()` (1477) = rect + floor row for Floor lane. |
| `Home { props, door: Option<DoorWall> }` | room.rs:1631-1637 | |
| `Home::rooms` | room.rs:1663-1676 | one room per furnished strip, role from unboxed offers. |
| `Home::extents` → `StripPlan`s | room.rs:1711-1716 | `plan_strip` (1720-1774): the space kept iff laying out with it costs the strip no piece (the door batch's yield rule). |
| `wall`, `settle_door`, `frame` | room.rs:1779-1838 | `frame` = settle door, `project_with`, closet the stranded. |
| `on`, `lays_from`, `strip_door`, `pin_anchors` | room.rs:1841-1880 | |
| `layout` / `laid_out` / `laid_and_shifted` / `strips_laid*` | room.rs:1889-1990 | pure geometry; text doesn't take a piece out. |
| `project_with` | room.rs:2159-2183 | a strip whose floor doesn't pack → `move_off`; then `fits` per piece. |
| `move_off` | room.rs:2197-2296 | **targets every `raw_strips` strip** with the floor that holds them. |
| `spot` (stage gift) | room.rs:2301-2390 | iterates `raw_strips`; `let Strip::Bottom(nook) = strip` at 2321. |
| `doorstep` (delivery) | room.rs:2419-~2560 | iterates `nooks.zip(raw_strips)`; flaps only in nook walls. |
| `faces` | room.rs:2705-2717 | same strip, `FACING_GAP`, facing. |
| `free`, `fits` | room.rs:2822, 2839-2860 | `fits`: body cells blank and `clear`; **floor row cells must be `strokes(c)` glyphs** (unless hung). |
| `Keep { chat, spaces }` | door.rs:382-438 | `refuses` = chat or space; `for_made` → `MadeKeep` (space only, door.rs:441-449); `takes_in(was, now)` = a placement pushes a showing piece newly into a kept-out rect. |

**Hung pieces.** `Spec.hang` (room.rs:169-174): poster `Some(4)`
(room.rs:321), clock `Some(4)` (337), window `Some(1)` (356). They hang
on the raw strip (`StripPlan::along(Wall)`), `pack_each`ed, a low one
(window) kept clear of standing pieces unless `may_overlap`
(room.rs:99-104, window/sofa only). A wall never moves a room. A
crowded wall hangs each that fits. There is no rule for hanging above a
shelf; `clock_on` (mod.rs:213-229) reads the clock's strip's raw extent.

**The door's reserved space.** `Home.door` (saved `DoorWall`), chosen
by `choose` (edge wall first, then her pieces' strips, then kept, then
pane order). Its space is `SPACE` columns by `HEIGHT + 1` rows at the
wall. Kept iff it costs the strip no piece, else the strip lays out
doorless (`plan_strip`). `door::space` (door.rs:285-301) and
`space_rect` read it. `door_place` (door.rs:210-283) stands her door in
the space if `in_space` (door.rs:307-347): kept, no obstacle, out of
the chat, `ledge` under her box's floor row, `wall` (`│`/`┃`) up the wall
column, `stroke` at the wall's foot, nothing protected. Else the strict
fallback: `on_floor` (door.rs:353-367), any spot with a ledge under it,
no obstacle, out of the chat, nothing protected, **no `wall` glyph in
her box above the floor row**.

**What a new strip kind must touch.**
- The enum, plus every irrefutable `let Strip::Bottom(nook) = ...`
  outside tests: ledger.rs:205, 365, 553; rules.rs:433
  (`Repair::label`); mod.rs:1624 (stage `rooms()`), 4341
  (`stage_arrange`); room.rs:2321 (`spot`). `Prop::new` (728) and
  `raw_strips` (810) build `Bottom` only.
- A source of shelf extents (`raw_strips` has no `Home`), and every
  `raw_strips` caller: room.rs:1279, 1712, 1895, 1902, 1915, 1930, 2209,
  2317, 2429; mod.rs:218 (`clock_on`), 234 (`beauty_at`); tests.rs
  `long_visit_of` (1365-1374).
- The two nook-index zips (room.rs:1279, 2429), which must stay
  pane-only.
- A per-kind "may stand on a shelf" filter wherever any strip is a
  candidate: `move_off`, `spot`, `doorstep`, `placements`/`packed_on`,
  `rules::search`'s per-strip `Places` (rules.rs:930-958), and
  `stage_arrange`. Otherwise a sofa could be moved, delivered or
  repaired onto a wide shelf.
- No door space on a shelf (`space_of` is reached only via
  `strip_door`, but `choose` would offer one if shelves joined the zip).
- Ledger: `SavedAnchor.strip` is serde `Strip`. Older builds already
  degrade: `an_unknown_strip_falls_back_to_the_rooms_pane`
  (ledger.rs:859) feeds `{"Shelf":3}` and the piece goes back to its
  room's pane. **Once `Shelf` is a real variant, that test parses and
  stops testing "unknown"**: give it another variant name.
  `a_garbled_door_wall_reads_as_none` (ledger.rs:1305) covers a door on
  an unknown strip. Structures themselves need a new top-level field,
  written only when non-empty (as `unsettled` is) so goldens hold.

## 2. Rooms are strips today

phase4-design.md:34: "a room = a strip until phase 6". Every site:

| Site | File:line | What equates room and strip |
|---|---|---|
| `Rule::Apart` doc | rules.rs:33 | "(a room is a strip, for now)" |
| `Offer`, `Role`, `RoleRule`, `ROLES` | room.rs:524-581 | first row met; no `base`/`min_width` |
| `role`, `forbids` | room.rs:584-603 | pure on offers |
| `role_of`, `role_without`, `role_among` | room.rs:606-626 | filter `s.strip == Some(strip)` |
| `Home::rooms`, `furnished` | room.rs:1663-1686 | per strip |
| stage `rooms()` | mod.rs:1619-1627 | prints `nook role` |
| `faces` (Faces) | room.rs:2713 | `seat.strip == screen.strip`: watches **through** a partition unless changed |
| `judge`: Near | rules.rs:279-290 | same strip and `between ≤ gap` |
| `judge`: AgainstWall | rules.rs:291-298 | `against_wall(at, plan.floor)` (395-399): only the floor extent's ends are walls; a partition isn't one |
| `judge`: Apart | rules.rs:299-308 | `x.strip == y.strip` |
| `judge`: Belongs / `spoils` / `spoilt` / `newly_spoilt` | rules.rs:309-386 | keyed `(Strip, Furniture)` |
| `judge`: DoorClear | rules.rs:319-332 | per `plan.space` |
| `Before.roles`, `becomes_den`, `has_room` | rules.rs:517-597 | `Vec<(Strip, Role)>` |
| `evaluate` tier | rules.rs:753-770 | "a strip with nothing on it" = tier 2; role of `to.strip` |
| `search`: `apart_from`, `beside`/`keeps_company` | rules.rs:930-946 | per strip |
| `beauty_at` | mod.rs:233-245 | strip whose floor row is her `y` and whose raw columns hold her `x`; sums decor on that strip. **Decor on a shelf would count for no room she can stand in** unless rooms gather shelves. |
| `clock_on` | mod.rs:213-229 | the clock's strip's raw extent |

Roles and offers are used only in room.rs, rules.rs and the stage line;
brain/mind never see them. Grievance keys are `(row, piece)`
(rules.rs:138), so a room change doesn't touch the felt set; `on_sight`
(mod.rs:3642-3663) feels only DoorClear (when bumped) and InChat.

**Repair search** (rules.rs:905-1050). Per broken rule: each piece it
names (`Broken.pieces`), each strip whose lane holds it (filtered by
partners and Apart), each left along the extent (stride raised until
≤ `CANDIDATES` = 2,000, rules.rs:470), each facing (both only for the
Faces seat). `evaluate` (605-797) lays out a scratch home
(`relaid`, only touched strips), then refuses: a piece left out, a
pushed hung piece, the rule still broken, a newly spoilt room, a room
turned den, a kept door space lost, a held rule broken, a piece newly
closeted. Cost = cells moved (+1 for a turn); tiers 0-3
(`SETTLED_BEHIND`). `fits_now` (837-866) then checks the frame (blank
cells, room to use it, `Keep`). Keeps `REPAIRS` = 3. Measured
0.44-0.46 ms worst case (three 400-wide strips, 1,173 moves; plan.md
phase 4). A partition as a repair is not a piece move, so it needs its
own branch beside `places`, not a `Places` row.

**If a room becomes a region of a strip between walls (pane borders +
partitions):**
- A room identity finer than `Strip` (strip plus region) replaces
  `Strip` in every row above. `Faces`, `Near` and `Apart` start failing
  across a partition (that's the point for Apart; for Faces and Near
  it's a behaviour change worth stating).
- `AgainstWall` should count a partition side as a wall, so it needs
  region extents, not `plan.floor`.
- Packing: `pack` knows only the extent's two ends. A partition on the
  floor lane is either a fixed 1-column obstacle in `pack` (an anchored
  pseudo-piece in `order_key` order, guess: the least disruptive) or it
  splits the floor `Extent` into sub-extents, which changes
  `StripPlan.floor` from one `Extent` to several, and with it `pin`,
  `along`, `space_of`, `first_clear`, `packed_on` and the search's
  `lefts`. Either way, a piece whose body meets the partition's column
  fails `fits` today (body cells must be blank) and goes to the closet,
  so without a packing change a partition closets pieces rather than
  pushing them.
- `Anchor.side` counts from the strip's walls. Whether a piece in a
  partitioned room anchors to the partition is open.
- `beauty_at` and `clock_on` must find her room by region.
- Older builds see only strips; a partition record is unknown to them
  and dropped (fine: they show one room per pane).

## 3. Terrain

**`Terrain::read` / `read_guarded`** (terrain.rs:222-314). Per cell:
`skip` = image/untouchable; `open` = not solid and (ASCII, or line art
and quiet or narrow); `calm` = not solid and (ASCII, or quiet). `quiet`
= blank (not a wide glyph's trailer) or `strokes(c).is_some()`
(redrawable). Ledges: `ledge(symbol)` (terrain.rs:125-127: box drawing
with a horizontal stroke, i.e. not `vertical_only`, or `╱`/`╲`), and in
line art only if redrawable. Poles: `pole(symbol)` (129-131: box drawing
not `horizontal_only`). Flags kept per cell in `lines` (door batch):
`LEDGE` 1, `GUARDED` 2 (protected/untouchable/off-screen), `WALL` 4
(`│` or `┃` only), `STROKE` 8 (any box glyph). **The `poles` grid is a
local** (236, 264), handed to `find_links` (310) and dropped; `Terrain`
keeps no pole data.

Platforms (`add_platforms`, 320-345): a ledge run split where her box
(`WIDTH` 5 × `HEIGHT` 4) is `clear` above; at least `MIN_PLATFORM` = 3
standing positions, so a run of ≥ 7 cells.

Links (`find_links`, 518-623): for each upper/lower pair, overlapping
standing spots and a column within `lo-HALF-1..=hi+HALF+1` that is a
pole on every row strictly between them, her box `clear` at every row
from `upper.y+1` to `lower.y` → `Climb` both ways (549-570). No overlap
→ `clamber` (465-481): a pole within reach, step ≤ `WIDTH + 2`. Drops at
true ledge ends; `around_links` off screen edges.

**`furnish`** (terrain.rs:365-369): line art only; stores piece covers
for `restful`'s shared image. Furniture is not solid to her body
(`Visit::solid`, mod.rs:1020-1024 = protected + moved-text runs).

**`Ground` trait** (door.rs:141-150): `ledge`, `wall`, `stroke`,
`protected`, text-blind; impl on `Terrain` (terrain.rs:640-658);
`Ungated` (door.rs:157-187) unprotects the focused pane. Test impls at
door.rs:532 and 581 would need any new method.

**`osaka::Clear`** (osaka.rs:224-235): a spot whose box meets no
obstacle; the only way to open a door in space (`nearest_clear`, 242).

**What happens between frame and terrain today (visit,
`paint_state`):**

1. `observe`, `gate`, `whole_glyphs` (2501-2507); `errand_progress`;
   `nudge_due` → maybe `send` → **errand read** (3446).
2. Focus rain from `visit.painted` (2724-2745).
3. `visit.layer.validate(buf, &view.protected)` (2747).
4. `record` (2756), then `furnish` (2763-2775): `home.frame` →
   `project_with` → `fits` on `buf` (blank body, stroke floor row);
   deliveries (the lazy `fresh` read at 4120); the repair search
   (`rules::Frame { buf, .. }`).
5. `on_sight`; `Keep::of(...).for_made()`; `tend_made` (made pieces
   judged on `buf`).
6. Ops applied to the layer (`scenes::apply` with `base` = protected +
   shown covers + ghost); grip checks on `buf`; `let_go`.
7. **`Terrain::read_guarded(buf, solid, protected, graphics)`** (2842),
   `terrain.furnish(covers)`.
8. `door_place`, `door_follows`, `recheck`, `evict`, `settle` (2899-2909).
9. `scenes::pulls(buf, ..)` (2920), `visit.layer.paint(buf)` (2924:
   moved text goes into `buf` here), `swaps`, `builds`, `borrows`,
   `loose` (all read `buf`).
10. `nudge.paint` (3116), flap, `draw_props`, front door, her
    (`draw_art` / `draw`), then `visit.painted = layer` (3187).

**Where `read_ground` must go: after 3 and before 4**, not "just before
`Terrain::read`". `fits` needs a shelf's `─` in `buf` to stand a piece
on it, and must see a partition's `│` as non-blank so no piece is laid
over it. Written at step 7, furniture would already stand over
partition cells. Structures must therefore be placed clear of the
**layout** (`laid_and_shifted`, pure geometry, room.rs:1913) rather than
of what showed. Made pieces and layer cells come from the visit (last
frame's), so they're known at that point. App text or a protected cell
over any of a structure's cells needs an all-or-nothing rule: a shelf
with a missing `─` is a broken ledge, not a closeted shelf (proposal:
"closets it for that frame").

**Arrival** (2515) reads terrain *before* `home.frame` (2534), the
opposite order to the visit's; read_ground goes before both.
**Away** (`paint_empty`, mod.rs:5660-5770): `home.frame` (5710), then
`ground` (5716) for her door, `terrain` (5727). Structures must show
here too, and their `Frozen` cells must join `empty.painted` (5769) so
`rain_home` takes them. **Errand** (3446) reads `buf` without
structures: the 2026-10-01 rule's own bug class (a spot chosen on one
picture of the screen, used on another) unless it goes through
read_ground too.

**Frozen / rains.** `Frozen { x, y, glyph, ink, under, face, burst }`
(dissolve.rs:40-54). Focus rain: `visit.painted` cells in the focused
pane (mod.rs:2724-2745). Goodbye: `Dissolve::new(now, visit.painted, ..)`
(mod.rs:1773-1790). Out by her door: moved/made cells rain (2367-2380).
Structure `Frozen`s must be in `visit.painted` with `under` = the real
(blank) cell.

**Complication: the goodbye exception.** In `State::Leaving`,
`untouched = leaving.dissolve.unchanged(buf)` (mod.rs:2648) is computed
before `dissolve.paint`; `unchanged` (dissolve.rs:220-225) requires
every `burst` cell's `under` to equal the live `buf`. Her line-art
image's burst cells take `under` from `buf` as painted
(mod.rs:5601, `buf.cell(..).clone()`), which would hold a structure
glyph wherever her image covers one: standing on a shelf (floor row),
climbing a ladder, passing a partition. The goodbye frame has no
structures (the proposal's exception), so `unchanged` goes false and
she gets no startled beat or wave, only the burst. Fixes: write the
structures in the goodbye arm too, before `unchanged`; or record
`under` from the pre-structure frame. Moved text has the same latent
issue today, but her box never covers moved text (it's solid), so it
rarely bites. Guess: a cell may also appear twice in `visit.painted`
(the structure's Frozen, then her image's), with different `under`s,
as moved-text cells under her ASCII sprite already do (layer painted
before `draw`, mod.rs:2924 and 3170). The dissolve tolerates that for
the layer; check it for structures.

## 4. Climbing

`settle` (osaka.rs:10158-10244). Acts with `on_chat == Back` are left
alone. Arms:
- `Clamber` (10164-10168): `terrain.clear(x, y)` only.
- **`Climb { to_y }` (10169-10173): `platform_at(x, to_y)` and
  `clear(x, y)`. No pole check.**
- `Fall` (10174-10186): re-targets the landing.
- `Walk` in from off screen (10190-10196).
- Else (10197-10209): on a platform, or clamp the walk.

If none holds, "the floor went away under her": `landing` below →
`Act::Fall` (10216-10228), which lands `Dazed` only if it fell ≥ 3 rows,
else `Stand` (osaka.rs:5272-5291, `DAZED_MS` = 1500 at 475); no landing
→ teleport to the nearest platform and `Dazed` (10230-10243); no
platform → `false` (she leaves, "no room", mod.rs:2902-2909). So "drops
her, and she lands dazed" holds only for falls of 3+ rows.

The pole she climbs is `Osaka.pole` (set from `link.pole` at
osaka.rs:5159; `Act::Climb` itself has no column). A pole check needs
pole data the terrain doesn't keep: retain the `poles` grid (or a
`POLE` flag in `lines`) and check `Osaka.pole` on the rows between
`self.y` and `to_y`. `WALL` alone would miss tees and corners
(`├`, `┼`), which are poles today. The same check is wanted in the
`Clamber` arm. `aloft()` (10278) = `on_chat == Landed`: chat lets a climb
run its course (`chat_mid_clamber_doesnt_drop_her`, tests.rs:10956).

**The ladder needs no new terrain code (confirmed by reading).** Rails
`│` are poles; a rung `─` is a ledge run of one cell (rails aren't
ledges), far below a platform's 7. In line art her box is `open` over
the rails and rungs (redrawable), so `clear` holds while climbing.
`find_links` gives `Climb` where the two floors' standing spots overlap
and a rail column is within `HALF + 1` of them; otherwise `Clamber` if
the step is ≤ 7. Caveat: a ladder with no gap rows (floors adjacent)
adds nothing; the default layout's floors are already linked (see the
terrain snapshot), so `Connected` breaks only on layouts without links.

**A partition is climbable by the same rule.** A `│` column from the
floor's row − 1 up to the row under the top border is unbroken between
the pane's floor and the floor above it (the top border, a platform
when the pane above has headroom), so it adds a `Climb` link if the
standing spots overlap. The proposal wants this ("can climb it"), but
it means a partition *adds* links, and a door may then be reached by
climbing. `Ground::wall` reads `│`, so `on_floor` (door.rs:366) keeps a
fallback door off any box with a partition cell above its floor row
(wanted); a shelf's `─` is a `LEDGE`, so a fallback face-on door may
stand on a shelf at least 5 wide with clear, unprotected cells above
(guess: unwanted; exclude shelf cells or add a flag).

## 5. graphics.rs

| Item | File:line | Note |
|---|---|---|
| `strokes(c)` | graphics.rs:309-366 | redraws `─ │ ┌╭ ┐╮ └╰ ┘╯ ├ ┤ ┬ ┴ ┼` (light), `━ ┃ ┏ ┓ ┗ ┛ ┣ ┫ ┳ ┻ ╋` (heavy), `╱ ╲`. **Not** double (`═║╔…`), dashed (`┄┆┈┊╌╎…`), half lines (`╴╵╶╷…`), mixed light/heavy (`┍┝┽…`) or `╫`. All three structures use `─`, `│` only, so nothing new. |
| `draw_glyph` | graphics.rs:371-425 | redraws a stroke as the terminal does (`LineGeometry`, Ghostty's thickness). |
| `Layer`, `Cut { layer, clip, dx }` | graphics.rs:182-232 | `Cut` offsets by half columns and clips at a wall's column (her doorway); bounds in cells. |
| `paint_layers` → `paint_cuts` → `key` | graphics.rs:598-760 | one image over the union of the layers' boxes. Every cell must be `open` (caller's closure) or a **floor-row** cell; floor-row cells must be non-blank `strokes` glyphs; other non-blank cells are recorded in `Key.lines` with their colour (lines redrawn, text derezzed). |

So her image redraws structure cells from `buf` as it does borders,
provided they're in `buf` when she's painted: they are, written before
the terrain read. Structure colour is part of the image cache key
(`Key.lines` holds the `fg` RGB), so give structures one fixed ink.
Standing on a shelf passes the floor-row test as is. **ASCII**: her
sprite is text `put` over cells (`draw`, mod.rs:3791-3885); there's no
redraw, so the structure glyph shows wherever her sprite cell is blank
and her glyph replaces it otherwise, as with borders; `wanted.retain(|..|
terrain.open(x, y))` (3856) keeps her off protected cells.

## 6. "Her own overlays are painted after every read"

decisions.md:2263-2303, *Houseguest reads the frame she stands on
(2026-10-01)*: one solid set for every terrain she's placed on; "her own
overlays (the accordion's shake) are painted after every read". design.md
line 2621 has the shake half. Enforced by placement and comments, not
types:
- mod.rs:2596-2603: "The accordion's shake is hers, painted like the
  rest of her: after everything that reads the real frame"; `nudge.paint`
  at 2602 (Absent/Arriving), 2650 (Leaving, after the read at 2649),
  3116 (Visiting, after 2842), 5754 (Away, after 5716/5727).
- mod.rs:2643-2646 (Leaving): "Everything that compares against the real
  frame reads it before any of her pixels or glyphs go on".
- mod.rs:2744-2746: "Read before paint: the layer validates against the
  real frame".
- The layer itself is painted at 2924, after the read.

Structures would be the one overlay written before the reads: the rule's
text (decisions.md, design.md) needs the exception, and the comments
above need it too. Nothing in the types stops a later read site from
reading `buf` without them.

## 7. Tests a structure change would likely move

| Test / infra | File:line | Why |
|---|---|---|
| `long_visit_of`'s "each real piece stands on a strip here" | tests.rs:1365-1374 | uses `room::raw_strips(&view.nooks)` and `e.floor == prop.floor`: a shelf piece fails |
| `long_visit_of`'s "furniture on lines, over blank" | tests.rs:1376-1410 | floor-row cell read from `real`, which has no shelf: fails for a piece on a shelf. Body cells must be blank in `real`: fine |
| `Hidden::check_raining` | tests.rs:582-660 | a changed cell must be blank in `real` or in the layer; structures go only on blank cells, so they pass. Text hidden under her image ≤ 10 s: unaffected |
| `assert_untouched_but_feet` | tests.rs:518-560 | protected cells unchanged: structures never go there; passes |
| final `prop_assert_eq!(end, real)` in `long_visit_of` | tests.rs:1526 | the goodbye must rain structures out completely |
| the four `long_visits_*_never_touch_what_is_protected` | tests.rs:696-860 | random text includes `─│` already; structures need a generator (owned structures) to be exercised at all |
| `the_carry_keeps_every_promise`, `a_trial_keeps_every_promise`, `promised_frame` | tests.rs:13982-14220, 13764 | `fits`, `Keep`, covering checks per frame |
| `every_parcel_on_her_doorstep_gets_unpacked` | tests.rs:5632 | shelves as candidate strips for deliveries |
| `an_older_record_migrates_without_closeting_anything` (+ crowded) | tests.rs:7778-7830 | ledger migration |
| `errands_end_and_touch_only_the_accordion` | tests.rs:8771 | the errand's read |
| `chat_mid_clamber_doesnt_drop_her` | tests.rs:10956 | interacts with a pole check in `Clamber` |
| golden trajectories | tests/golden.rs (889-1323) | each frame hashes her act/x/y/appearance and every cell differing from the real frame, plus the record as saved (JSON). Unchanged if read_ground is a no-op with no structures and the new ledger field is skipped when empty; scenes that gain structures re-record |
| terrain snapshots | snapshots/*default_layout_terrain_* (tests.rs:1639-1655) | read the default layout without structures: unchanged unless `Terrain::read` changes what it yields. `osaka_at_home_seed_7` (1657): static, no furniture: unchanged |
| room.rs `packing_keeps_order_and_a_resize_and_back_restores` | room.rs:3330 | if partitions join `pack` |
| rules.rs `rules_hold` (property, 1564), `repair_search_is_cheap` (2295), door-space repair tests (2414-2797) | rules.rs | rooms by region, more strips (stride), partition repairs |
| ledger golden files, `an_unknown_strip_falls_back_to_the_rooms_pane` | ledger.rs:859 | see section 1 |
| door.rs Ground test impls | door.rs:532, 581 | a new `Ground` method |
| dissolve properties, `moved_text_rains_out_whole` | dissolve.rs:783-1066; tests/rain.rs | structure Frozen cells; the `unchanged` issue in section 3 |

Deep pass: `PROPTEST_CASES=256 cargo nextest run --release --profile
deep -p dessplay houseguest` (.config/nextest.toml `[profile.deep]`,
kill at 180 s). The proposal's phase-6 tests ("a structure never removes
a link; one closeted under her drops her; long visits with random
structures keep every covering property") have no existing generator
for structures; the four `long_visits_*` are the natural hosts.
