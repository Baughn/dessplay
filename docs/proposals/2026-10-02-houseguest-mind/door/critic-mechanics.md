# Door batch design: critique, correctness and sibling sites

**Critic, 2026-10-08.** Lens: does each site in the class get fixed, are the claimed invariants really
unrepresentable, is the lifecycle symmetric, and are there frame/tick races or migration and serde gaps. Every
claim below was checked against the tree at `3c4ed354` (`dessplay/` unchanged since `057311b1`). Paths are relative
to `dessplay/src/ui/houseguest/`. Severities: **blocker** (the design as written reintroduces the bug or breaks a
user decision), **major** (a sibling site or lifecycle path left broken), **minor** (an unclear spec, a wrong
claim, or a naming or compile trap).

## Blockers

1. **[blocker] D6 / C3: setting `leaving` when she sets off ends the visit mid-walk, at a door in space or a screen edge.**
   - `gone_out` (osaka.rs:5296) is `leaving && hidden(now) && door(now).is_none()`.
   - A Leave walk across floors goes through `go_to` (osaka.rs:8026). With no route, that is `through_door` (8052-8060 → 9076: `Act::Door { gap: 0 }`). Beat 6 of `DOOR` (`door: None, her: false`, 600 ms, osaka.rs:1239ff) then satisfies `gone_out`, so `out_by_door` ends the visit **at the door in space**.
   - With a route that is `Route::Around`, the `Act::Out` step (osaka.rs:4013-4030) sees `leaving.is_some()` and sends her `Act::Away { until: u64::MAX }`: out for good **off the screen's edge**. That is the very exit settled fact 3 removes.
   - The same holds for evict's leaving arm (osaka.rs:9345-9370) and for any other `through_door` while `leaving` is set.
   - So D6's "every exit through her door" is broken by D6's own latch. Today it is safe only because `leaving` is set at the door (osaka.rs:5279).
   - **Amendment:**
     - Split the two jobs. A new `set_off: Option<Routine>` is the line latch: set at first entry, and cleared wherever `leaving` is cleared today (`errand` 9181, `place` 8611) and on the visit's end.
     - `leaving` is set only when the external door opens (`start_job(Job::Leave)`, or the at-spot arm), as today.
     - Alternatively, make `gone_out` require `matches!(act, Act::Door { to: Through::Home(_) | Through::Space(_), gap: u64::MAX, .. })` and the `Act::Out` arm require `heading` not to be a Leave. The split is simpler and makes mid-walk "out for good" unrepresentable.
     - Evict mid-walk then takes the non-leaving arm: a door elsewhere, and the reflex re-enters. Say so, and rewrite the D6 evict bullet.
     - Test: `a_leave_walk_across_floors_never_ends_the_visit_before_her_door` (two floors with no route, and one with an `Around` route; both modes). It must be red under the design as written.

2. **[blocker] D1: `kept` is computed before `pin_anchors`, so on a migrated record a whole strip vanishes or moves.**
   - `Home::on` (room.rs:1176-1183) keeps only anchored pieces (`p.anchor?`). For a record whose pieces have only `at` (ledger.rs:8-13, records older than `anchors`), `kept = pack(&self.on(..), narrowed)` sees an empty or partial set and is true.
   - D1's trap says "pin floor pieces on `floor`", so they are pinned on the narrowed extent. Once they are pinned, `on` returns them all. If they don't pack narrowed, `laid_and_shifted`'s `let Some(..) = pack(.., e) else { continue }` (room.rs:1241-1243) lays out **nothing** on that strip for that frame, if it still uses the stale `Room`.
   - With a recomputed `Room`, `floor` flips to raw, and the anchors were pinned against an extent the strip no longer uses.
   - Whether `project`'s `packs` check (room.rs:1356-1360) reads raw or floor isn't specified either. On `floor` with a stale `Room`, the whole room is `move_off`'d.
   - **Amendment:** `pin_anchors` always pins on the **raw** extent, as today (`at` is a share of the pane, the older builds' meaning), and `extents` is computed **after** pinning. Write it in D1 and in step 2's traps, replacing "pin floor pieces on `floor`".
   - Make the stale case unrepresentable. `laid_and_shifted`/`laid_on`/`hung_shifts` take `nooks`, call `self.extents(nooks)` themselves, and return the rooms with the layout (`struct Laid { shown, shifts, rooms }`). `broken`/`judge` take that bundle, not a separate `&[Room]`. That also removes the "548/573/595 must use `scratch.extents`" trap: `scratch.laid(nooks)` can't be handed `before`'s rooms.
   - Test: an older record without `anchors` whose door strip packs raw but not narrowed lays out every piece on raw, with nothing closeted and nothing moved (extend step 2 test 5).

## Major

3. **[major] D2 / D3 / D9: the first delivery (every new user's TV) is admitted in what becomes the space; no-home spaces aren't kept out.**
   - D2 chooses `door` only once the home has a prop. The first `doorstep` (mod.rs:3322) runs on a doorless home, so `admits` packs at `Anchor{Right,0}` on the raw extent (room.rs:1555-1560) and its "refuse a delivery after which the space would yield" judges `with.extents` with no door: nothing to refuse.
   - The next `project` chooses the wall, and the TV box jumps 6 columns. Its `roomy`/seat checks were done at the old spot, and in step 9 its first flap is `draw_flap`'s, not the door's.
   - Symmetrically, `Keep::of(rooms, chat)` on a doorless home has no spaces. A makeshift piece can be built where the **no-home door** (`choose_wall` unsaved, D4.1) stands, and D4 step 2 doesn't look at pieces, so the door opens on it. That is the bug class, on every user's first visits.
   - **Amendment:** one function, `fn wall(home, nooks, chat, screen) -> Option<DoorWall>` (`home.door` or `choose_wall` unsaved). `door_place`, `Keep::of`, `doorstep`/`admits` (judged on `with` plus `with.door = wall(&with, ..)`) and D9's "her door's wall first" all read it.
   - Test: a fresh ledger's first TV rests with its trailing edge at `w-7` on the frame it arrives (red as designed). No made piece meets the no-home door's space.

4. **[major] D4: the strict fallback ignores makeshift pieces, the pocketed piece and closeted pieces.**
   - `Chances.door` is `door_place(.., &visit.shown, ..)` (D6), and the Away arm uses `paint_empty`'s projection, which is blocked by **gated** `view.protected` (mod.rs:4697-4704). `visit.made` (mod.rs:493) and `visit.ghost` (mod.rs:3403; the carried piece is removed from `shown`, 3406) aren't in `shown`, nor is a piece closeted by text or under a focused pane.
   - So a fallback door can stand on a makeshift piece in a visit. It can also stand on a piece hidden under a focused pane while Away: when the pane is released the piece comes back under the door, and the door hops. That defeats C2 on the fallback path.
   - **Amendment:** the strict check reads `home.layout(nooks)` covers (laid, not shown: closeted pieces count), plus `made` covers, plus `ghost`. Name it `obstacles: &[Rect]` so every caller passes the same set. D4 step 2 also refuses a space whose rect meets any of them (defence in depth: free while it's kept, and it catches finding 3's case).

5. **[major] D2: the chooser saves an inner wall over an edge wall for an older home, against the user's decision.**
   - "The first wall whose strip has `rows >= HEIGHT` and whose floor pieces pack narrowed wins." For an older home whose edge strips are crowded, that saves an **inner** wall, or another strip's wall, for good.
   - The user said: a space at the screen's edge, and for older homes "she clears it herself through a felt rule; until then the space yields".
   - **Amendment:** the chooser qualifies walls by geometry only: outside the chat, `raw.rows >= HEIGHT`, and `raw` wide enough for the space plus the strip's widest piece. Packing narrowed is only a **tie-break among edge walls**. Inner walls are chosen only when no edge wall qualifies geometrically.
   - Rewrite step 2 test 6 to include "a crowded edge strip is still chosen; the space yields; DoorClear is broken".

6. **[major] D2 / C7: `move_off` is a placement site that no section keeps out of the space or the chat, and its re-choose rule contradicts itself.**
   - `move_off`'s target test packs on the target's `e` from `strips` (room.rs:1415) and checks only `fits(.., !blocked)` (1420).
   - With `extents` it must pack on the target's **`floor`**. Otherwise moving pieces onto the door's strip silently makes its space yield. It must also refuse a target where a moved piece's cover meets `Keep`. D3's site list omits it.
   - D2 says to re-choose "when its strip isn't among `nooks`", and also that "`move_off` with no target leaves `door` as it is". When the door strip vanishes and `move_off` finds no target (room.rs:1423-1425 returns silently), the pieces stay on the gone strip, but the first rule moves the door permanently. A pane hidden for a moment then leaves furniture and door apart for good.
   - **Amendment:** re-choose only (a) when `door` is `None` and a wall qualifies, or (b) when `move_off` actually moved the door strip's pieces (to the strip they moved to). Never write `None` over a saved wall. Add the "hide a pane and show it again" case to step 2 test 6.

7. **[major] D7: DoorClear on a strip that can never keep the space (short terminals) makes her rearrange for nothing, on every return.**
   - D1 always builds `space.rect` for the door strip, and DoorClear judges it "kept or not". Map B5 says that at H ≲ 23 the space **always** yields (`rows < 4`), so every short-terminal return comes through `Set::Floor` (`fallback_in`). She feels DoorClear for any piece by the wall and, if Industrious or Ordinary, moves it. The moves are permanent, for a space the frame can't hold.
   - `hung_clear` (D1: "refuses `space.rect`, kept or not") likewise closets a window for an impossible space. `Keep.spaces` "kept or not" refuses placements there too.
   - The rect itself underflows: `f-4` as u16 when `f < 4` (`Rect::new` from i32).
   - **Amendment:**
     - `Space` exists only when `raw.rows >= HEIGHT` and `raw` is wide enough for the space and the strip's widest piece. Otherwise `space: None`, and there is nothing to judge, keep out or refuse.
     - `fallback_in` is set only for `Why::Yield` (pieces in the space), not for `Protected`/`NoWall`/short.
     - Test: `on_a_short_terminal_she_never_feels_her_door_blocked` (H 18-22, both modes).

8. **[major] D6: a Leave walk with a stale spot opens her door at the old spot; the "came_to_nothing" claim is wrong.**
   - D6 claims "A Leave walk whose spot moved ends in `came_to_nothing` (osaka.rs:4411)". The walk's `to` is the **stale** x (`pursue`, osaka.rs:8500-8511), so she arrives exactly at `job.spot()`. The check at osaka.rs:4405 (`job.spot() == (self.x, self.y)`) passes and `start_job` opens the door at the old spot.
   - Visits survive a resize (`visit.size = size`, mod.rs:2301), and the space also moves without a resize: a piece moved off un-yields it, or `door` is first chosen mid-visit. The old spot can then be over a piece: the bug class.
   - Also: with `chances.door == None` mid-visit, D6 keeps a `Through::Home` act's stale spot ("kept as it was when `None`"), so the overlap is representable again.
   - **Amendment:**
     - `start_job(Job::Leave(spot))` (it has `chances`, osaka.rs:4636) compares `spot` with `chances.door`. If they differ, it re-routes (`go_to` with the fresh spot, saying nothing because of the latch). If it is `None`, it opens at her feet as `Through::Space`.
     - `take_in` refreshes a `Through::Home` act's spot when `Some`, and **converts** it to `Through::Space(her feet)` when `None`, before the `there` beats.
     - Test: `a_resize_mid_walk_moves_where_she_goes_out` (shrink the width so the right wall moves; both modes).

9. **[major] D6: no gap source when a Leave walk reaches its spot; work and school share `Job::Leave`.**
   - D6 gives the gap only for the at-spot arm (`go_out`: MAX; `go_to_work`: `rng.range(SHIFT_MS)`). When the walk arrives, `start_job` (osaka.rs:4636ff, "starts the door") has no rule.
   - School can also begin mid-walk to work: `cut` → `cut_shift` + `school_from_work` (osaka.rs:3521-3526, 3552) sets `leaving = School`, and D6 makes the Leave walk uncuttable, so she arrives with both states mixed.
   - **Amendment:** `start_job(Job::Leave)` derives the gap from state, in order: `leaving`/`set_off` is `Some` → `u64::MAX`; `shift == Some(Going)` → `rng.range(SHIFT_MS)`; else (both were cut) she decides instead of opening the door.
   - Or make `Job::Leave { spot, why: Leave::School | Leave::Work }` and have `school_from_work` rewrite the job's `why`.
   - Test: `walking_out_to_work_as_school_begins_she_walks_on_to_school` (osaka.rs:13561) asserts `gap == u64::MAX` at the door.

10. **[major] D6: evict's non-leaving `Act::Door` arm rewrites an external door's `to` (work has no `leaving`).**
    - In evict, `Act::Door { to, .. }` not yet `there` and `!clear(*to)` → `*to = calm_elsewhere(..)` (osaka.rs:9375-9387). Work's door never sets `leaving`, so with `to: Through` this arm must be specified.
    - As written it either won't compile or silently turns her work door into a door in space with a shift gap. She would then come home from work out of a random calm spot. `errand`'s retarget (osaka.rs:9198-9208) needs the same `Through` mapping (specified: `Space(accordion)`).
    - **Amendment:** in evict, `Through::Home(spot)` whose box meets the focus → `Through::Space(calm spot)` with the gap kept. Her return is then through her door: at `there`, a shifted `Through::Space` with `shift == Going` re-reads `chances.door` (or say plainly that she comes home where she was evicted to).
    - Test: `evicted_at_her_door_on_her_way_to_work_she_comes_home_by_it`.

11. **[major] D5 / D6: placing against `unkept` makes arrivals and Leave walks target a focused pane.**
    - C2 (place against `unkept`) is right for the **drawn Away door**. D5 also uses it for arrivals: "run `door_place` on the `unkept` terrain". Today's arrival reads the **gated** terrain (mod.rs:2026), so she never comes out inside a focused pane. As designed she can, when the resident's focused pane is the door's (Users/Playlist): a regression.
    - Likewise `Chances.door` (D6, "against `unkept`") sends a Leave walk into the focused pane, which `evict` then pushes her out of. That is a loop (the latch keeps her quiet but she shuttles).
    - **Amendment:**
      - Arrivals: if the `DoorSpot`'s door cells or her box meet gated `view.protected`, take the no-room path (wait), as today.
      - Visit: `Chances.door` is `None` while the spot's box meets gated `protected`, so she leaves by a door in space at her feet (the user's own fallback). The Away draw keeps C2.
      - Test: `with_her_doors_pane_focused_she_neither_comes_out_nor_walks_into_it`.

12. **[major] D4: the strict fallback hops with pane text (per-frame, no stickiness), on the path short terminals always take.**
    - Today's Away door is sticky: it moves only once it no longer fits (`if !door.is_some_and(fits)`, mod.rs:4712-4720). D4 recomputes "nearest the space's spot … her box calm" every frame. A line of text appearing under or near the fallback moves the door, and it moves back when the text goes.
    - By finding 7 / map B5, the fallback is the **normal** path below H≈23. The stillness rule says change draws the eye ([memory] her movement draws attention).
    - **Amendment:** in Away and in a visit, keep last frame's `Set::Floor` spot (`Empty.door`, or a visit field) while it still passes the strict check with text ignored, the same as the space's text rule (hidden while not calm). Search again only when a piece, the chat, a protected rect or the floor refuses it.
    - Test: `her_fallback_door_doesnt_hop_with_pane_text` (wordy panes, H 20, both modes).

13. **[major] D7: the DoorClear trigger misses work's return and fires on the wrong reasons.**
    - `fallback_in` is set only on arrivals (D5). Work's return is in-visit (`there` → `back_home`, osaka.rs:4064-4072), so a lamp in the space is never felt after work.
    - The trigger also fires for any `Set::Floor`, including `Protected`/`NoWall`/text, where she didn't "bump into" a piece.
    - **Amendment:** set the flag at `there` for a `Through::Home(spot)` with `spot.at == Set::Floor` and `why == Yield`, and on arrivals only for `Why::Yield`. `DoorSpot` carries its `Why`, or `door_place` returns it alongside.
    - Also say in D7 that `How::Idle` arrivals and unfed clocks (no school) never feel a use-less piece in the space. That is accepted, and the lamp waits.

14. **[major] D10: `Scene::School` as written ends the visit at beat 6, so the coming-in thirds are never seen.**
    - "She sets off … comes back after a 5 s gap (`Through::Home`, gap 5000)". If the scene goes through `go_out`, `leaving` is set, so `gone_out` fires at beat 6 and the visit ends. The 5 s return never happens.
    - **Amendment:** the scene doesn't set `leaving`/`set_off`. It sets `returning = Some(Routine::School)` (consumed by `back_home`, osaka.rs:9487) and walks a Leave job whose gap is 5000. Its stage test asserts she is visiting throughout and says "I'm home!".

## Minor

15. **[minor] D1: name collision.** room.rs already has a private `enum Room { None, ToUse, ToLook }` (room.rs:1789), used by `admits`. D1's `pub(super) struct Room` in the same module won't compile. Rename it to `Laid`/`StripNow` (or rename the old enum `Ask`), and fix every D1/D3/D7 mention.

16. **[minor] D2 / D3: the `project` call sites list is incomplete.**
    - `doorstep`'s `offers` closure calls `with.project(buf, nooks, blocked)` (room.rs:1576). It needs `chat` (and it runs the chooser on a scratch home, which is harmless; say so).
    - The arrival's new `project` (D5) mutates the ledger, so it needs `self.unsaved |= changed` like the Away arm (mod.rs:2123). Otherwise a wall chosen there is never saved: the next `paint_empty`/`furnish` diff sees no change.
    - The rules tests calling `project` (rules.rs:1331-1374, 1412, 1732, 1918, 2310, 2326) and room.rs tests (1937-1955) need the new argument. Count them in step 2's size.

17. **[minor] D2: a saved wall whose space comes to meet the chat (a layout change) is never re-chosen.**
    - D2 refuses chat walls only at choose time. After a layout change it keeps narrowing a strip for a door that `door_place` (D3: refuses the chat) never uses, so the door sits at the fallback for good.
    - **Amendment:** re-choose (saved) when the saved wall's space meets `chat`. Add the case to step 2 test 6.

18. **[minor] D3: "one shared predicate" vs cell-based checks.** `Frame::free` (rules.rs:428-441) tests a piece's **rect** cells. `Keep::refuses(cover)` tests covers. D3 says "`Frame::free` refuses keep cells", which is cell-based and misses the floor row of a piece whose rect ends just above the chat's top in overlapping layouts. **Amendment:** in `search`/`evaluate`, call `keep.refuses(at.cover())` once per candidate, not per cell, so there is truly one predicate.

19. **[minor] D3: `made_stands` has two callers.** `tend_made` (mod.rs:3798) and `furnish`'s delivery `seats` closure (mod.rs:3313) both call it. The new `Keep` parameter must be plumbed to both, and for the delivery the keep is the **after** home's (finding 3). Say so in step 5's sites.

20. **[minor] D7: `stranded` is purely geometric, while the InChat repair has more conditions.**
    - A piece can be not stranded (some strip "takes it") yet have no repair `evaluate` accepts (`becomes_den`, `spoils`, "no rule that held is broken", mood: Lazy never mends). It then stands in the chat **indefinitely**, which the user accepted only "until she moves it out".
    - Two stranded candidates competing for one free stretch are each judged "taken".
    - **Amendment:** at least document the gap (the user's rule reads "if no strip takes it"), and test that a Lazy home with a chat piece keeps showing it. Better: count a stretch as taking a piece only if the pieces it would host fit together (pack all stranded candidates for that room jointly).

21. **[minor] D5: arrivals must build `Through::Home` acts.** D8 tells the external door apart "by type", but D5 only says `back_through_door`/`dash_in` "take the `DoorSpot`". Spell out `Act::Door { to: Through::Home(spot), .. }` for both, and for the stage `dash_through`. Otherwise the coming-in door is drawn face-on and the census miscounts.

22. **[minor] D5: no door at all means she never comes home.**
    - "No spot: the existing no-room path (2059-2077)" retries after each idle delay. In a layout where `door_place` always returns `None` (every floor outside the chat full or covered, which the user allowed for going out: "a door in space where she stands, no door drawn"), the return never happens.
    - **Amendment, for symmetry with going out:** with no `DoorSpot`, an arrival comes out of a door in space at a calm spot (`calm_elsewhere`, outside the chat), as `Through::Space`.

23. **[minor] D8: `gone_out` must keep reading `DOOR`, not `wall_beat`.** `gone_out` keys on `door(now).is_none()` (osaka.rs:5296-5299) during beat 6. D8 draws `Shut{away: true}` on beat 6 for the external door. Keep that in `wall_beat` (draw only), and never change `DoorBeat.door` for beat 6, or a school door never ends the visit. Add an assertion to step 8 test 3.

24. **[minor] D2 ledger: anchors' meaning changes under older builds.**
    - Anchors on the door strip are now relative to the **narrowed** wall, and `at` is pinned on narrowed (set-down pin, mod.rs:3259). An older build (the `stable` track can lag `master`) reads the same anchors against the raw wall, so those pieces stand 6 columns nearer the wall, in the space, and its re-save drops `door`. Nothing is lost.
    - Say this in the ledger module doc and in decisions.md. Keep `at` a share of the **raw** extent (finding 2) so older builds' fallback stays right.

25. **[minor] D6: `go_to` returning `false` is unhandled.** `go_to` returns false when the spot isn't on a platform of the visit's terrain (osaka.rs:8062), for example under a gated pane or with text splitting the platform. D6 says "`Decision::reflex("routine/away")` either way". **Amendment:** on `false`, use the `chances.door == None` arm (a door in space at her feet).

26. **[minor] Step 2 test 0 / test 5 depend on the chooser picking Users.** With D2's "her pieces' strips first", the chooser picks Users in `rooms(100,30)` only if Users' right wall is a screen edge there (tests.rs `nooks()`, List at `(0,0,w/2,h-3)`). Assert the chosen `DoorWall` explicitly in both tests, so a chooser change fails them loudly rather than moving the fridge for a different reason.

27. **[minor] D1 / D3: `hung_clear` can move a hung piece into the chat.** `hung_clear` (room.rs:1281ff) moves a hung piece to the nearest clear column using only the strip's extent. By D1, `extents` knows nothing of the chat. In an overlapping custom layout, that can shift a window **into** the chat, frame by frame. D3's keep-out covers only new placements, and D7's InChat judges where a piece is laid (at a stand-in place), so neither covers this move. **Amendment:** the move's candidate filter (`!meets(l)`, room.rs:1320) also refuses a left whose rect meets the chat. That means passing `chat` to `laid_and_shifted`, or applying it in `project`. Otherwise, state that a shifted window in the chat is judged InChat like any other laid piece. This arises only in custom layouts, like finding 17.
