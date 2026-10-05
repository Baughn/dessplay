# Phase 5c critique: feasibility (will the plan meet the band, and is the step order right?)

Critic `feasibility`, 2026-10-05, against phase5c-design.md and map.md at HEAD `cabf5d1`. Paths are
relative to `dessplay/src/ui/houseguest/`. Tags: **(run)** comes from the mapping session's census
logs (map §4.2; scratchpad `48e093b0…/scratchpad/visit_census_moods.log`), unfed, hidden time
included. **(estimate)** is my own arithmetic, with its assumption stated. Nothing here was re-run.

## Verdict

The band is reachable in every room, but the plan doesn't treat the rooms alike, and they aren't alike.
**The stage is need-limited:** Tidy and Mischief set how many walks to text she makes a minute, so
longer still acts barely move it. **The home and resident are time-limited:** she's busy with uses and
her needs sit high, so her moving share is about walk ÷ (walk + use), and longer acts dominate.

With the design's full starting package, my estimate is that the lazy stage lands at its ceiling and
the home and resident fall to (or under) their floors. The step-7 tuning order (needs last) is written for
one regime only. The floors are too low to catch the overshoot. And the band test, as specified, lets
the arrival burst into a short window, so tuning would converge on an Osaka stiller than the user asked
for. One blocker, five majors, the rest minor.

## Rough budget (estimate): where each room lands, and what drives it

Model: share ≈ Σ(choice rate × moving s) ÷ Σ(choice rate × act s), at the design's starting values
(Restless 300 s, Tidy 240 s, Walk 9, lengths ×≈2.5, linger, settle-in), fed afternoon, census chat.

| Room | Mood | Today, moving % (run, hidden incl.) | After step 3 (estimate) | What drives it |
|---|---|---|---|---|
| stage | lazy / ord / ind | 34.1 / 38.1 / 39.5 | ~15–19 / ~17–20 / ~23–27 | walks to text: pulls ≈ Tidy rise ÷ (0.6 × completion); swaps ≈ 0.31/min, the same for every mood |
| home | lazy / ord / ind | 40.4 / 42.4 / 47.3 | ~8–10 / ~15–17 / ~24–26 | walk to each use ÷ (walk + use); uses ×2.5, ×linger, Lounge→Nap |
| resident | lazy / ord / ind | 37.3 / 37.9 / 40.4 | ~6–9 / ~11–14 / ~20–24 | the same, with a thinner still roster; Restless pegged near 1 today |

Evidence for the regimes (run):
- **Stage:** Tidy reads 1.0 at every 5-minute mark but one, and Pull + Swap are 42% of choices (ordinary). It makes 4.8
  choices a minute.
- **Home and resident:** Restless, Fun, Comfort and Daydreams sit at 0.6–1.0 even when lazy, at 1.7 and
  2.8 choices a minute, and the Walk act is 33% of home time.

So the levers that carry the band are disjoint by room:
- **Stage:** Tidy and Mischief rise times, and pull/swap locality.
- **Home and resident:** use lengths and Restless.

That's good news (two independent knobs), but the plan should use them that way.

## Blocker

**B1. The band test's window takes in the arrival burst, and "shorten minutes before adding seeds" makes
it worse.**
- *What's wrong.* A fed idle arrival is `Osaka::arrive` (mod.rs:1830). Half the time that's a walk-in
  from off the screen edge to a uniform column (osaka.rs:1795-1808); otherwise a fall from row 0
  (osaka.rs:1817-1826).
- Then come the arrival levels: Restless 0.7, Tidy, Fun and Daydreams 0.5 (brain.rs:87-95; the
  Afternoon uses `arriving`, brain.rs:100-108). With Walk at 9, the first rolls score Walk 4.4,
  Travel 4.9, Jacks 3.5 against Stand 2.0, so she opens with a run of two or three walks.
- I estimate 40–60 s of movement up front. That's 7–10 points of a 10-minute window, which is most
  of lazy's 17. Step 7 would then tune the steady state down to about 8% to pass. That steady state is
  what the user, running a resident for hours, actually sees.
- *Amendment.*
  - The band tally (and the set-off count) starts after a warm-up, e.g. the first 3 sim-minutes, or
    from steady state. Print the warm-up apart.
  - Spend the gate budget on seeds × (warm-up + M), not on a shorter M.
  - Drop the "arrival Restless 0.7 → 0.5" fallback (D4 tuning). It would change the deliberate arrival
    to fix a measuring artefact.

## Major

**M1. The step-7 order is backwards for the stage, and the home will overshoot.**
- *What's wrong.* D4 says "settle-in odds, then lengths, then Walk base, then needs". On the stage:
  - Pull at Tidy 1.0 scores 16 × (0.1 + 1.2) = 20.8 (brain.rs:693, score at brain.rs:793-817),
    about 10× any still want.
  - Each pull is served only by offset/goal (osaka.rs:5000-5002 per the map), and Tidy rises only
    while there's text (osaka.rs:5517).
  - Until Tidy unpegs, nothing else moves the stage much. After Tidy 240 s, the pulls a minute needed
    are fixed by Tidy's rate (lazy ×0.7, industrious ×1.6), and longer still acts don't change that.
- In the home, though, the lengths compound with linger (×1.5) and settle-in (0.6 × Sit/Nap chains),
  so lazy and ordinary land at about 8–10 and 15–17 (estimate). That passes the design's floors but
  sits well under the user's "about 15" and "20–25".
- Needs also rise between decisions (osaka.rs:5522-5526). In the time-limited rooms, longer uses bank
  Restless, and it pegs. That's why Walk and Travel survive in the top four even at 300 s.
- *Amendment.* Tune in this order, each against its own room:
  1. Rise times (Restless, Tidy, and Mischief, M4) plus the Walk/Travel bases (M3), read on all three
     rooms.
  2. Pull/swap locality, on the stage.
  3. Base lengths at partial strength, on the home and resident.
  4. Linger and settle-in last, as the **spread** knob between moods.
- Expect to **back off** lengths in the home and resident and to push harder on the stage. Write the
  stop rule as "inside the band in every room, nearest the user's target", not "under the ceiling".

**M2. The floors (7 / 12 / 15) are far below the user's targets, and nothing pins the spread.**
- *What's wrong.* The user asked for about 15 lazy, 20–25 ordinary and dreamy, and up to 30
  industrious. The design's pass ranges are 7–17, 12–26 and 15–31.
- An Osaka at half the user's ordinary target passes, and per M1 that's the likely failure. Pinning
  ceilings tightly and floors loosely biases every tuning move toward too still.
- *Amendment.*
  - Raise the floors to about target − 4: lazy 11, ordinary and dreamy 16, industrious 20 (re-check
    against step 1's spread, as D2 already says).
  - Add a per-room spread assertion: industrious mean ≥ 1.6 × lazy mean. The user's band implies about
    2×; today's measured spread is 3–7 points (map C3).
  - Tune to the band's middle, not its edge, so 5-seed noise ("a few percent", census.rs:1445-1449)
    can't flake it.

**M3. Walk drops 14 → 9, Travel stays at 10: wandering moves to Travel, which costs more.**
- *What's wrong.* Walk and Travel cool apart (brain.rs:815: `w == want`). Travel is a walk to a link
  plus a climb or a door.
  - On the stage that's the doors: 10.1% of time, "Handy, these doors." 21 times in 16 visits (run).
  - In the resident it's the climbs: 4.8–5.9% (run).
- At quiet needs, Walk's floor score drops to 0.9 but Travel's stays 1.0 (brain.rs:691-692). So the
  idle top four keeps Travel, and every Travel brings more set-offs and hidden door time.
- *Amendment.* Lower Travel with Walk, to about 7.
  - That keeps `a_restless_osaka_mostly_moves` (brain.rs:1034-1042) passing. Its other needs are 0
    (`Needs::with`, brain.rs:415-424), its offers floor-only (`all()`, brain.rs:912-923), no cooldown.
    At Restless 1.0, Walk 8.1 + Travel 6.3 against Jacks and ToeTouch 6.6 each is 52%.
  - At Travel 6 it's 50.6%, a coin-flip test. Re-state it with the reason if you go lower.

**M4. Mischief and pull/swap locality: the stage's set-off floor is barely touched.**
- *What's wrong.*
  - Swap serves Mischief 0.8 (brain.rs:694), and Mischief rises in 4 minutes with no mood rate
    (brain.rs:72). Nothing else serves it, so about 0.31 swaps a minute, every mood, each a walk to
    text, often off-floor.
  - The design's ×4 for her own floor (D4, mind.rs:318-340 uniform today) buys less than it sounds.
    With a fifth of the offers on her floor, ×4 moves her from 20% to 50% same-floor jobs, not
    "mostly".
  - It also contradicts the design's own test, "pulls stay on her floor when there's text there".
  - The lazy stage cap (≈ 0.56 × today's rate: 15/31 × 1.15) is the hardest cell. Jobs alone (≈ 0.44
    pulls + 0.31 swaps a minute, about 1.5 legs each) use roughly half of it (estimate).
- *Amendment.*
  - Same floor **first** whenever her floor has an offer (as `leftover` sorts, osaka.rs:5818-5832),
    then nearer.
  - Add Mischief to the step-3c tuning list (e.g. 4 → 8 min).
  - Split set-offs by purpose in step 1 (D1 splits only moving ms), so the cap's binding term is
    visible before step 2 pins it.

**M5. D0b empties the bare-room still roster, and Stand keeps re-rolling.**
- *What's wrong.* Once SpaceOut credits Daydreams (D0b), Daydreams will sit low in bare rooms (SpaceOut
  was 4.7% of stage choices, serving 0.6 each, against a 10-minute rise). Then:
  - SpaceOut, Gaze and LieFront fall toward their floor, 6 × 0.1 = 0.6.
  - Floor Sit and LieBack are already dead at about 0.46 (map §brain 4).
  - The idle top four becomes Stand 2.0 (NEUTRAL × 4, brain.rs:670), Travel 1.0, Walk 0.9, and one
    still: about half movement by construction.
- Stand lasts 2–5 s (osaka.rs:5739-5741). It isn't in linger's list or settle-in's chains, so it's a
  fast re-roll. Settle-in only fires after a still act she rarely chooses.
- The step-1 baseline is rightly taken after D0b. The plan just has no lever for this regime.
- *Amendment.* Pick one; each is lint-safe (no factor below 1, no mood term):
  - SpaceOut `own_sake` (its fit ≥ NEUTRAL, so 3.0 at rest): idling defaults to stillness.
  - Or a ×k (k > 1) in the `factor` closure (osaka.rs:5669-5682) for offers that bind where she
    stands: `Here` binds, or a seat within a few cells. The same pattern as D5's ×3 for `Job::Build`.
- Also Stand ×linger (or 4–10 s), or Stand as a settle-in source into SpaceOut. The in-place factor
  also helps the home, where no distance term reaches the want choice at all (map §brain 6).

## Minor

- **m1. Re-baseline after step 5.** The watch dropping 15 → 5 s is settled (Q1). Taken alone, though,
  it frees about 10 s of standing per line: 36% standing on the stage, 24% in the resident and 16% in
  the home (map G2) are mostly the watch. Those seconds go to fresh rolls unless she resumes, so the
  chat-condition share **rises** (stage roughly +8–10 points, estimate) before longer acts pay it back.
  Re-run the fed census after step 5 and record it in baseline.md, so step 7 doesn't read that rise as
  a lever failing. (The cap formula's rate ÷ share is invariant to the watch's dilution.)
- **m2. A cut walk to a job loses the job.** A same-floor job clears `heading` and walks
  (osaka.rs:5888-5896). A chat line replaces the act (`interrupt`, osaka.rs:6508-6522), so after the
  watch she rolls afresh, and any restart is another set-off. That's about 0.27 cuts a minute on the
  stage at 45 s (estimate). Resume a cut `Walk { then: Job }` as a continuation like a heading (Q1's
  spirit), or at least count "restarts after a watch" in step 1.
- **m3. Land step 6 golden-neutral.** At neutral parameters (linger 1.0 for every mood, settle-in
  p = 0, a session with n = 1, distance weights all 1, no 3-cell exclusion), no golden moves:
  - `pick_weighted` with all weights 1 is the plain `below(n)` (osaka.rs:291-293).
  - A new whims label consumes no stream (mind.rs:37-49).
  - So step 6 re-records nothing, and the goldens prove the plumbing inert. Step 7 then flips the
    values with one re-record.
  - This needs every new draw (session n, musing spacing) to come from **whims**, not `rng`: a new body
    draw shifts every later one. The design names whims labels only for "settle in" and "resume".
- **m4. Dreamy is banded with ordinary but tuned toward lazy.** Linger 1.25, settle-in 0.45 and Restless
  ×0.7 (brain.rs:189-205) put dreamy 3–5 points under ordinary (estimate). Per M1, that's at the 12
  floor in the home and resident. The user didn't specify dreamy. Give it ordinary's linger and odds,
  and let it differ in *what* it does (sessions, cloud-watching).
- **m5. Which home?** `furnished_room` owns no window (census.rs:67-86); DAY_HOME does (census.rs:902-912,
  look-outs about 19 a game day). If the band's home is windowed (what a long-running user has), fold
  D6's numbers (base 3, 60–180 s) into step 7. They need no art; the sill pose can follow. Otherwise
  D6 re-tunes the home after the band is un-ignored.
- **m6. The cap's 1.15 slack** should come from step 1's per-seed spread, as the share tolerance does.
  At 5 seeds × 10 min, set-off rates swing more than ±15% (estimate).
- **m7. Nearer `walk` and `place` are noise.**
  - The weight 1/(1 + |dx|/8) is heavy-tailed: a mean wander of about 20 cells instead of 25 from mid
    floor, about 34 instead of 50 from an edge, and it lowers no set-off count. Once Restless slows,
    wandering is minor anyway.
  - `place` has one or two seats per use in the home and resident. It only helps her stay on the sofa
    across Watch, Lounge and Nap.
  - Keep both if cheap, but don't spend a golden cycle or a tuning try on them.
- **m8. The pull's heave** is bucketed "mischief" (`census_group`, osaka.rs:5249-5290), so the band
  counts it as still. The design is silent. Decide it in D1 (it's small: Pull time is 1.1%, run).
- **m9. Afternoon as the measure.** It's about 48% of a school day's awake game time (routine.rs:30-47:
  Morning 75 min, Afternoon 315, Evening 120, Homework 150) and has no work (`work_open` is days off
  only, routine.rs:463-465) or errands (the accordion only, osaka.rs:6766). That makes it a fair,
  clean primary. Evening and Homework are probably stiller and weekend mornings busier. Make the day
  census's per-mood in-sight share a hand-checked acceptance at steps 7 and 11. The real chat cadence
  is unknown; quiet plus the census cadences bracket it, which is right.

## On the step order (asked)

- **Steps 6 and 7 split:** wise only with m3 (golden-neutral mechanisms). Otherwise it's an extra
  re-record of every hash for values that step 7 changes anyway.
- **Art and step 4 before tuning:** no. D5 adds still options to bare rooms (floor homework in place, a
  pulled-line read) and few set-offs (a tear walk per build), so tuning first is conservative for the
  stage. Make step 9 keep the band green, or re-tune inside step 9. The exception is D6's numbers (m5).
- **Steps 1–2 before 5:** right. Then add m1's re-baseline.

## Right, and should not change

- The band's quantity: moving in sight ÷ awake time in sight, with hidden door beats, Away and her shift
  out.
- A set-off cap beside the share, derived as rate × target ÷ share, so shorter walks alone can't pass.
- The fed afternoon with the mood forced and **asserted** (`expect`, not `if let`), in line art, quiet
  and at the census cadence.
- D0a and D0b each alone before step 1 measures; the credit-class test over `Want::ALL`.
- The band test committed failing and `#[ignore]`d, then un-ignored at step 7; the `CENSUS_BAND_SEEDS`
  knob.
- Resume and settle-in as continuations that skip the roll and stay out of `recent`. A resumed act's
  credit scaled by left ÷ whole (consistent with `set` crediting the cut share, osaka.rs:2742-2747).
- Settle-in after `leftover` and the routine reflexes; bedtime and school win.
- The ×3 for `Job::Build` in the `factor` closure, not `place()` (map C4).
- The mood difference through lengths and settling only, with no mood factor on walking.
- One golden re-record at the end of step 7, not one per tuning try.
