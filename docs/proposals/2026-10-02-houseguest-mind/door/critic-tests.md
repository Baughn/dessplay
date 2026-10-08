# Door batch design: tests critique

**Critic, 2026-10-08. Lens: tests.** The question for each test-first: will it fail today, and pass after, and
by what mechanism? Also covered: generator width, tests that cannot fail, gate cost, and how stillness, the
census and goldens interact. Claims were checked against the tree at `3c4ed354`. Paths are relative to
`dessplay/src/ui/houseguest/` unless noted. Severity is blocker, major or minor.

## Blocker

1. **Step 2, test 5 (`an_older_record_keeps_its_fridge_by_her_door`) expects the wrong wall, and step 2 breaks
   window.rs tests it doesn't list.**
   - The template is `an_old_window_loads_hung_low_drawn_and_in_reach` (tests/window.rs:307). It runs on
     `home_screen()` (tests.rs:2264), where Users is `(0,8,50,9)`. Users' right wall (col 49) is an inner wall,
     and its left wall (col 0) is the screen edge.
   - D2's chooser takes her pieces' strip first, then sorts edge first. So it picks **Users' left wall**, not
     the right one the test expects (`[U-R inner, U-L edge, P-R edge, P-L inner]` sorts to U-L first).
   - The fridge at `Anchor{Right,0}` therefore stays at `w-4..=w-1` of the inner wall. A correct build fails
     the test, and a builder may "fix" production to match it.
   - The same push also moves these, none of which step 2 lists:
     - the record's window at `Anchor{Left,1}` (hung low, so `hung_clear` must now refuse the space);
     - the sofa or TV at `Left,0`;
     - `sofa_and_window_at`'s pinned columns (window.rs:11);
     - away.rs `HOME` (sofa at Users 300).
   - **Amendment:**
     - Build test 5 on a screen whose Users right wall is the edge (`rooms(100,30)`, as test 0 does). Or keep
       `home_screen` and put the fridge at `Anchor{Left,0}`, expecting the left wall with its left edge at
       `w+7`.
     - Add to step 2's migrations every window.rs and away.rs assertion that hard-codes Users-left columns
       (window.rs:11-40, :307-380, away.rs `HOME` users).
     - Write down that on `home_screen` the chosen wall is Users' left edge.

## Major

2. **Step 3, test 1 and step 4, test 1 (bed/sofa/desk at 08:14): the cue mapping is wrong and the red is luck.**
   - stage.rs:88-96: `Nap` and `Lounge` are **both the sofa**, and the bed is `Scene::Sleep`. The bed case is
     the user's own report, and the list "Bed|Sofa|Desk → Nap/Lounge/Homework" has no cue for it.
   - The timing is also tight:
     - `home_at(.., tue(8,14))` arrives when the gate opens, after `DELAY` = 5 s real (stage.rs:730), which
       is about 08:14:30 game time at 6×.
     - That leaves about 5 s real to walk into the piece before the 08:15 cut.
     - If she's still walking, `cut`'s `to_job` (osaka.rs:3507-3517) interrupts the walk. The door then opens
       at her feet on plain floor, misses every cover, and **the test passes today**.
   - The stated assertion ("the closed door is `door_place`'s spot") can't compile today, so "confirmed failing
     for the stated reason" can't be shown.
   - **Amendment:**
     - Use Bed→`Sleep`, Sofa→`Lounge` and `Nap`, Desk→`Homework`.
     - Start at `tue(8,12)`.
     - At the last frame before the cut, assert the precondition: `Act::Use` on that piece, with
       `her_box(x,y)` meeting the piece's `cover()`.
     - Write the red first on today's API: `guest.closed_door()`'s box (or the Away image's door cells)
       meets an `empty.shown` cover. Confirm it fails, then add the `door_place`/`Set::Wall` assertions.
     - Compute the expected spot independently from the nook rect and side, not by calling `door_place`.

3. **The Away door properties are vacuous whenever no door is drawn. On `home_screen` the space is under text.**
   - Step 3 #8 (a) and (c), step 3 #2, the cold start, and step 4 #8 (b) all hold trivially when `empty.door`
     is `None` or the door is hidden (D5 hides it over text or protected cells).
   - `wordy_home_screen` (tests.rs:5954) writes the user list at col 2, rows 9-14. The chosen wall is Users'
     left (finding 1), so the space is cols 1-6, rows 12-16. Rows 12-14 are text, so **the Away door is
     hidden on every frame of the wordy screen.**
   - `door_shows` requires `seen > 0` (away.rs:63-88). Every test looping `home_screens()` therefore fails
     after step 3, for a reason the design doesn't anticipate: `a_school_morning_out_through_her_door_and_home_again`
     :125, `a_cold_start_in_school_hours` :626, and :376.
   - **Amendment:**
     - For each home_screens test, state the expected state per screen: quiet means drawn at the space; wordy
       means hidden with the spot kept, asserted explicitly.
     - Give every property a non-vacuity counter. In each unit test, assert at least one frame with a drawn
       `Set::Wall` door. In the proptests, count such frames and require a floor across the run (a final
       `prop_assert!` over all cases, or a deterministic companion test).
     - The step-3 exit already asks how often the space is calm on `chatty_ui`. Add the wordy fixture's answer
       ("never") to that report for the user.

4. **The day-long class guard rarely sees the class, and this can be fixed cheaply.**
   - In `her_days_never_touch_what_is_protected` (tests/away.rs:1232), `somewhen()` picks `near(8,15)` in 1 of
     4 branches. Only starts at 08:12-08:14 visit across the cut (≈3/7 of those, given the 30 s game gate),
     so about 11% of cases.
   - Being *in* a piece at the cut needs a chosen use on top of that. At 32 cases this is about 0-1 cases per
     run.
   - `owned` only draws `Furniture::ALL[0..4]`. Lamp, Fridge, Plant and Window never appear, so the window
     against the space and the no-use DoorClear path never occur either.
   - **Amendment:** add a new proptest, `her_school_mornings_never_put_her_door_on_a_piece` (a new function,
     not a new parameter: see finding 5):
     - `start` in Tue-Fri 08:12..=08:14;
     - `owned` with at least one of Bed/Sofa/Desk, plus an optional Lamp/Fridge/Window;
     - `cue: Option<Scene>` from {Sleep, Lounge, Nap, Homework}, fired once visiting (extend `long_visit_of`
       with a `cue` argument);
     - `apart: bool`;
     - a 60-120 s span.
     
     At `her_days`' measured 2 s per 32 cases (map tests §11) this stays far off the long pole (29 s). With
     this, the property becomes the reliable red (most cases are in a piece at 08:15) and the unit tests
     become pins.

5. **Adding parameters to existing proptests silently drops their regression seeds.**
   - Step 1 adds `apart` to `her_days`. Step 5 #4 runs `long_visits_never_touch_what_is_protected` "on
     `chat_apart` too".
   - `proptest-regressions/ui/houseguest/tests.txt` holds **26** seeds. About ten have the `long_visits`
     shape (sizes/text/skips/chats/protect).
   - The `cc` lines are RNG seeds. A changed strategy shape replays them as different cases (map tests §8).
     Step 1's trap pins only away.txt's three.
   - **Amendment:** don't change existing strategies. Add `apart` (and step 4's cue) as **new** proptest
     functions sharing the body. Or, before changing any shape, pin every affected tests.txt case as a unit
     test (the away.rs:1257 pattern) and say so in the commit.

6. **The "never in the chat" clauses can't fail on `chat_apart`.**
   - `chat_apart` makes the chat `nooks[0]` and drops it from `nooks`. No strip, no anchored piece and no
     space can ever meet it. The chooser's `space meets chat` refusal and `door_place`'s
     `!box_meets(chat, ..)` therefore never decide anything there.
   - The real layout is disjoint too (map: every nook's right wall is an edge outside the chat).
   - Step 3 #5 (`her_door_is_never_in_the_chat`, on `chat_apart` and `real_frame`) can only fail through the
     fallback, which needs the space to yield, and it doesn't at normal heights.
   - **Amendment:**
     - Construct the cases deliberately:
       - a layout whose space yields (full strip or short) where the nearest calm floor to the space is the
         chat pane's floor (the List box border is still a platform: `Terrain::read` reads the whole buffer);
       - a layout whose only edge walls meet the chat (the chooser must pick an inner wall, user answer 1);
       - a layout with no wall outside the chat (no door drawn while out).
     - Assert each case's precondition: `!space.kept`, the chosen wall is inner, and so on.
     - Add an **overlapping** variant to the long-visit property: the chat meets part of a nook, as
       ui/app.rs:4003's grid overlap. This exercises InChat, the per-frame closet, Hidden and the change budget
       together. Only that geometry makes D7 reachable in a long run.

7. **Step 3, test 4 (short terminal): `rooms()` keeps the space at H 18-22.**
   - In `rooms(w,h)`, Users' rows = `(h-3)/2 - 2` = 5..=7 for H 18-22, which is at least `HEIGHT`, so the
     space is **kept**.
   - "The space always yields" holds only on the real layout (map B5: H ≳ 23). In `rooms()` the space yields
     only at H ≤ 14.
   - **Amendment:** run the test on `real_frame(&mut real_ui(), w, 18..=22)`. Assert `!space.kept` for the
     door's strip on every frame as a precondition. Assert that the drawn door is `Set::Floor` meeting no
     cover, or that there's no door and the arrival waits.
     
     Cost: build `real_frame` once per size, not per frame. Keep 16 pinned cases (32 at the gate).

8. **`door_shows` tolerates a door overlapping pieces.**
   - Its ASCII branch skips every cell under a cover or over non-blank text (away.rs:72-84). A door drawn
     through the bed passes.
   - Step 3 migrates the helper to `DoorSpot` but keeps its semantics, so every test using it (the cold start
     :626 among them) stays blind to the bug.
   - **Amendment:** when migrating, make it strict. Every drawn cell (wall column and slippers) is outside
     every `empty.shown` cover, and covered cells count as failures, not skips. Keep the text tolerance only
     where D5 says it's hidden, asserted as hidden.

9. **Made pieces break steps 3 and 4's door properties until step 5.**
   - `visit.shown` includes made pieces (mod.rs:2258).
   - Makeshift keeps out of the space only from step 5 (`builds`, `made_stands`).
   - D4's `Set::Wall` branch checks no `shown` cover, only the platform, the wall glyph and protected cells.
   - So between steps 3/4 and 5, a scrap built in the space gives a door drawn over it. Step 4 #8 (b) and
     step 3 #8 (a)/(c) can then fail, or worse flake at 256 deep cases, for a reason that belongs to step 5.
   - **Amendment:** move the space half of the makeshift keep-out (`builds` and `made_stands` refusing the
     door's space) into step 3, where the door first stands at the space. Or make `Set::Wall` also require
     every `shown` cover to miss the space (belt and braces: it costs one intersects per piece). Either way,
     say which step's properties include scraps.

10. **Step 6, test 1 (`a_lamp_in_her_door_space_is_felt_and_moved`): the arithmetic isn't pinned, and a
    second piece is involved.**
    - The lamp is 3 wide (room.rs:242). The test needs three things:
      - "packs raw" means sum ≤ raw;
      - "not narrowed" means sum > raw−6;
      - "kept once the lamp leaves" means sum−3 ≤ raw−6.
    
      So the widths must sum to **raw−5..=raw−3**, not "within 6 of the extent". Otherwise the next frame
      isn't `kept` and the test fails for a design-irrelevant reason.
    - With the strip full and packed against the wall, the lamp's neighbour also covers cols `w-6..=w-4` of
      the would-be space. So DoorClear is broken for two pieces, and the search may move the neighbour.
    - **Amendment:**
      - State the width window.
      - Make the neighbour a no-use piece that is settled, or assert which piece moves.
      - Assert the preconditions (`!space.kept`, the lamp's DoorClear broken, `fallback_in`) before the
        arrival.
      - Force the mood for tests 2 and 3 too. Lazy never mends (brain.rs:209 `home_acts`), so a seed-chosen
        mood makes them seed-dependent.

11. **Step 4, tests 2 (latch) and 3 (cut) can't be red today for the reason given.**
    - Today `go_out` opens the door at her feet with no walk (osaka.rs:5266-5291), and `Job::Leave` doesn't
      exist. Neither can fail "because the line is re-said" or "because a boundary cut the walk". Both are
      guards a reviewer must prove with a mutant.
    - Test 2 can pass vacuously if the generated route has no landing.
      - **Amendment:** assert that `go_out` was re-entered at least twice (a hop landing, or a door in space)
        as a precondition.
    - For test 3, `cut` boundaries are slot changes (`next_cutting`, routine.rs:505; `refresh_cut`,
      osaka.rs:3411). Within a walk of a few seconds, the only reachable boundary is a dash's way out
      crossing 12:45. That also raises a behaviour question: does she still go out at 12:45:00, or stay?
      - **Amendment:** name that trigger, ask or decide the behaviour, and write the test through it. If
        that's impossible, say the test is a direct `cut()` unit test on a constructed `Walk{then: Job(Leave)}`.
    - Prefer making it structural: `to_job` matching only `Job::Use(..)`, so Leave is excluded by
      construction.

12. **The census and stillness section cites the wrong code, and the slide's stillness exposure is ungated.**
    - "tests/stillness.rs:844-859 counts them as moving" is wrong. Those lines are the film-swap exemption.
      The door count is census.rs:858 (set-offs with `Body::Door`).
    - The thirds don't move the census either. `Act::Door` is `Moves::Off(Body::Door)` (osaka.rs:887)
      whatever is drawn. Only the Leave walk (`Body::Walk`, purpose "routine") is new.
    - The drawn stillness tests (stillness.rs:989/1027) leave out acts that take her somewhere. They never
      see a parcel either: an order arrives on a later visit (`bought_on < visits`, mod.rs:3321), after
      their 10-minute run.
    - So nothing gated sees D9's slide: 5 changes in 650 ms (0/150/350/500/650), against `USE_FRAME_MS` =
      1400 (osaka.rs:1640).
    - A parcel arriving mid-act (watching, napping) would breach the drawn rule unless the arrival ends her
      act. Today's flap closing at 800 ms has the same exposure, so check what happens. Per the memory "her
      movement draws attention", this is the user's budget, not a free change.
    - **Amendment:**
      - Correct the references.
      - Add a step-9 unit test: `Scene::Watch` held over 40 s, a parcel due mid-watch, then run the drawn
        judge (`judge`, stillness.rs:893) on the paints. Either the arrival ends the act, or the slide is
        a sixth exemption. A sixth exemption is the user's call: design.md names five.

## Minor

13. **Step 1 understates who reads `view.chat`, and `long_visit_of` needs a frame provider.**
    - Resident tests built on `view()` through `home_screen` lose their chat when it goes empty:
      tests/calendar.rs:139, tests/dash.rs:1032, :1302. That changes `Chances.chat` (mod.rs:2302) and so
      `in_chat`/`elsewhere` (osaka.rs:353, 4107, 9272), not only `drop_in`/`shaken`. List them and check each.
    - `long_visit_of` builds both `real` (`rooms(w,h)`) and the view (`nooks(w,h)`, `view(..)`) inside its
      size loop and at its final dissolve (tests.rs:794-806, 965-971). "Takes its view from the caller" needs
      a `frame: impl Fn(w,h) -> (Buffer, IdleView)` parameter. `long_visit_on_the_real_layout` needs that
      anyway.

14. **Step 2, test 1 needs an independent oracle and both sides.**
    - Compute the expected space rect from the nook rect and side, per the geometry words. Don't read
      `space.rect` from `extents`: if the rect and the narrowing are both off by one, they agree.
    - Draw `door: DoorWall` over both sides and both strips of `two_panes` (left walls are `from-1`: the
      asymmetry the traps warn of).

15. **Step 2, test 3 (`pack_feasibility_is_anchor_independent`) can't fail.**
    - It restates today's `pack` (room.rs:831-864); the equivalence holds now.
    - D1 actually relies on `Home::extents(..)[i].space.kept` being invariant under re-anchoring that strip's
      pieces. Test that instead. It fails if `kept` is ever computed from `laid_on`/`fits`/text.

16. **Step 2, test 4 is redundant.**
    - `hung_pieces_clear_every_standing_piece` already asserts `hang >= tallest` for every non-window item
      and `tallest >= HEIGHT` (room.rs:2433, :2452).
    - Replace it with a geometric check: for each hung item, `stand(..)`'s rect on an extent intersects that
      extent's space rect (rows `f-4..=f`) **iff** it's the window. That catches an off-by-one in the space's
      rows: with `f-5..=f`, the poster at hang 4 (rows `f-6..=f-5`) would meet it.

17. **Pinning anchors on `floor` versus `raw` is untested.**
    - Tests 0 and 5 both use `Anchor{Right,0}`, which pins identically on either extent.
    - Add a case with a share-only prop (`anchor: None`, e.g. `at: 700`) on the door's strip with `door`
      already chosen. Assert its pinned left against the narrowed share.
    - Note that on a record's first `project`, `pin_anchors` runs **before** the chooser (room.rs:1354-1355
      versus D2's "after `move_off`"), so first-frame pins are raw. State that this is intended.

18. **Step 2, test 6 (chooser) must include a left-edge-only layout.**
    - `doorstep`'s edge test reads the **nook** rect (`rect.x == screen.x`, room.rs:1529-1530).
    - D2's wording ("`rect.right() == screen.right()` / `rect.x == screen.x`") could be read as the space
      rect, whose `x` is `w+1` and never equals `screen.x`. A left-edge case catches that misreading.

19. **Step 6, test 5 (`an_older_record_migrates_without_closeting_anything`) belongs in step 2.**
    - Narrowing lands in step 2, and so does the risk of closeting through `blocked`.
    - Make it a proptest over records (including 1000-share pieces) on `home_screen`, `rooms` and
      `real_frame`, and keep re-running it in step 6.

20. **Step 4, test 6 (resize mid-gap) needs a precondition that the spot moves.**
    - Assert old spot != new spot.
    - On `home_screen` the door is on Users' left wall at x=0, so a width shrink doesn't move it. Use
      `rooms()`, where the Users right wall is the edge.

21. **Placing her in the chat must be deterministic.**
    - Step 3 #5's "red today" (door at her feet in the chat) depends on where she is at 08:15.
    - `osaka.place` her on the chat pane's floor at 08:14:59 game time, and assert the precondition.

22. **Step 8, test 2 ("no pixel of hers beyond the wall") must name its observable.**
    - At the frame level that is the placed image's cell bounds, which must not exceed `w`. Pixels are only
      observable in step 7's `compose` test.
    - Say which assertion is made where.

23. **Some golden predictions are unsupported.**
    - Golden `stage_room` cues no `Work` or `Dash` (golden.rs:209-219: MakeSofa, Swap, Sneeze, Pull,
      Shopping, MakeBed, Parcel). Step 4's "`stage_room` (Work)" will move only if the unfed stage goes to
      work.
    - Let the trace diff decide, and drop the claim.

24. **Each step's gate is narrower than the project's.**
    - Steps 2, 3, 4, 5, 6, 8 and 9 add CHANGELOG entries, but the targeted
      `-E 'test(/houseguest::/)'` run excludes `changelog::tests::embedded_changelog_parses`.
    - `Summary` (ledger.rs:369) feeds `dessplay --dump` outside the module.
    - The stop hook is skipped while subagents are live (memory), so add the workspace gate to every step
      exit: `PROPTEST_CASES=32 cargo nextest run --workspace --all-targets`.
    - Give each new long-visit proptest a time budget of about 3 s at 32 cases, measured in the commit.

25. **`stranded` (D7) is a pure function with only unit tests.**
    - Add a proptest over overlapping layouts: a piece is stranded **iff** no outside-the-chat placement
      exists (brute force over strips and lefts).
    - Also assert that `project` never hides a non-stranded piece. This is the class guard for "never
      deleted, hidden only while nothing fits".
