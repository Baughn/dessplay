# Her door: the bug and the reserved space

**Investigation brief, 2026-10-07** (read-only, written during phase 5c; kept here as the door batch's
starting point). Where it and docs/plan.md's *Next: the door batch* disagree, **plan.md wins**: the user
has since answered its questions (§8): parcels come through a flap **in her door** (its Q1, B, not A),
the approved door is the sheet's **B, two columns, turned slightly** ([art/snippets.md](art/snippets.md)),
older homes whose furniture fills the space have her **clear it herself** (§7's optional felt rule, now
in the batch), work leaves by the same door, and poster and clock may hang above it. Line numbers are
from the working copy of 2026-10-07 and will drift; `placement.md` and `teardown.md`, cited below,
were the two investigators' notes and are not kept.

Report (2026-10-06): *"she just left for school directly from bed, from a door that's now stuck
visible overlapping said bed."* Asks: *"her external door has a reserved empty space next to the
edge of the screen"*. Later, through the coordinator: *"The door can stay drawn while she's out;
there's really no other way to know she *is* out. But that might work better with a side-on drawing
of the door."*

Paths are relative to `dessplay/src/ui/houseguest/` unless noted. Line numbers are from the
**working copy** as of 2026-10-07. osaka.rs has uncommitted edits, so the numbers in teardown.md
for osaka.rs are 4 to 6 lines off; the ones here are the ones I read. Sources:
`scratchpad/door/placement.md` and `scratchpad/door/teardown.md` (not kept). I checked every claim below
against the code myself.

Logs: neither investigator found a 2026-10-06 log on this machine. `log_dir` is
`~/.local/share/dessplay` (`dessplay/src/logging.rs:39-42`), and its newest file is 10-05. That
file has two houseguest DEBUG lines and no door lines. Nothing to quote. Even with the log, the
door's spot would be missing: `go_out` logs only `?why` (osaka.rs:5164), and `out_by_door` logs
only `kind` (mod.rs:1695). See the logging fix in section 4.

---

## 1. What is by design and what is the bug

- **The closed door staying drawn while she's out is by design.** The user confirmed it today.
  It's also in docs/design.md:2101-2106 and docs/decisions.md:3334-3361. Every exit clears it
  correctly (teardown's lifecycle section): `leave` (mod.rs:1200-1222), `door_rain`
  (mod.rs:2603-2625), `school_out` (mod.rs:1615), `begin_visit` (mod.rs:2378). Nothing is
  "stuck".
- **The bug is where the door stands.** It stands wherever her feet were. Nothing checks that
  spot against her furniture, so the door can stand inside a piece in both drawing modes. It
  then stays there for 4.5 hours (08:15 to 12:45), and she comes home out of it there.

## 2. Root cause (verified)

1. **The 08:15 cut leaves her inside the piece.** `cut` (osaka.rs:3395) treats any `Act::Use`
   as cuttable (osaka.rs:3436). It calls `interrupt(Cause::Routine)` (osaka.rs:8710-8727),
   which sets `Act::Look` where she is. For `Use::Lounge | Nap | Sleep | Homework`, her seat is
   *in* the piece (`Shown::seat`, room.rs:1000-1004). The bed's sit column is 3 of its 10.
   `wake` (osaka.rs:5208) steps her `beside` the bed (osaka.rs:5212-5222). The school cut has no such step. The
   design wants the cut (design.md:2097-2100).
2. **`go_out` opens her door where she stands.** The next decision reaches
   `go_out(Routine::School, ..)` (called at osaka.rs:7181). `go_out` sets
   `Act::Door { to: (self.x, self.y), gap: u64::MAX }` (osaka.rs:5151-5179). The "off text"
   check just before it (osaka.rs:7113-7116) uses `restful`, which accepts her standing in a
   piece, so nothing moves her first.
3. **The guest records that spot.** `out_by_door` stores
   `DoorAt { x: visit.osaka.x, y: visit.osaka.y, facing }` (mod.rs:1691-1703).
4. **The only check accepts a door inside a piece.** `paint_empty` moves the door only if
   `!door.is_some_and(fits)` (mod.rs:4466-4476). `door_fits` is
   `platform_at && restful` (mod.rs:4365-4367). `Terrain::restful` (terrain.rs:328-361) answers
   "may *she* stay here". It deliberately passes the cells of any piece her image takes in
   (`calm || floor || piece`, terrain.rs:356-358), because that's how she lies in her bed.
   - **Line art:** the door then fits inside the bed. `paint_empty` puts the bed in the door's
     image (`terrain::image(..).with`, mod.rs:4508-4512). `draw_door` pushes the door layer
     last, over it (mod.rs:4619).
   - **ASCII:** `furnish` does nothing without graphics (terrain.rs:318-322). The door's
     glyphs are `put` over the bed's (mod.rs:4564ff and cells.rs:53, which overwrites any cell
     that isn't untouchable).
5. **She comes home there too.** The arrival builds its terrain with
   `Terrain::read(buf, &view.protected, ..)` and never furnishes it (mod.rs:1840).
   `door_spot(&terrain, near)` (mod.rs:1847-1851, 4371-4381) then sees none of her pieces.
   `back_through_door` and `dash_in` both come out of the door inside the bed.

**The design line that should have prevented this is wrong.** design.md:1380-1381 says "The door
stays within her box, so it never covers anything". That holds for a door in space that lasts a
second. It fails for a door that outlasts her, because her box may contain a bed, sofa or desk.

Draw order is not the cause. The window ordering from 5c step 11 is not involved.

## 3. The class, and every sibling site

The class: **an external door's position comes from her feet, and the only check is "she may
stay here" (`restful`). Nothing asks "her door's box meets no piece."**

| Site | Spot | Lasts | Status |
|---|---|---|---|
| School: `go_out` (osaka.rs:5151, called at :7181) | her `(x, y)`: the seat of any in-piece `Use` cut at 08:15. That includes bed, sofa, desk, a box she's unpacking or a heap she's crumpling (room.rs:1006). Also a walk cut halfway, or the LookOut spot under a low window | 4.5 h | **the report** |
| Out again after a dash or errand at school (`go_out` through `dash_on`, osaka.rs:5568) | where she stands. That's usually the fridge's `beside` seat, or the spot she came in at, which is only as good as the old door's spot | until 12:45 | same class |
| Work by door: `go_to_work`'s None arm (osaka.rs:9263-9282, called at :7511) | `here` | 1–3 min gap | same class, short-lived |
| `school_from_work` (osaka.rs:3456-3474) | Opens no door of its own, but freezes the work door's gap to `u64::MAX` wherever `go_to_work` put it, and `out_by_door` records that spot. **Resolution:** teardown is right that it plants nothing, and placement is right that the stuck door comes from that spot. The real site is `go_to_work`. Work is open only on days off (osaka.rs:3427-3428), so only the stage reaches this path | 4.5 h | fixed once work goes through her door's space |
| Away: `out_by_door` (mod.rs:1691), then `paint_empty` (mod.rs:4466-4486) | the recorded spot, re-placed only when `restful` fails | 4.5 h | the check |
| Return and Dash arrival (mod.rs:1840-1851) | `door_spot` on an unfurnished terrain | the door beats, then she stands there | the check |
| Cold start in school hours (`out == None`): `door_spot(fit, middle(size))` (mod.rs:4471-4474) | the middle of the screen's foot. `restful` lets it land in a piece | 4.5 h | same class |
| Stage `dash_through` (osaka.rs:8963) | beside her fridge | seconds | stage only |
| Doors in space: `go_to` with no route (osaka.rs:~7866), `find_rest`, `head_for_errand`, `send_to_bed` going through `go_to`, `door_away` (mind.rs:440-459) | her box, then a target spot | ~1.5 s | **Out of scope.** The near door covers only what she already covers, and the far end is a spot she may stay at. `door_away`, which picks an x with no check at all, is optional cleanup |

## 4. The fix, making the overlap unrepresentable

**The data-shape change.** Stop storing an `(x, y)` taken from her feet. Her external door's spot
is **computed each frame** from her home and the frame. It is never remembered.

- `Out.door: Option<DoorAt>` becomes `Out { door: bool }`. That's enough to say whether her
  closed door stands. Its facing is part of the place now (into the room).
- New type `DoorSpot { x, y, facing, wall: Option<DoorWall> }`. Its only constructor is
  `door_place(home, nooks, shown, terrain) -> Option<DoorSpot>` (section 5.5).
  - `go_out`, `go_to_work` (door arm), `out_by_door`, `paint_empty`, the Return, the Dash and
    the cold start all take a `DoorSpot`.
  - No call site can make one from her feet.
- **Inside the reserved space, the overlap can't happen by construction.** Layout packs pieces
  into an extent that doesn't contain the space (section 5.3). No piece can be there for a door
  to meet.
- **Outside it, a strict check remains, and it is still only a check.** This applies when the
  space yields, when makeshift pieces are laid on lines, and with no home. The spot must satisfy
  `her_box(x, y) ∩ s.cover() = ∅` for every shown piece. That's the existing
  `room::free(shown, blocked, None, x, y)` case, "her box may overlap none" (room.rs:1792-1807),
  applied over `her_box` (room.rs:1773-1787).
  - The check never uses `restful` and never depends on `furnish`, so it's the same in both
    modes.
  - `door_place` is the one place that runs it, so no other site can forget it.
- **Logging.** `out_by_door` logs `x, y, wall` at info. `door_place` logs at debug when it falls
  back or yields, with the reason, so the next report has the spot in the log.
- **Interim stopgap, only if a fix is wanted before the space lands.** Swap `door_fits` for the
  strict predicate, furnish the arrival's terrain, and have `go_out` step her `beside` the piece
  she's in first, as `wake` does (osaka.rs:5208-5222). It's small, but it fixes the listed sites
  rather than the class. I recommend going straight to the space (section 7, step 1).

## 5. The reserved space, designed for a side-on door

### 5.1 The rule (for design.md, *Strips and anchors*)

> **Her door** is in a wall of one of her strips, a wall at the screen's edge first. The door is
> seen side-on, set in the wall's line. Beside it, inside the room, is **her door's space**: six
> columns of floor against that wall, four rows high. No piece stands in it, and no piece hangs
> low enough to meet it. A poster or clock may hang above it. Every exit and return by her
> routine, and work by door, is through her door. She walks (or hops, or takes a door in space)
> to the space, faces the wall, and goes through. While she's out, her door stands there, shut.
> Where a strip can't hold its pieces and the space too, the space gives way for that frame. Her
> door then stands, face-on, at the nearest floor spot where her box meets no piece. With no
> such spot it isn't drawn.

### 5.2 Geometry (right wall shown; the left wall is mirrored)

`Extent` (room.rs:738-743) runs `from..to`, and `to = rect.right() - 1` (room.rs:803-806) is the
**wall's own column** (the pane's right border). The floor is row `f = e.floor`.

| What | Columns | Rows |
|---|---|---|
| The space (kept clear of pieces) | `to-6 ..= to-1` (6 = `sprite::WIDTH + 1`) | `f-4 ..= f-1`, plus floor cells `to-6..=to-1` on row `f` |
| One cell of air between the door and the nearest piece | `to-6` | — |
| Her spot to go through (anchor) | `x = to-3`, so her box is `to-5 ..= to-1`, flush against the wall, facing Right | her box rows `f-4 ..= f-1` |
| **Side-on door, shut** (next batch's art) | the wall column `to`, maybe 1 column into the room (`to-1`) | `f-4 ..= f-1` (4 rows, her height), standing on the floor row |
| Side-on door, open | leaf swung into the room: at most 3 columns, `to-3 ..= to-1`, inside the space | `f-4 ..= f-1` |
| Hung pieces | poster and clock hang 4 rows up (`hang: Some(4)`, room.rs:~321/337, 2 rows tall). `top()` is `floor - lift - rows` (room.rs:956), so they sit on `f-6..=f-5`, above the door. **May hang above it.** The window hangs 1 row up (room.rs:~355), on `f-3..=f-2`, so **it must hang clear of the space** | — |

**Painting in the wall column.** There's a precedent. `draw_flap` (mod.rs:3901-3935) repaints
the wall's own `│`/`┃` cells, skips protected cells, and records `Frozen` cells so the rain
restores them. The side-on door uses the same rules for its cells in column `to`:
- it paints only over a plain vertical wall line;
- it's hidden while any of its cells are protected (today's `standing` filter, mod.rs:4478-4484);
- in line art, it's an image over those cells.

### 5.3 Brief for the side-on door's model sheet (next batch)

- **Canvas.** Her canvas is 100×160 units for 5×4 cells, so 20 units per column and 40 per row
  (art/door.svg's header, art.rs:1101). The external door needs its own canvas, **2×4 cells =
  40×160**, feet at y = 158. Column 0 is the room side and column 1 is the wall column. A cell
  is about 2:1 tall, so a 1-column door is a 1:8 sliver. **Whether a 1-column edge-on door
  reads as a door is the sheet's main question.** A 2-column version (the frame post in the
  wall, the leaf's edge and knob one column in) is the fallback.
- **Frames.**
  - Shut, the Away cue that stands for hours: pink, a visible knob or edge highlight, readable
    at 1–2 columns.
  - Ajar.
  - Open: the leaf swung into the room, seen face-on, at most 3 columns, the doorway showing
    outside light.
  - Her stepping through: side-on poses. The existing `door_sheet` (art.rs:3025-3082) already
    draws her side-on stepping in, so it can serve as the template.
- **Hinge at the back jamb.** The open leaf then stands *behind* her as she goes through, and is
  never drawn over her.
- **ASCII.** Pure ASCII, at most 2 columns × 4 rows shut, for example a pink `|` with `o` for
  the knob on row `f-2`, and at most 3 columns open. ASCII art gets no review (dev/test only).
- **Code.**
  - A new look beside `Look::Door`, which stays the face-on door in space (graphics.rs:69-110,
    `render_door` art.rs:1101).
  - A sibling of `door_cells` (sprite.rs:475), which assumes her 5×4 box.
  - A sibling of `Door::closed` (mod.rs:308).
  - The door-in-space art doesn't change.

### 5.4 Where the space lives, and migration

- **`Home.door: Option<DoorWall>`**, with `DoorWall { strip: Strip, side: Side }`, added to
  `room::Home` (room.rs:1119-1121). It's saved as a new `Saved` field with
  `#[serde(skip_serializing_if = "Option::is_none")]` (ledger.rs:567ff) and read as an
  `Option<serde_json::Value>` in `Raw` (ledger.rs:615ff), as `clock_sent` already is. Older
  builds ignore it. An older record reads as `None`.
- **When it's chosen.** The first frame it's needed, with or without furniture: among the quiet
  strips this frame, a wall at the screen's edge first. That's the same test `doorstep` already
  uses: `rect.right() == screen.right()` and `rect.x == screen.x` (room.rs:1519-1523).
  - Strip order: strips she has pieces on (in `Nook` order), then the rest. The right wall
    comes before the left.
  - The first wall whose strip still packs its floor pieces in the narrowed extent wins. In the
    default layout (chat left, panes stacked right) that's the right wall of her room's strip.
- **When it changes.** Only when its strip leaves `nooks`, the same moment `move_off`
  (room.rs:1377) moves a room for good. It follows her room: the room's new strip first.
  - A resize never re-chooses it. The space only yields for that frame, so a resize and back
    puts the door where it was, as with pieces.
- **Rejected: deriving the wall every frame.** A derived choice moves her door when a parcel
  lands on an earlier strip or a repair reorders `props`. That would be a new bug.
- **No home.** The same chooser runs over the frame's strips, unsaved. With no strips at all,
  the strict fallback spot nearest `middle(size)`.
- **Migration of existing homes needs no record rewrite.** The narrowed extent (5.5) pushes
  pieces anchored at that wall along, and keeps their anchors. A fridge at
  `Anchor { Right, 0 }` stands at `to-6`'s left on the first frame: not closeted, not moved
  off, and its anchor is unchanged.
  - **Why not put the space into `blocked`:** `project`'s `fits` (room.rs:1366-1370) would
    closet any piece standing there. Her fridge would vanish on the first frame after the
    update.

### 5.5 How the space is kept clear

- **`Home::extents(&self, nooks) -> Vec<(Strip, Extent)>`.** This is `room::strips`
  (room.rs:797), with the door's strip narrowed by 6 columns on its side (`to -= 6` or
  `from += 6`). It narrows **only if** `pack(on(strip, Floor), narrowed).is_some()`. Otherwise
  the extent stays raw, and that frame's door falls back.
  - So the door never sets off `move_off`, never closets a piece, and a room that fits today
    looks the same tomorrow apart from the push.
- **What switches to `extents`:**
  - in room.rs: `layout`, `laid_and_shifted`, `project` with `move_off`'s targets, `spot`,
    `doorstep`/`admits`;
  - in rules.rs: `rules::Before::new` (rules.rs:465);
  - in mod.rs: `broken`'s caller (mod.rs:3141), set-down pinning (mod.rs:3047) and
    `stage_arrange` (mod.rs:3269).
- **What keeps the raw strip:** `clock_on` (mod.rs:154) and `beauty_at` (mod.rs:167). They say
  which room she's in, and the space is still in that room.
- **`against_wall`** (rules.rs:324-327) uses the narrowed extent. A piece beside her door is
  still "against the wall", so migration doesn't trip `AgainstWall` and set her mending.
- **The wall lane** packs on the raw extent, since poster and clock may hang above the door.
  `hung_clear`'s `meets` (room.rs:1298-1303) also refuses a window whose rect meets the space.
- **Makeshift pieces** stand on lines, not extents. `builds`' `clear` (mod.rs:~3696) and
  `rules::Frame::free` (rules.rs:428-440) refuse the space's cells.
- **Her own spots** (`beside`, look-out spots, the unpack seat) may be in the space. It's floor
  for her, just not for pieces.
- **Builder traps** (from placement.md, both checked):
  - `rules::evaluate` judges scratch homes against `before.strips`, computed once
    (rules.rs:465, used at :534 and :548). Since narrowing depends on packing, recompute
    `extents` for each scratch home.
  - `Prop.at` (thousandths along, for older builds, room.rs:668-670) is pinned on the narrowed
    extent, so an older build reads it up to 6 columns off. That's harmless drift.
- **`door_place(home, nooks, shown, terrain)`:**
  1. The space's spot `(to-3, f)`, facing the wall, if it isn't yielding and is clear of
     protected cells.
  2. Otherwise, the floor spot nearest it (or nearest `middle`) that passes the strict
     predicate from section 4 and `platform_at`. The face-on door stands there.
  3. Otherwise `None`. The Away door isn't drawn, an arrival waits for another idle delay as
     today (mod.rs:1872-1875), and `go_out` goes out at the nearest strict spot, or where she
     stands if that passes.

### 5.6 The external sites, changed

- **`go_out`.** A new `Job::Leave { spot, why }` (scenes.rs:235). `Chances` gets
  `door: Option<DoorSpot>` from the guest's `door_place`, next to `clock` (osaka.rs:~70). The
  reflex calls `go_to(want, Job::Leave, ..)` (osaka.rs:7834), which walks, hops, or takes a door
  in space.
  - On arrival she faces the wall and does today's `Act::Door { gap: u64::MAX }` at the space.
  - The cut still happens at once (design.md:2097). She gets up, says her line as she sets off,
    and walks.
  - Census cost: one short walk per school morning, against the stillness budget.
- **`go_to_work`, door arm.** The same `Job::Leave`, with today's gap. Work out at a screen edge
  is unchanged unless Q3 says otherwise.
- **`out_by_door`, `paint_empty`, Return, Dash, cold start.** Each takes `door_place` for this
  frame. Each arrival builds its terrain furnished with `ledger.home.project(..)` covers, the
  same projection `begin_visit` makes.
- **`door_rain`.** Rains the cells actually drawn, which are already frozen.
- **Stage `dash_through`.** Use the space too (a stage convenience, low stakes).

### 5.7 Deliveries (see Q1)

`doorstep` stands a parcel at `Anchor { side, offset: 0 }` and draws its flap at the wall column
`to`, rows `f-2..f` (room.rs:1536-1569). On the door's wall, that's the side-on door's own
column and rows, and the narrowed extent would also put the parcel 6 columns from its flap. The
recommendation is that `doorstep` skips the door's wall. Its edge-first sort (room.rs:1525)
still applies to the others.

## 6. Regression tests to write first (all fail today)

1. **Property test, the main one.** Extend `her_days_never_touch_what_is_protected`
   (tests/away.rs:1232), which proptests sizes, text, protection and owned pieces over
   `long_visit_of` (tests.rs:759), for `graphics in [false, true]`:
   - (a) In `State::Away`, the closed door's box (her box at the door spot, or the side-on
     door's cells once it exists) meets no `empty.shown` piece's `cover()`.
   - (b) While visiting, any routine or work door that's open (`osaka.door(now).is_some()`
     with `leaving`/`shift` set) meets no `visit.shown` cover.
   - (c) Whenever `door_place` returned the space, no shown piece's `rect()` meets the space,
     except hung pieces at or above `f-5`.
   - (d) In line art, `terrain::image(door, covers).with` is all false.
   
   Make sure the generator includes owning a bed, sofa or desk and school mornings. Otherwise
   (a) won't fail today.
2. **Unit regression, both modes.** She's in her bed (Nap or Sleep) at `tue(8, 14)`; run to
   `State::Away`. Assert:
   - the door is at the space's spot;
   - her box there misses the bed's cover;
   - the Return at 12:45 comes out of the space.
   
   Today it fails, with the door at the bed's sit column. Add a sofa (Lounge) variant and a desk
   (Homework) variant.
3. **room.rs proptest** next to `hung_pieces_clear_every_standing_piece` (room.rs:2383), over
   random homes and strips:
   - `layout`, `project`, `doorstep`, `spot` and every `rules::search` repair place no floor
     piece in the space (and no window meets it) whenever the strip packs with it;
   - every home that packs both raw and narrowed keeps its anchors and its order (a push, never
     a reorder or a closeting).
4. **Migration.** An old record (no `door` field) with a fridge at `Anchor { Right, 0 }` on the
   door's strip. Assert:
   - the fridge shows at the space's inner edge with the same anchor and isn't closeted;
   - `AgainstWall(Fridge)` isn't broken;
   - the record round-trips with `door` set.
5. **Yield.** A strip that holds its pieces only raw:
   - the layout is identical to today's;
   - the door falls back to a spot meeting no piece, or, with none, isn't drawn and the arrival
     waits.
6. **Rewrite** `tests/window.rs:207` (`her_door_at_her_sofa_keeps_her_window_behind_it`). It
   asserts a door forced over the sofa and window, which the rule now forbids. It becomes "a
   door forced onto her sofa stands in its space instead; its image takes in no piece".
7. **Keep** `tests/away.rs:466` as the fallback's test, with the strict predicate. Rename
   `tests/away.rs:376` to "her door stays in its space whatever takes her home away".

## 7. Order of work

1. **This batch: geometry.** The tests above first, then:
   - `Home.door` and `Home::extents`;
   - `door_place` with the strict fallback;
   - `Job::Leave`;
   - every external site in section 5.6;
   - the `Out` shape change;
   - logging.
   
   The **existing face-on door** stands at the space's spot `(to-3, f)`, facing the wall. This
   closes the bug class without waiting for art.
2. **Next batch: the side-on door.** Model sheet first (5.3). Then the new look, which moves the
   shut door from `to-3` into the wall column `to`.
3. **Optional, later:** a felt rule `Rule::DoorClear`, so that when the space keeps yielding she
   frees it herself with the phase-4 mending. Also clean up `door_away`.
4. **Docs.**
   - design.md: replace "where she left" (2103-2104) with the space rule from 5.1. Fix the false
     line at 1380-1381 (true only for doors in space). Update work by door (1734-1735) and
     deliveries (1754-1757).
   - decisions.md: a new entry, "Her door has its own space at the screen's edge (2026-10-07)",
     with this post-mortem (`restful` reused for the door) and the rejected alternatives:
     `blocked` (it closets pieces), a wall derived each frame (it hops with deliveries), and
     moving the room off its strip (too drastic).
   - Note that the user's call recorded at decisions.md:3334 ("closed where she went out") is
     superseded in *where*, not in *whether*.

The morning-routine idea (kitchen, bathroom) fits this naturally: a bathroom-then-breakfast
chain would end with `Job::Leave`, her walk to her door.

## 8. Questions for the user (the ones they've answered are dropped)

1. **Deliveries.** The flap's wall and her door's wall coincide, and so do their column and rows.
   - **Recommend (A):** parcels come through another wall, screen edge first.
   - (B) Through her door, side-on: the leaf opens and the box slides out. It's charming, but it
     needs a new beat and the new art, so it belongs to next batch at the earliest.
2. **Poster and clock above her door.** They hang on rows `f-6..f-5`, clear of a 4-row door.
   - **Recommend yes:** keep only the floor lane and the window out of the space. The
     alternative is the whole wall above it empty too.
3. **Work through her door.** Today she goes out at a screen edge if her floor reaches one,
   else by a door where she stands.
   - **Recommend yes, always by her door.** One external door reads clearly, and the edge exit
     looks like her leaving through the wall right beside it.

Not asked, decided here (veto welcome):
- ~~The space yields quietly; she doesn't mend it herself for now.~~ *(Superseded: she clears it herself, through a felt rule, "Can't get to the door!"; plan.md, "Next: the door batch".)*
- With no home, the door uses the same edge chooser, unsaved.
- The door shows only while she's out or going through. Its presence is the "out" cue the user
  wants, so it isn't a permanent fixture.
