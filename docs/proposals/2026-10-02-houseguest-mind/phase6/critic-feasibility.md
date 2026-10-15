# Phase 6a design: feasibility and step-ordering critique

Lens: can each step be one reviewable commit in its line budget; does each step compile and pass the gate on its own;
are the per-step "goldens byte-identical" promises believable (rng draws, decision order, image cache keys, ledger
bytes in a home with no structure); per-paint cost; what an implementer would have to invent; simpler alternatives.
Checked against the tree at `ad7f4af7` (`dessplay/` unchanged since `4c5c7151`). Paths are relative to
`dessplay/src/ui/houseguest/` unless noted.

**Spot-check of refs (about 90 checked):** almost all correct: mod.rs 160-245 (`CATALOGUE`, `advert`, `clock_on`,
`beauty_at`), 1486-1510 (`give`, `send_parcel`, `wishlist`), 1624, 2499-2507, 2515, 2534, 2649, 2724-2747, 2799,
2842, 3187, 3411/3432/3446, 3590-3596 (`Said`), 3642, 3688, 4003, 4120, 4156-4190, 4266-4287 (`middle_of`, `reach`),
4339-4341, 4537-4600, 5345, 5499, 5601, 5660-5805; room.rs 20-140, 296-356 (Spec rows, hangs), 410-440 (`Use`),
629-733, 741-799, 805-820, 823-915, 1225-1265, 1398-1420, 1491, 1546, 1631-1716, 1720-1774, 1779-1838, 1839-2010,
2321, 2419-2470, 2684, 2738, 2885, 3237-3256, 3429; ledger.rs 180-240, 355-375, 388-430, 545-592, 593-690, 859, 877,
1256-1264, 1345; rules.rs 19, 74 (`RULES: [_; 8]`), 191, 433, 548, 575, 605, 745-770, 837, 930, 1459, 1564, 2295,
2798; osaka.rs 29, 406, 1787, 4365, 7647, 8444, 9639, 10230; brain.rs 650 (`Want::ALL`: 29), 883-920 (`choose`:
whims keyed by name, so new wants that never bind draw nothing); terrain.rs 222-314; dissolve.rs 40-54, 220-225,
500-540; layer.rs 510-560. Verified feasible: `Terrain::read` has exactly six production callers, all in mod.rs (2515,
2649, 3446, 4120, 5716, 5727; everything else is under `#[cfg(test)]`, nothing in examples or other crates), so D4's
"`Terrain::read` becomes `#[cfg(test)]`" is a clean compile-enforced change. `gathered` (room.rs:2000) already
tolerates a furnished strip with no plan (a hidden shelf). `Want` offers are only wants with a method that binds
(osaka.rs:8662-8667), so `Want::Build` and `Use::Admire` without a shelf change no decision. The mend basis hash
(mod.rs:4441) changes *value* once `Home` gains `structures` (`Vec::hash` writes the length even when empty), but it's
only compared for equality and isn't traced: harmless. D3's numbers check out against the Spec rows (`HIGH_RISE` =
hang 4 + 2 rows + 1 = 7; tallest standing piece 4 rows; `Extent::holds` keeps the 3-tall plant off a 2-clear low
shelf). Wrong or incomplete refs are in F9 and F21.

## Blockers

### F1 (blocker) Tests / D7 / D9: growing `Furniture::ALL` and `CATALOGUE` changes existing proptest strategies, and generates unboxed kits
The design's rule is "**No existing proptest's strategy changes** (their `cc` seeds would replay as other cases)". But
step 5 appends `ShelfKit` and step 7 `Gremlin` to `Furniture::ALL` (and the kit, then the gremlin, to `CATALOGUE`),
and these existing strategies are built on them:
- rules.rs:1459 `pieces()` and rules.rs:2798 `pieces3()`: `subsequence(Furniture::ALL.to_vec(), ..)` (they feed
  `rules_hold_across_a_resize_and_text`, `every_repair_mends_and_breaks_nothing` and the repair-search properties);
- room.rs:3237 `standing()` (ALL filtered by `hang.is_none()`, which both new kinds pass), feeding `pieces()` and
  `decorated()` (room.rs:3248, 3256);
- tests.rs:5656-5658 `every_parcel_on_her_doorstep_gets_unpacked`: `0usize..Furniture::ALL.len()` and
  `0usize..=CATALOGUE.len()`.

Committed seeds exist for all of them (`dessplay/proptest-regressions/ui/houseguest/{rules,room,tests}.txt`), so the
regressions replay as other cases from step 5 on. Worse, the strategies generate **unboxed kits**: rules.rs's
`boxed` is `weighted(0.15)` (85% unboxed), room.rs's `placed` is always unboxed, tests.rs's `owned` goes through
`Prop::new` (unboxed). With D2's `Home::add` debug-assert, `rules_hold` panics at `prop_assert!(home.add(p))`
(rules.rs:1576; also 3102), so step 5's gate can't go green as written.
**Amend:** pin the existing strategies to the kinds they were written for: a `#[cfg(test)] const` of the 12 pre-6a
kinds (or `&Furniture::ALL[..12]`, `CATALOGUE[..10]`), named in step 5's sites with the reason, so they stay
byte-identical and their seeds keep meaning what they meant; the gremlin and the kit get their own new properties
(the design's "shelves are new proptest functions" rule, applied to kinds too). Also see F3 for the debug-assert.

### F2 (blocker) D7: the kit's gated unpack seat contradicts `every_parcel_on_her_doorstep_gets_unpacked`
That property (tests.rs:5632-5660) states, for "each piece the channel sells (on order)", that once delivered it has a
seat to unpack it from *at once*, unpacking is on offer at every choice, and it's unpacked within two minutes. D7 offers
the kit's unpack seat "only on a frame where `shelf_spot` finds a spot", and its random screens (`(60..130, 18..45)`,
random text) mostly have no 9-row pane with blank rows. So once the kit is in `CATALOGUE` the property is false by
design, not by bug; F1's pin hides that rather than resolving it.
**Amend:** say in D7 (and design.md *Getting furniture*) that a kit is the one parcel exempt from "unpacked at once",
and why; keep `which` pinned to the pre-6a catalogue (F1); and add a kit-specific property: a delivered kit's unpack
seat is offered iff `shelf_spot` finds a spot on that frame, and when it's offered the property's other promises
hold.

## Majors

### F3 (major) D2/D7: "a kit is never an unboxed prop" has more construction sites than `from_json` and `Home::add`
Unboxed props of any kind are built at: `doorstep`'s scratch piece (room.rs:2448-2462, `boxed: false`: "the piece,
standing there or hung above, must fit with room to use it"), `spot` for stage gifts (room.rs:2351), `Prop::new`
(room.rs:721-733: `from_json`, `place_gift`, tests), `Guest::give` (mod.rs:1486: the stage can give any `Furniture`,
unboxed), and `Home::unbox` (reached by `HomeEvent::Unpacked` and the stage). A debug-assert in `add` catches only the
paths that call `add`; the doorstep's scratch check would judge a 5×2 flat pack standing unboxed, which means nothing.
**Amend (makes it unrepresentable in effect):** one accessor every reader goes through, e.g. `Prop::boxed()` /
`Shown::boxed()` returning `self.boxed || self.item.spec().kit.is_some()` (read by `lane`, `lift`, `needs`, `uses`,
beauty, `piece_look`), and `Home::unbox` returning false for a kit. Then no path, scratch or stage, can show or judge an
unboxed kit, and `doorstep` judges a kit as its box only. Drop the debug-assert (or keep it as documentation only).

### F4 (major) Step 5 / D7: the art lints need an unboxed drawing of the kit
`every_prop_renders_inside_its_footprint` (art.rs:4325-4337) iterates `Furniture::ALL` and asserts
`render_prop_layer(item, Layer::Whole, ..)` inks more than a quarter of the footprint. D7 gives the kit "`art::parts`
arms (boxed only)" and says it's "never drawn unboxed", so the lint goes red in step 5 with no stated fix (the same
loop shape is in the ignored review sheets, art.rs:3406 and 4384-4391). The ASCII lint (room.rs:2885) is handled; this
one isn't.
**Amend:** either exempt kinds with `Spec.kit` from that lint (and the sheets), saying why, or give the kit a flat-pack
`Whole` drawing (a few rectangles in `props.svg`), which also gives the stage something sane to show if F3's accessor
is ever bypassed. Name the choice in step 5's sites.

### F5 (major) D3: the shelf's geometry depends on her door; a margin of `SPACE` removes the coupling
`Shelf::run(host, door)` must miss "the columns of her door's space on this strip wherever the space can exist, kept
or yielding". That reads `Home.door`, which is unsaved (`choose_wall` each frame) until `settle_door` (room.rs:1787)
and moves with her pieces (`move_off`, room.rs:1785-1786 doc); and "can exist" is ambiguous: `space_of`
(room.rs:1232-1244) also depends on `widest_standing` on the host. So a shelf put up while the door was on another
pane, or unsettled, or "couldn't exist", can later be hidden for good by a door that settles or moves onto its side;
and every shelf strip's layout gains door inputs (`strip_door` for a shelf, D6; `StripLaid`/`relaid` recording them).
**Simpler alternative:** keep every run at least `max(WIDTH, SPACE)` = 6 columns from **both** walls (the space is
`SPACE` columns against its wall: `space_rect`, room.rs:1246-1258). Then no door space on either side can ever meet a
shelf, whatever the door does: `Shelf::run` takes no door, the shelf strip's layout has no door input, the "no shelf
above her door's space" rule holds by construction, and the door test in `a_shelf_hangs_as_high_as_fits` becomes a
one-line margin check. Cost: one column less of choice per side.

### F6 (major) Step 5 is three commits' worth (~1550 lines)
It carries: the kind and two `Spec` fields; `CATALOGUE`/`advert`/`shelf_room`; `shelf_spot` (both heights, the
free-cells-upward pass, ordering rules: pure geometry with its own proptest); `Home::raise`, `record`, `furnish`, the
unpack-seat gate; `Want::Build`, the method, `Job::PutUp`, and `Act::PutUp` (a new `Act` variant meets the exhaustive
matches at osaka.rs:1003, 1229, 1282, 3516, 4864, 5569, 8078, 8132, 11247 among others); `paced` and `Said` → `Dated`
moved to ledger.rs; three stage scenes; a census. Two reviewers can't tell a scenes typo from a pacing bug in that.
**Amend:** split: **5a** `shelf_spot` (pure; `a_shelf_hangs_as_high_as_fits`; nothing user-visible; could even sit
after step 4, which it depends on for the low block); **5b** the kit: kind, channel, delivery, unpack → `raise`
(`built: 0`), `Scene::Build`'s setup; **5c** building: want, method, job, act, pacing, `Dated`, census, CHANGELOG. With
no push of master mid-phase, the intermediate "kit raised, never built" tree is fine.

### F7 (major) Step 2 mixes a mechanical ripple with the behaviour (~1300 lines)
The `&Written` threading (`read_guarded`, `nudge_due`/`send`, `furnish`, `paint_empty`, the six `Terrain::read` sites
→ `#[cfg(test)]`, the four `under` sites) is a behaviour-free change; `read_ground`'s body, the per-cell test, the
`STRUCTURE` flag, rains, the goodbye and the hiding tests are the behaviour. The door batch's critique (F3 there) asked
for exactly this split for the same reason.
**Amend:** **2a**: `Structure`/`Shelf`/`Run`, the ledger, `Written` with `none()` threaded everywhere,
`Terrain::read` → `#[cfg(test)]`; goldens byte-identical by construction. **2b**: `read_ground` and its tests. Also note
in D4 that it overrides plan.md's brief, which puts `read_ground` "right after `layer.validate`" ("plan.md wins over
everything here"), and amend `TextLayer::validate`'s doc ("call before anything of hers is painted", layer.rs:513-515)
along with the mod.rs comments D4 lists.

### F8 (major) Tests: "every frozen cell's `under` equals `real`" is false today, without any shelf
`long_visit_of`'s new check (Tests, *Long visits with shelves*) can't pass: `under` is taken from `buf` as painted so
far, which holds her earlier pieces and moved text, not the real frame: `draw_props`' `unders_of` (mod.rs:5345) in
back-to-front order sees the pieces drawn before it; `draw_art`'s `under` closure (mod.rs:5499) and ASCII `draw`
(mod.rs:3871) see every piece drawn before her (furniture isn't solid to her: she walks in front of it). So the step
that adds it goes red for reasons unrelated to shelves, and an implementer must invent the narrowing.
**Amend:** "every frozen cell **at a written position** (`Written::contains`) has `under == real`". Add mod.rs:3871
(ASCII `draw`) to D4's list of `under` sites; D4's "two frozen cells at one place never disagree" holds only at
written positions, so say that. And D4's "in 6a her box never meets a shelf" is true of pane floors only: she can stand
on an in-pane `─` run (UI separators; the long-visit text generator draws `─`) a few rows under a shelf, so the
`under` fix is live in 6a, not only a 6b guard. Fine, since it's built anyway; reword.

### F9 (major) D11 / step 5: the stage-room facts are wrong, and the fallback is left to the implementer
- At 100×30 in the bundled layout (`snapshots/*default_layout_terrain_100x30.snap`), **two** panes are 9 rows tall: The
  List (rows 0-8) and **the Playlist** (rows 17-25, `[Add New]` on its first inside row), not "only The List".
- `every_scene_has_a_spot_in_the_stage_room` is at tests.rs:2355, not 1762, and loops `(100, 30)` **and** `(80, 24)`;
  at 80×24 no pane is 9 rows (List 6, Users 7, Playlist 7), so `Scene::Build`/`Shelf`/`Admire` can never have a spot
  there.
- "Runs these three on the smallest bundled frame step 3 measures" asks the implementer to build a per-scene frame
  switch the test doesn't have.

**Amend:** follow the `LookOut` precedent in that very test (tests.rs:2371-2384: where no pane can hold the window,
the note is `Err` and the test asserts so and asserts the frame is the small one). Where no pane takes a low shelf by
geometry, the three scenes' notes are `Err`, asserted. At 100×30, name the pane now (List or Playlist, whichever
`stage_ui`'s content leaves blank; check it in step 0, not step 5). The census rooms are already right: `rooms(100, 30)`
gives Users 11 rows inside (exactly a high shelf), `home_screen()` 9-row panes (low).

### F10 (major) Step 4's byte-identical promise rests on `hung_clear`'s generalisation being an exact identity
D3a turns `hung_clear` (room.rs:2074), which today moves only the window, into a search over every hung piece with
blocks as obstacles. With no block, it must leave the poster and the clock exactly where they are (they hang over
standing pieces by design) and the window exactly where it went. The goldens barely exercise hung pieces (the clock
gift in the fed goldens, mod.rs:4156ff, at most), so "goldens byte-identical" is weak evidence here.
**Amend:** step 4's exit gains an explicit identity property over `decorated()` homes (the pre-6a kinds, F1) with no
shelf: `shifts` lists only the window, and the layout equals the one before the step (keep the old `hung_clear` as a
`#[cfg(test)]` oracle for the step, deleted in step 8, or pin today's outputs on the existing wall-lane cases).

## Minors

### F11 (minor) Step ordering: `takes` before `Spec.shelf`, `LOW_RISE` before step 4, `paced` in two steps
- `Strip::takes` (step 1) means "a shelf takes small decor only", which is `Spec.shelf` (step 3); `decor()` won't do
  (the poster and the window are decor). Step 1 would ship a placeholder and step 3 rewrite its unit table. Move
  `Spec.shelf` (one field, twelve rows) into step 1.
- `LOW_RISE` is read in step 2 (the ledger skips a rise under it; D3's geometry `LOW_RISE <= rise`;
  `rises_clear_every_hung_and_standing_piece` computes it) but listed as added in step 4. Add it in step 2 (a
  hand-placed low shelf in steps 2-3 just lacks its block, which is harmless in-tree), or have the ledger read only
  `rise >= HIGH_RISE` until step 4. Say which.
- D2: "Step 4 adds `paced`"; step 5 owns it.

### F12 (minor) D3a: say the block is geometry only
It isn't stated whether a low shelf hidden this frame (text over it, D3's per-cell test failing) still blocks the wall
lane. If the block followed the per-cell test, the pane's layout would read the frame's text, and `relaid`'s inputs
with it; layout is "pure geometry: text over a piece doesn't take it out of the layout" today (room.rs:1884-1886).
**Amend:** the block exists whenever the shelf's D3 geometry holds (from `built: 0`), whatever the cells.

### F13 (minor) Per-paint cost: gate the new reads; the perf test can't see a shelf
- `read_ground`'s "not in a laid piece's cover" needs `laid_and_shifted` every paint: keep it behind
  `structures.is_empty()` (as the design says, measured in step 2).
- `Chances` is rebuilt every paint (mod.rs:2999-3021), and `advert`'s `shelf_room` and the kit's unpack-seat gate in
  `seats` (mod.rs:4537, no `buf`/plan today: it needs a precomputed flag) would each run `shelf_spot` (a layout plus
  a cell pass over every pane) every paint if done naively. Compute `shelf_room` only when `advert` is due and idle and
  its queue resolves to the kit; the seat flag only while a boxed kit is shown.
- The perf test the design cites isn't `tests/perf.rs` in `dessplay/` but
  `dessplay-rendezvous/tests/perf.rs::houseguest_visit_cpu_is_negligible`, which runs a guest with no shelf: it can't
  measure any of this. Add an ignored timing test beside `repair_search_is_cheap` (rules.rs:2295): `read_ground` plus
  `shelf_spot` on a 400-wide, dense, three-pane frame, with a bound.
- `repair_search_is_cheap` itself is safe: a 10-wide shelf adds at most a handful of lefts per small-decor piece per
  pane.

### F14 (minor) D1: `evaluate`'s tier keys on the strip
rules.rs:757-765: an unsettled piece moved to "a strip with nothing on it" is tier 2, and `role_without`/`role_of(laid,
to.strip)` judge the destination strip as a room. An empty shelf is such a strip. D1's *Rooms are floors* list (which
names `role_among`, `rooms`, `has_room`/`becomes_den`, `Before.roles`) doesn't mention it. Say whether the tier keys on
`to.strip.room()` or deliberately stays per strip (shelf moves stay rarer, matching "rare in practice"). The `shifted`
cost (rules.rs:768-775) also counts the rise in rows; fine, but say so.

### F15 (minor) D7/D9: the wishlist when the wished kit can't be sold
With a wish and no `shelf_room` on this frame, does `advert` sell the next catalogue piece, or nothing this visit? An
implementer must choose; the user-visible difference is whether the gremlin's wish blocks other shopping.

### F16 (minor) D7: building on a hidden shelf
`Want::Build` is offered while today's stage isn't done. If the built half (or the next half's cells) is hidden by
text, a completed act commits a stage nothing shows. Offer `Build` only while the next half's cells pass D4's per-cell
test on the frame (the same test `shelf_spot` uses).

### F17 (minor) Dead code under `-D warnings` between steps
Items introduced before their production reader trip clippy's `dead_code` in the gate: `HIGH_RISE`/`LOW_ABOVE` before
`shelf_spot` (step 5, or 5a with F6), any `Written` accessor only tests read, the block before step 4. Each step: a
production reader, or `#[cfg(test)]` until there is one.

### F18 (minor) D4: `read_ground` in frames that then show no home
`read_ground` runs at the top for `Arriving`, `Away` and `Visiting`, before the arms decide: an `Arriving` that fails
(`!present()`, mod.rs:2570-2587) or an `Away` too small (mod.rs:2604-2615, `vanish`) leaves the shelf written for one
frame with nothing in `painted` to rain it. Skip writing below `MIN_WIDTH`/`MIN_HEIGHT`; accept (and say) the failed
arrival's one frame, or write there only after `begin_visit`.

### F19 (minor) D: `SHELF_INK` from `style.css`
A fixed grey shows the shelf in another colour than the user's themed borders. Simpler alternative with the same
cache behaviour: take the host pane's floor-row cell's `fg` on the frame, as `wall_glyph` takes the wall's
(mod.rs:5166-5167). Her image keys already hold those border colours, so it adds no new key. If the fixed ink stays, it
needs a truecolor/16-colour pair (as `Spec.ink` has).

### F20 (minor) D1: `DoorWall`'s `try_from` guards only deserialization
Code can still build `DoorWall { strip: Strip::Shelf(..) }`. The unrepresentable version is `DoorWall.strip: Nook` in
memory with a serde `with` that keeps the wire form `{"Bottom": ..}` (ledger bytes unchanged). It touches every
`d.strip == strip` comparison, so it's a trade-off: say why the guard is enough if you keep it.

### F21 (minor) Refs and naming
- tests.rs:1762 → tests.rs:2355 (F9).
- D8's script arms "525, 642, 744, 1453, 2465, 2512, 2546" are `Unpack`'s lines; `LookOut`'s arms are at script.rs:469,
  500, 539, 584, 619, 643, 685, 724, 758, 845, 1458, 2479, 2517, 2558: fourteen sites, not seven. The compiler finds
  the exhaustive ones; the or-pattern arms (`| Self::LookOut`) are the ones to read.
- dissolve.rs:513 already has a local `written` (cells the rain has written this frame); a `Written` type in the same
  module family invites confusion: consider `Ground`/`Raised` or rename the local.
- `Summary.owns` (`Owned.strip`, ledger.rs:464-473) prints each piece's strip: a shelf piece's line is a dump.rs-visible change; covered by
  "skipped when empty" only for `structures`, so say what `owns` prints for a shelf piece.

### F22 (minor) D2's older-build note is true only on the living-room pane
An older build places a piece whose anchor it can't read on its kind's room pane from `rooms` (ledger.rs:546-562,
`RoomKind::of`: Living for the plant and the clock), not on the shelf's host. "Lands under where the shelf was" holds
only when the host is the Living pane; elsewhere it lands on the Living pane at that share. Reword.
