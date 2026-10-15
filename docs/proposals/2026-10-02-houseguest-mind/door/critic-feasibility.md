# Door batch design: feasibility and step-ordering critique

Lens: can each step be done by one agent and leave the tree green; hidden dependencies; parallel safety; step size;
type ripples; cited refs. Checked against the tree at `3c4ed354` (`dessplay/` unchanged since `057311b1`). Paths
relative to `dessplay/src/ui/houseguest/` unless noted.

**Spot-check of refs (about 120 checked):** almost all correct (room.rs 736/795/1120/1187/1209/1217/1225/1230/1281/1348/
1378/1445/1517/1617/1682/652/400; rules.rs 19/63/168/183/193/262/324/404/428/462/465/517/534/548/573/595/734/760/
1118/1319/2038/2281; the 10 `Frame {` literals; mod.rs 211/228/300/381/409/500/645/669/691/700/713/842/1294/1308/
1365/1391/1422/1744/1792/1849/1869/2026/2092/2107/2353/2403/2805/2893/3206/3220/3227-3359/3366/3386/3485/3495/
3553/3579/3591/3631/3702/3791/3837/3922/3938/4138/4143/4482/4581/4600/4607/4613/4657/4704/4722/4752/4786;
osaka.rs 676/928/1230/1239/1946/2770/2861/3491/3552/4064/4411/4636/5200/5266/5684/7065/7213/7371/8026/8121/8208/
8430/8611/9090/9111/9138/9163/9181/9198/9408/9463/9531/9540/9857/10824/13497/13561/12038/15650/15866/15887/16151;
scenes.rs 235/277/315/327/342/353; mind.rs 226/440/464/1473; ledger.rs 46/369/568/623/1214/1226; graphics.rs
61/136/146/154/179/503; art.rs 1121/1146/1221/1385/3430/3615/3869; sprite.rs 453/479/780; stage.rs 28/465/485/607/
636; the tests.rs/away.rs/dash.rs/window.rs/golden.rs/census.rs refs). Wrong or incomplete ones are findings below
(F17-F20).

## Blockers

### F1 (blocker) D1 Types: `struct Room` collides with the existing `enum Room`
room.rs:1789 already has `enum Room { None, ToUse, ToLook }` (used by `admits`, room.rs:1568-1569, 1654-1656). D1's
`pub(super) struct Room` in the same module does not compile.
**Amend:** name the new struct `Laid` / `StripPlan` / `Floor` (or rename the old enum `Asks`); fix every D-section
use of `Room`/`&[Room]` and `Keep::of(rooms, ..)`.

### F2 (blocker) D2 vs step 2 traps: unit tests cannot opt out of the door choice
D2 writes `Home.door` inside `project` whenever it is `None` and the home has a prop. Step 2's trap says "give them
`door: None` explicitly where the door isn't the subject". That has no effect: `None` *is* the trigger, so the
first `project` in every room.rs/rules.rs unit test with a right-anchored piece on an edge strip chooses a wall and
pushes pieces 6 columns. About 45 room.rs tests and 9 rules.rs tests call `project` (e.g. room.rs:1937-3241,
rules.rs:1331-2326), many asserting exact `left`s. The step can't be green without rewriting their expectations, and
the trap's escape hatch doesn't exist.
**Amend (pick one, say which):** (a) make the choice explicit, not implied: `Home::settle_door(&mut self, nooks,
chat, screen)` called by the production sites (`furnish` before its first `project`, `paint_empty`, the arrival's
projection in D5, `stage_arrange`/`place_gift` if they can see a home without a door) and **not** by `project`;
`project` only reads `self.door`. Unit tests then keep `door: None` and mean it. Add a debug-assert/test that every
production `project` caller has settled first (the "forgot to call" risk is three call sites, all in mod.rs).
(b) Keep it in `project`, and make the field tri-state (`enum DoorWall { Unchosen, None, At(..) }`, `None` meaning
"no door space, by choice", serialised as absent) so tests can say `None`. (a) is simpler and keeps `project`'s
signature (see F3).

### F3 (blocker for step size) D2: `project` gaining `chat` ripples to ~60 call sites; the design lists 6
D2: "`paint_empty` (4704) and `furnish` (3227, 3231, 3265, 3329, 3359) pass `view.chat`". Also calling `project`:
mod.rs:3493, 3523 (`stage_arrange`), 3658 (`place_gift`); room.rs:1576 (inside `admits`, so `doorstep` and `spot`
need `chat` too, and their ~12 test callers at room.rs:2880-3087 and mod.rs:3322/3352/3644); about 45 room.rs tests
and 9 rules.rs tests. Combined with F2, step 2 is well past its ~1100-line estimate and mixes a mechanical signature
change with the behaviour change, so a reviewer can't tell which test edit is which.
**Amend:** either follow F2(a) (the chooser outside `project`, so `project` needs no `chat` until step 6's closet)
and give the closet its own call (`shown.retain(|s| !stranded..)` at the same production sites), or add a **step
1b**: a mechanical, behaviour-free commit that threads `chat` (or a `Plan { nooks, chat }` bundle) through
`project`/`doorstep`/`spot`/`admits` with `Rect::default()` in tests, all goldens byte-identical. Either way, list
every production caller in D2.

## Majors

### F4 (major) Step 7 "parallel-safe": not with jj in one working copy, and `mirror` moves goldens
1. Two agents in the same jj working copy share one change: `jj commit` by either commits both agents' half-done
   edits, and step 7's `cargo clippy --workspace` gate fails whenever step 3/4's tree is mid-edit. Per the
   project's own memory (jj in worktrees hits the main repo), a parallel builder must run in a git worktree and
   commit with **git**, and the result is rebased onto the batch afterwards.
2. Step 7 changes `sprite::mirror` (sprite.rs:453) to swap `[`/`]`. `mirror` also flips **her** sprites
   (sprite.rs:591), and four of them contain `[]` (sprite.rs:349-350, 395-396: book/homework rows). Left-facing,
   those rows change from `][` (today: reversed, unmapped) to `[]`, so every ASCII golden with those poses facing left moves, contradicting "no golden
   moves". 
**Amend:** step 7 runs in a git worktree (`EnterWorktree` or `git worktree add`), commits with git, and is
rebased before step 8; or just run it sequentially after 6 (it's ~600 lines; parallelism saves little). Use a
wall-door-only mirror (`fn mirror_wall(c)` mapping `[`/`]` then delegating to `mirror`) and leave `mirror` alone.

### F5 (major) D8 beats: drawing the door in beat 6 breaks `gone_out` and the gap stretch if routed through `Osaka::door`
`gone_out` (osaka.rs:5295-5298) is `leaving && hidden(now) && door(now).is_none()`; the only beat with `door: None`
is DOOR[6] (osaka.rs:1286-1291), and `door_beat` stretches **that** beat by `gap` because `beat.door.is_none()`
(osaka.rs:1320-1325). D8 says "beats 6-9 `Shut{away: true}` (beat 6 draws the door, unlike the door in space)". If
the builder makes `Osaka::door(now)` (osaka.rs:9067) return the wall door for beat 6, school never ends the visit
(gone_out never fires) and a gap stops stretching.
**Amend D8/step 8:** `DOOR`, `door_beat`, `hidden`, `door()` and `gone_out` stay keyed on the face-on table;
`wall_beat` is a separate read used only by the draw paths. Add a step-8 test: a school exit with the wall door
still reaches `State::Away` (today's `a_school_morning_out_through_her_door_and_home_again`, away.rs:125, covers
it if run in both modes; say so).

### F6 (major) D10 `Scene::School`: with `leaving` set, a 5 s gap never comes back
D6 sets `leaving` at set-off; `gone_out` then ends the visit at beat 6 regardless of the gap, so the stage scene goes
to `State::Away` and waits for 12:45, not 5 s. Without `leaving`, `back_home` (osaka.rs:9475-9485) returns `false`
(no `returning`, no shift) and she reappears with no homecoming. Also `Scene::ALL: [Scene; 57]` (stage.rs:166) and
`every_scene_has_a_spot_in_the_stage_room` (tests.rs:1762, which panics unless `State::Visiting` after the cue)
must accept the new scene; tests.rs:9128/9187/9240 map wants/ids onto `Scene::ALL`.
**Amend D10:** specify the mechanism: e.g. the scene sets `returning = Some(School)` with `leaving = None` and
`Through::Home(spot)`, gap 5000 (so `back_home` → `home_from`), or drives the guest to `State::Away` and
fast-forwards the clock. List the stage tests to update in step 4.

### F7 (major) D1 / step 2: `evaluate`'s line 534 and `search`'s line 760 read an extent before the move
D1 says 548/573/595 use `scratch.extents(..)` after the move. But 534 (`e` used for `at: e.pin(e.left(..))`) and
`search` 760 (`p.e`, used for the candidate's `lefts` and for the anchor offset `p.e.to - left - cols`, rules.rs
796-805) also read `before.strips`. Whether the target strip is narrowed depends on the move itself (a piece moving
onto the door's strip can make it yield). With raw `e`, a candidate anchored from the door's side gets an offset measured from the wall, not from the space's
inner edge, so laid on the narrowed floor it stands 6 columns from `left` and `repair.at.left == left`
(rules.rs:821) drops it; the far-side candidates survive, so repairs onto the door's strip are silently skewed.
**Amend D1:** since `kept` is anchor-independent (step 2 test 3), compute the target's extent from the scratch home
with the piece's strip already changed (any anchor) before pinning: `let e = scratch_with_strip.extents(..)
[to.strip].floor`; `search` takes per-strip floor extents computed the same way per candidate strip (one pack per
strip, not per left). Say this in D1's list and step 2's traps.

### F8 (major) Step 2: `doorstep`'s flap x comes from the extent it zips
`doorstep` builds the flap at `e.to` / `e.from - 1` (room.rs:1595-1598) from the same `e` it packs on (zip at
1528). If the builder passes `floor` (narrowed), the flap's x is `w-6`, mid-floor, where `draw_flap` finds no wall
glyph and silently draws nothing. Step 2's goal says "its draw_flap stays at the wall".
**Amend step 2 traps:** "doorstep packs on `floor` but takes the flap's x from `raw` (the wall column)"; add an
assertion to `a_poster_is_delivered_where_it_fits_boxed_and_hung` (room.rs:2915) that `flap.x` is a wall column.

### F9 (major) Gates: the per-step nextest filter misses tests outside `houseguest::`
Each step's gate is `-E 'test(/houseguest::/)'`. Step 2 adds `door` to `Summary` (ledger.rs:369), and
`dessplay/src/dump.rs:500-552` asserts the exact `--dump` JSON (it would gain `"door": null` unless skipped); the
CHANGELOG entries need `changelog::tests::embedded_changelog_parses`; `perf.rs` gates frame cost (D5 adds a
`Terrain::read` per visiting frame, F11). The memory note "verify with workspace --all-targets" applies.
**Amend "Steps" preamble:** the per-step exit is `cargo nextest run --workspace` (PROPTEST_CASES=32) plus `cargo
clippy --workspace --all-targets`; the targeted filter is for the edit loop only. Make `Summary.door` an
`Option<String>` (`"Users right"`) with `skip_serializing_if`, matching the other `Summary` fields (all strings or
plain values; a `pub(super)` `DoorWall` in a `pub` field of the `pub` `Summary` may also be refused as a private
type in a public interface), and update dump.rs's expected JSON if it isn't skipped.

### F10 (major) D2: a saved wall whose space comes to meet the chat is never re-judged
D2 refuses a wall whose space meets the chat only when choosing, and "a resize never re-chooses". D4 step 2
(`Set::Wall`) checks `kept`, the platform, the wall glyph and protected cells, but not the chat. The chat can move
under a saved wall (runtime layouts, docs/ui-layouts.md; a pane drag). Then her door stands in the chat, the case
the user reported. D3's "door_place refuses the chat for every candidate" is not in D4's list.
**Amend D4 step 2:** add `!space.rect.intersects(chat)` (else `Why::Chat`, falling through to the strict fallback,
which already refuses the chat). Decide whether that also re-chooses the saved wall (suggest: unsaved, per frame,
like a yield, so a pane drag and back restores it). Add to step 3 test 5 a layout where the chat moves over the
saved space.

### F11 (major) D5/D6: `Chances.door` per visiting frame needs an `unkept` terrain the Visiting arm doesn't build
D6 fills `Chances.door` at 2353/2403 from `door_place` "against `unkept`". The Visiting arm has no `unkept` (only
the Away arm computes `whole_glyphs(buf, as_drawn.clone()).protected`, mod.rs:2107) and its `visit.terrain` is
read with `view.protected` and furnished. A second `whole_glyphs` + `Terrain::read` per visiting frame is real cost
(perf.rs gates frame time, memory: "TUI lag").
**Amend D4/D5:** `door_place` needs calm cells only in the space's rect, the wall column and the fallback's
search; pass a cell predicate (`calm: &dyn Fn(i32, i32) -> bool`) built from `buf` and `unkept` on demand, not a
`Terrain`. Or compute `door_place` only when it can change (hash of home, nooks, chat, size, as the mend throttle
does) and cache it on the visit. Name the cost check in step 3's exit (`cargo nextest run --profile full --release`
perf tests).

### F12 (major) Step 4 is two steps; split school from work
Step 4 (~1300 lines) does `Job::Leave` and its 12 exhaustive sites, the latch, `cut`, `evict`, the C5 refresh,
`Through`, **and** work (`mind::work`, `go_to_work`, `Shift::Back` removal, `cut_shift`, `evict` for work, the
`choose_next` shift arm, three work tests to rewrite, stage `Work`), plus `Scene::School`. Two reviewers can't hold
that.
**Amend:** 4a: `Job::Leave`, `Through`, the latch, `cut`, `evict`'s leaving arm, C5, C6, school (tests 1-3, 6-8).
4b: work through the door (tests 4-5, `Shift::Back` removal, `mind::work`, `cut_shift` (osaka.rs:9520-9545) whose
`(Act::Door{gap>0}, _)` arm becomes the only shift share, stage `Work`). 4a leaves work on `Route::Around` and is
green. `Scene::School` goes with step 8 (it exists to watch the art), or 4a.

### F13 (major) D6: the `Act::Door` gap at the end of a Leave walk is unspecified
`start_job(Job::Leave)` "starts the door" (D6 Types), but school needs `gap: u64::MAX` and work `rng.range(SHIFT_MS)`.
D6 only says what `go_out`/`go_to_work` do when she's already at the spot. `go_to` also needs a `Want` for the
heading (D6 writes `Want::?`): `Heading { want, job }` is logged and used by `drop_heading`/credit.
**Amend D6:** `start_job(Leave)` picks the gap from `self.shift` (`Some(Going)` → shift gap, else `MAX`, with
`leaving` asserted set); the heading's want is `Want::Work` for work and, for school, a decided value (e.g.
`Want::Walk`, with `credit = None` as `go_out` does today). Write both into the step-4 traps.

### F14 (major) D2: "its strip isn't among nooks" re-chooses and saves on a resize that drops a pane
`IdleView.nooks` drops empty panes (ui/app.rs:664-671), so a resize or layout that hides the door's nook makes D2
re-choose and **save** another wall; resizing back keeps the new wall. That contradicts "a resize never re-chooses"
and step 2 test 2 ("a resize and back restores `room` (door included)") whenever the door's strip had no pieces
(`move_off` moved nothing), e.g. a door on an empty edge strip.
**Amend D2:** re-choose and save only when `move_off` actually moved the door strip's pieces (the door follows the
room); otherwise choose unsaved for the frame (as for the no-home door). Test 6 gets that case.

### F15 (major) Test-only construction of `DoorSpot` is unspecified
`DoorSpot`'s fields are private and "only `door_place` makes one". Step 4 must rewrite osaka.rs unit tests that
build `Act::Door` (15650, 15866, 15887, 16151), `go_to_work` callers (tests.rs:3838, away.rs:996) and the new
osaka.rs unit tests (latch, cut, evict) with `Through::Home(..)`; none of those has a frame to call `door_place`
on.
**Amend D4:** a `#[cfg(test)] pub(super) fn DoorSpot::at(x, y, set)` in door.rs (or a `door_place` on a fixture
frame helper in tests.rs), stated in step 3 so step 4 can use it.

## Minors

### F16 (minor) Step 3 sites omit `Chances`'s declaration
Step 3 fills `Chances.door` (mod.rs:2353/2403) but its site list names osaka.rs only for 9111/9138/9163; the field
is declared at osaka.rs:26 (step 4's list). `Chances` derives `Default`, so `Option<DoorSpot>` needs nothing else.
**Amend:** add osaka.rs:26 to step 3's sites; drop it from step 4's. Also D6 lists `dash_through` under step 4 and
step 3 lists it too: say step 3.

### F17 (minor) Incomplete caller lists for `strips`/`broken`
D1 says `strips` becomes `raw_strips`, private to room.rs. Unlisted callers that would break: rules.rs:1309
(`judged`), 1754, 2166 (test helpers), tests.rs:833 (the `long_visit_of` floor check), 1785 (stage window check),
12076 (`breaks`). D7 lists `broken` callers "rules.rs:466, 573, 595, tests 1035, 1112, 2336, mod.rs:3367" and misses
rules.rs:1309 and tests.rs:12076.
**Amend:** add them; tests.rs:833 must compare a piece's floor to `room.raw.floor` (equal to `floor.floor`), and
tests.rs:1785 reads `raw.rows`.

### F18 (minor) Census citation wrong
"tests/stillness.rs:844-859 counts them as moving" — stillness.rs has no door code at 844-859 (that's the look-up
exemption) and no `Body::Door` anywhere. The door set-off counting is tests/census.rs:844-859 (`if body ==
Body::Door { doors += n }`).
**Amend:** cite tests/census.rs:844-859, and say whether stillness counts the thirds (it judges cell changes, so the
thirds are new changes in its budget: name `tests/stillness.rs`'s door handling or say it has none).

### F19 (minor) `PARCEL` is already production
D9 and step 7: "`lean` ... and the `PARCEL` constant move into production art.rs". `PARCEL` is at art.rs:1465,
production (used by 1475-1476); only `lean` (a closure in `wall_door_sheet`, art.rs:3615) moves.
**Amend:** "`lean` moves; `PARCEL` is already there".

### F20 (minor) Small ref slips
- stage.rs:533 is a `Scene` match producing jobs, not an exhaustive `Job` match. D6 names `act_summary`, `by_ref`,
  `start_job` and `Heading::row` without lines; give them: osaka.rs:2740, scenes.rs:284, osaka.rs:4891, mind.rs:1540.
- `Act::Door` sites: 30 in osaka.rs, D6 cites about eight. Those that bind `to` and must match `Through`: osaka.rs:2750
  (`act_summary`, `{to:?}`), 4062 (the door tick and `there` teleport), 9205 (the errand retarget writes `to`), 9377
  (`evict`). Those that construct it: 5283, 9079, 9091, 9118, 9139, 9167, 9215, 9426, 9473, 12047 and the tests
  15650, 15866, 15887, 16151. List them in step 4's sites.
- design.md "2397-2399 kept" is the paragraph 2390-2399.
- D9's "in one `paint_layers` call": the parcel needs a `dx` and a clip, so it is one `paint_cuts` call (step 7's
  API).

### F21 (minor) `evict`'s leaving arm assumes she is already through
osaka.rs:9341-9370: "she ... is through her door already (or off the screen)". With `leaving` set at set-off, a
focused pane that doesn't cover her returns `true` and leaves her walking; one that covers her turns the walk into
`Act::Away` at her feet. D6 says "unchanged in shape", which is fine, but the doc comment and the caller's reading of
`true` ("handled") should be re-read. A Leave walk whose path crosses the focused pane is otherwise unhandled.
**Amend:** in step 4a, update the comment and add an evict-mid-Leave-walk unit test (pane not over her: she walks
on, a path into the pane re-plans through `came_to_nothing`).

### F22 (minor) D2: `project` order of pin, choose, move_off
D2 says the wall is chosen "after `move_off`", but `pin_anchors` runs first in `project` (room.rs:1355) and D1 says
it pins floor pieces on `floor`. For an older record (`door: None`), the first frame pins on the raw extent, then
chooses. For anchored pieces that's harmless, since anchors are side+offset. For never-anchored pieces (`anchor: None`,
pinned by share) the share is taken of the raw extent and then shifted by 6 on the next frame. Step 2 test 5 only
covers an anchored fridge.
**Amend:** order `project` as choose (if `None`, on the pre-move strips) → `pin_anchors` → `move_off` → re-choose
only per F14, and add an unanchored piece to step 2 test 5.

### F23 (minor) Pushing between steps ships intermediate states
`install.sh` follows `master`. Step 2 leaves a 6-column gap between flap and parcel; step 3 shows the visit's door
at her feet and the Away door at the space (a hop as she vanishes); steps 3-7 draw the face-on door at a wall.
**Amend:** don't push `master` mid-batch (or push only after steps 4 and 9); say so in the Steps preamble.

### F24 (minor) Step 2's CHANGELOG wording promises the door
At step 2 the door still stands at her feet. "keeps the floor by her door ... clear" is untrue until step 3.
**Amend:** step 2: "Changed: furniture by the screen's edge stands a little further along, leaving room for Osaka's
door"; or fold step 2's entry into step 3's.

### F25 (minor) Step 6 test 1 wording
"her door stands side-by-the-wall at the space" in step 6: the side-on draw is step 8. Write "her door is `Set::Wall`
at the space (face-on until step 8)".
