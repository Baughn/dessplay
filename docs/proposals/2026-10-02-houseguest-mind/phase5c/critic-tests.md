# Phase 5c critique: tests, determinism and gate cost

Critic KEY `tests`, 2026-10-05, against [phase5c-design.md](../phase5c-design.md) at HEAD `cabf5d1`.
Paths are relative to `dessplay/src/ui/houseguest/` unless they start with `docs/` or `.config/`.
Code claims were read at the cited lines; nothing was run. *(est.)* costs derive from the map's (run) timings.

## Verdict

The order is right: D0 as separate commits before any measuring, a fed band in line art, hidden
time left out, both chat conditions, and a cap on set-offs beside the share. But three of the
tests the design names can't be built as written:
- the fed-afternoon setup (wrong arrival step, a day that underflows, parcels it doesn't know about);
- the made-piece bound, which turns the property vacuous;
- D0b's class test, which can't see the credit paths it exempts.

The band's thresholds also need a variance budget before they're pinned. Otherwise every later
change that moves the goldens risks failing the band test for no real reason. Fix the three
blockers in the design before step 1.

## Blockers

### B1. The fed-afternoon setup is wrong in four places

D1 and D2 rest on "each room fed at a fixed weekday 13:00 … mood forced after the first paint and
asserted … No shopping".
- **(a) She isn't there at the first paint.** `Guest::restore` starts `State::Absent`
  (mod.rs:843-851). `home_at`'s doc says she "arrives once the idle gate opens"
  (tests/golden.rs:351-353). So `set_mood` turned into an `expect` after the
  first paint panics. `furnished()` shows the working route: `guest.cue(Scene::Arrive)`, then
  paint (tests/golden.rs:271-272).
- **(b) A Monday underflows.** `GameTime::minutes` is `day*DAY_MIN + hm - START`, with `START` =
  16:00 Monday (routine.rs:24, 548-550). Monday 13:00 panics in debug.
- **(c) Parcels change the room.**
  - The wall-clock gift fires whenever `fed && !ledger.clock_sent && awake` and a TV stands
    unboxed (mod.rs:3063-3093). So it arrives early in the home and resident afternoons: a
    parcel, a walk to it, an Unpack.
  - `day_guest` sets `clock_sent` only when a clock is owned (tests/census.rs:965).
  - The stage owns nothing. `Ledger::new_at` sets `visits: 1` (ledger.rs:111-117), arrival bumps
    it to 2 (mod.rs:2336), and at `FIRST_TV_VISIT` = 2 a TV is ordered and delivered on the spot
    (mod.rs:120, 3038-3041, 3049-3060). So a fed "bare stage" can't exist without a hook.
- **(d) Shopping is switched on by the census loop.** `visit_from` calls `guest.shop()` every
  time (tests/census.rs:356). That makes `advert` due (mod.rs:130-131). The fed census can't
  reuse `visit_from` as it stands.
- **What holds:** `catch_up_day` only fires once `next_morning(from) <= game`
  (osaka.rs:4155-4166), and 13:00 + 90 game minutes stays in Afternoon (12:45–18:00,
  routine.rs:38-40, 414-417). So the forced mood holds if nothing re-arrives.

**Amend.** Add one named helper, `fed_afternoon(room, seed, graphics, mood)`:
- Tuesday (day ≥ 1) 13:00, date `None`.
- `clock_sent = true`, no `shop()`, and `cue(Scene::Arrive)`.
- Paint, then `expect` Visiting, then `set_mood`.
- Every owned piece shown unboxed in *both* modes, as `the_census_home_shows_every_piece` checks
  unfed (tests/census.rs:1402-1420). `day_guest` gives the pieces in ASCII only (948).
- Pick one answer for the stage, and write it down: it owns a TV, its band is read unfed, or a
  `#[cfg(test)]` knob suppresses the first TV.
- In every run, **assert**: `home.props` and `ordered` are unchanged at the end, the state stayed
  Visiting with no re-arrival, and `mood() == forced` at every step. "Rooms stay as set" and
  "mood holds" then become checks instead of a one-time look in step 1.

### B2. The derived made-piece `BOUND` makes the property vacuous

`every_made_piece_is_used_or_let_go` runs 180 s (tests.rs:7480), with chat every 20–40 s (7455)
and `BOUND = 90_000` (7460). The bound is checked as `now - since <= BOUND`.

The design's new formula, at Lazy, is: longest still act + chat gap + watch + walk.
- That is Nap 150 s × 1.5 = 225 s, or day Sleep 270 s, plus about 49 s plus a walk. That is
  already over 180 s.
- A settle-in Lounge→Nap chain on the same made sofa (135 + 225 s) makes it worse.
- So does about 9 s of Look and watch per chat cut.

The assertion could never fire. The gate's long pole would keep running and check nothing.

The root cause is D3's placement. Resume goes "after the routine reflexes and the watch and before
every other continuation", which puts it ahead of `leftover` (osaka.rs:5620-5633). With a made
bed cued mid-lounge (`bed_at`, tests.rs:7456), she would resume the cut lounge rather than go to
the bed.

**Amend.**
- Put resume **after `leftover`**, next to settle-in and just before the offers (osaka.rs:5634).
  A waiting made piece then joins the routine as a reason not to resume, in the spirit D3 already
  has. In the test, every act is cut within 40 s, so `BOUND` keeps today's meaning: a chat gap,
  the watch and a walk.
- Add `assert!(RUN_MS > BOUND + MAKE_MS)` in the test, so the bound can't silently outgrow the run.
- If the user wants resume ahead of a made piece instead, state the cost. The run grows 3–4×:
  about 40–50 s at 32 cases, over the 30 s SLOW flag; and about 150 s at 256 in release, past
  the 120 s kill (.config/nextest.toml:19-22). Each would need a new override with its reason.

### B3. D0b's class test can't see most credit paths

"Assert `credited` holds it" doesn't work:
- `credited` is pushed only inside `credit_done` (osaka.rs:5006-5007).
- `credit_whole` (5023-5028), the set-off credit for Walk/Travel (5705-5707) and Arrange's
  credit (6064) all call `serve` directly.
- Walk and Travel have no `Scene` (stage.rs:28-140).

So the test needs an exemption list (Walk, Travel, Pull, Swap, Arrange). That list is the hole
the test means to close: a future want "credited some other way" just joins it.

**Amend.**
- Record in `serve` (`#[cfg(test)] served.push(..)`), the one place every credit goes through.
- Drive by **real choices** in quiet unfed sims: every decision whose want has non-empty `serves`
  and that ended uncut must show up in `served` with share > 0.
- Add a wildcard-free `match want` that names how each want is covered (sim, or a cue where no
  sim reaches it), so a new want fails to compile.
- It still fails on HEAD for SpaceOut and Work. Add the "a shift cut short credits its share"
  case explicitly (`SHIFT_MS` 60–180 s, osaka.rs:1043; cost is fine).

## Major

### M1. The metric doesn't see Pull, which moves her

While heaving, `Act::Pull` steps her (`self.x = next`, osaka.rs:3282-3290) and drags a line of
text. `census_group` files it as "mischief" (osaka.rs:5269-5276), and D1's Body axis leaves it
out. On the stage, Pull is 28.7% of choices (map C2). A lever that turns walks into pulls would
pass the band.

`census_motion` is wildcard-free, so the builder will have to class it anyway. **Amend:** decide
it in the design (suggested: heaving steps count as moving, a "text" body) and print it in its
own column. Swap, Sneeze and PutBack move text without moving her, so print them apart. Raise
this with the user next to Q3.

### M2. The band is deterministic but brittle; budget the variance before pinning

Fixed seeds reproduce exactly. But every later commit that moves the goldens re-rolls the draws
(`pick_weighted`, map G8). `her_mood_shows` already notes moving swings "a few percent" between
seed sets at 4 × 15 min (tests/census.rs:1446-1449), and that is with today's 6–40 s acts.

After 3c, a Lazy still act runs up to about 225 s. A 4-seed × 10-min Lazy cell is then a handful
of decisions, and the Lazy band (7–17) is only ±5 points wide. The design also asks for 48
assertions (12 cells × 2 chat conditions × share and cap). Each one needs a very small chance of
failing on a re-roll, or the band test fails on some unrelated commit most months.

**Amend:**
- Step 1 measures σ of the **N-seed mean** over about 20 disjoint seed sets, in the band's exact
  setup. `CENSUS_BAND_SEEDS` can drive this.
- 3c pins each threshold at least 3σ from the tuned mean.
- Throw away a warm-up (about 3 min). The arrival run at Restless 0.7 is a much bigger share of a
  short window than of a real session (about 2 h in sight per game day, map G10).
- Keep M ≥ warm-up + 2 × the longest still act.
- "Shorten minutes before adding seeds" stops being right once acts are long. Say so.
- If the budget can't buy 3σ, pool across rooms per mood for the floor and keep per-room
  ceilings.
- Assert the **pooled ratio** (Σ moving / Σ in-sight), not the mean of per-seed ratios.

**Cost (est.):** `the_census_counts_her_vignettes` spends 2.40 s on 45 ASCII + 45 line-art home
minutes (map census §7). At about 17 ms/min for ASCII, that leaves about 36 ms per line-art
minute, and the stage at about twice that. At ≤ 3 s per test across two chat conditions, the home
and resident fit about 4 seeds × 10 min each, and the stage about 4 × 5. That is too little for
3σ at Lazy, so the measurement, not the hope, sets N and M.

### M3. Set-offs must be counted where they happen, not sampled

`visit_from` steps up to 1000 ms (tests/census.rs:366-369). A one-cell walk, a hop's next leg, or
a walk re-set after a Look can begin and change inside one step.

**Amend:**
- Add a `#[cfg(test)] set_offs` counter on `Osaka`, bumped in `set` or `go_to`, the way
  `set_downs` is kept (osaka.rs:6056).
- Add a unit test pinning which transitions count. A walk re-set after a Look counts. A climb
  continuing a hop doesn't.
- Fix the cap formula's "target share": the user's number (15 / 22.5 / 30) or the ceiling
  (17 / 26 / 31). The formula itself is sound: a lever that only shortens walks raises set-offs
  per minute and fails it. Make the cap face the same σ rule as M2.

### M4. Tests the levers break that the design doesn't list

Each needs an honest restatement in the step that moves it:
- `an_uninterrupted_trip_runs_its_course`, `set_off >= 8` (tests/census.rs:1394). Nearer seats
  (+40 per floor) and longer acts cut the cross-floor trips in a quiet home. It's a coverage
  guard: keep 8, and get trips back with more seeds or a room that has them.
- `every_pooled_line_shown_was_drawn_from_its_pool`. Its cued muses run only to
  `SPACE_OUT_MS.0` (tests/census.rs:1607), then assert no riddle is left pending (1582). Daydream
  sessions say several musings over 20–60 s+. Run each cued muse to its act's end.
  `shortest_body` (script.rs:520) and script.rs:2293 also read `SPACE_OUT_MS.0`.
- `the_census_counts_her_vignettes` (tests/census.rs:1637-1700). Shopping must happen once in
  15 min, and some andagi and chopsticks must appear over 3 seeds. Longer uses and settling in
  thin these out. Lengthen the span; don't drop the claims.
- `looking_out_lasts_and_counts_as_spacing_out` pins `(15_000, 30_000)` (osaka.rs:12412-12416).
  D6 names only the row test.
- `chat_mid_carry_and_she_carries_on` (`after.first() == "watching chat"`, tests.rs:9553-9555)
  and `a_lively_chat_never_keeps_her_over_text` (`watched`, tests.rs:10974-10978). Both survive
  only while `WATCH_MS` > `LOOK_MS` (4000, osaka.rs:369). At 5 s that margin is 1 s. Add
  `const _: () = assert!(WATCH_MS > LOOK_MS);`, or restate both on the Look.
- `her_mood_shows`: the *assertion* "dreamy spaces out more than ordinary"
  (tests/census.rs:1469-1472) is at risk, not just its doc. D0b's SpaceOut credit lowers
  Daydreams, and a Dreamy settle into cloud-watching LieBack moves time to "floor rest"
  (osaka.rs:5255). Re-check it at 3c, and decide which group cloud-watching belongs to.
- `at_home_her_furniture_beats_the_floor`: 10× → "e.g. 3×" (tests/census.rs:1442) is a
  loosening with no derivation. Either restate it on choices (rest she *chose*: furniture > 10×
  floor) with settled floor rest printed apart, or pin it at half the 3c measurement with the
  reason.
- The surf test's host pose `Pose::Sit` away from a sofa (osaka.rs:8712-8714) changes with D5's
  cross-legged pose. List it.

### M5. Re-recording falls short of testing-strategy.md; it can be cheaper and stricter

docs/testing-strategy.md ("Golden Trajectories") requires every re-record to say where the
traces first differ and why, with the rest byte-identical. No `UNFED_*` move may land without a
trace diff. The design asks for verification only for "narrow fixes".

**Amend:**
- **Every** behaviour commit runs a small script that prints each trace's first differing line
  and the act there. A first difference outside the acts the lever touches gets investigated.
  The `UNFED_*` table comments list the move.
- Capture the before-traces in the main checkout **at the step's start, before the first edit**,
  with `HOUSEGUEST_GOLDEN_TRACE` (the directory must exist: tests/golden.rs:91-98, strategy doc).
  That needs no worktree and no second cold build. Keep the `git worktree` (never jj there) as
  the fallback for a step already edited.
- Make the `WATCH_MS` change its own verified commit, ahead of resuming. It should first differ
  5 s after a chat line.
- For D0a, require the diff to be **insertions only**: extra frames at `t0 + DURATION_MS`
  (dissolve.rs:17, 194-196). That's stronger than "first differs by one added frame". `drive`
  paints only when `advance` says so (tests/golden.rs:144-147).

## Minor

- **D0a's property.** `next_tick` lands exactly on a fade's end (dissolve.rs:199-201), and on
  HEAD the Visiting arm returns false there whenever `tick` does (mod.rs:1499-1500). Add one
  deterministic unit case beside the property (a seated Osaka, a focus rain-out, a tick at
  `t0 + DURATION_MS`), so failing on HEAD doesn't depend on 32 cases finding it. Note also that
  Away (1482-1486) and Leaving (1540-1546) can't fail it today, so the property's strength is
  in Visiting.
- **The `#[ignore]` path is sound.** Ignored tests still compile, and the reason names the step.
  Add: each commit in steps 5–7 runs it with `--run-ignored only` and puts the table in its
  message. Step 7's un-ignore must not touch thresholds, except under M2's σ rule.
- **`CENSUS_BAND_SEEDS`** needs a helper in the spirit of `proptest_cases` and a line in
  testing-strategy.md. The 256-case deep pass (`PROPTEST_CASES`) won't scale it otherwise.
- **"Every test loops graphics [false, true]"** contradicts the line-art-only band. Make the
  exemption explicit (cost), and have the fed census print the ASCII share.
- **Settle-in and resume** should push `decisions` with their own method labels but not
  `choices`. Then the made-piece check (tests.rs:7498-7511, which watches `choices`) and the
  census choice tables keep their meaning, and tests can still see both continuations.
- **`a_restless_osaka_mostly_moves`** at Walk 9 (brain.rs:1035-1043): `score` (brain.rs:793-813)
  gives Travel 9.0 + Walk 8.1 against Jacks and ToeTouch 6.6 each, about 56% in the top four.
  It passes, but thinly. Any 3c move of Walk below about 6 breaks it. Say so in 3c's order of
  moves.
- **`sleepiness_draws_her_to_lie_down`** (brain.rs:986-1019) reads bases only, with Restless 0.1
  (Walk scores about 1). It is less sensitive than the design says, and lengths don't touch it.

## Right; keep

- D0a and D0b each land alone, with traces, before step 1 measures anything. The `tend_fades`
  helper makes the class unrepresentable.
- The band is fed, in line art, with hidden time (Away, the door's hidden beats, the shift) out
  of both numerator and denominator.
- It is judged both quiet and at the census cadence, with a set-off cap beside the share.
- `census_motion` is wildcard-free, beside `census_group`, and the chain tag is test-only.
- `arrive_drawn`'s `if let` becomes an `expect` (tests/census.rs:313-315).
- Per-seed min/max is printed, and the thresholds are read off step 1.
- The goldens are re-recorded once at the end of 3c, not per tuning try; the name `settle_in`, fresh whims labels.
