# Phase 5c design: critique synthesis

2026-10-05, against [phase5c-design.md](../phase5c-design.md) at HEAD `cabf5d1`. Sources:
[mechanics](critic-mechanics.md) (mech), [character](critic-character.md) (char),
[tests](critic-tests.md) (tests), [feasibility](critic-feasibility.md) (feas). Paths relative to
`dessplay/src/ui/houseguest/`; file:lines mark where I settled a disagreement in the code.

**Verdict.** The skeleton holds, and all four critics keep it: D0 as class fixes first, the
in-sight share with a set-off cap, the fed afternoon with its mood asserted, mood through lengths
and settling only, ×3 for `Job::Build` in `factor`, the ignored-then-un-ignored band test, one
re-record per behaviour step. Broken as written: the chat look (it stands her up), resume, D0b's
test and the band's setup; the tuning would leave the home and resident too still. Apply B1–B6
before step 1; B1 waits on Q1.

## 1. Blockers

**B1. D3: she looks in place (pending Q1, recommended).** (char B1; moots mech B2/M3/M4, tests B2)
- Old: a chat line cuts a still act (`interrupt` → `Act::Look`, a standing side view,
  osaka.rs:6477, 6508-6524, 7297-7304), then `Resume` re-enters it.
- New: for a still act on restful terrain, `look` doesn't call `interrupt`. It sets a look
  overlay on the running act. The act's clock and credit run on, and its pose stays.
  - Where the pose has a facing, she turns to the chat. Then `Surprised` + `!` for
    `SURPRISED_MS`, `Curious` + `?` until `LOOK_MS`, and a plain watch face until `watch_until`.
  - The overlay hides the act's own bubble, as the grievance overlay does (osaka.rs:7205-7210). A
    riddle's pending answer is said after the look.
  - Dozes (LieBack, Nap, day Sleep) get the night's `stir` instead (osaka.rs:4169-4173,
    7130-7145): `Blink`, "Mm?".
- Still cut, as today: Walk, Pull, Swap, chores (Unpack, Crumple, Snack, Pet), making, exercise,
  and anything on text (`ChatPassing`).
- `Resume`, its guards and the credit scaling go; D3 keeps `WATCH_MS` 15 → 5 s. design.md:1393-1395
  becomes "in a still act she looks up where she is" (reason in decisions.md). If Q1 says stand
  up, apply B2 instead.

**B2. D3 resume fallback (only if Q1 says stand up).** (mech B2, M3, M4; tests B2; char m1, m2)
- Old: "plain body, fresh since, credit × left/whole, after the watch, before every other
  continuation".
- New: the scaling arithmetic was right (`set` pays the cut share, osaka.rs:2742-2747,
  4945-5005). The defect is that a fresh `since` replays the script from key 0 (Homework writes
  again, LookOut says "Sunny!" at night, Sleep relights the lamp). Instead:
  - keep `play.own` and `branch`; drop `before`/`after`, `grievance` and `answering`;
  - back-date `since' = at − (cut − body_start)` and `until' = at + left`, and carry `credited`
    in `Resume`, so `credit_done` pays `max(0, share − credited)`. Idle and SpaceOut have no
    `whole`, and back-dating needs none;
  - build the act directly, never through `start_job`'s Use arm, and restore `facing`;
  - the slot is **after `leftover`** (osaka.rs:5627-5635), beside settle-in;
  - `set` clears `resume` unless the new act is Look, Stand or Glance. Never capture during
    `episode`, `just_set`, a dash, an errand, `shift`, `leaving` or `returning`;
  - exclude `use/borrow` from resume, and extend the look to `watch_until` when under 2 s
    would be left.

**B3. D0b's class test.** (mech B1, tests B3)
- Old: "cue each want's Scene, run it to its end, assert `credited` holds it". Stage cues never
  set `credit` (stage.rs:380-648). `credited` is pushed only in `credit_done` (osaka.rs:5007),
  so Walk, Travel, Pull, Swap and Arrange would need exemptions, which is the hole the test
  means to close.
- New: a `#[cfg(test)] served` record in `serve()` (osaka.rs:5032), where every credit goes
  through. Two parts:
  1. Per **(want, method)**, drive `choose_next` with a test-only offer filter, in a room where
     the method binds, and assert `served` gets the want with share > 0. Keying on the method
     catches D5's `use/borrow`.
  2. A wildcard-free `credit_path(want, method)` match, so a new variant won't compile without
     a path.
- Add Arrange to the design's list (`set_down_done`, osaka.rs:6062-6065). It still fails on HEAD
  for SpaceOut and Work.

**B4. The fed-afternoon setup.** (tests B1)
- Old: "fixed weekday 13:00 … mood forced after the first paint … no shopping". Four things are
  wrong:
  - she is `Absent` at the first paint, so the `expect` panics;
  - Monday 13:00 underflows `GameTime::minutes` (routine.rs:24, 548-550);
  - the clock gift arrives as a parcel (mod.rs:3063-3093);
  - the stage gets a TV at visit 2 (mod.rs:120, 3038), and `visit_from` calls `shop()`.
- New: one helper, `fed_afternoon(room, seed, graphics, mood)`:
  - Tuesday 13:00, `clock_sent = true`, no `shop()`, then `cue(Scene::Arrive)`;
  - paint, `expect` Visiting, then `set_mood`;
  - every owned piece shown unboxed in both modes;
  - the bare stage holds back its TV with existing fields: `ordered = Some(Tv)` and `bought_on`
    far ahead (not `u64::MAX`: `advert` adds `SHOP_EVERY`, mod.rs:131). Check that nothing
    speaks from `ordered`.
- Each run asserts at the end that props and `ordered` are unchanged, there was no re-arrival,
  and `mood() == forced` at every step.

**B5. The band window and thresholds.** (feas B1, M2; tests M2)
- Old: tally from arrival; "shorten minutes before adding seeds"; floors 7/12/15; thresholds
  checked against the per-seed spread.
- New:
  - **Warm-up:** the share and set-offs count from minute 3 (warm-up printed apart). M ≥ warm-up
    + 2 × the longest still act. Spend budget on seeds, not shorter M.
  - **Floors:** target − 4, so 11 / 16 / 20; the ceilings stay 17 / 26 / 31. Per room, add
    industrious ≥ 1.6 × lazy (a starting value, re-checked against σ). Tune to the middle.
  - **Assertions:** assert the pooled ratio (Σ moving ÷ Σ in-sight). Step 1 measures σ of the
    N-seed mean over about 20 disjoint seed sets.
  - **Gate:** the gate runs a reduced N, with thresholds at least 3σ (at that N) from the tuned
    mean. `CENSUS_BAND_SEEDS` (a helper like `proptest_cases`, plus a line in
    testing-strategy.md) runs the full-strength check at step boundaries. If 3σ doesn't fit, pool
    the floors per mood across rooms and keep per-room ceilings.
  - Drop D4's "arrival Restless 0.7 → 0.5" fallback: it would change the arrival to fix a
    measuring artefact.

**B6. Daydream sessions are one act, not repeated `muse` calls.** (mech B3, char M5)
- Old: "up to n musings … each a fresh `muse`; n 1–3 × linger". Each `muse` ends in `set`
  (osaka.rs:2244-2251), which pays the first segment and clears `credit`. An hour glance clears
  it first (3822). Every musing reuses the decision's whims, so a session is all riddles or none.
- New:
  - `Act::SpaceOut { …, session: Option<Session { left, next }> }`, whose `fire` arm `say`s
    musing k at `next`, with no `set`, drawn from `whims.series("daydream", k)`;
  - riddles, the Escalator rare and the hour glance only at k = 0; those that end the session
    call `credit_done(at)` first;
  - musings by mood, not ×linger: Dreamy 2–4, Ordinary 1–3, Lazy 0–2, Industrious 0–1;
  - spacing and n drawn from whims, never `rng`;
  - say that MUSINGS (8 lines, 10-minute cooldown, mind.rs:673-687, 878) runs dry over back-to-back
    sessions, and that a silent session just spaces out.

## 2. Majors

**M1. D0a: hoist fades onto `Guest`.** (mech minor, promoted under "fix the class"; tests M5)
- Old: a `tend_fades` helper that all three arms call.
- New: `fades: Vec<Dissolve>` on `Guest`, tended once before the `match`, painted over every
  state, and included in `next_tick`. This also makes the sibling bug impossible: Away →
  Arriving and `school_out` drop `Empty.fades` outright (mod.rs:1480-1493).
- Verification: the moved traces are insertions at `t0 + DURATION_MS`, plus changed frames only
  where a fade overlaps Away → Arriving.
- Keep the property, and add one deterministic case: seated, a focus rain-out, a tick at the
  fade's end (Leaving's arm returns true, so the property's strength is in Visiting).

**M2. D0b details.** (mech M1, M2; feas M5)
- The SpaceOut arm skips `ScriptId::ClockGlance` (`choose_next` sets `credit` after a glance,
  osaka.rs:5709).
- Work: `credit_whole(Want::Work)` in `come_home` (osaka.rs:7096). Drop "a shift cut short
  credits its share": chat can't cut Away or Door (`OnChat::Back`, osaka.rs:859).
- Once SpaceOut credits, the bare-room idle top four is Stand, Travel, Walk and one still, with
  Stand re-rolling every 2–5 s (osaka.rs:5739). The tuning list (M5) gets SpaceOut
  `own_sake: true` (lint-safe: it serves, brain.rs:1150) and Stand 3–8 s, measured.

**M3. D1 metric definitions.** (tests M1, M3; feas M4, m8; mech minor)
- **Set-off:** a move from a still body (or a decision) into a walk, climb or
  door. Look or Stand → Walk counts. Moving → moving within one trip doesn't, and nor does hidden
  → moving (a door's far side, back from Away).
- Count set-offs with a `#[cfg(test)] set_offs` counter bumped in `set`, as `set_downs` is
  (osaka.rs:6056), not by sampling. A unit test pins which transitions count.
- Split set-offs by purpose, and print doors apart.
- **Decided unless the user objects:** Pull's heaving steps move her (osaka.rs:3282-3290), so they
  count as moving, with a "text" body in its own column. Swap, Sneeze and PutBack move text, not
  her, and are printed apart.
- The cap's "target share" is the user's number (15 / 22.5 / 30). Its 1.15 slack comes from
  step 1's σ, like the share's tolerance.

**M4. Travel moves with Walk.** (char M2, feas M3, tests minor)
- Old: Walk 14 → 9, Travel unchanged at 10, so Travel outscores Walk and hops and doors become
  her answer to restlessness.
- New: Travel 10 → 7. `a_restless_osaka_mostly_moves` asserts > 50% (brain.rs:1042); it reads
  52% at 7 and 50.6% at 6. Note the margin in 3c: Walk below about 6 breaks it.

**M5. Step 7 tuning order and stop rule.** (feas M1, M4, M5)
- Old: "settle-in odds, then lengths, then Walk base, then needs", stopping under the ceiling.
- New:
  1. Rise times: Restless 300 s, Tidy 240 s, **Mischief 4 → 8 min**. Then the Walk and Travel
     bases. Read on all rooms.
  2. Pull and swap locality, on the stage, which is need-limited.
  3. Base lengths at partial strength, on the home and resident, which are time-limited.
  4. SpaceOut `own_sake` and Stand length (M2).
  5. Linger and settle-in last, as the knob for the spread between moods.
- Stop rule: inside the band in every room, nearest the user's target. Expect lengths to back off
  in the home and resident.

**M6. Nearer spots.** (char M7, feas M4/m7, mech M6)
- Drop the distance weight on `walk`: it shortens wanders without lowering set-offs.
- `pull`, `swap` and `pick_build` (mind.rs:546-575, a sibling the design missed) go same floor
  **strictly first** when her floor has an offer, as `leftover` sorts, then nearer. This makes
  "pulls stay on her floor" true. ×4 would only move her from 20% to 50%.
- `place` keeps its weight. `Place::Make` takes the distance to its nearest `chances.builds`
  entry. "Floor away" is +40 for any other platform.
- Land the weights neutral first (M11).

**M7. Settle-in mechanics.** (mech M5)
- Capture `ended = (self.act.clone(), self.credit)` before `credit_done` (osaka.rs:5395), and
  settle only when `ended.1` is a chosen still want. Otherwise she'd settle after a post-swap
  SpaceOut, Setsubun, a glance or a trial sit.
- Lounge → Nap: the same piece's Nap seat, found in `chances.seats`, not via `mind::bind`'s whim
  pick. Test that it's in `places(Nap)`.
- Keep `facing`, since `idle_act` rerolls it (osaka.rs:6385-6390).
- Use `whims.odds("settle in", depth, p)`. Push `decisions` (own labels), not `choices` (tests).
- Under B1, a still act that ends during an in-place watch settles first (before the watch
  reflex, osaka.rs:5495-5503), and the watch carries on in the new pose.

**M8. Mood linger.** (char M6, M4; feas m4; mech minor)
- Linger applies at the call sites (`idle_act`, `plan`, `muse`/`wonder`, `start_job`'s `usual`),
  never inside `duration()`. Not to night-on-floor, trials, glances, the post-swap SpaceOut or
  Setsubun.
- Homework leaves linger. Mood moves its nod-off point instead: Lazy ⅓, Ordinary ½,
  Industrious ⅚, plus floor homework's face-down doze. Decided unless the user objects.
- LieFront keeps today's length, with no linger: 2 Hz kicks for 75 s draw the eye.
- Gaze shows `Ooh` for its first 3 s, then `Curious` with no bubble.
- Dreamy starts at Ordinary's linger and settle odds, and differs by what it does (sessions,
  cloud-watching).

**M9. Made-piece `BOUND` stays 90 s.** (tests B2)
- Old: derive it from the longest still act at Lazy, which makes the property vacuous (over the
  180 s run).
- New: the property forbids a still act while a piece waits, and making is never still, so the
  bound keeps its meaning (chat gap + watch + walk) under either Q1 answer. Add
  `assert!(RUN_MS > BOUND + MAKE_MS)`.

**M10. D5 wiring the design leaves open.** (mech M7, M8, M9; char m3, m4)
- **Floor homework:** its own method with the desk guard (`IDLE` binds unconditionally,
  mind.rs:227-232). Test shown pieces, not `seats_of` (a seat blocked by text drops out,
  mod.rs:3410-3414). Frames at `USE_FRAME_MS`. "My back..." only when `ached` is first set, in
  `choose_next` on `ended`, holding a Stand while it shows.
- **Made desk:**
  - its seat is beside it, by a per-piece rule (`Use::Homework.inside()` is true, room.rs:435;
    `spots_for` puts it inside, mod.rs:3449);
  - its pose resolves from the host (HOMEWORK names the stool pose);
  - chopsticks are gated on a new `SpliceCtx.makeshift` (script.rs:884-901) and on `Cue::plays_on`;
  - it also needs a `Loss::says` arm and to join `Scene::ALL`, `stage::direct`, `Want::ALL` and
    `Activity::ALL`.
- **`use/borrow`:** `Job::Borrow(Pull)` and `Act::Borrow { phase }`. It needs `props`, `job()`,
  `lost_grip`, mending on place, evict and leave, a `credit_done` arm and a motion class. She
  reads **beside the tear** (one set-off), on her own floor first. Name the slide-back operation.
- **D6:** carry the sky musings on `fire`, as in B6, or as `Say::Drawn` with `drawn: [u8; 4]`.
  Have at least 4 sky lines per sky. Say what settles from the beside-the-sill spot.

**M11. Goldens: verify every behaviour commit.** (tests M5, feas m3)
- Every behaviour commit trace-diffs: each trace's first differing line and the act there.
  Before-traces come from the main checkout at the step's start (`HOUSEGUEST_GOLDEN_TRACE`), with
  a git worktree only as the fallback.
- `WATCH_MS` is its own verified commit: the traces should first differ 5 s after a line.
- Step 6 lands golden-neutral: linger 1.0, settle p = 0, n = 1, all weights 1 (`pick_weighted`
  with all weights 1 is the plain `below`, osaka.rs:292-293). Step 7 flips the values with one
  re-record.

## 3. Minors

1. Tests the levers move, each restated in its step (tests M4):
   - `an_uninterrupted_trip_runs_its_course` (keep ≥ 8; add seeds);
   - `every_pooled_line_shown_was_drawn_from_its_pool` (run cued muses to the act's end;
     `shortest_body` and script.rs:2293 read `SPACE_OUT_MS.0`);
   - `the_census_counts_her_vignettes` (lengthen the span);
   - `looking_out_lasts_and_counts_as_spacing_out` (osaka.rs:12412);
   - the surf test's host pose;
   - `her_mood_shows`' dreamy assertion (decide cloud-watching's group).
2. `const _: () = assert!(WATCH_MS > LOOK_MS);` for two tests (tests.rs:9553, 10974; tests M4).
   Under B1 they, and the pin at osaka.rs:8377, are restated on the overlay.
3. `at_home_her_furniture_beats_the_floor`: restate it on rest she chose (> 10× floor), with
   settled rest printed apart. Don't loosen it to "3×" (tests M4).
4. `sleepiness_draws_her_to_lie_down` reads bases only: drop "sensitive". The band tests are line
   art only: state the exemption from "every test loops graphics" (tests minor).
5. Steps 5–7 run the ignored band test with `--run-ignored only` and put its table in the commit
   message. Un-ignoring doesn't touch thresholds (tests minor).
6. Re-baseline after the `WATCH_MS` commit. The freed watch time raises the share before the
   levers land (feas m1).
7. Count "restarts after a watch" (a cut walk to a job) in step 1 (feas m2).
8. Print exercise share and starts per minute, and bubble onsets per minute. If exercise rises
   over 3 points above the baseline, the numbers go back to the user before pinning (char M3,
   M5).
9. The band's home is `furnished_room`, which has no window. Step 10 re-reads the band on a
    windowed home, printed (feas m5).
10. The day census's per-mood share is a hand-checked acceptance at steps 7 and 11 (feas m9).
11. Cloud-watching is a held pose (period 0, eyes open) with one static bubble and at most one
    line (char m5).
12. Art sheet: an optional sitting doze for a second settle branch (char m9). Budget the band's
    gate CPU against the stop hook in step 2 (mech minor).

## 4. Questions for the user

- **Q1. When chat speaks during a still act, does she look up where she is (sitting or lying),
  or stand to look and then go back?** *Recommended: where she is.* This builds your own reason
  ("the act of looking is there solely to draw attention, which only happens during change"):
  standing up and lying back down are the biggest changes her sprite makes, and longer acts catch
  more lines. It also deletes the resume machinery (B1 vs B2). The andagi answer and the night
  stir already work this way.
- **Q2. The TV: a held programme picture after it switches on and while she surfs, instead of
  static every 400 ms the whole time?** *Recommended: yes.* Today's static (script.rs:91) would
  run up to 3 minutes per watch after the lengthening, and it would be the most eye-catching
  thing in a TV home. It needs a little new art; colour bars are already static. If yes: a test
  that no act longer than 30 s flips cells faster than `USE_FRAME_MS` after its first 10 s
  (char M4).
- **Q3. A brief blink every 6–12 s on held poses (Sit, Lounge, cross-legged, long Gaze)?**
  *Recommended: yes, in step 7's re-record.* Only Stand blinks today (osaka.rs:2721-2723), and
  minutes-long holds read as frozen rather than still.

## 5. Rejected critic points

- **char M1 (the watch reflex pre-empts settling) as stated.** In the design as written, a line
  cuts the still act (osaka.rs:6477), so no still act can end on its own inside the 5 s watch,
  apart from the andagi answer. It becomes real under B1, and M7 handles it there.
- **char M2's door ×2 in the set-off cap.** Travel 7 (M4) removes the cause. Q3 counted walks,
  hops and doors alike, and doors are printed apart (M3).
- **char M5's global 12 s floor between self-started lines.** There's no measured chatter
  problem. Bubble onsets are printed first (minor 8).
- **tests B3's "a shift cut short credits its share" case.** Chat can't cut a shift
  (`OnChat::Back`, osaka.rs:859). Only `place`/`evict` can, and that's not worth a credit path.
- **mech M3's resume slot (before `leftover`).** Superseded by after `leftover` (B2). With
  mech M4's capture guards, the two differ only in favouring a waiting made piece.
- **mech B2's "double-credit".** The design's left ÷ whole is arithmetically right. Its other
  points (script replay, no `whole` on Idle/SpaceOut) are kept in B2.
- **mech M6's zero-weight `walk` fix.** Moot, since `walk` loses its weight (M6). `pick_weighted`
  still falls to `n − 1` at total 0 (osaka.rs:298-304); guard it if any caller can zero everything.
- **char m6's moving bird, char m7's seated lazy SpaceOut.** Outside the band, and each adds an
  onset or a pose variant. Revisit in step 10 or later.
- **feas m7's "drop `place` weighting".** Kept: golden-neutral in step 6 (M11), so it costs nothing.
