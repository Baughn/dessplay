# Phase 6a design: critique, mechanics (correctness against the code)

**Critic, 2026-10-10.** Lens: is the model right against the code as it stands: sibling sites the design misses,
paint/tick ordering, states left representable that shouldn't be, and whether the design's file:line claims hold.
Every claim below was checked against the tree at `ad7f4af7` (`dessplay/` unchanged since `4c5c7151`). Paths are
relative to `dessplay/src/ui/houseguest/`. Severities: **blocker** (the design as written reintroduces the bug or
breaks a user decision), **major** (a sibling site or lifecycle path left broken, or a state the design means to rule
out that stays representable), **minor** (an unclear spec, a wrong claim, or a naming or compile trap).

No blockers. Six majors, eleven minors.

## Major

1. **[major] D4: `read_ground` keys on the state at the top of `paint_state`, but the state changes within the paint.
   On those frames the shelf doesn't rain, and pieces on it lose their floor mid-goodbye.**
   - D4 runs it once, after `observe`/`gate`/`whole_glyphs` and before `errand_progress`/`nudge_due`, "for `Arriving`,
     `Away` and `Visiting`", and says "`Leaving` writes nothing". Several transitions to `Leaving` or `Absent` happen
     after that point, in the same paint:
     - `errand_progress` (mod.rs:2505) → `self.leave(now)` (mod.rs:3403): a visitor's errand ends.
     - The Arriving block: `call_off` (mod.rs:2585) → `rain_home` (mod.rs:1858) → `State::Leaving`; or `vanish`
       (mod.rs:2580).
     - `out_by_door` with `!stands` → `self.leave(now)` (mod.rs:2359).
     - The Visiting arm's "no room" `vanish` (mod.rs:2907), and the Away arm's (mod.rs:2613).
   - Failure (normal goodbye path aside): a visitor's errand is done and `errand_progress` calls `leave`.
     1. `read_ground` has already written the shelf this frame.
     2. The Leaving arm's `dissolve.paint` judges each frozen shelf cell (`under` = the real blank) against the live
        `buf`. That cell now holds the `─` that was just written, so `live != under` settles it at once, and the settle
        is sticky (dissolve.rs:485-490, 519-545).
     3. The shelf never rains. It shows this frame and is gone the next.
     4. From the next frame on, `paint_prop_art` (mod.rs:2662) redraws the pieces on it (`leaving.props`) over a
       blank floor row, which line art refuses. The plant and the gremlin pop out before the rain reaches them.
     5. That breaks the record's own rule, "Nothing of hers just vanishes" (mod.rs:2336ff).
   - The same happens through `call_off`'s `rain_home`. `vanish` leaves a one-frame shelf with no home.
   - The converse also happens: `send` from `Absent` (mod.rs:3456-3466) begins a visit after a write that wrote
     nothing. That frame `furnish`'s `fits` closets every piece on the shelf (its floor row isn't drawn), and
     `note_closet` logs a change at info. The shelf and its decor pop in a frame late.
   - `send`'s own read (mod.rs:3446) is effectively immune: `accordion_spot` reads only the chat's accordion. D4's
     "closes the map's three missed sites" overclaims there. Taking `&Written` is still right, since `begin_visit`
     hands that terrain on.
   - **Amendment:** keep the single write at the top (the arrival's reads at 2515/2534 need it, or D5's no-ledge rule
     is lost there). Add `Written::undo(buf)`, which puts each written cell's `under` back. Every arm that doesn't own
     structures applies it before it reads `buf`: the Leaving arm, the Absent arm, and the `vanish` returns. Then
     "Leaving writes nothing" holds for the frame, whatever state the frame began in.
   - **Test:** `an_errand_goodbye_rains_her_shelf_out` (a non-resident errand ending with `leave_after`, a finished
     shelf with the plant). Assert every frozen shelf cell rains (is painted at least once after
     `RAIN_FROM_MS`), not settled on the first frame. Add the same assertion for an arrival called off from her empty
     home. It must be red under the design as written.

2. **[major] D4 / Tests: the "`under` through `Written::real`" rule is stated for every cell, but the dissolve stacks
   cells. Its order in the painted list is unspecified, and the proposed long-visit assertion fails on today's tree.**
   - `Dissolve` keeps several frozen cells at one place. The **first** has the real UI's `under` (`first_at`,
     dissolve.rs:106-110). Each later cell's `under` is what the frame showed beneath it, and `paint` judges it
     against what the earlier one wrote (dissolve.rs:513-520).
   - Today's ASCII sprite relies on that. `draw` takes `under` from `buf` after `draw_props` has drawn the pieces
     (mod.rs:3127 then 3170; the `unders` at 3865-3884). Her glyph over the sofa she lounges in has the sofa's glyph as
     its `under`, not the real cell.
   - So the test bullet "every frozen cell's `under` equals `real`" (Tests, *Long visits with shelves*) is red on
     today's tree, for a reason unrelated to shelves.
   - D4's four sites are in fact exactly the **burst** sites: the `under` closure (mod.rs:5499, her box and doorway),
     the `with` pieces (5601), line-art `unders_of` (5345) and `draw_front` (5805, labelled "`paint_empty`'s"; it's
     shared with the visit at 3138). Only burst cells are read by `unchanged` (dissolve.rs:220-225). So the rule is
     right for those cells and wrong as worded.
   - The order isn't stated either. "Join `visit.painted` (mod.rs:3187)" reads as appending: today the order is
     layer, flap, props, front, her (3116-3187). Appended last, the shelf cell sits on top of her cell at the same
     place:
     - In ASCII, her sprite cell (first, `under` = `─`) settles on the goodbye's first frame, and the shelf's
       `─` replaces a glyph of her during the wave.
     - In line art, the shelf cell under her box settles when her burst noise is written.
   - **Amendment:** structure `Frozen` cells go **first** in `visit.painted` and `empty.painted`: they're the bottom
     layer, written before everything. Word the rule as "every **burst** cell's `under` reads through
     `Written::real`; other cells keep the cell they were drawn over". Write the long-visit check as two
     assertions:
     - every burst cell's `under` equals `real`;
     - the first frozen cell at each place has `under == real`.
   - **Alternative (one site):** have `unchanged` compare each burst cell with the `under` of `first_at` at its place,
     and keep every recorder as it is. That also covers the moved-text case the map raised (map-terrain §3), and
     makes `Written::real` a reader for `shelf_spot` only.

3. **[major] D4 / D5: "In 6a her box never meets a shelf (even a low one hangs a row above her head), so this is a
   guard for 6b" is false. The `under` fix is a 6a requirement.**
   - A `─` run in a pane's text with four clear rows above it is a platform (`ledge` from the glyph, terrain.rs:263;
     `add_platforms` 320). Text `│` columns are poles for a `Climb` (`find_links`, terrain.rs:518).
   - So her box can cross a shelf's row in two ways: standing on a text rule inside the host pane, or climbing a
     table's `│` through it.
   - The long-visit proptests' alphabet already has both: `"[a-z漢─│ ]{1,6}"` (tests.rs:708).
   - Only the side-border poles are kept away, by the `WIDTH` margin: a pole at `from - 1` puts her box at most at
     `from + 4`.
   - Her fallback door has the same gap. `on_floor` (door.rs:353-367) refuses only `WALL` glyphs in the box, so a
     face-on door can stand with its box over a shelf's `─`. `draw_front` then draws over the shelf, since its cells
     are `open`.
   - **Amendment:**
     - Drop the claim. Test the `under` rule in `long_visit_of` with shelves under real conditions: text rules and
       `│` columns in the host pane. The hand-built 6b dissolve case is not enough.
     - Either keep the fallback door's box off structure cells (a `STRUCTURE` flag readable through `Ground`, so
       the door.rs test impls at 532/581 need it), or state that her door may stand in front of her shelf.

4. **[major] D3 / D6: the door-space rule depends on her furniture, feeds back into D3a's blocks, and the door
   chooser can move her door under a finished shelf, which is then hidden for good.**
   - `Shelf::run(host, door: Option<Space>)` and "wherever the space can exist … (`space_of`)" tie the rule to
     `space_of(raw, side, self.widest_standing(strip))` (room.rs:1723, widest at 1688-1695). That is `None` while the
     host's widest standing piece doesn't leave `SPACE` columns.
   - Furniture-dependent: a shelf chosen while a 9-wide sofa made `space_of` `None` may take offset 5 on the door's
     side. When she later moves the sofa (a repair), the space exists and the run meets its columns. From then on
     the shelf is hidden because a piece on the floor moved. D6's `relaid` inputs for a shelf strip don't include the
     host's widest piece, so a reused shelf layout goes stale too.
   - Circular: the `Space` that `plan_strip` builds carries `kept`, which comes from `lay_strip`. In step 4 that
     reads the low shelf's block, which comes from `run`, which needs the `Space`.
   - Sibling sites: `choose` (room.rs:1274-1321) and `move_off`'s door relocation (room.rs:2186-2192: "her door goes
     with them … to the target's wall `choose` picks") know nothing of shelves.
     1. The host pane of the door's strip is hidden for a moment, so `move_off` carries her pieces and her door to
        the shelf's pane.
     2. The space now meets the shelf's run, so the shelf is hidden.
     3. 6a never re-hangs a shelf, and the door batch keeps a moved door. The shelf she bought and built stays hidden.
   - **Amendment:**
     - The run's keep-out is geometric: the `SPACE` columns against the door's wall whenever `home.door` is on the
       host, `space_of` or not. `shelf_spot`, `run` and the test all compute it that way (the test already does).
       That removes `Option<Space>` from `run` and with it the cycle.
     - The chooser and `move_off`'s door target refuse, or rank last, a wall whose `SPACE` columns meet a placed
       shelf's run.
   - **Test:** `hiding_a_pane_never_moves_her_door_under_her_shelf`. Also add a case where moving the sofa off the
     host doesn't hide the shelf.

5. **[major] D3 / D6: a piece can show on a shelf that is hidden. `fits` reads the glyph, not the structure.**
   - D3 hides a shelf "when … every cell … passes D4's test" fails, but its strip stays in `Home::extents`: D6 drops
     a shelf strip only when the geometry fails. So its pieces are still laid on row `y`.
   - `fits` (room.rs:2839-2860) accepts any `strokes(c)` glyph in the floor row.
   - Failure:
     1. The host pane's text draws a `────` rule across the run. A table border, a separator line or a `├──` tree
        will do.
     2. The per-cell test fails (the cells aren't blank), so nothing is written.
     3. The plant laid on the shelf strip has blank body cells and the app's own `─` under it. It passes `fits` and
        shows standing on the app's text, where no shelf is drawn.
   - That is "a shelf hidden while a piece stands on it" in the user's sense.
   - The proposed check in `long_visit_of` ("accepts a floor row that is a written cell") doesn't catch it if it
     reads "a real stroke **or** a written cell".
   - **Amendment:** a piece on `Strip::Shelf` stands only on written cells. Either `project_with`/`fits` take
     `&Written` and require every floor cell of a shelf piece to be in it, or `raw_extents` takes the frame's
     standing shelves and drops a shelf strip that isn't written this frame. The first keeps the layout pure, and the
     rules judge "up there" while it's hidden, as D9 wants.
   - **Test:** a shelf piece over a host row of `─` text is hidden; long visits assert a shelf piece's floor cells
     are all written.

6. **[major] D7 / D9 / Tests: "No existing proptest's strategy changes" can't hold. `Furniture::ALL` grows, and an
   unboxed kit, ruled out by D2, is generated by existing tests and the stage.**
   - Domains that change with `ALL`:
     - `proptest::sample::subsequence(Furniture::ALL.to_vec(), ..)` (rules.rs:1459 `pieces`, 2798 `pieces3`);
     - `0usize..Furniture::ALL.len()` (tests.rs:5656, `every_parcel_on_her_doorstep_gets_unpacked`);
     - `standing()` (room.rs:3238);
     - `Furniture::ALL.filter(!owns)[which.index(..)]` (room.rs:4519, 5264).
   - So their saved `cc` seeds replay as other cases. Index ranges like `0usize..4` into `ALL` (tests.rs:716ff,
     away.rs:1367ff) stay stable only if the new kinds are appended last; say so.
   - These generate an **unboxed `ShelfKit`** prop. In room.rs:4519 `spot` builds `Prop::new` (unboxed) and then
     `with.add(prop)`, which D2's debug-assert turns into a panic. `pieces()` produces "boxed or not" kits anchored on
     floors.
   - The stage does the same: `give`/`wishlist()` (mod.rs:1486, 1508) reach the kit once the first twelve are owned.
   - Lints that iterate `ALL` and must handle a kit:
     - `every_prop_renders_inside_its_footprint` (art.rs:4327), which renders it unboxed;
     - `every_parcel_and_channel_renders_inside_its_box` (art.rs:3357);
     - `lean_rests_the_flap_on_the_parcel` (art.rs:4236);
     - `every_loss_is_listed` (mind.rs:1708);
     - the `ANY_USE` lint (rules.rs:1447);
     - brain.rs:995, tests/credit.rs:129.
   - **Amendment:** `Furniture::PIECES`, today's twelve (the gremlin added for its own new tests), is what every
     existing strategy, the stage helpers and the "real piece" lints iterate. A kit is a `Furniture` only for the
     channel, `ordered` and the parcel (`KITS`). Name the art lints' kit arm (parcel only). Restate the Tests rule:
     appending to a strategy's domain is a strategy change.

## Minor

7. **[minor] D3 / D9: a shelf the chat comes to meet is "hidden, never moved", but InChat, judged on the layout,
   moves its decor off.**
   - The chat isn't part of the shelf's geometry (`extents` "needs no screen and no chat", room.rs:1711). After a
     layout change, the strip stays in the plans, its pieces are laid in the chat, InChat breaks, and a repair carries
     the plant down for good.
   - This only arises with custom or overlapping layouts (as door finding 27 did).
   - **Amendment:** InChat's judge and `closetable` skip `Strip::Shelf` pieces (they're hidden with it), or the shelf
     strip leaves the plans when its run meets the chat. Say which.

8. **[minor] D1: `evaluate` also lanes by the old strip.**
   - D1 names the search's `plan.along(old.lane())` "(rules.rs:932)". It's at 935, and `evaluate` repeats it at
     rules.rs:626 (`e`) and 634 (`pin`).
   - It's harmless onto a shelf (a shelf's `raw == floor`). It's wrong for a move from a shelf to a wall on her door's
     strip: the anchor is pinned on the narrowed floor, up to `SPACE` columns off.
   - **Amendment:** list both sites, and use the candidate strip's lane.

9. **[minor] D1: `reach` isn't the only reader of a piece's standing row.**
   - `middle_of` (mod.rs:4266) gives the shelf row to `reach`, and also to `set_down_refused`'s way back
     (mod.rs:4079) and `Judged.home` (mod.rs:4316). A refused set-down on a shelf would send her to a row that's no
     platform.
   - **Amendment:** read `stood` in `middle_of`, not in `reach`.

10. **[minor] D8: `Admire` comes from the strip, but the "which kind offers this use" lookups read `spec().uses`.**
    - `Guest::cue`'s `_` arm (mod.rs:1541-1548), `Scene::furniture()` (stage.rs:296) and brain.rs:995 find no piece
      for `Admire`. So `Scene::Admire` must set up its own shelf and decor; say so in D11.
    - The `ANY_USE` lint (rules.rs:61, 1430-1445) forces a choice. Admire is either felt-on for Belongs, DoorClear and
      InChat or it isn't. State it (it can never mend one of them from a shelf).

11. **[minor] D2: the older-build claim is wrong about the pane.**
    - An older build drops the whole `SavedAnchor` it can't parse (`object::<SavedAnchor>(v)?.ok()`,
      ledger.rs:180-184). It then places the piece on its **`RoomKind` pane** first (`rooms`, ledger.rs:200-206): for
      the plant, the clock and the gremlin that is the Living room's pane, the sofa's, TV's or cat bed's. It stands
      at its `at` share of **that** pane, so it lands "under where the shelf was" only when that pane is the host.
      The test's own name says this (`…falls_back_to_the_rooms_pane`).
    - Fix the claim, and fix `an_older_build_reads_past_the_structures`'s expectation.
    - In the new build, `from_json` must read `structures` before props, to validate a shelf anchor. Say so in step 2.

12. **[minor] D2 / D3 / Steps: the ledger rules and the step plan disagree.**
    - The ledger skips "a rise of 0" (D2) or "a shelf whose rise is under `LOW_RISE`" (D3).
    - `paced` lands in "step 4" (D2) or step 5 (D7, the step-5 sites).
    - The per-frame `rise <= host.rows` admits a shelf at the top interior row with no rows above it. It's drawn
      with nothing able to stand on it, while D3 says "one that shrinks hides it".
    - **Amendment:** one rule (under `LOW_RISE` skipped; hidden unless `host.rows - rise >= LOW_ABOVE`), and `paced`
      in step 5.

13. **[minor] D7: a build stage is offered and committed without the next half's cells, and nothing keeps the unbuilt
    half clear.**
    - `Chances.build` is "the spot under the next half … where she may stay"; the run's cells aren't checked. If
      they're under text when `Built` commits, `built = 2` hides the **whole** shelf, the first half too: the act
      was mimed for nothing.
    - D4's keep-out (`base`, made keep) adds only cells actually written, so the unbuilt half and a hidden run are
      open to her own text moves and makes. That's rare, since it needs a text ledge in the host pane.
    - **Amendment:** offer `Build` only while the next half passes the per-cell test, and re-check at `Built`
      (uncommitted if not). Keep-out takes the whole placed run of a shelf whose geometry holds, written or not.

14. **[minor] D7: the unpack-seat gate has a second caller, and `raise` crosses frames.**
    - `doorstep`'s `seats` closure in `furnish` (mod.rs:4125, `seats_of(piece, &room, terrain, true)`) asks for the
      parcel's seats too. Say whether a kit's delivery waits on `shelf_spot`; if not, the gate is asked by
      `spots_for` only.
    - `every_parcel_on_her_doorstep_gets_unpacked` (tests.rs:5632) will fail for a kit whose seat is gated; say how.
    - `record` runs twice a paint (mod.rs:2756 and 3189, the second after `furnish`), so `Unpacked(ShelfKit)` can set
      `visit.raise` after this frame's `furnish`. State that `raise` persists on `Visit` until the next `furnish`, and
      what happens to it if the visit ends first: the box stays shut and she unpacks it again.

15. **[minor] D7 / D9: the channel's tiers, and the cost of `shelf_room` and of an unmendable `High`.**
    - `advert`'s `next(decor)` (mod.rs:194-207) keys on `item.decor()`, from `offers`. The Window, with no offers,
      already sells as furniture. "Kits last" needs a three-way `next` (furniture, decor, kits), not a filter.
    - Compute `shelf_room` only when the queue's next item is the kit. Otherwise every paint runs a whole
      `shelf_spot`, with its layout.
    - While the gremlin waits for a shelf (days), `High` is felt and searched, and every floor gets up to
      `CANDIDATES` evaluated, though only a shelf can mend it. Restrict `High`'s places to shelf strips in `search`'s
      loop (rules.rs:930-958). `repair_search_is_cheap` should include an unmendable `High`.

16. **[minor] Naming traps.**
    - `Chances.build` (the stage spot) beside the existing `Chances.builds` (makeshift pieces from torn text).
    - `Want::Build`/`Scene::Build`/`build/stage` beside `Job::Build` (scenes.rs:240) and the makeshift `Build`
      (mind.rs:13, 643, 735).
    - Name the shelf's want, job and chance after `PutUp`, as the act already is.

17. **[minor] D1: two more sites key rooms on a strip.**
    - `evaluate`'s tier (rules.rs:753-770) keys on `to.strip`: "nothing else on it" is tier 2, and
      `role_without`/`role_of(laid, to.strip)`.
    - `spoils(laid, to.strip, …)` and `newly_spoilt(…, was.strip, to.strip)` (rules.rs:689-690) do the same.
    - Decor forbids nothing and offers no role, so only the tier changes: an empty shelf is tier 2 for an unsettled
      piece. List them, and say whether the tier reads `to.strip.room()` or stays per strip deliberately (for the new
      gremlin, the only mending move is tier 2 either way).

## Claims checked and holding

- `layer.validate` reads only the layer's own cells (layer.rs:516-560; `fits` 301-327 tests targets that are layer
  cells, and their left neighbour's width). So D4's move to before validate, against plan.md's brief ("after
  `layer.validate`"), is sound, given the pre-validate layer as keep-out (step 2's trap).
- `HIGH_RISE` = 7 and `LOW_RISE` = 5:
  - The poster and the clock hang 4 with 2 rows (rows `f-6 ..= f-5`); the window hangs 1 (`f-3 ..= f-2`).
  - The tallest standing piece and parcel is 4 rows (bed, bookshelf, fridge, lamp; room.rs:199-271).
  - A low shelf at `f-5` sits in the poster's bottom row, so the block is required, as D3a has it.
- `Extent` is `from = rect.x + 1`, `to = rect.right() - 1` (exclusive), `floor = rect.bottom() - 1`,
  `rows = height - 2` (room.rs:805-820). D3's `Side::Right` arithmetic matches `Extent::left`.
- The seven irrefutable `Strip::Bottom` sites are as listed (ledger.rs:205, 365, 553; rules.rs:433; mod.rs:1624,
  4341; room.rs:2321). The two zips are at room.rs:1279 and 2429.
- `fits`' floor test accepts a written `─` (room.rs:2851-2858). The trouble is that it accepts the app's too (finding 5).
- Text-taking never takes a structure glyph: `segment` and `loose` stop at `strokes` glyphs (scenes.rs:834, 854), and
  layer targets must be blank (`free`, layer.rs:80-91).
- `out_by_door` rains only moved and made cells (mod.rs:2367-2380), so a shelf carries over into `Away` unrained, as
  intended. `door_rain` takes only her door's cells.
- `evaluate`'s "every piece laid out before still is" and `pushes_hung` compare with before (rules.rs:646-663). A
  hidden shelf's pieces, or a block that shifted a poster after a resize, don't refuse every repair.
- `plan_strip`'s yield rule (room.rs:1720-1774) compares the layout with the space against the layout without it. A
  block that leaves a poster out either way keeps the space kept, as D3a says.
- `to_mend` (osaka.rs:9639-9654) doesn't ask for a repair, so D9's "Arrange offered, `lift` finds none" reading is
  right.
