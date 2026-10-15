# Phase 6a design: tests critique

**Critic, 2026-10-10. Lens: tests.** For each proposed test, four questions: can it fail (which plausible mutant
does it catch), is it red first at the step that adds it, does it test the spec rather than the implementation, and
is its cost acceptable at the gate (`PROPTEST_CASES=32 cargo nextest run`, with a 60 s kill)? Then the tests that are
missing. Claims were checked against the tree at `ad7f4af7` (`dessplay/` is unchanged since `4c5c7151`). Paths are
relative to `dessplay/src/ui/houseguest/` unless noted. Severity is blocker, major or minor.

## Blocker

1. **Steps 5 and 7 change the strategies of at least ten existing proptests. The design's own rule forbids this, and
   the steps don't list it.** (*Tests*, "No existing proptest's strategy changes"; D7, D9.)
   - Adding `ShelfKit` (step 5) and `Gremlin` (step 7) to `Furniture::ALL`, and both to `CATALOGUE`, changes every
     strategy built from either list:
     - room.rs:3238 `standing()`, which filters `ALL` by `hang.is_none()`. Both new kinds pass the filter. It feeds
       `pieces()` and `decorated()`, and through them `packing_keeps_order_and_a_resize_and_back_restores`,
       `the_wall_lane_keeps_order_and_never_moves_the_room`, `a_move_relaid_strip_by_strip_is_laid_out_as_a_whole`
       and `stranded_iff_no_room_takes_it` (both variants).
     - rules.rs:1459 and :2798 (`subsequence(Furniture::ALL.to_vec(), ..)`), which feed `rules_hold_*` and
       `every_repair_mends_and_breaks_nothing_*`.
     - tests.rs:5656 (`0usize..Furniture::ALL.len()`) and :5658 (`which in 0usize..=CATALOGUE.len()`).
     - tests.rs:7789 and :7810 (`an_older_record_migrates_without_closeting_anything`, and its crowded variant).
     - Not a strategy but the same drift: room.rs:5264. This proptest body picks a delivery from
       `ALL.filter(!owns)` by `which.index(..)`, so a replayed seed delivers a different kind. It can now deliver
       the kit and then unbox it "out of its box there", which D7 forbids (`Unpacked(ShelfKit)` must never reach
       `Home::unbox`). room.rs:4764 is a unit test that iterates hung kinds, and it's unaffected.
   - The consequences:
     - Every `cc` seed in `proptest-regressions/ui/houseguest/room.txt` (39 lines), `rules.txt` (19) and `tests.txt`
       (33) replays as a different case. The door critique's finding 5 says the same about this exact mechanism.
     - room.rs's `placed()` builds `Home { props, .. }` directly, bypassing `Home::add`. So D2's
       `debug_assert` never sees it, and an **unboxed shelf kit** is laid out on a floor as a piece. The kit has no
       unboxed art (D7), so the art and layout checks in those properties meet a state the design calls impossible.
     - The gremlin turns up as "a piece that stands" in properties written about furniture.
     - The gate goes red mid-step for a reason no step names. The likely "fix" is to patch the strategy, which
       loses the seeds silently.
   - **Amendment:**
     - Freeze the old strategies and the room.rs:5264 body on a named slice, for example
       `const PRE_6A: [Furniture; 13] = ..` (today's `ALL`), and `CATALOGUE[..10]` for tests.rs:5658. Those
       strategies then keep their exact value space and their seeds.
     - Cover kits and the gremlin in **new** proptest functions that share the bodies.
     - List every site above in the traps of steps 5 and 7, with a grep (`Furniture::ALL` and `CATALOGUE` inside
       `proptest!` blocks and strategy fns) as part of each step's exit.

2. **No proposed test can fail on the goodbye's two structure rules: `Leaving` writes nothing, and every `under`
   reads through `Written::real`.** (D4 *The goodbye*; *Tests* "Long visits …" and "The goodbye still waves …";
   step 2 traps.)
   - **The final `end == real` (tests.rs:1526) can't see a `Leaving` that writes.**
     - `run(.., now, now + DURATION_MS)` ends with `advance(DURATION)`. `Leaving` sees `dissolve.done` and becomes
       `Absent` (mod.rs:2184-2187), and the last paint is `Absent`'s, which writes nothing.
     - A mutant that writes the shelf in every `Leaving` frame therefore leaves `end == real` green. The design
       says this check proves the goodbye rains every shelf out. It doesn't. (It does catch an `Absent` that
       writes, and a `rain_home` that leaves cells behind.)
   - **`she_waves_goodbye_under_her_shelf` can't fail for a shelf reason in 6a.**
     - `Dissolve::unchanged` (dissolve.rs:220-225) checks only `burst` cells. Structure cells are `burst: false`.
     - Her burst cells (her box, her door, the pieces in her image: mod.rs:5520-5605) never meet a shelf in 6a, as
       D4 itself says.
     - So `untouched` is true with or without the fix, and with or without a `Leaving` that writes.
   - **The hand-built dissolve unit test runs no production code.** It builds `Frozen` cells with `under` set to
     the real cell, or to `'─'`, and calls `unchanged`. That only restates what `unchanged` already does.
     Mutating the three production `under` sites (`draw_props`' `unders_of` mod.rs:5345, `draw_art`'s closure
     mod.rs:5499, `paint_empty` mod.rs:5805) to read `buf` leaves it green.
   - **Amendment:**
     - **A frame-by-frame goodbye test** (both modes) with a finished shelf and the plant on it:
       - for `t < RAIN_FROM_MS` (675), every shelf cell shows the frozen `'─'`;
       - at every frame with `SETTLE_BY_MS` (3600) ≤ t < `DURATION_MS` (3750), every shelf cell equals `real`;
       - in no `Leaving` frame does a shelf cell show `'─'` in `SHELF_INK` after its column has settled.
       
       This catches the `Leaving` that writes: the cell is re-written, `holds` fails, the cell is `Settled` at
       once, and the live `'─'` shows until `Absent`. Confirm it red against that mutant.
     - **Direct unit tests on the three `under` sites:**
       - call `draw_art`, `draw_props` (both modes) and `paint_empty` with a hand-made `Written` whose cell lies
         inside her box or a piece's footprint (`Written` needs a `#[cfg(test)]` constructor);
       - assert that every returned `Frozen` there has `under ==` the real cell;
       - the mutant is "the closure reads `buf`".
       
       Describe these as 6b's guards, and drop the hand-built `unchanged` test.
     - Keep `she_waves_goodbye_under_her_shelf` as a smoke test (the plant's image placed in the startled beat, in
       line art). Don't count it as the guard.

## Major

3. **`long_visit_of`'s per-frame checks don't cover structures, and a correct build breaks its change budget.**
   (*Tests* "Long visits with shelves …".)
   - **The budget.**
     - `most = 25 + 24 + furniture + spanned` (tests.rs:1487) counts her box, one bubble, piece covers and her image.
     - A shelf's written cells are none of these. With her whole box changed and a full 24-character bubble, the
       10 shelf cells take `changed` past `most`. That's rare, so it will show up at 256 deep cases as a confusing
       failure.
     - **Amendment:** add the frame's written cells to `most` (the structures are hers, like pieces).
   - **What the structures themselves must never touch.** The design adds no per-frame check for the written
     cells; it only relaxes the piece checks. In `long_visit_of`, read the cells by diffing `frame` against `real`
     at the shelf's run, computed as in finding 4. Assert on every visiting and away frame:
     - each shelf is all or nothing;
     - no written cell is protected, in `view.chat`, a layer cell (`layer`), in a made piece's cover, or in the
       cover of a shown piece that isn't on this shelf;
     - no written cell is in her door's space columns (where a door exists: finding 9);
     - no shown hung piece meets a low shelf's block (D3a's wall rule, every frame, not only in room.rs);
     - every shown piece with `strip == Some(Strip::Shelf(_))` has `spec().shelf`, and its shelf is finished and
       written this frame.
   - Of these, the property in step 2 checks the first four only on `read_ground` in isolation, given `keep_off`.
     Only the integrated run sees the input step 2's trap names: the **pre-validate** layer passed to it, rather
     than `solid`.

4. **Two of the extended checks would use the code under test as their own oracle.** (*Tests* "Long visits …";
   step 3 sites tests.rs:1365-1410.)
   - **"each real piece stands on a strip here", reading `home.extents`.**
     - If `raw_extents` gave a shelf the wrong floor (`host.floor - rise ± 1`) or the wrong run, the check would
       agree with it.
     - **Amendment:** compute the expected floor and columns from the `Shelf` record and the nook rect alone:
       - `host.floor - rise`;
       - the run from `anchor` and `SHELF_WIDTH` (the `space_of_door` pattern, tests.rs:1121);
       - require the piece's `floor` and columns to fall inside them.
   - **"accepts a floor row that is a written cell".**
     - Which cells are written can't come from `Written`.
     - In line art the frame's floor-row cell is the piece's image (its standing layer redraws the line), not
       `'─'`.
     - **Amendment:**
       - in both modes, the floor row is the independently computed run;
       - in ASCII, `frame` shows `'─'` in `SHELF_INK` there;
       - in line art, the cell is in the piece's image.
   - **"every frozen cell's `under` equals `real`" is false today, for reasons unrelated to shelves.**
     - ASCII `draw` takes its unders from `buf` after `draw_props` and the layer have painted (mod.rs:3127 before
       3167; unders at 3866-3874).
     - So her sprite over a piece, or over text she moved, records the piece's glyph or the moved text.
     - **Amendment:**
       - scope the check to cells that are written cells: `under == real` there;
       - run it on the step-1 tree first, to confirm it holds;
       - in 6a it is vacuous for her box (finding 2). Its live case is the shelf's own `Frozen` cells.

5. **`a_shelf_hangs_as_high_as_fits` passes when `shelf_spot` always returns `None`.** (D3 *Choosing the spot*;
   *Tests* "Where a shelf goes"; step 5.)
   - Every clause is conditional on a result. "Low only when no pane has a high spot" needs an oracle for "has a
     spot", and the design names none.
   - A too-cautious `shelf_spot` means the kit is never sold (`shelf_room`), the box waits for ever, and nothing
     else notices.
   - **Amendment:**
     - **A brute-force oracle** over the frame, written without the shared per-cell function. It enumerates every
       (pane, rise, left) and checks D3's geometry and D4's per-cell test from the buffer and rects directly.
     - Assert `shelf_spot(..).is_some() == oracle.any()`.
     - Assert the result is the oracle's **first** in D3's order: high before low, the lowest rise, the gremlin's
       pane, `kit_at`'s pane, then `plan.nooks` order, the nearest middle, left on ties.
     - Count `Some(high)`, `Some(low)` and `None` over the run, and require each to be non-zero (a final
       assertion, or a deterministic companion test).
     - Widen the generator beyond "text, doors, chats, hung pieces". Add:
       - standing pieces (their covers);
       - made pieces;
       - layer cells;
       - protected and focused rects;
       - wide glyphs (trailers);
       - a pane exactly 9 and exactly 13 tall (the two thresholds).

6. **The step-2 property is vacuous when its random shelves never stand, and it skips D3's geometry.**
   (`read_ground_takes_only_blank_free_cells`, D3, D4.)
   - "Any geometry" makes most shelves invalid, and all-or-nothing then writes nothing. Every clause then holds,
     and the mutant ("skips the trailer check") never meets a trailer next to a valid run.
   - **Amendment:**
     - Bias the generator: half the shelves valid by construction for the frame they're drawn on.
     - Assert written-shelf counts across the run.
     - Add the geometry clauses:
       - every written cell lies in `host.from + WIDTH .. host.to - WIDTH`, on row `host.floor - rise`, with
         `LOW_RISE ≤ rise ≤ host.rows`;
       - `built: 1` writes exactly the half nearer its wall;
       - `built: 2` writes `SHELF_WIDTH` cells.
     - Say what `built: 0` writes (nothing, presumably: D3a gives it a block, but D4 doesn't say it draws) and
       test it.
     - As in finding 5, the per-cell oracle must not be the shared function.

7. **The new long-visit proptests are underspecified: which frame they run on, how a piece gets onto a shelf,
   proof that they aren't vacuous, and their cost.** (*Tests* "Long visits with shelves …".)
   - `long_visit_of`'s `owned` goes through `Prop::new(item, Nook, ..)` (tests.rs:1169), which always makes
     `Strip::Bottom`. "Small decor on it" needs a new path, for example `(item, Strip, along, left)` or a
     `structures` argument.
   - Say which frame each test runs on:
     - `rooms_frame` gives both heights: List is ≥ 13 tall at every size, and Users is 7-21 tall.
     - `chat_over` is the only fixture where the chat rule can decide anything (door critique, finding 6).
     - `chat_apart` makes the chat clause vacuous.
     - Use `rooms_frame`, and add a `chat_over` variant.
   - Add a non-vacuity companion, as `long_visits_with_the_chat_over_a_nook_make_pieces` (tests.rs:849) does. In
     both modes, require frames where:
     - a high shelf is written;
     - a low shelf is written;
     - a piece stands on a shelf;
     - a shelf is hidden by text;
     - a shelf comes back.
   - Add a size step whose frame provider drops the host pane, then restores it. Assert that the record is
     unchanged and that the pieces are back at their anchors. That is the hidden-pane rule ("never moved") under
     a long run.
   - **Cost:**
     - Pin `proptest_cases(16)` and a 60 000 ms span, as `long_visits_with_decor` does (tests.rs:725).
     - The existing whole-visit properties already take 21-28 s at the gate (`.config/nextest.toml`), and two more
       of that size must stay off the long pole.
     - Measure each in the commit (target ≈ 3 s at 32 cases).

8. **Existing room.rs proptests "run with shelves" contradict the rule, and their generators can't produce what
   they're asked to test.** (*Tests* "No furniture on a shelf", "The wall lane packs beside a low shelf"; D6.)
   - Changing `stranded_iff_no_room_takes_it` and `a_move_relaid_strip_by_strip_is_laid_out_as_a_whole` to take
     shelves changes their strategies (finding 1).
   - Their heights also block both shelf heights:
     - `a_move_relaid_…` draws pane heights `6u16..14` (room.rs:4667). A high shelf needs 13 rows; a low one
       needs 9.
     - `two_panes` caps Users at `tall` `8u16..14` (room.rs:4167), so a high shelf appears only at 13, and then
       only at rise 7.
   - `decorated()` (room.rs:3254) holds a poster and maybe a window, **never the clock**. A low shelf must push the
     clock too (D3a).
   - **Amendment:**
     - Write new functions with their own strategies: heights up to 20, a clock in the hung set, and a shelf of
       either height.
     - Assert each height turns up.
     - Keep the old functions unchanged.

9. **The door-space and chat-pane exclusions aren't stated as properties where they can fail.** (User decisions
   "Door-batch rules hold"; D3.)
   - Her door's space exists only once a door wall is chosen.
     - In `long_visit_of` on `rooms_frame`, the door's space comes from `door_space_of` (tests.rs:95), when there
       is one.
     - `her_days` (tests/away.rs:1528) always has a door, and an Away home that writes too.
   - **Amendment:**
     - In `her_days_with_a_shelf_…`, assert on every frame, visiting and away: no written cell is in the door's
       space columns of its strip, computed by `space_of_door`/`walls_of` from the nook rect.
     - Also test D3's "kept **or yielding**" clause directly: a strip whose space yields (narrow or short) still
       keeps the shelf off those columns.
     - Put the chat clause on the `chat_over` variant (finding 7).

10. **The pacing test can't use the stage, is seed-dependent as written, and doesn't pin the day boundary.**
    (D7 *Pacing*; *Tests* "The kit"; D11.)
    - `Scene::Build` "clears today's pacing" (D11). Cueing it a second time bypasses the very rule under test.
    - "Half up that day, whole not before Wed's start" waits for her to choose `Build` (`row(12.0, ..)`, chosen
      among `TOP` by a keyed roll, brain.rs:883-916). Whether she does depends on the seed, so "not before
      Wednesday" passes trivially for a seed where she never picks it.
    - The game day turns at midnight (`routine::split`, routine.rs:356), and a resident's visit crosses midnight.
    - **Amendment:**
      - Set up deterministically: `Ledger::new_at(.., tue(15, 50))` with the kit boxed in the home (or
        `ordered: Some(ShelfKit)` with `bought_on < visits`), and assert that precondition.
      - Assert on what she is **offered**, not what she does: after stage 1 on Tuesday, `Want::Build` is absent
        from her offers for the rest of Tuesday, across a restart
        (`Guest::restore(Ledger::from_json(&guest.ledger.to_json()))`) taken right after stage 1.
      - Across 23:59 → 00:00 it becomes offered again.
      - The mutant this targets: `paced` not saved, or reset by a restart.
      - Add the unfed case: one stage per visit, and the next visit offers it again.
      - Confirm red against each mutant.

11. **The wishlist tests can't tell "first" from "last".** (D9 *The wishlist*; step 7.)
    - In a home that already owns all the furniture and decor, the kit is next whether or not it's wished.
    - `wish` reads the per-visit `Osaka.felt`, so a rule felt in an earlier visit is not a wish this visit.
    - **Amendment:**
      - Run `a_felt_high_rule_puts_the_kit_first` and the end-to-end gremlin test on a home lacking at least one
        piece of furniture **and** one of decor, and assert the kit comes before both.
      - Add `a_rule_felt_last_visit_is_no_wish_this_one` (or the opposite, if that's what's intended) to pin the
        per-visit behaviour.
      - `no_kit_is_sold_where_no_pane_takes_a_shelf` needs two cases:
        - geometry: the bundled layout at 80×24;
        - text: a pane tall enough, but its run rows full of text.
        
        The text case is the only one that catches a text-blind `shelf_room`.

12. **Older builds: the design's model of them can't see what's on a shelf, and today's reader contradicts the
    claim.** (*Decided* "Older builds"; D2; *Tests* "Ledger".)
    - `as_an_older_build_reads` (ledger.rs:738) models the **oldest** builds. It knows `OLDEST_KINDS` (8 kinds),
      so it skips the plant, the clock and the gremlin, which are everything a shelf takes.
      `an_older_build_reads_past_the_structures` is therefore vacuous for "a piece on a shelf falls back … under
      where the shelf was".
    - The older build that matters is today's (pre-6a). It can't parse an anchor of `{"Shelf":"Users"}` and so
      drops it. The piece then goes to `RoomKind::of(item)`'s pane (ledger.rs:198-206). `rooms()`
      (ledger.rs:545-565) lists only `legacy()` pieces, and the plant, clock and gremlin aren't legacy.
      - Shelf host ≠ the Living pane: the piece lands on the wrong pane.
      - No sofa, TV or cat bed: the piece is **dropped**, and that build's next save loses it for good.
    - **Amendment:**
      - At step 2, before any reader change, write a 6a record by hand: a shelf, the plant on it, a boxed kit,
        `paced`.
      - Run today's `Ledger::from_json` on it, on the step-1 tree, and record the outcome in the commit and in
        decisions.md. Correct the claim if it's wrong.
      - Freeze that reader as a model: `as_a_pre_shelf_build_reads`, beside `as_an_older_build_reads`.
      - Golden files both ways:
        - `the_record_with_a_shelf_as_written`, character for character, with `structures`, `seen`, `door`,
          `paced` and `clock_sent` all present (this pins the field order that `a_door_wall_round_trips`'s
          `ends_with` and `the_clock_sent_round_trips` rely on);
        - `the_record_as_written` unchanged;
        - the pre-shelf model reading the 6a golden.

13. **Sibling sites of the "rooms are floors" class have no tests.** (D1 *Rooms are floors*; D6; D10.) Each takes
    one forgotten `.room()` or `stood` to break, and nothing listed would catch it:
    - **`clock_on`** (mod.rs:213-230): `raw_strips(nooks).find(|(s, _)| s == strip)` returns `None` for a shelf
      strip, so her glance at a mantel clock is silently lost. Add a unit test (a clock on a shelf gives
      `ClockOn` with the host's floor) and a fed run where she reads the time from it.
    - **`Home::rooms`/`role_among`/`has_room`/`becomes_den`/`Before.roles`**: the decor-only shelf "would read as a
      den". Add a unit test: a shelf holding only decor adds no entry to `rooms()`, and moving the plant onto it
      changes no role.
    - **`Shown::seat`, `room_for` and `reach` reading `stood`**: a unit test where a piece on a shelf gives a seat
      or reach at `host.floor`, not at the shelf's row.
    - **`move_off`, `project_with`, `doorstep`, `spot` and the door's chooser never targeting a shelf**: one
      unit test per site, with the shelf as the only free run. That's the cheap way to check "no parcel, gift or
      door ever lands on a shelf" (D1).
    - **Beauty when the shelf is hidden**: the unit test on `beauty_at` is tautological (a `shown` without the
      pieces). Instead, put text over the shelf in a running visit and check that the beauty she feels drops.

14. **Repair-search properties with shelves come too late.** (Step 3 exit; step 7 exit.)
    - Moves onto a shelf first exist in step 3, and blocks first push hung pieces in step 4. Yet
      `every_repair_mends_and_breaks_nothing` and `rules_hold` "with" anything shelf-shaped appear only in step 7,
      "with a gremlin", and through the `ALL`-based strategies of finding 1.
    - **Amendment:**
      - Step 3: add `every_repair_mends_and_breaks_nothing_with_a_shelf` as a new function (shelves of both
        heights, small decor and furniture breaking `InChat`/`DoorClear`).
      - Step 4: add a low-shelf variant (blocks).
      - Step 7: add the gremlin variant.
      - `a_plant_in_the_chat_goes_up_on_her_shelf` must force her mood (lazy never mends: door critique,
        finding 10) and assert its precondition: `search` returns exactly the shelf move.

## Minor

15. **Step 1, `a_garbled_record_never_puts_her_door_on_a_shelf`, can be red within step 1.**
    - Add the `Shelf` variant first; the test then parses a door on a shelf and fails. Then add the `try_from`.
    - So it's red-first, not "a guard until the variant exists".
    - Also assert that the rest of the record reads intact (only `door` is `None`).

16. **The ledger boundary is untested, and D2 and D3 disagree about it.**
    - D2 skips "a rise of 0"; D3 says the ledger skips any rise under `LOW_RISE`.
    - Test at `LOW_RISE - 1` = 4. Rise 0 can't catch a `rise == 0` mutant.
    - Also pin:
      - a shelf with `built` exactly `SHELF_STAGES + 1`;
      - shelves on two different panes (6a's "one in all");
      - a kit recorded both as a prop and as a shelf.

17. **`a_shelf_is_no_floor_of_hers` is blind to its mutant unless its shelf is finished and has room above it.**
    - `add_platforms` (terrain.rs:318) needs `MIN_PLATFORM` = 3 standing positions inside `HALF` = 2 margins. A
      `built: 1` run (5 cells) gives one, so dropping `!structure` from `ledges` makes no platform there.
    - `clear` (terrain.rs:349) needs `HEIGHT` open rows above. They are almost always open: `open` is
      `free && (!graphics || quiet || narrow)`. The exception is a shelf near the screen's top.
    - Use a finished shelf with ≥ 4 on-screen rows above it, in both modes, and compare `Ground::ledge` as well as
      platforms and links.

18. **The stage room's scene test is cited at the wrong line and is near the 60 s kill.**
    (D11; `every_scene_has_a_spot_in_the_stage_room` is at tests.rs:2355, not 1762.)
    - It loops **(100, 30) and (80, 24)**, both modes. Its own comment says running every case to the cap costs
      about 50 s.
    - At 80×24 no pane takes a shelf, so `Build`, `Shelf` and `Admire` need a `LookOut`-style exception: a
      `Some(Err(_))` note asserted, with `width < 100`.
    - Each new scene needs an early "visibly happened" observable in the `happened` match (tests.rs:2454):
      `Pose::Stretch` held for Build, `Pose::Gaze` for Admire. Admire lasts 8-16 s against the 10 s cap.

19. **The text, focus and resize tests leave the empty home and the transitions unspecified.** (*Tests* "Text,
    focus, resize".)
    - Repeat them with her out (`State::Away`, the `paint_empty` path, `empty.painted`), not only visiting.
    - Pin what happens when text arrives: does the whole shelf vanish at once, or rain?
    - Pin a per-cell toggle: a spinner or cursor over one run cell flickers all 10 cells at the UI's rate, with
      no hysteresis in D3. Assert whatever the design decides; the user's "movement draws attention" rule makes
      this a visible choice.

20. **Nothing pins how a shelf looks.** No golden has a shelf (*Goldens*). Add:
    - a fixed-frame test of `Scene::Shelf` in ASCII that pins exact cells and ink (`'─'`, `SHELF_INK`, the plant
      standing on it);
    - a line-art check that the shelf's strokes reach the image with `SHELF_INK` (the cache-key concern, D4).

21. **Per-paint cost of `shelf_spot`.** `shelf_room` is computed where `advert` is (mod.rs:2955, 3008), on every
    paint the shop is due, not only while the box waits. Add an operation-count test, `shelf_spot_is_cheap`, in
    the manner of `repair_search_is_cheap` (rules.rs:2295): cells visited ≤ k × pane cells. A wall-clock
    measurement isn't enough.

22. **Admire's "never at a piece on a floor" should be a property, not a seeded run.** Over the shelf long visits,
    assert:
    - every `Act::Use` with `Use::Admire` has its piece on a shelf;
    - her feet are at `host.floor`;
    - her x is under the piece or beside it.
    
    Then `she_gazes_up_at_what_is_on_her_shelf` only has to show that it happens (a seed with its precondition
    asserted: Admire was offered).
