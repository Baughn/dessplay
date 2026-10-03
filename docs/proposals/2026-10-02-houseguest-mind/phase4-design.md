# Phase 4 (organising) — implementation design

Brief: docs/plan.md "Phase 4 — organising (brief)". Proposal: docs/proposals/2026-10-02-houseguest-mind.md
("Rules, not an objective", "One repair a visit", "Every want answers a need", "Her mood").
Code map (read it; file:line refs): scratchpad/map.md. Paths relative to dessplay/src/ui/houseguest/.

## User decisions (2026-10-02, this session)
- **Poster is a real wall piece** (hangs above the floor, may hang over floor pieces). This
  reverses the 2026-09-29 "props only stand on floors" rejection (record in decisions.md).
- **Decor art: show the user a props sheet before wiring it in.** Everything else proceeds.
- **Home-act caps by mood:** lazy 0, ordinary 1, dreamy 1, industrious 3.
- **Felt rules are remembered for the visit only** (an Osaka field; nothing persisted).

## Design decisions (mine; record the non-obvious ones in decisions.md)

### D1. Layout vs projection; rules judged on the layout
- Split `Home::project` into `Home::layout(&mut self, nooks) -> Vec<Placed>` (anchor pinning,
  `move_off`-free packing per strip; pure geometry, no buffer) and the existing fit filter.
  `move_off` still needs the buffer (it checks fits), so `project` = pin anchors → move_off where
  pack fails → layout → fits filter. Rules read `layout` output (`Shown` for every packed piece,
  closeted or not) plus `strips(nooks)` extents. Text over a piece (closet) never breaks/satisfies
  a rule; a strip that is gone/too small is what moves pieces (unchanged).
- Boxed pieces are ignored by every rule (a parcel isn't furniture yet). A rule applies only when
  every piece it needs is owned and unboxed (Near: the lamp and at least one of its partners).

### D2. Rules (new module `rules.rs`)
```rust
pub(super) enum Rule {
    Faces { seat: Furniture, screen: Furniture },            // Sofa→Tv: same strip, gap FACING_GAP, seat turned toward it
    Near { a: Furniture, b: &'static [Furniture], gap: i32 },// Lamp by Bed|Desk, ≤ 3 cells between, same strip
    AgainstWall(Furniture),                                  // Fridge, Bookshelf: projected edge ≤ 1 cell from a wall
    Apart { a: Furniture, b: Furniture },                    // Bed, Tv: not on one strip (a room = a strip until phase 6)
    Belongs,                                                 // see D3; one row, judged per unsettled piece
}
pub(super) struct RuleRow { pub rule: Rule, pub felt_on: &'static [Use], pub grievance: &'static str }
pub(super) const RULES: &[RuleRow] = &[...];
pub(super) struct Broken { pub row: usize, pub pieces: Vec<Furniture> /* the movable ones, rule order */ }
pub(super) fn broken(layout: &[Shown], strips: &[(Strip, Extent)], home: &Home) -> Vec<Broken>
```
- Faces now checks the seat's facing: `seat.facing == Right` iff the screen is to its right. The
  TV is frontal, so its facing doesn't count. `room::faces` gains the check, and `spots_for` uses the
  sofa's own facing for the Watch seat (fixes the `tv.left > sofa.x` derivation). Made sofas
  (strip None) keep the same-floor rule and are never judged by rules.
- `felt_on`: the uses during which the grievance is felt — Faces: Lounge, Nap (on the seat) and
  Watch (beside the TV: "Can't see from the sofa..."); keep one line per row for now. Near:
  Sleep/Homework ("Too dark in here..."). AgainstWall: Snack/Read ("This wants a wall..."). Apart:
  Sleep/Watch ("Too noisy to sleep..."). Belongs: any use of the piece ("Hm, not here..."). Every
  line ≤ 24 chars (add RULES to `every_fixed_line_fits_a_bubble`). Final wording is the
  implementer's; keep her voice (cf. "...my sofa.").
- Which pieces may move: Faces → seat then screen; Near → a; AgainstWall → the piece; Apart → both
  (unsettled first); Belongs → the piece.

### D3. Unsettled deliveries
- Persisted flag: new top-level ledger field `unsettled: [Furniture]`, `skip_serializing_if` empty
  (so `the_record_as_written` and older records stay byte-identical; a missing key = all settled).
  In memory: `Prop.settled: bool` (Prop::new → true). `doorstep` deliveries and stage gifts (`spot`)
  are unsettled (she never chose where they stand); a committed SetDown settles. Unpacking doesn't.
- `Rule::Belongs` is broken for an unsettled, unboxed piece that **spoils** its room: the strip's
  role without the piece is not Den and differs from the role with it (a bed in a living room, a
  fridge or TV in a bedroom). Apart overlaps for bed+TV; fine.
- Repair cost for an unsettled piece is ranked by role tier first, then cells: 0 a room whose role
  the piece completes (role with it ≠ role without it, and not Den); 1 a room it doesn't spoil; 2
  an empty strip; spoiling candidates don't qualify (they would break Belongs anyway). Settled
  pieces: tier 0 everywhere (cells only).

### D4. Feeling, nesting, Arrange
- `Need::Nesting` and `Need::Beauty` appended (Need::ALL order = levels index). Nesting arrives at
  0, rises (rise_ms ≈ 3 min, tune in visit_census) **only while a felt rule is broken**; Beauty see
  D8. Replace `Needs::pass`'s positional `mess: bool` with `Rising { mess, grieved, plain }`.
  Mood: Industrious nesting ×1.6; `Mood::home_acts()` = Lazy 0, Ordinary 1, Dreamy 1, Industrious 3.
- Feeling: `Chances.broken: Vec<Broken>` built at paint (both Chances literals). In `start_job`
  for `Job::Use(seat)` on a real piece, if a broken rule names that piece and its `felt_on` contains
  `seat.what` and it isn't felt yet → `Act::Use { grievance: Some(row), .. }`; `use_look` shows it
  as a timed bubble window (like `advert`; e.g. the first 2 s after the first 1.4 s frame, with a
  craning `Face::Curious`/`Pose` beat), and `fire`'s frame tick marks it felt (`Osaka.felt: Vec<usize>`)
  once the window has passed. Grievances do **not** use `Lines` (no budget/cooldown): each rule is
  felt once a visit by construction.
- `Want::Arrange` appended to `Want::ALL`; `def`: `row(6.0, &[(Need::Nesting, 1.0)])`. Offered only
  when `Ctx.may_arrange` (felt ∧ still broken ∧ `home_acts < mood.home_acts()` ∧ no visit cap
  reached) and a method binds. Cap is read from `mood()` when needed (census forces mood after
  arrival; stage cycles it).

### D5. The repair search (room.rs, buffer-taking; run at paint, throttled)
- **Deviation from the proposal** (record): runs in `furnish`/paint, not `advance`, so it uses the
  one `fits`/`roomy` definition on the real frame (decisions 2026-10-01 "one definition"); no blank-
  interval snapshot. Throttled: recomputed when the home, the nooks or the felt∧broken set changes,
  or at most once a second otherwise; only while Osaka wants one (felt ∧ broken ∧ may arrange ∧ no
  pocket). Result goes to `Chances.repairs: Vec<Repair>` (up to 3, cheapest first).
- `Repair { piece: Furniture, to: Placement { strip, anchor, facing }, at: Shown /* where it'd
  stand */, cost: u32, tier: u8 }`. Candidates: each movable piece of the felt broken rule (D2), each
  strip, each distinct packed left (dedupe via `pin`), both facings; one piece moved per candidate,
  applied to a clone (index unchanged), `layout` on the clone. Cheap filters first (geometry only):
  (a) the rule is satisfied, (b) no rule satisfied before is broken after (all applicable rows,
  Belongs included), (c) every strip still packs (no `move_off`). Then in (tier, cost, item order)
  order, fit-check lazily until 3 qualify: the moved piece fits on blank free cells and is `roomy`
  (wall/decor pieces: fits only), and every other piece that showed before still fits after the
  re-pack (the `doorstep` check shape). Made pieces' rects count as occupied.
- Cost = Σ over pieces whose layout changed of |Δx| + |Δfloor row| (neighbours pushed count), + 1
  for a turn. A pure turn in place is the cheapest repair for a sofa facing the wrong way.
- Budget: count candidates, ≤ 2,000 (stride lefts on very wide strips); test asserts the count
  always and ≤ 1 ms only in release (`repair_search_is_cheap`, terrain_read_is_cheap pattern).
- Ties broken by item order then a whim in the guard (`w.below("repair", n)` over the tied set).

### D6. The pocket carry
- `Osaka.pocket: Option<Carry>`; `Carry { piece: Furniture, to: Placement, from: (i32,i32) /* old
  spot, for the glance */, tries: u8, trials: Trials }`. Lives outside `Act`, so chat, errands,
  eviction (through her door with her) and settle leave it; goodbye drops the Visit (ledger anchor
  never changed → piece where it was). `place()` (stage cues) drops it back silently.
- While carried, the piece keeps its ledger anchor (its slot stays reserved, neighbours don't
  reflow) but is filtered out of `visit.shown` after `furnish` (no draw, no cover, no seats, not in
  the goodbye rain). Guest reads `osaka.carrying()`.
- Methods, `ARRANGE` in this order: `arrange/use-it` (just set down: bind the felt use's seat on that
  piece — for a sofa prefer Watch, then Lounge; credit goes to `Want::Use(what)`), `arrange/carry`
  (pocket: `Bind::Job(Job::SetDown(SetDown{piece, to, spot}))`, where `Chances.drop_at` gives the
  spot she stands at, beside/centre of `Repair.at` on its floor, recomputed at paint from the pocket's
  target by meaning, so resizes are followed), `arrange/lift` (no pocket: pick from
  `Chances.repairs`; `Bind::Job(Job::Lift(Lift{piece, to, spot}))`, walk to the piece).
- Acts: `Act::Lift { since, until, piece }` (~900 ms, "Hup!", ToeTouch beat; at the end the pocket is
  filled) and `Act::SetDown { since, until, piece }` (~900 ms, "There!"; at the end pushes
  `HomeEvent::SetDown { piece, strip, anchor, facing }`). Classify in `props()` (Job/Look),
  `first_due`, `fire`, `acting`, `act_summary`, census `group()` ("home").
- Commit at paint: `record()` moves `SetDown` to `visit.set_down` (pending); `furnish` applies it on
  a clone, checks the moved piece fits (+ roomy) and every other showing piece still fits; on
  success writes strip/anchor/at/facing, `settled = true`, and calls `osaka.set_down_done(piece)`
  (pocket → None, `just_set = Some(piece)`, `home_acts += 1`, `credit_whole(Want::Arrange)`); on
  failure `osaka.set_down_refused(now)` → dropped carry. `commits_survive_the_visit_ending` keeps
  holding for Bought/Unpacked (still committed in advance); a SetDown pending at goodbye is dropped,
  by design (the piece stays at its old anchor) — test it.
- Continuation: after the reflexes and the owed beat, a full pocket binds `Want::Arrange` as a
  Continuation (like `leftover`), up to `TRIES` = 3 settings-off; then dropped. `just_set` is a
  one-shot continuation for use-it.
- Dropped carry (refused, target gone with no fitting re-target, tries out): pocket → None, the
  piece shows at its old anchor again, owe `Loss::Moved(item)` (a glance toward `from`, maybe
  "Oh well...").
- Trials: when ≥ 2 repairs are within `TIE_CELLS` = 4 of the cheapest at the same tier, the lift
  takes the whim's pick and keeps the others as `Trials`; after set-down she sits/uses it briefly
  (use-it, short) and says "hmm…"; at the end she keeps it with p = exp(−Δ/T) (Δ = (cost − min) /
  TIE_CELLS, T = max(restless, 0.05)), rolled with `Whims` at the next decision (mind stream; no
  body-rng draws); else she lifts again ("Hup!") toward the next trial spot. At most 3 spots; the
  last is always kept; the episode is one home act (counted at the first commit). A trial spot
  always satisfies the rule, so whatever she keeps breaks nothing.

### D7. Wall pieces
- `Spec.hang: Option<u16>`: rows between the floor row and the piece's bottom row (None = stands on
  the floor). Poster ≈ 4×2, hung with its bottom `HEIGHT + 1` rows above the floor (above her
  head, clear of a sofa). Wall pieces pack in their own lane on the strip (pack floor pieces and
  wall pieces separately, same anchor rules), may hang over floor pieces' columns; `Shown.rect()`
  lifts by `hang`; `cover()` = rect (no floor row) for wall pieces; `fits` drops the floor-line
  requirement for them; `Extent.holds` needs `rows ≥ hang + rows`. In line art `prop_layer`
  uses `standing: false` with `at` at its bottom row. No seat, no uses (roomy trivially true);
  terrain covers include it (her image merges it when she passes under).
- Delivery: through the flap like any piece, hung against that wall; the parcel stands on the floor
  while boxed (a boxed wall piece is a floor parcel; unpacking hangs it).

### D8. Decor and beauty
- New kinds appended to `Furniture::ALL` (keep proptest `0..4` indices): `Plant` (floor, ~3×3,
  beauty 1.0) and `Poster` (wall, beauty 1.0). `Spec.beauty: f64` (0 for every other piece),
  `Offer::Decor` (no RoleRule names it). `CATALOGUE` grows (append); `RoomKind::of` → Living.
- `Need::Beauty`: arrives 0.3, rises (rise_ms ≈ 20 min) only while she is in a plain room (the strip
  she stands on has 0 beauty, `Chances.beauty_here`), and is served passively by rest/use acts in a
  pretty room in `credit_done` (share × min(beauty, 1)). Not part of `score` (no want answers it).
- The channel (`advert`) sells the first unowned decor instead of furniture when beauty is her most
  pressing need, or when no furniture is left to sell; the rest as today.

## Commit plan (each passes fmt + clippy -D warnings + nextest; golden re-records with reasons)
1. `refactor(houseguest): her layout, apart from what fits` — `Home::layout`; golden unchanged.
2. `feat(houseguest): she watches only from a sofa turned toward the TV` — facing in `faces`,
   Watch seat facing; the test table flips; re-record goldens if moved.
3. `feat(houseguest): the rules of her home` — rules.rs, `broken()`, unsettled flag + ledger field +
   Belongs; tests incl. ledger golden files; stage `x` row shows broken rules.
4. `feat(houseguest): she feels what's wrong` — grievance in Use, felt, Nesting, Rising, mood caps,
  `Want::Arrange` row (no methods yet → never offered); lints.
5. `feat(houseguest): the repair search` — room.rs search + properties + budget test.
6. `feat(houseguest): she pockets a piece and sets it down where it's right` — pocket, acts,
   SetDown commit/refusal, continuation, use-it, Scene::Arrange cue, interruptions tests.
7. `feat(houseguest): trials` — tie trials.
8. `feat(houseguest): pieces that hang on the wall` + `feat(houseguest): decor and beauty` (art
   waits for the user's sheet review; mechanics can land with placeholder-free art only after it).
9. docs: design.md rules, decisions.md entries, plan.md record (censuses), CHANGELOG.

## Tests (beyond updates)
- property: a home whose rules all hold offers no repair and no Arrange (satisfied rules never move).
- property: every repair the search returns satisfies its rule, breaks none, fits, ≤ 2,000 candidates.
- property (multi-visit): no piece returns within 3 visits to a spot it left, unless a resize forced it.
- property: home acts per visit ≤ the mood's cap; lazy never arranges.
- image budget with a carry (extend `a_furnished_home_gets_used_and_stays_cheap` or new kitty test).
- unit: sofa turned away → lounge grievance → felt → nesting rises → Arrange → turn → watches.
- unit: chat mid-carry resumes; goodbye mid-carry leaves piece at old anchor (ledger unchanged);
  refused set-down → old anchor + glance; eviction mid-carry carries on; unsettled TV joins the sofa.
- every test in both drawing modes with text-dense panes where terrain matters (memory).

## Amendments after review (these override the sections above)

A1 (D7). **The wall lane never fails a strip.** `move_off` and the "strip too small" test consider
the floor lane only. A wall piece that doesn't fit its strip's height, or can't pack in the wall
lane, goes to the closet alone (like text over it); it never moves the room. `graphics::Layer::bounds()`
for `standing: false` puts `at.1` on the row *below* the image: so `at.1 = floor - hang`. Check the
fixtures' pane heights (`home_screen` Users is 9 rows → 7 clear; resident `rooms()`) against
`hang + rows` and pick `hang` so the poster shows in them (HEIGHT = 4; a hang of HEIGHT+1 = 5 plus 2
rows needs 7 clear rows — on the edge; prefer hang 4 or 5 and verify).

A2 (D6). **SetDown ends in a short timed act, not `decide`.** As Unpack/Crumple end in `Admire`, so
the paint commits before her next choice. While a SetDown is pending (pushed but not committed or
refused), `arrange/carry` refuses to bind. `Chances.drop_at` and the paint-time commit check are
both computed on a clone **with the move applied** (the live ledger still reserves the old slot, so
a same-strip target near it, a turn in place especially, would collide with the piece's own ghost).

A3 (D6 trials). The carry/episode holds explicit trial state; while trials remain `arrange/lift`
binds regardless of the cap, and the episode counts once (at its first commit). **Trials are
decided after step 6**: measure the kitty image budget with a carry first; ship trials only if the
budget holds (else record why not in plan.md).

A4. **`Scene::Arrange` cue.** The stage room owns nothing and `gift` holds one item, so the cue sets a
pending setup resolved in `furnish` (it needs the buffer): place a settled TV and a settled sofa on
one strip, in range, with the sofa turned away (Faces broken by facing alone, so a turn in place is
the repair), mark Faces felt, and have `stage::direct` pursue the lift from `chances.repairs`. Its
`every_scene_has_a_spot_in_the_stage_room` criterion (within 10 s, at 100×30 and 80×24, both modes):
she lifts (a pocket) or the sofa's facing changed. Needs `cue_note == Ok` at the first paint.

A5. **CHANGELOG entries go in the same commit** as each user-visible step (2, 4, 6, 7, 8).
design.md/decisions.md may land at the end but before the phase is called done.

A6. **The trip to lift and the carry are certain**, like a made-piece heading: a heading for
`Job::Lift`/`Job::SetDown` isn't rolled away from (no `let go: Other`); it ends by arriving, by the
dropped-carry rule, or after `TRIES`. (`an_uninterrupted_trip_runs_its_course` must keep passing.)

A7. **"No piece returns within 3 visits to a spot it left"** — the spot is the anchor
`(strip, side, offset)` (a SetDown's old anchor), not the projected left; parcels pushing
neighbours and `move_off` don't count as her moving them.

A8. **Ordering:** `broken()` and the search run on the home after `project` (with its `move_off`)
in `furnish`. Faces' `felt_on` is the sofa's uses only (Lounge, Nap) — one line per row.

A9. Minor: recompute the search also when `visit.layer.cells()` changed (if cheap); `Loss::Moved`
gets an arm in `beat_lines_fit_a_bubble`; grievances, "Hup!", "There!", "hmm…" go in
`every_fixed_line_fits_a_bubble`; check whether `golden_resident`'s sofa–TV gap is within 2..=14
before assuming it doesn't re-record.

## Amendments, round 2 (from the design critics; scratchpad/critique.md has their evidence). Override everything above.

### Geometry & persistence
G1. **Lanes.** `Home::on(strip, lane)` chooses the lane (a boxed piece is always Floor; an unboxed
wall piece is Wall). `move_off` and "strip too small" use the floor lane only; when a floor lane
moves, that strip's wall pieces go with it (re-anchored by share on the target) and are simply
closeted there if they don't fit. Add one `Shown` accessor for the vertical lift (`hang`, 0 while
boxed) and use it everywhere vertical geometry is computed: `rect`, `cover`, `cells()`, `fits`'
floor-row requirement (none for a hung piece), `Extent::holds`, `prop_layer`. Assert in a test
that every hang ≥ the tallest floor footprint (4), so a hung piece never overlaps a floor piece.
G2. **Wall-piece delivery checks both states**: the boxed parcel in the floor lane (pack, fits,
Unpack roomy, every showing piece still fits) and the hung piece in the wall lane (fits). Accept
only if both pass. (Lands with decor, step 8.)
G3. **`unsettled` is read as `Vec<serde_json::Value>`** and filtered (unknown items skipped), like
props/anchors; test a later build's unknown entry is ignored. `ledger::rooms()` skips decor kinds.
G4. **Rules apply only to laid-out pieces** (present in `layout`, i.e. on a strip that packs);
Belongs/Apart judged on layout strips, not ledger strips. Search criterion (c): every strip that
packed before still packs.
G5. **The SetDown commit re-runs the geometry filters** (rule satisfied, no satisfied rule broken,
packing strips still pack, no strip's role worse) on the clone, plus the fit checks. `furnish`
order: project → commit pending SetDown (then re-project) → doorstep (re-project) → gift
(re-project) → `broken` + search on the final home/projection (so the gift early-return must not
skip them; restructure). 
G6. **Anchors for candidates.** The candidate at the piece's current left keeps its existing anchor
(a turn changes facing only). For Near and Faces generate the anchor from the partner's wall side
(so the pair keeps together across resizes); otherwise `pin`. Dedupe on (resulting layout, anchor).
G7. **No strip's role gets worse** under a repair (apply the spoil test to every moved piece,
settled or not). A helper `role_without(strip, piece)` on the layout.
G8. Made pieces occupy their rects in the search, `drop_at` and the commit (use `visit.made` — the
commit runs before `tend_made`). Neighbours that were `roomy` before stay roomy.
G9. Made sofas keep the `tv.left > sofa.x` derivation for the Watch seat's facing (no strip, no
facing). Real sofas use their own facing.

### Act & decision flow
F1. **Lift also ends in a short timed act** (like SetDown/A2). `Chances.drop_at` is tagged with the
`(piece, to)` it was computed for; while the tag is missing or doesn't match the pocket, the pocket
continuation **waits** (a short Stand reflex, "waiting"), never drops. While a SetDown is pending,
the same waiting reflex (makes A2 certain even when `tick` runs many events without a paint).
`may_work &&= pocket.is_none()`.
F2. **`Broken { row, pieces /* movable */, involved /* every piece the rule needs */ }`.** The
grievance fires when the seat's piece (for Watch-from-sofa the sofa; so Apart is felt on Sleep
only — drop Watch from Apart's felt_on) is in `involved` and `seat.what ∈ felt_on`. Near is felt on
Sleep/Homework (bed/desk are involved). `felt: Vec<(usize /*row*/, Furniture /*the piece judged,
for Belongs; else the rule's first piece*/)>`. use-it binds the felt `(piece, use)` pair recorded
when felt (sit back down where she felt it), not necessarily the moved piece.
F3. **Grievance window aligned to `USE_FRAME_MS` ticks**; it wins over the advert's bubbles (an
advert Watch is never a grievance use, since Apart isn't felt on Watch any more — assert the two
don't co-occur). Felt is marked only when the window was shown (the frame tick past its end).
F4. **Lift is interrupted if its piece goes into the closet** (paint-time check like `lost_seat`, via
a `lifting()` accessor; `Cause::SeatGone`). `Act::Lift` gets an `at_job` arm (Stays::Job).
F5. **Ctx holds owned copies** (`pocket: Option<Carry>`, `just_set`, `set_down_pending`,
`may_arrange`, trials) — guards are `fn(&Ctx, ..)` and `self` is mutably borrowed around them. Build
`Ctx` before `leftover`; the pocket continuation goes before `leftover`.
F6. **Carry headings are dropped silently** (`Letting::Carry`): the pocket owns the loss
(`Loss::Moved`, once). A Lift heading re-validates its own repair (the guest re-checks that exact
move at paint, `Chances.lift_ok`) rather than relying on top-3 membership; the lift trip counts
`TRIES` too (in the episode state, which exists from the moment she sets off to lift).
F7. A pending SetDown commits only if `osaka.carrying() == Some(piece)` (`place()` may have dropped it).
F8. "There!" is said in `set_down_done` (the commit), not at the act's start. Refused → glance
"Oh well..." (or none).
F9. `Loss::Moved` glance target computed at the drop from the re-shown piece (mod.rs passes it to
`set_down_refused`/drop), not a stale `Carry.from`.
F10. While carrying, the reserved slot is a ghost cover for builds, `tend_made` and LayerOp bases
(not drawn, not terrain): she can't build in the slot she emptied.

### Trials (A3 refined)
T1. Each trial's `Repair` is stored in the episode; a re-lift is a continuation that skips the cap
and the broken gate; `Chances.lift_at: Vec<(Furniture, (i32,i32))>` from `shown`. Still: decide
after step 6's image-budget measurement.

### Tests & determinism
T2. `Want::Arrange` (variant, ALL, def, lint arms) lands **with** `Scene::Arrange` in step 6, not
step 4 (exhaustive cue lint). Step 4 lands Nesting, Rising, felt, the grievance, mood caps.
T3. **Drop the "no piece returns within 3 visits" property** (no mechanism; record why in plan.md:
satisfied rules never move and repairs break none, so a return needs a layout change, which the
exemption covers). Replace it with: within a visit with no resize and no delivery, no piece is
moved twice except by trials.
T4. **Image budget**: add a `#[cfg(test)]` counter of cache clears / images encoded and assert no
clear happened; a kitty test forcing Industrious over a deterministic broken home (anchored
`Prop::new`) asserting ≥ 1 SetDown committed; record counts in plan.md.
T5. **`stage::Want::Nesting`** (a needs press; example keys 1–6). One short new property presses
Nesting at t=0 over a deterministic broken home, both modes, text-dense, reusing the covering
checks (`assert_untouched_but_feet`, `Hidden::check`), and asserts home acts ≤ cap (lazy: none).
T6. **The A4 cue**: resolved inside `furnish` (setup → project → broken → felt → search), the
`offered` Chances carries `broken`/`repairs`, the cue bypasses the cap and the throttle, and moves
pieces she already owns instead of adding (`add` refuses duplicates). Check 80×24.
T7. **`give` (stage gifts) stays settled**; only `doorstep` deliveries are unsettled (so the census
and golden furnished homes stay lived-in; Faces still repairs gifts that landed apart).
T8. `needs_stay_in_range` passes `Rising` with every flag true. room.rs `pieces()` samples a
floor-only list; a separate wall-lane property on a taller pane (with decor). tests.rs's `0usize..4`
stays. Nothing iterates the `HashSet` in `furnish` for decisions or change detection (hash a sorted
Vec if needed).
T9. The wall-lane mechanics land together with decor (step 8), so the `Some(hang)` paths are tested.
T10. Golden re-records: check with `HOUSEGUEST_GOLDEN_TRACE` diffs that only the expected lines move
(e.g. step 3 should move nothing if only deliveries are unsettled... the stage golden's 255 s
parcel makes the ledger line move — confirm only the final ledger line changed).

## Revised commit plan
1. layout split (refactor, goldens unchanged) · 2. Faces with facing (+CHANGELOG) · 3. rules table,
`broken()`, unsettled deliveries + ledger field, stage `x` row · 4. feeling: grievance, felt,
Nesting/Rising, mood caps (+CHANGELOG) · 5. the repair search (+properties, budget) · 6. Arrange,
the pocket carry, Scene::Arrange, Nesting press, covering property, image-budget test (+CHANGELOG)
· 7. trials, if the budget holds (+CHANGELOG) · 8. wall lane + decor + beauty + channel, after the
user's art review (+CHANGELOG) · 9. docs (design.md, decisions.md, plan.md record), censuses, 256-case run.

## User, 2026-10-03: decor art approved
The plant and poster art (scratchpad/decor/snippets.md, worktree.diff) is approved as drawn.
For plan.md (a later stage, not phase 4): posters delivered in cardboard tubes; more posters,
paintings and plant varieties; maybe a greenhouse/solarium she builds (not now).
