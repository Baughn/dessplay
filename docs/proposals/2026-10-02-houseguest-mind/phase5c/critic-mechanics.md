# Phase 5c design: mechanics critique

**Critic: mechanics and correctness against the code (HEAD `cabf5d1`).** Paths are relative to
`dessplay/src/ui/houseguest/`. I checked every ref below against the code.

## Verdict

The levers are the right ones, and most of them plug in where the design says. Three mechanisms won't
work as written:
- D0b's class test can't observe or drive what it claims to.
- D3's resume on a "plain body, fresh since" double-credits the act and replays its script from the
  first key.
- D4's daydream session, built as repeated `muse` calls, loses its credit and repeats its rolls.

Each has a small fix (below). The majors are mostly spots the design leaves unspecified: where resume
sits in `choose_next`, how a stale resume dies, how settle-in learns what she chose, and the made desk's
seat and pose. None of this touches a user decision.

## Blockers

**B1. D0b's class test can't be built as specified.**
- It says "cue each want's `Scene` … assert `credited` holds it". But `stage::direct` never sets
  `credit`: it calls `place`/`pursue`/`idle`/`muse`/`go_to_work` directly (stage.rs:380-648). Only
  `choose_next` and a few named paths set it (osaka.rs:5630, 5704-5710, 3724, 6031).
- There's no Scene for plain SpaceOut, Stand, Walk, or Travel as a want.
- `credited` is pushed only inside `credit_done` (osaka.rs:5007). Wants paid by other paths never reach
  it: Walk/Travel are served at set-off (5704-5707); Pull/Swap go through `credit_whole` (5023-5028);
  Arrange is a direct `serve` in `set_down_done` (6062-6065; the design's list leaves Arrange out). So
  the test fails for wants that are correctly credited.
- **Amend:** move the `#[cfg(test)]` record into `serve()` (5032), so every path records. Drive each
  case through `choose_next` with a test-only offer filter (`#[cfg(test)] only: Option<Want>`, applied
  after `offers` is built at 5639). Key the test on **(want, method)**, not want: D5's `use/borrow` is a
  second path for Use(Read) that a per-want test would never reach. Build rooms where each method binds.
  A wildcard-free `credit_path(want, method)` match alongside the test would also catch a new variant at
  compile time.

**B2. D3 resume: "plain body, fresh since, credit × left/whole" double-credits and replays scripts.**
- The cut already credited the done share. `interrupt` → `set(Look)` → `credit_done` pays the share
  done and clears `credit` (osaka.rs:6518-6524, 2742-2747, 4945-5005). A resumed act needs only `done/whole`
  more, and Idle and SpaceOut have no `whole` to scale by (Act at 445-449, 520-525).
- A fresh `since` replays the body from key 0:
  - Homework writes again after she'd nodded off (script.rs:1339-1358, `Span::Upto` fractions of the
    body).
  - `Play::plain`/`of` gives branch 0 (script.rs:1045-1074). For LookOut that's "Sunny!", a day line,
    even at night (script.rs:1703-1705, 1735). It's also a fresh line after the chat, which defeats the
    point.
  - Day Sleep relights the lamp for `LAMP_ON_MS` (script.rs:1264-1272).
  - A riddle in progress restarts.
- **Amend:**
  - Resume keeps `play.own` and `branch` and drops `before`/`after` (and `grievance`, `answering`).
    Surf, Shopping and FirstSunrise collapse to plain Watch.
  - **Back-date** the start: `since' = at − (cut − body_start)`, `until' = at + left`. The body is then
    exactly as long as it was, so keys pick up where they were. `whole` is unchanged, and Idle/SpaceOut
    frames keep their phase.
  - `Resume` carries `credited: f64` (the share paid at the cut). `credit_done` pays
    `max(0, share_now − credited)`, through a `credit_from: f64` on `Osaka` that resets with `credit`.
  - Build the act directly, never through `start_job`'s Use arm (osaka.rs:3400-3580). That arm would
    re-fire cues, splices, surf, `HomeEvent::Bought/Used`, meal lines and the grievance draw.
  - Restore `facing`: `look` turned her to the chat (6464), and `start_job` sets `seat.facing` (3391).

**B3. D4 daydream sessions as "each a fresh `muse`" break the credit and repeat the rolls.**
- Every path in `muse` ends in `set(...)` (osaka.rs:2244-2251; `wonder` 2265-2276; `glance_up` →
  `glance_at_clock` 3817-3833). So the second musing's `set` credits the first segment and clears
  `credit`, and later segments are never paid (which undoes D0b). Each `set` also redraws `until`.
- `glance_at_clock` sets `credit = None` *before* its `set` (3822). An hour glance mid-session therefore
  forfeits the whole session's credit.
- The decision's whims are reused: `self.whims` is drawn once per decision (5393), and
  `Whims::chance`/`Lines::pick` are pure in `(whims, label, salt)` (mind.rs:66, 1064, 1078).
  - Every musing's riddle roll (RIDDLE 1/3, mind.rs:858-863) comes out the same: all riddles or none.
  - The same holds for "rare-musing" and "hour-glance" (2191, 2294). Today that makes "once a session"
    true by accident, but then the Escalator rolls `try_play` again at each musing.
- `Act::SpaceOut.play` holds one `Play` (445-449), so a riddle as the second musing has nowhere to live
  without a `set`.
- **Amend:**
  - One act: `Act::SpaceOut { since, until, play, session: Option<Session { left: u8, next: u64 }> }`,
    with a `fire` arm that `say`s each musing at `next` without a `set`.
  - Draw each musing from `self.whims.series("session", k)`.
  - The Escalator and the hour glance roll only at k = 0 and end the session through an explicit
    `credit_done(at)` first.
  - Riddles only at k = 0.
  - Resume (B2) carries `session`.
- Note: musings aren't budgeted (mind.rs:954-962), so the design's "no new line budget" changes nothing.
  The real bound is MUSINGS' 8 lines at a 10-minute cooldown (mind.rs:673-687, 878): back-to-back
  sessions go quiet quickly. That's fine, but say so.

## Major

**M1. D0b's SpaceOut arm credits clock glances.**
- `glance_at_clock` clears `credit` ("a glance eases nothing", osaka.rs:3822, pinned at 12224-12230).
  But that test calls `muse` directly.
- On the real path, `plan` → `muse` → `glance_up` returns true, and `choose_next` then sets
  `credit = Some(SpaceOut)` (5709). The new arm would pay the glance.
- **Amend:** the arm excludes `play.own == ScriptId::ClockGlance`, or `plan`'s Muse arm returns whether
  to credit. Extend the test to go through `choose_next`.

**M2. Work's credit has one real site, and "a shift cut short" barely exists.**
- `credit_done` falls through to `_ => return` *without* clearing (5003), so `Some(Work)` survives the
  whole shift. That's fine, since credit is meant to wait.
- Away and Door are `OnChat::Back` (859), so chat can't cut a shift. A cut on the way out ends as "came
  to nothing" (5422-5424). The only other cuts are `place`/`evict` (6413).
- **Amend:** `credit_whole(Want::Work)` in `come_home` (7096). Drop "credits its share" and "the hop
  out", or define the share on `Door { since, gap }` / `act_since` for the `place` case only.

**M3. D3 doesn't say where resume sits among the steps between the watch and the first continuation.**
- Between "watching chat" (5495-5503) and `heading/hop` (5582) come: the greeting, `needs.pass` with
  `decided` (5514-5526), `calendar_beat` (5530), day off (5538), and the owed beat (5557).
- `look` sets every pending put-back due now (6444-6447), so owed glances after a chat are likely.
- **Amend:** after the owed beat and `calendar_beat` (one-shots in place; the spot guard covers a beat
  that moves her), before `heading`/`arrange_next`/`leftover`. Test a chat that both cuts a still act and
  owes a beat.

**M4. A stale resume survives stage cues and other `set`s.**
- `place` (6402-6419) doesn't know about resume.
- The stage's activity arm doesn't `place` when she's on a floor (stage.rs:629-641). So a cued Sit at
  the same spot within 30 s ends in resuming the pre-chat act.
- **Amend:** one funnel. `set` clears `resume` unless the new act is `Look`, `Stand`, or `Glance`
  (2742). The resume path `take()`s it before its own `set`. Then no new caller can leak it.
- Also don't capture while `episode.is_some() || just_set.is_some()`, or under `dash`, `errand`,
  `shift`, `leaving` or `returning`.
- A second line during the Look cuts a `Look`, not a still act. Capture only from a still act, and never
  clear on such a cut.

**M5. settle_in can't tell what was chosen, and Nap must be built directly.**
- `credit_done` at the top of `choose_next` (5395) has already consumed `credit` when settle-in runs
  (after `leftover`, 5627-5635). So it can key only on the act's shape.
- Shape alone would settle after a post-swap SpaceOut (3040-3048), Setsubun (4531), an hour glance, or
  a trial sit.
- **Amend:** capture `let ended = (self.act.clone(), self.credit)` before `credit_done`, and settle only
  when `ended.1` is a chosen still want.
- It's right that the ended act is still `self.act`: Idle (2761-2768), SpaceOut (3029-3032) and Use
  (2922) all go straight to `decide`, and a cut goes through `Look`.
- `mind::bind` for Use(Nap) runs `place`, a whim pick over *every* Nap seat (mind.rs:434-441), and on
  the 1-in-20 makeshift whim it drops the real seat (405-416). Requiring bind to return this sofa's seat
  makes settling a lottery.
- **Amend:** take `chances.seats.find(piece == lounge.piece && what == Nap)` and `go_to` it (same spot,
  since Lounge and Nap are both `inside`, room.rs:170, 435-440). Test that it's a member of
  `places(Nap)`.
- Use `whims.odds("settle in", depth, p)` (mind.rs:79). `chance` takes n/d.
- Keep `facing`: `idle_act` rerolls it for Sit/LieBack (6385-6390), so a settle chain would visibly
  flip her.

**M6. Nearer spots: `Place::Make` has no distance, and an all-zero walk falls through.**
- `Place::Make(item)` has no position (mind.rs:370-375). It needs a weight, e.g. the nearest
  `chances.builds` entry for it.
- Weight 1 would make making (×3 in D5) likelier exactly where seats are far.
- "No target within 3 cells" zeroes every column of a short floor. `pick_weighted` then returns
  `Some(n − 1)` (osaka.rs:285-305), a walk of ≤ 3 cells.
- **Amend:** `walk` returns `None` when the total weight is 0. Multiply the distance weight with the
  existing chat weight (mind.rs:257-262). Define "floor away" as +40 for any other platform (there's no
  floor graph).
- Missed sibling: `pick_build` (mind.rs:546-575) picks where she tears text, which is the
  "walk to text" the band counts, and D5 multiplies it. Give it the same distance weight.

**M7. D5 floor homework and the makeshift desk.**
- `methods(Want::Idle(_))` is `IDLE`, whose `idle` binds unconditionally (mind.rs:181, 227-232). Floor
  homework needs its own method with the desk guard.
- Detecting the desk through `chances.seats` misses a desk whose seat is blocked by text, because
  `seats_of` keeps only restful seats (mod.rs:3410-3414). Test shown pieces instead.
- "Ends with a sore moment": an Idle ends straight into `decide` with no coda (2761-2768). Say where the
  line and `ached` go (in `choose_next` on `ended`, holding a Stand while it shows, like day off at
  5538-5555), and whether a cut or resumed session aches.
- The made desk's seat "beside it": `Use::Homework.inside()` is true (room.rs:435-440), so `spots_for`
  puts it inside (mod.rs:3446-3450). The rule must be per piece, e.g. `inside() && !(made && Desk)`,
  and `roomy` and `builds`' `then` must agree.
- HOMEWORK's keys name the stool pose outright (`Posed::Bob(Pose::Homework…)`, script.rs:1339-1358).
  The floor pose at the cube needs host-style resolution like Watch's (osaka.rs:7189-7194), or its own
  ScriptId.
- Chopsticks "its `when` checks the seat": `SpliceCtx` has no seat (script.rs:884-901). Add
  `makeshift: bool`, and also gate `Cue::plays_on` for the stage.
- Also: `Loss::says` needs a Desk arm (mind.rs:596-604; `_` gives "...my sofa."), `Scene::ALL` grows,
  and `stage::direct`'s MakeSofa|MakeBed arm (stage.rs:509) grows. `Want::ALL` goes `[27]` → 28, and
  `Activity::ALL` and its four matches (1183-1251) need the new variant.

**M8. D5 `use/borrow` names no Job, act or credit.**
- `Job::Use` is a seat (scenes.rs:122-133). The act machine has `Reel` toward her hands but no way
  back: `Restore` is instant (scenes.rs:266-280).
- **Amend:** name `Job::Borrow(Pull)` and `Act::Borrow { phase }`, plus:
  - `props` (Stays::Job, OnChat::Look);
  - `job()` and `lost_grip` (Reel grips, 290-293);
  - mending on `place`, `evict` and leave;
  - a `credit_done` arm. Without one it's B1's class bug again, and the per-method test catches it;
  - a `census_motion` class (the walk to the line is "to text").
- Pick the honest name for the slide-back: an instant `Restore`, or a new unreel op.

**M9. D6 musings inside a Use have no carrier.**
- `Say` has Bubble, Pitch, Riddle and Answer only (script.rs:63-73), and `Play.drawn` is `[u8; 2]`
  (1040).
- **Amend:** `Say::Drawn(k)` plus `drawn: [u8; 4]` (Play is Copy), or `say` from `fire` as in B3.
- Sitting under the sill only exists where `look_out_spots` gave the under-sill spot (mod.rs:3448). Say
  what settles from the beside spot.

## Minor

- **D0a, structurally.**
  - The paint sites also `retain` (mod.rs:1996-1998, 4193-4195). They're harmless, since the paint
    draws the frame without the dropped fade, but "no arm touches `retain`" overstates it.
  - Away → Arriving drops `Empty.fades` outright (1487-1493, and `school_out` 1607 via 1488 and 1571).
  - Leaving's property is vacuous: it returns true (1541-1546).
  - The unrepresentable version is `fades: Vec<Dissolve>` on `Guest`. It's tended once before the
    `match`, painted last for every state, and next-ticked once. Then no arm or transition can drop or
    misjudge a fade.
- **Mood linger at the call sites, not inside `duration()`/`use_duration()`.**
  - Scale at `idle_act` 6383, `plan` 5742-5746, `muse` 2247, `wonder` 2270, and `start_job`'s `usual`
    3414.
  - Don't scale: `night_on_floor` (which reads `LieBack.duration()`, 3685), trials (`TRIAL_USE_MS`,
    `whole` midpoint 3539), the glance (`CLOCK_GLANCE_MS`), the post-swap SpaceOut (3044), or Setsubun.
- **Resume guard wording.** `Osaka` can't see `Visit.made`, but made seats appear in `chances.seats`
  only while the piece stands, so "re-read `chances.seats`" covers both. `Seat` is `PartialEq`
  (room.rs:468).
- **Set-offs.** Say whether `travel` (a Walk to the link, then a climb) is one set-off or two, before
  the classifier counts.
- **D2 cost.** 12 tests × 2 chat conditions at ≤ 3 s each add about 36 s of CPU to every gate run. Fine
  under nextest, but budget it against the stop hook.

## Right; keep

- `tend_fades`' shape and its property, and re-recording only after verifying the traces moved.
- Use credit can already resume: `whole` and `body_start` (4985-4999) make B2's subtraction a one-line
  change.
- `use_real`/`use_made`/`use_make` share one `place` draw (mind.rs:443-467), so weighting `place` keeps
  offering and planning agreed.
- Every SpaceOut-shaped act (muse, riddle, Escalator, glance, Setsubun) is `Act::SpaceOut`, so one arm
  covers them, with M1's exception.
- Every pose shares the 5×4 box (sprite.rs:112-129), so LieBack needs no extra room on a platform.
  Lounge and Nap seat at the same spot.
- Settle-in after `leftover`, the routine reflexes and `arrange_next` is the right slot. Resume only on
  `Cause::Chat` (not ChatPassing, Restless or SeatGone) is right. Night stirs and the aloft path never
  `interrupt` (6450-6457), so they can't make a resume.
- The ×3 for `Job::Build` in the `factor` closure (5669-5682), as map C4 says. Watch from the floor
  already binds, so cross-legged is one host-pose edit (7190-7194).
- `WATCH_MS` → 5 s is a one-constant change. Its pin (8377) reads the constant.
