# Phase 5c (stillness) — implementation design

**Working design, 2026-10-05.** Brief: docs/plan.md, Phase 38, "Phase 5c — stillness (brief, 2026-10-05)".
Code map with file:line refs at HEAD `cabf5d1`: [phase5c/map.md](phase5c/map.md). Its last section
("Gaps and corrections") overrides the sections above it. Paths are relative to
`dessplay/src/ui/houseguest/`. HG is docs/proposals/2026-09-28-houseguest.md.

## User decisions

From the brief (2026-10-05):
- **A target band per mood** for moving, as a share of awake time: about **15 lazy, 20–25 ordinary and
  dreamy, up to 30 industrious**. It becomes a census band test per room and mood.
- **Walking to a job counts** as movement.
- **New art is fine** (model sheets first).
- **The window is a daydreaming place:** look-outs become rarer and much longer.
- **A dash keeps looking at chat lines.**
- **Apply the rain-out fix** and re-record the traces it moves, with the reason.

This session (2026-10-05):
- **Q1, chat.** A still act cut by a chat line **resumes** once she's looked. The band is judged both in a
  quiet room and at the census chat rates, and must pass both. **The watch after a line drops from 15 s
  to 5 s** (`WATCH_MS`): "the act of looking is there solely to draw attention, which only happens during
  change."
- **Q2, mood.** The difference between moods comes from **still-act lengths and settling in**: a lazy
  Osaka lingers and settles further more often, an industrious one is briefer and gets up. No mood factor
  on walking, so the `sane_factor` rule stands.
- **Q3, the metric.** The band pins the **in-sight time share** per mood **and a cap on set-offs per awake
  minute** (walks, hops, doors), so shorter or faster walks can't pass on their own. Exercise in place
  (jacks, toe touches) and lying kicking her feet count as still, and are printed.

Told to the user (not questions): the brief's 35–45% included time she's off screen (away at work, a
door's hidden beats); the band's quantity is moving in sight ÷ awake time in sight, roughly 33–38% today
(to be measured in step 1). Two older bugs land first as their own commits (D0).

**Decided unless the user objects** (each recorded in decisions.md when it lands):
- The band is pinned in **line art** (the user's client is kitty); step 1 measures both modes once.
- A door's **visible** beats count as moving; its hidden beats, Away and her work shift are out of the
  band entirely (neither moving nor still).
- A walk to text torn for a made piece (`Job::Build`) is "to text" (the walk's goal), with the want shown
  in a second table.
- D4 of 5b ("Gaze ×more with a window") is dropped: the window is now the daydream place itself (D6).
- `hour_glanced` stays once a visit unless step 5's measure says the afternoon glance is still starved.

## Design decisions

### D0. Pre-commits (each alone, before step 1 measures anything)

**D0a. The rain-out's last frame** (5b "Open"; map §rainout). The Visiting arm of `Guest::advance`
computes `fading` after `retain` (mod.rs:1499-1500), so the tick a fade ends on can report "nothing
changed". Away's arm has the order right; Leaving ignores the result. Make the class unrepresentable: one
helper all three arms call,

```rust
/// Drop the fades done at `now`. Whether the screen could change: any was
/// running (its frame, or the frame without it).
fn tend_fades(fades: &mut Vec<Dissolve>, now: u64) -> bool
```

and no arm touches `fades.retain` itself. Test first: a property over the states with fades (Visiting,
Away, Leaving): **whenever a fade was live before `advance(now)`, `advance` returns true**. It fails on
HEAD for Visiting. The ~24 moved traces (resident fed/unfed, errand seeds 1 and 3) are re-recorded; the
procedure in *Goldens* confirms each moved trace first differs by one added frame at a fade's
`t0 + DURATION_MS`.

**D0b. Wants that serve needs but are never credited.** `credit_done` (osaka.rs:4964-5003) has no arm for
`Act::SpaceOut` (SpaceOut serves Daydreams 0.6; design.md:1459 says spacing out answers daydreams) or for
Work (serves Restless 0.5). Fix the class:
- SpaceOut (and Muse, which is a SpaceOut) credits by the share of its span done, `Spot::Any`.
- Work credits whole when the shift (or the hop out) completes; a shift cut short credits its share.
- **Class test:** over `Want::ALL`, every want with non-empty `serves` is credited by some path (cue each
  want's `Scene`, run it to its end, assert `credited` holds it with share > 0). Stand and Sneeze serve
  nothing; Walk/Travel are credited at set-off; Pull/Swap by `credit_whole`/offset. A future want with
  `serves` and no arm fails it.
- Two commits (SpaceOut, Work) or one; each re-records all goldens with the reason.

### D1. Measuring (step 1; test code only, no golden moves)

**The classifier.** A `#[cfg(test)] pub fn census_motion(&self, now) -> Option<Motion>` on `Osaka`, beside
`census_group` (osaka.rs:5249), wildcard-free so a new act won't compile without a class. `Motion` has two
axes:
- *Body:* `walk` (Walk, Out), `climb` (Climb, Clamber), `fall` (Peer, Fall, Dazed), `door` (visible
  beats), `hidden` (Away, a door's hidden beats, i.e. `hidden(now)`).
- *Purpose*, in priority order: `heading.is_some()` → to a job by kind; `Walk { then: Job(job) }` → to a
  job by kind (**text**: Pull, Swap, Build; **seat**: Use; **home**: Lift, SetDown); `shift` → work;
  `leaving`/`returning` → routine; `dash`; `errand`; otherwise by a chain tag: `walk/along` → wander,
  `travel/*` → travel, off text, arrival/return, accident.
- **The chain tag:** a `#[cfg(test)] chain: &'static str` set in `decide()` from `Decision.method` and
  overwritten at the non-decision `set` sites (arrival walk-in, floor gone, back from Away, climb after a
  hop keeps its hop's tag). No behaviour changes.

**What's counted.** Per visit: in-sight ms (Visiting, not hidden, not asleep for the night), moving ms by
(purpose, body), and **set-offs**: each start of a Walk, a Travel hop, a door, or a `go_to` hop (one per
leg; a climb that continues a hop is not a new set-off). Also exercise ms and animated-still ms (LieFront)
printed apart.

**Where.**
- `visit_census` (unfed, 5a-comparable): adds per room×mood: moving % of in-sight time by purpose and body,
  set-offs per in-sight minute, and **per-seed min/max** of the share.
- A **fed afternoon census** (new, `#[ignore]`d, printed): each room fed at a fixed weekday 13:00 (school
  done, the Afternoon slot) for 15 real minutes, mood forced after the first paint and **asserted**
  (`set_mood` behind an `if let` today fails silently: make it an `expect`). No shopping (rooms stay as
  set). Both chat conditions: quiet, and the room's census cadence. Both drawing modes. This is the band's
  own setup, so step 2's thresholds read off it.
- `day_census`: `Stretch` gains moving by purpose and a mood bucket (`format!("{:?}", osaka.mood())`);
  per-room week table of purpose × mood. Dash-ins and the Asleep/Away slot lines (<0.5% of their time)
  are excluded from any band reading. Bare-room numbers come from the unfed stage, not a fed week (its
  rooms gain furniture).
- Step 1 checks the forced mood holds through the fed afternoon (no `begin_day` inside 90 game minutes)
  and records the cost per sim-minute in line art.

**Output of step 1:** a baseline table in the design dir (`phase5c/baseline.md`): in-sight share and
set-offs/min per room × mood × chat × mode, fed and unfed, with per-seed spread.

### D2. The band test (step 2)

- One `#[test]` per room × mood (12; a `macro_rules!` is fine), each running the fed-afternoon setup in
  **line art**, **quiet and at the census cadence**, N seeds × M minutes (set from step 1 so each test is
  ≤ 3 s under the gate; shorten minutes before adding seeds).
- Each asserts, per chat condition, the seed-mean in-sight moving share inside its band, and set-offs per
  in-sight minute under its cap:

  | Mood | Share ceiling | Share floor ("too still") |
  |---|---|---|
  | Lazy | 17 | 7 |
  | Ordinary, Dreamy | 26 | 12 |
  | Industrious | 31 | 15 |

  Tolerances (the ceilings above sit a point or two over the user's targets) are re-checked against step
  1's per-seed spread before pinning.
- **The set-off cap** per room × mood: today's rate × (target share ÷ today's share) × 1.15, computed from
  the baseline, so a lever that only shortens walks fails it.
- It fails at today's numbers (confirmed by running it), and is committed `#[ignore = "5c: un-ignored when
  step 3 meets the band"]`; step 3c un-ignores it. A `CENSUS_BAND_SEEDS` env knob scales it for deep runs.
- The stage and resident are judged as well as the home; a TV-only home is printed.

### D3. Chat: a shorter watch, and resuming (step 3a)

- `WATCH_MS` 15 s → **5 s**. After a line she looks (`LOOK_MS` 4 s), then watches until 5 s after the last
  line, then resumes. A lively chat still holds her (each line renews it).
- **Resume.** When a chat line cuts (`Cause::Chat`, not `ChatPassing`) a **still act** (Idle Sit, LieBack,
  LieFront, Gaze; SpaceOut, a daydream session included; a Use of Lounge, Nap, Watch, Read, Homework,
  LookOut, and day Sleep), she keeps `resume: Option<Resume>`: the want, the seat (for a Use), her spot,
  and the ms left. In `choose_next`, after the routine reflexes and the watch and before every other
  continuation, a `Bucket::Continuation` "resume" re-enters it if:
  - she is still at that spot, the terrain there is still restful, and the seat still stands (re-read from
    `chances.seats`; for a made seat, `Visit.made` still has it);
  - the routine hasn't claimed her (to bed, to school, out to work), and it's within 30 s of the cut;
  - at least `RESUME_MIN_MS` (8 s) was left.
  Otherwise the resume is dropped and she chooses afresh.
- The resumed act lasts what was left. A Use resumes on its host's **plain body**: no prelude, no splice
  drawn afresh, no rare, no new line budget; a splice that was playing is over. The resumed act's credit
  is scaled by `left ÷ whole`, so a cut-and-resumed act eases no more than an uncut one.
- Cut again, it resumes again on what's left. A dash, an errand and the night don't resume (the dash keeps
  looking at chat, as built).
- **Rule** (design.md, Houseguest): resuming is a continuation of what she chose, like a leftover or a
  heading, not a new choice, so it skips the roll. decisions.md records why (attention is drawn by change;
  a fresh roll after every chat line was a walk half the time).

### D4. Cheap levers (step 3b mechanisms, step 3c tuning)

**Mood lingering.** `Mood::linger(self) -> f64`: Lazy 1.5, Dreamy 1.25, Ordinary 1.0, Industrious 0.7
(starting values). It scales the drawn length of every still act (the Idle restful activities, SpaceOut,
and the still Uses: Lounge, Nap, Watch, Read, Homework, LookOut, day Sleep), one draw as now. Not exercise,
chores (Unpack, Crumple, Snack, Pet) or the night. The script lints (`shortest_body`, `BODIES`) use the
shortest over moods (base minimum × 0.7), so base minimums rise to keep them.

**Settling in** (name: `settle_in`; `Osaka::settle` is taken). When a still act **ends on its own** (the
ended act is still `self.act` in `choose_next`; a cut one is a `Look` and goes through D3 instead), a
`Bucket::Continuation` "settle in" rolls `whims.chance("settle in", p(mood))`, p = Lazy 0.6, Dreamy 0.45,
Ordinary 0.35, Industrious 0.15 (starting values), and binds in place, no walk:
- SpaceOut or Gaze (standing) → Idle(Sit) → Idle(LieBack) (the doze; for Dreamy, cloud-watching, D5).
- Use(Lounge) → Use(Nap) on the same sofa.
- Use(LookOut) → sitting under the sill (step 5, with its art).
- Watch and Read on the floor: nodding off, if the art sheet has it (step 4); otherwise nothing.
- A chain ends at its last state. A settled act only binds what `mind::bind` would bind for that want at
  her spot (the offering/planning invariant), is credited as its own want, and doesn't enter `recent`.
- Settling sits after `leftover` and the routine reflexes, so bedtime, school and arranging win.
- Day dozes ease Sleepy (LieBack 0.15, Nap 0.1): re-check the 5b night numbers (bed by 22:00) in the day
  census after this lands.

**Daydream sessions.** A chosen SpaceOut lasts longer (D4 tuning) and its `muse` arm, instead of one line,
says up to `n` musings spaced 12–20 s apart (n drawn 1–3, ×linger rounded), each a fresh `muse` (riddle,
musing, seasonal pool); the hour glance rolls once a session, and the Escalator rare once a session. A
session with nothing left to say just spaces out. It needs D0b's credit first.

**Nearer spots.** All pure functions of `(Ctx, Whims)`, so offering and planning still agree:
- `walk` (mind.rs:253): the column weighted toward nearer ones, weight `1 / (1 + |dx| / 8)`, no target
  within 3 cells.
- `place` (mind.rs:434): seats weighted `1 / (1 + d / 10)`, d = `|dx|` on her floor, +40 per floor away.
  `use_real`/`use_made`/`use_make` keep calling it with the same whims and label.
- `pull`, `swap` (mind.rs:318-340): offers on her own floor ×4, then nearer first by the same weight. This
  is the stage's big lever (731 off-floor hops in 16 visits; map critic G7).

**Tuning (step 3c),** starting values, re-measured against the band, then pinned:
- `rise_ms`: Restless 90 s → **300 s**, Tidy 60 s → **240 s**.
- Walk base 14 → **9**; arrival Restless 0.7 → 0.5 if the fed afternoon still opens with a walk run.
- Base still-act lengths (before ×linger): SpaceOut 20–60 s; Sit 30–90; LieBack 40–120; LieFront 20–50;
  Gaze 10–25; Lounge 40–90; Nap 60–150; Watch 45–120; Read 40–90; Homework 45–90 (its slot's 120–240
  stays); LookOut in step 5.
- Order of moves if the band isn't met: settle-in odds, then lengths, then Walk base, then needs. Each
  tuning run uses the fed-afternoon census; the goldens are re-recorded once at the end of 3c, not per
  try.
- **What the levers hit:** `a_restless_osaka_mostly_moves` (keeps passing at base 9 per the map),
  `sleepiness_draws_her_to_lie_down` (sensitive: re-derive, don't loosen blindly),
  `at_home_her_furniture_beats_the_floor` (floor rest grows: re-state the property, e.g. furniture > 3×
  floor rest, with the reason), `her_mood_shows` (its doc's "moving is no measure" is retired: the band
  test now pins it), the made-piece property's `BOUND` (now derived: longest still act at Lazy + a chat
  gap + the watch + a walk, not the literal 90 s), and design.md's numbers (fit formula untouched; the
  rise times, Walk base, watch length and lengths move).

### D5. Bare-room stillness (step 4, after the art is approved)

**Floor homework.** A new `Activity::Homework` (Here::Idle, `Spot::Floor`), lying on her front with a
paper and pencil (new art), offered only where no desk (real or made) stands. It gets `HOMEWORK_FACTORS`
(×3 in the Homework slot, exams), serves Daydreams 0.3, Comfort 0.1. It ends with a sore moment ("my
back...") and sets `ached` for the visit.

**The makeshift desk.** `Furniture::Desk` joins `MAKES` (no new `Furniture` variant, so no ledger format
change): a low cube of crumpled text, 3–4 × 2 cells, explicit arms in `footprint`, `cell`, `heaps`, a seat
**beside** it (not in it), `roomy` allowing her spot, a "...my desk." loss line, `Scene::MakeDesk`, and a
floor-seated writing pose at it (new art, no stool). Chopsticks splice only at a real desk (its `when`
checks the seat). While `ached`, Use(Homework)'s `use/make` bind gets the D5 make factor again (×3, so ×9
in all): her back drives the desk as the brief asks.

**More makeshift while she owns no real piece.** In `choose_next`'s `factor` closure (osaka.rs:5669), an
offer whose bind is `Job::Build(_)` gets ×3 (above 1, so no lint change). It drops away by itself as real
furniture arrives: with a real piece of the kind, `places()` offers make only 1 in 20.

**Reading a pulled line (HG #72).** A method of Use(Read) where no bookshelf stands, `use/borrow`: she
walks to a line of text (counts as moving, one set-off), tears a strip (reel), sits on the floor reading it
(the `Read` pose with a strip `hold` part instead of the book), then slides it back (`unreel`). A cut
read mends at once (Loss::Tear). No persistence.

**TV from the floor.** Watch already binds with a TV alone and seats her beside it (map §brain 7). The host
pose away from a sofa becomes **cross-legged** facing the screen (new art) instead of hugging her knees.

**Cloud-watching.** By day, a LieBack settled into by a Dreamy Osaka, or from a daydream session, shows a
Curious face with a musing bubble instead of the doze's blink and Zzz (art check in line art; ASCII
differs only by bubble).

### D6. The window as a daydream (step 5)

- LookOut base 8 → **3**, its length **60–180 s** (×linger), and it becomes a session: the sky line, then
  a long sit or lean at the sill (new art) with up to three sky musings (a new small pool: clouds, birds,
  the moon at night), stars after dark as now.
- Settling in from it: sitting under the sill.
- `looking_out_is_a_daydream_and_some_fun` updates to the new row; the day census's look-out count should
  fall from about 19 to a few a game day, and the afternoon clock glance be re-checked. If it's still
  starved, the hour glance rolls once per daydream session (it already belongs to a musing's place).

## Art (model sheets first; drawn in parallel with steps 0–3)

An art agent in a worktree (committing with **git**, not jj) draws, at 1×, 1×-nn3× and 3× over the usual
background, a sheet of: floor homework (lying on her front writing, 2 frames, plus a face-down doze);
the crumpled-cube desk and homework at it (kneeling or cross-legged, writing, nodding off); reading a torn
strip on the floor; cross-legged before the TV; sitting or leaning at the window sill (and sitting under
it); cloud-watching LieBack face; nodding off while watching/reading on the floor (if it reads well). Saved
under `phase5c/art/` with `snippets.md` and `worktree.diff`. **The user approves the sheet before step 4
wires anything.** ASCII rows need no review but every new pose needs them (the sprite lints).

## Goldens and determinism

- Every lever moves every hash (112 plus the `osaka_at_home_seed_7` snapshot). Re-record once per commit
  that changes behaviour, with the reason in the commit message; the failing test prints the new table.
- **Narrow fixes are verified, not just re-recorded** (D0a, the WATCH change if done alone): run the
  golden tests with `HOUSEGUEST_GOLDEN_TRACE=<dir>` on the old revision (a `git worktree` in the
  scratchpad with its own `CARGO_TARGET_DIR`) and the new, diff, and check every moved trace first differs
  where the change predicts.
- New weights go through `pick_weighted`, which changes the draw even when it picks the same thing (map
  critic G8): expected.
- No `Date::now`-style or unseeded randomness; whims labels are new strings ("settle in", "resume").

## Tests (written first where they apply)

- D0a: the fades property (fails on HEAD). D0b: the credit-class test over `Want::ALL` (fails on HEAD).
- D2: the band tests (fail on HEAD, committed ignored until 3c).
- D3: a cut still act resumes after the watch (each still kind, both modes); not when she was moved, the
  seat went, the routine claimed her, or too little was left; a cut-and-resumed act's credit totals one
  act's; the watch is 5 s.
- D4: settle-in chains (SpaceOut → Sit → LieBack; Lounge → Nap) only on an uncut end, never past bedtime;
  mood linger scales lengths; nearer spots are preferred (a seeded distribution test on `walk` and
  `place`); pulls stay on her floor when there's text there.
- D5/D6: floor homework only without a desk; the desk is made, used and let go (the made-piece property
  covers Desk); a borrowed line is always slid back or mended; Watch's host pose is cross-legged away from
  a sofa; LookOut's new row and length.
- Every test loops `graphics in [false, true]` and fills panes with text where terrain matters.
- Reviewers prove a new test can fail by a mutant.

## Steps (each implemented, reviewed twice, then fixed, minors included)

0. This design and the map (docs commit). Art agent starts.
1. D0a rain-out (helper, property, verified trace re-record).
2. D0b credit class (SpaceOut, Work; class test; re-record).
3. D1 measuring; `phase5c/baseline.md`.
4. D2 band test (ignored), thresholds and caps from the baseline.
5. D3 watch 5 s and resuming.
6. D4 mechanisms: linger, settle in, daydream sessions, nearer spots.
7. D4 tuning to the band; un-ignore the band test; re-measure all censuses.
8. (after art approval) Art wired: poses, parts, ASCII rows.
9. D5 floor homework, makeshift desk, more makeshift, reading a pulled line, cross-legged TV,
   cloud-watching.
10. D6 the window as a daydream; glance re-check.
11. Census pass (256-case houseguest run in release, day census, fed afternoon, visit census both modes),
    docs: design.md rules, decisions.md reasons, plan.md record, CHANGELOG (she's calmer; resumes after
    chat; floor homework, the paper desk, reading a line, the window daydream).

## Docs that change

- design.md, Houseguest: the watch (5 s), resuming, settling in, mood lingering, the rise times and Walk
  base, nearer spots, floor homework and the paper desk, reading a pulled line, cross-legged TV, the
  window daydream, the band as a stated aim (share in sight per mood, with its set-off cap).
- decisions.md: why each (attention control; why mood acts through lengths, not a walking factor; why the
  band excludes hidden time; why resuming skips the roll).
- plan.md Phase 38: the 5c record.

## Round-1 amendments (2026-10-05)

Four critics ([mechanics](phase5c/critic-mechanics.md), [character](phase5c/critic-character.md),
[tests](phase5c/critic-tests.md), [feasibility](phase5c/critic-feasibility.md)) and their
[synthesis](phase5c/critique.md). **Every amendment in the synthesis is adopted** (B1, B3–B6, M1–M11,
minors 1–12) except where this section says otherwise; B2 (the resume fallback) is **not** adopted, since
the user chose B1. Where this section and the synthesis disagree with the sections above, they win, this
section first.

### The user's calls, round 1

- **Q1 → she looks up in place** (synthesis B1). A chat line during a still act on restful terrain doesn't
  `interrupt`: a look overlay on the running act turns her to the chat (where the pose has a facing), `!`
  then `?`, then a plain watching face until `watch_until`; the act's clock and credit run on. Dozes
  (LieBack, Nap, day Sleep) stir instead ("Mm?", as at night). Walking, pulling, chores, making, exercise
  and anything on text are still cut as today. **D3's `Resume` and its guards are gone**; `WATCH_MS` 15 →
  5 s stays, as its own verified commit. design.md's "she stops and looks" becomes "in a still act she
  looks up where she is".
- **Q2 → the TV shows the film** (new D7 below). The user: "could it use screenshots from the running mpv
  player, if any (changing infrequently)?"
- **Q3 → a slow blink** on held poses (Sit, Lounge, cross-legged, a long Gaze, reading, homework at rest):
  150 ms every 6–12 s, drawn from whims/the body stream (not wall time), landing in step 8's re-record.

### D7. The TV picture (new)

- **Static only as it switches on and between channels** (about 1 s), then a **held picture**: animated
  static for up to 3 minutes would be the most eye-catching thing in a TV home.
- **With a film loaded in mpv, the held picture is a screenshot of it.** The player already takes
  screenshots for commentary (`PlayerCommand::Screenshot(path)`, actors/player.rs:176, 634; requested from
  session.rs:2801; `Player::screenshot_to_file`, player/mod.rs:173). The UI asks for one when she switches
  the TV on, and at most once every 60 s while a watch runs; the file is decoded (the `image` crate,
  already a dependency), cropped to the glass's aspect and scaled to the glass at the cell size, and handed
  to the guest as the TV's picture (`IdleView` or a setter; not persisted). Stale after 10 minutes.
  - Line art only (kitty images): the screenshot replaces the glass's fill. ASCII keeps a held glyph
    pattern.
  - No film, mpv not running, the screenshot failing or late (> 2 s), or no player at all: the held
    picture is drawn art (colour bars, or a small set of programme cards from the art sheet).
  - The shopping channel and the sunrise keep their own pictures; surfing flips held pictures per channel
    (each a new screenshot only if the 60 s allows, otherwise drawn cards).
  - Each distinct screenshot is a new kitty image: at ≤ 1 a minute that's well inside the image cache
    (`image_census` re-run in step 13).
  - The houseguest stays pure: the request and the file I/O live in the shell/session side, the guest
    only receives `Option<Arc<RgbaImage>>` with a timestamp; tests use a fixed picture.
- **Test:** no act longer than 30 s flips cells faster than `USE_FRAME_MS` after its first 10 s (both
  modes), which also guards against future animation churn. The slow blink (step 8: her face
  alone, 150 ms every 6–12 s on a pose she holds) is exempt: count pose, prop and bubble
  changes, or cells outside her face, not the blink's two flips.

### Steps, renumbered

0. This design, the map and the critiques (docs commit). The art agent starts (worktree, git commits),
   its sheet: floor homework (writing ×2, face-down doze), the paper desk and homework at it (kneeling or
   cross-legged, no stool, nodding off), reading a torn strip, cross-legged before the TV, sitting or
   leaning at the sill and sitting under it, cloud-watching LieBack (held, eyes open), an optional sitting
   doze, the look-in-place heads for seated and lying poses (`!`/`?` while seated or lying), and 3–4
   drawn programme cards for the TV.
1. D0a: fades hoisted onto `Guest` (M1), the property and the deterministic case; verified re-record.
2. D0b: the credit class (B3, M2), `served` record, per-(want, method) test; re-record.
3. D1: measuring (M3), the fed-afternoon helper (B4), `phase5c/baseline.md` with σ over ~20 seed sets.
4. D2: the band test, ignored (B5), thresholds and caps from the baseline.
5. `WATCH_MS` 15 → 5 s alone (verified: traces first differ 5 s after a line); re-baseline (minor 6).
6. D3: looking up in place (B1), dozes stir; the tests that pinned the standing look restated.
7. D4 mechanisms, golden-neutral (M11): linger at call sites (M8), settle in (M7), daydream sessions as one
   act (B6), nearer spots for pull/swap/pick_build and place (M6).
8. D4 tuning (M5's order and stop rule; Travel 7 with Walk, M4), the slow blink; un-ignore the band test;
   one re-record; re-measure all censuses.
9. (after the user approves the sheet) Art wired: poses, parts, ASCII rows.
10. D5 (M10): floor homework, the paper desk, more makeshift, reading a borrowed line, cross-legged TV,
    cloud-watching.
11. D6: the window as a daydream; glance re-check; the band re-read on a windowed home.
12. D7: the TV picture (held pictures, then the mpv screenshot path).
13. Census pass and docs (as step 11 before).

## Round-2 amendments (2026-10-06): the art, and steps 1–4's hand-offs

### The art, approved by the user

The sheet, its wiring notes and the code diff are in [phase5c/art/](phase5c/art/) (`snippets.md` maps
each item to its `Pose`, `Rig` and hold); the poses and parts are in the tree, unwired. Two rounds of
the user's redraws:
- **Floor homework** (`FloorHomework`): lying on her front, the paper out in front of her face (her arms
  reach only under her chin, so she writes at its near edge); a doze with her head on her arms.
- **Reading on her back** (`LieRead`, new, the user's idea): holding the book close over her face; a
  doze with the book turned side-on over her eyes ("which is largely why one uses books this way").
  It serves floor reading, and floor homework with a book.
- **The paper desk** (`PaperDesk`): kneeling in seiza at a 4×2 block of crumpled text.
- **Reading a torn strip** (`ReadStrip`), **cross-legged at the TV** (`CrossLegged`), **the sitting doze**
  (`SitDoze`, "looks great": a second settle-in branch from Sit), **looking up in place** and stirring
  (no new heads; LieBack and Nap don't flip to face the chat), **the TV's four programme cards**.
- **The window hangs lower** so she can lean on the sill, chin in her hands (`SillLean`); it may be
  drawn **behind a sofa** (the user: fine for the sofa, unlike other furniture), but must still clear
  every other standing piece. `UnderSill` and `SitDoze` before it are the settle-in poses. Wiring (step
  11): the new hang, a sofa-only exception in `hung_pieces_clear_every_standing_piece` and the placement
  rules (one overlap rule for floor and wall), the window drawn before standing furniture, where she
  stands to lean. Existing homes' windows move with the new hang (check the ledger keeps them valid).
- **Cloud-watching only lying under a window** (or a solarium, which remains to do).

### Hand-offs from steps 1–4 (their commits carry the detail)

- **Step 1** made fades live on `Guest`, and `paint_state` returns `Rains` so each arm decides who
  paints them. Rule: a rain is cut short only when she's sent away (off, moved out) or her room goes;
  every other exit lets it fall. A goodbye overlapping a rain wakes about twice the paint rate. Still
  open: a busy client's `school_out` makes the empty home vanish with no rain (other class).
  **Resolved (5c tail T3, 2026-10-07):** an arrival carries her empty home until she's in, and one
  that comes to nothing rains it out (plan.md; decisions.md).
- **Step 2** credits SpaceOut and Work, and `cut_shift` settles a cut shift by the share worked (the
  errand, `place`/`dash_through`, school, her next decision on the way out). **Deferred to step 8:**
  needs should rise to the credit's moment before it lands (`rise_to` at the top of `serve`); landing
  it alone collapses late-visit dozing in `her_needs_shape_long_visits` (rates were tuned while credits
  were lost), so it lands with the tuning, un-ignoring `a_credit_eases_her_needs_as_they_are`.
  *(Landed alone in step 8's review, the tuning having stopped: the long-visit claim holds over
  twelve visits, four having been too few; see baseline.md, "Step 8: tuning stopped".)*
  SpaceOut stays not restful for Beauty ("done on her feet").
- **Step 3**'s baseline: [phase5c/baseline.md](phase5c/baseline.md). Every home and resident cell is
  over its ceiling (lazy 29–35, ordinary/dreamy 28–47, industrious 35–51); the line-art stage is near
  or inside the band but its set-offs (4–5 a minute) bind; industrious ÷ lazy is only 1.2–1.6; the
  home and resident numbers don't depend on drawing mode (no text in her reach there).
- **Step 4**'s band tests (`tests/band.rs`) are ignored until step 8. The gate runs 2 seeds: there, 3σ
  is 13–34 points, so the gate catches only gross excess and the set-off cap; **full strength**
  (`CENSUS_BAND_SEEDS`, `--profile band`, N up to 187, minutes) is the real check, run at step
  boundaries from step 5 on and in step 13. Step 5 re-baselines at the band's 9-minute length
  (`CENSUS_MINUTES=9 CENSUS_MODES=line fed_afternoon_census`, `BASELINE_MINUTES = 9`). Step 7's
  lengths go through `use_duration_in`, `Activity::duration` or `SPACE_OUT_MS` (or `longest_still_ms`
  learns what they use). Step 8 fills all 24 `TUNED` rows from one 9-minute line-art census. Step 11
  adds a TV-only home print and a home or resident with text at her floors' heights.

## Round-3 amendments (2026-10-06): the band rule, after step 8's stop

Step 8's tuning stopped ([baseline.md, "Step 8: tuning stopped"](phase5c/baseline.md)): with every
lever on, the shares fell well below the old floors on the stage and resident, while M3's per-room
set-off cap (frozen at the baseline's seconds moved per set-off) held the home at about 1 set-off a
minute. Steps 5–7 shipped (the 5 s watch, looking up in place, the levers landed neutral); step 8
shipped the slow blink and the credit-ordering fix only.

**The user's call: per-mood set-off caps.** A set-off draws the eye the same in any room, so the band
becomes, in every room and both chat conditions:

| Mood | Set-offs a minute in sight (cap) | Share ceiling | Share floor ("not dead") |
|---|---|---|---|
| Lazy | ≤ 1.5 | 17 | 5 |
| Ordinary, Dreamy | ≤ 2.25 | 26 | 8 |
| Industrious | ≤ 3.0 | 31 | 12 |

plus industrious ÷ lazy ≥ 1.6 per room (full strength only). This replaces D2's floors, B5's floors and
M3's per-room cap formula. The gate keeps its 2 seeds; full strength remains the real check (the
user: "I'll know to check them if something feels off").

Decided in steps 5–8 unless the user objects (each in decisions.md):
- A line she can't look at within its 5 s (climbing, through a door, a far calm spot, the andagi
  playing on) is let go, not watched late.
- LieFront keeps its facing at a look (turning flips her body end to end); a grievance plays out
  first, then the look.
- A silent daydream still lets the escalator and the hour glance through (5b's frequencies kept).
- Spacing out doesn't blink (a vacant stare); homework blinks on its writing frames.

Steps from here, renumbered (the art "wiring" step folds into the steps that use each pose):
8b. The retune to this rule: the census and band harnesses deliver chat lines at their own times
    (re-measure first), then tune from step 8's move 7; Watch stays out of lingering until D7's held
    picture (step 12); `BAND_MINUTES` 12 if lingering lands; fill `TUNED`, un-ignore the band.
10a. D5 part 1: floor homework (`FloorHomework`), reading on her back (`LieRead`: floor reading where
    no bookshelf, homework with a book), the paper desk (`PaperDesk`), more makeshift, `ached`,
    cross-legged TV (`CrossLegged`).
10b. D5 part 2: reading a borrowed line (`use/borrow`, `ReadStrip`).
11. D6, the window: the lower hang with the sofa exception, the sill lean (`SillLean`), `UnderSill`
    and `SitDoze` before it, cloud-watching under it only; glance re-check; the band re-read on a
    windowed home; a TV-only home and a text-filled home or resident printed.
12. D7, the TV picture, after its own two-lens critique.
13. Census pass and docs.

**Step 8b stopped** ([baseline.md, "Step 8b: the retune, stopped"](phase5c/baseline.md)): the
harnesses deliver chat lines at their own times (committed alone), the band has the per-mood rule
(still ignored; `BASELINE` re-measured at the new driver), and watching is out of lingering. Under a
per-mood cap a cell moves at most its cap × the seconds a set-off moves her; on the stage that's a
point or two over its floor in every mood. Within industrious the home moves 2.35–2.97× the stage
across the runs (the band's own edges: 31 ÷ 12 = 2.58), so holding the home under 31 leaves the
stage at or within about a point of its floor. Every lever that moved the stage moved the home with
it (the one stage-only lever tried, Tidy 90 s, moved the stage only within noise), so the next move
(a room-selective lever, a stage rule, or shipping the home and resident alone) is the user's.

## Round-4 amendments (2026-10-06): D7 after its critique, and steps 8b–11

### D7, amended

D7 had its own two-lens critique ([phase5c/d7/](phase5c/d7/): mechanics, character, synthesis; the
mock script is there too, while the true-size mock itself stays out of the repo because it was made
from real episodes). **The synthesis's amended D7 (in `d7/critique.md`, "The amended D7") replaces
Round-1's D7 in full**, with its blockers, majors and minors, except where the user's answers below
differ.

The user's answers:
- **Q1 → the film, no sheen.** The look is the mock's column 7: centre crop, 1.3× zoom, one levels
  stretch, saturation ×1.25. No glass sheen. Drawn cards when there's no film, the frame is black or
  flat, or this client doesn't hold the file.
- **Q2 → refresh about once a minute during a long watch** (this overrides the synthesis's "never
  swapped mid-watch"). While a Watch or Surf act runs, once 60 s have passed since the picture's
  request (failures count), the shell asks again. A good new frame cuts straight in at its next
  paint, with no static. A failed or black refresh keeps the picture. The purity rule still holds:
  her trace and ASCII frames are byte-identical with and without pictures, because only
  presentation changes. D7's "no act longer than 30 s flips cells faster than `USE_FRAME_MS` after
  its first 10 s" test exempts a once-a-minute picture swap, as it already exempts the blink.
- **Q3 → no reaction now.** Later: the user is considering replacing the marquee commentary with
  Osaka's own comments on the actual episode (plan.md's idea list).

Step 12 is two steps, as the synthesis says: **12a**, the drawn held pictures (static only at
switch-on and between channels, Chiyo-chichi's bob ending after the hook; Watch rejoins lingering
here; golden re-record); **12b**, the film feed (shell-side, private screenshot slot, a property
that her trace is unchanged).

### Steps 8b–11 as built, and the user's call on the stage

- **8b** shipped the per-mood rule (ignored), the census and band harnesses delivering chat lines at
  their own time, and Watch out of lingering. Its retune stopped: on the bare stage a set-off moves
  her only 2–3.5 s, so the per-mood caps hold the stage near its floors, and bringing the
  industrious home under 31 put the stage's industrious cell within a point of its floor. **The
  user's call: ship it, with lower floors on the bare stage** (lazy 4 / ordinary and dreamy 6 /
  industrious 9, for a room that keeps her things close). The home and resident keep 5 / 8 / 12.
  That is step 8c: switch the levers on with 8b's best run (c3, `scratchpad/8b-c3.diff`),
  re-measured with 10a–11 in.
- **10a** (floor homework, `LieRead`, the paper desk, more makeshift, cross-legged TV), **10b** (a
  borrowed line, with grip as one class for held text and pulls), and **11** (the window hung low,
  allowed behind a sofa, the sill lean and its session, cloud-watching under it only) are built,
  each with its CHANGELOG entry. Their records are in their commit messages and baseline.md.

## Round-5 amendments (2026-10-07): step 8c as built

- **The golden driver** cuts a step at each event's own time, as the census drivers do (one
  deliberate re-record, its own commit before the shipping one).
- **The levers ship** (`Stillness::TUNED`): 8b's c3 (step 8's move 7 values, watching unlingered,
  Tidy 90 s, industrious linger 0.85, spacing out 10–28 s) with settling on (`STARTING`'s odds), so
  sitting under the window, dozing sitting up and watching the clouds come alive; and **dreamy
  linger 0.85** (not in the brief: at 1.0 the resident's dreamy afternoon with chat fell under its
  floor once 10a–11 were in). Sitting in front of the window lasts as sitting does, watching the
  clouds and the doze under a book as lying back does. `BAND_MINUTES` 17 (the guard: the sill's
  chain, lazy, is 7 minutes).
- **The user's answers of 2026-10-06:** LookOut base 6; the afternoon hour glance has no roll (its
  other gates stay); a borrowed strip doesn't turn her at a chat line.
- **The bare stage's floors** (lazy 4 / ordinary and dreamy 6 / industrious 9) are `Band::of`'s
  for the stage room.
- **Shipped short, by the user's word:** the stage's industrious afternoon with chat (8.4% at full
  strength against 9) and the stage's spread (1.39 against 1.6), both ignored with their numbers
  (`tests/band.rs`, `SHORT`). Every other cell is in, at full strength (N = 525).
- **The facing-right lean** stands a column further in (her box centred on the window's first
  column), as the approved sheet has her: at a column outside, her hands met the frame.
- Records: [phase5c/baseline.md](phase5c/baseline.md), "Shipped (step 8c)", and decisions.md, "Her
  stillness ships".

## Round-6 amendments (2026-10-07): step 12a as built

- **The held picture** (D7's drawn half): `WATCH` is static for `STATIC_MS` (1.2 s), then the
  act's programme; `SURF`'s rest key is the programme; `SHOPPING` bobs through its hook only.
  A script key names what it shows as `script::Shows` (held as it is, static, the hook, or the
  act's programme): the synthesis's `Channel::Held` placeholder is a key-level variant instead,
  resolved by `Osaka::prop` to `Prop::Tv(Channel::Programme(card))`, so `Channel` (the image's
  cache key) only names what can be drawn and 12b's stable key is `Channel::Programme(card)`.
  The card is `Play::card`, drawn on every watch from the decision's whims
  (`Whims::below("programme", 4)`), which draws nothing from her generator.
- **Watching lingers** (`Use::lingers`); `BAND_MINUTES` stays 17 (the sill's chain is still her
  longest still act; a lazy watch is at most 123 s).
- **The stillness test** samples every 100 ms (a client paints for other reasons too) and leaves
  out acts that take her somewhere (a walk the screen's width is 31 s) and chat (quiet rooms). It
  runs every mood, in the TV rooms, the windowed home and a home with the shopping channel on, and
  compares her model's state, not drawn cells (12b's picture swap needs a drawn-cells variant).
- **Static wakes her** on its 400 ms frames (`Key::frame_ms`), so all three switch-on frames are
  painted in a client that paints only on her wakes; Chiyo-chichi's hook still bobs at paint time
  (review fix; decisions.md, "Her TV holds a picture").
- **Open, for the user:** the home's industrious quiet afternoon at 31.4 against 31, and a lazy
  shopping act's hook at up to 49 s (two fifths of a lingered body).
- Measured: [phase5c/baseline.md](phase5c/baseline.md), "Step 12a: the held TV picture".

## Round-7 amendments (2026-10-07): step 12b as built

- **The film on her TV** (D7's film half). `Guest::tv_wants_picture` (output only: in line art, a
  real TV, `Osaka::tv_bound`: heading or walking to a watch, or in a Watch or Surf act) drives the
  shell's `TvFeed` (`ui/tv_feed.rs`, pure): ask while she wants one and this client holds the
  now-playing file (`derive::held_now_playing`, shared with commentary's gate), then again once
  60 s have passed since the last question (failures count), one at a time, given up after 3 s; a
  change of film (or the file no longer held) clears her stills and asks afresh. The session answers
  `UiInput::TvPicture { ask, answer }` (latest wins) from the TV's own private slot
  (`screenshot::Slot`, `dessplay-tv-*`; commentary keeps its own, `dessplay-commentary-*`).
- **Review fixes (folded into the step).** *The real video:* `Ready` comes on prefetch and while
  the placeholder (same hash) still shows, so the player is asked only through
  `SessionShell::request_screenshot(path, file, view)`: `PlayerWiring::may_screenshot` (held, and
  the `loaded` real video, the gate that speaks for the group), then the player actor's
  `PlayerCommand::Screenshot { path, file, taken }`, taken only of the file last loaded once the
  player's path echo confirms it (`player_on_current_file`), answering at once. Commentary's request
  goes through the same gate. *Answers:* `TvAnswer::{Still, Failed, NotAsked}`; `NotAsked` (no
  player or slot, not held, not showing it yet, the slot busy) costs no minute and is asked again
  after 5 s (`NOT_ASKED_MS`); `Failed` keeps the minute. *Numbered questions:* `TvAsk { seq, file }`;
  only the answer to the question out counts. *One frame at a time:* `Slot::claim` (a `Claim`
  frees the slot on drop), so polls never race on the one path. *The shell's wiring* is
  `TvFeed::turn` / `TvFeed::deliver`, which the film tests drive too; a closed session queue ends
  the UI loop (it spun).
- **The two slots are a latch at paint**, Q2 having replaced "never swapped mid-act": `Graphics`
  holds the still on show and the one delivered since; each paint latches the delivered one, so a
  fresh still cuts in at the paint its arrival triggers. `Look::Film(id, programme)` carries the
  act's card, so a key whose still is gone draws the card (the synthesis's canvas test).
  `Guest::set_tv_picture(None)` is the file change (both slots go); a failed answer never calls it.
- **Treatment** (`film.rs`, off the UI thread): decode within the chat images' caps, centre crop at
  58:46 zoomed 1.3×, box-scaled to 128×102, then black (mean luma < 20) and flat (p98 − p2 < 24)
  rejected, one luma levels stretch (gain ≤ 3), saturation ×1.25; the pixel hash is the id. The
  stretch's statistics are taken on the 128×102 still, as the mock took them on its scaled one.
  At paint: box-scaled once to the glass's whole-pixel rectangle (27×22 at 9×19 cells, 29×23 at
  10×20), laid in through the rounded mask unmirrored, the outline over it.
- **mpv**, measured with `--vo=null`: a `video` grab answers in 73–128 ms at 3840×2160 and
  41–63 ms at 1080p HEVC, and a command queued behind it waited as long; with `"async": true` the
  queued command answers at once, so every `screenshot-to-file` (commentary's too) goes out async.
- **Tests through the real loop:** `loop_rig_with_player` (the auto-acking mock and a media root)
  runs `a_tv_still_is_asked_only_of_the_held_file_into_its_own_slot`: not asked before anything
  plays or of another file, asked into the `dessplay-tv-*` slot once held and shown, the frame read
  and deleted.
- **Saturation** (closed in Round 8): the Q1 answer, the synthesis and the code said ×1.25; the
  mock's column 7 (`lifted()` in `d7/mock.py`), which the user picked by eye, applies ×1.3. The
  user chose the column's ×1.3 (step 12c).
- Records: decisions.md, "The film on her TV"; baseline.md, "Step 12b".

## Round-8 amendments (2026-10-07): the user's answers after 8c, 12a and 12b (step 12c)

- **The drivers paint her at every input** (its own commit, before the answers): the golden driver at
  every event (a chat line, text arriving, a key press, a focus change) and every cue, the census
  drivers and the film tests' at every chat line, as the client draws after every input. One golden
  re-record; every chat cell re-measured.
- **Industrious watching isn't shortened** (the user: "Don't shorten industrious watching"): a watch
  (watching TV, flicking through the channels, the shopping channel) lingers as the mood's linger
  but where `Stillness::watch` overrides it, which ships ×1 industrious and nothing else (so a lever
  that changes the linger changes watching too); every other mood and every other still act as
  before. The home's industrious quiet afternoon is back to 30.6% at full strength
  (31.4 at step 12a, ceiling 31).
- **The film's saturation is ×1.3**, the mock's column 7 that the user picked by eye (Round 7's open
  point, closed).
- **Looking out's base is 4.** The user asked for about two long window daydreams a game day in a
  furnished home with a window, and chose the outcome, not the number: the day census read 36
  look-outs in its 42 game days at base 3, 82 at 4 (1.95 a day), 140 at 5 and 175 at 6, with the
  levers on. The day census now holds the outcome (1.5 to 2.5 a game day in a room with her window),
  not only the pinned base. *(Since T1, 2026-10-07: base 5, 3.3 a day, mid-way in the 2 to 6 the
  user accepts, and the day census holds 2 to 6.)*
- **Left as they are, by the user's word:**
  - The shopping channel's hook bobs through its whole span, up to about 49 s on a lazy watch. The
    user: "Continuous cyclical movement gets filtered out by the human optical system almost as
    quickly as static scenes." Steady periodic motion is not what draws the eye; a change is
    (design.md's stillness rule says so; decisions.md, "Her TV holds a picture").
  - A resident staying in the chat pane is not weighted: only going in is a tenth.
- **The TV's `NotAsked` retry after 5 s stands** (the orchestrator's call, unless the user objects).
- **The stillness test, drawn** (12a's hand-off): `no_long_act_flips_drawn_cells_faster_than_a_frame`
  paints every 100 ms, with chat and the film fed, in the TV-only home, the shopping home, the home
  with her window and the resident, and compares drawn cells and images; its exemptions are
  design.md's four (five since T1, 2026-10-07: the world's clock), each only its own change with
  nothing else drawn alongside (a chat line's stir, dozing, counts as the look up at it: it comes
  when the line does; since T2 it lasts a frame, checked).
- **Open, for the user** (each found by the stillness tests; each fix would change how she looks, a
  golden re-record; until then the test allows exactly the case, provisionally, or keeps it ignored):
  - *A bob's key ending off its frame grid.* A key that bobs (her writing, her breathing asleep)
    ending at a share of the use (her homework nodding off) always comes less than a frame after the
    bob's last flip. The test allows exactly that: the change that ends a bobbing key, within a
    frame of that same key's last flip; any other change near a bob's flip counts (design.md says
    "for now"). The fix in what she does: hold a bob's last frame through the partial period before
    its key ends (and its first flip until a frame after its key starts).
    **Resolved (T2, 2026-10-07):** the user: fix it. A bob keeps a frame clear of its key's start
    and end (`script::bob_frame`), on its part's grid as before, for every bobbing key and the
    idle bobs on the frame (lying back, reading on her back, reading a borrowed strip); the
    allowance is gone from both stillness tests. Short bobbing keys lose flips: the lamp-on moment
    and the andagi's chewing are still poses now; the Dream's last two lines hold (open in
    plan.md, with in-place retiming of an act's end).
  - *A stir is shorter than a frame by day.* A chat line stirring her dozing by day ("Mm?") lasts
    `speech_ms("Mm?")` = 1380 ms, under the 1400 ms frame, so it comes and goes inside one (a
    night's "mm..." lasts 1500 ms). The test exempts a stir as the dozing form of a look up. The
    fix: `stir_until = now + speech_ms(line).max(USE_FRAME_MS)`.
    **Resolved (T2, 2026-10-07):** the user: fix it. Whatever she says lasts a frame at least
    (`speech_ms` floors at `USE_FRAME_MS`; only "Mm?" was under one), so the stir's turn and its
    murmur end together a frame on; the stir keeps the look up's exemption (it comes when the
    line does), and the drawn test checks it holds a frame. Her look up's `!` (1.2 s) is under a
    frame too, left (inside the look up's exemption; plan.md).
  - *The world's clock.* In the home with her wall clock, the dial (each game quarter-hour, about
    150 s at 6×) stepped 656 ms after her breathing's flip asleep ("home, clock and window Lazy
    seed 0", both modes); the window's sky would do the same at dawn and dusk. It's no act of
    hers, and steady and slow (this round's principle may cover it), so whether the rule holds it
    is the user's call: that room's run is
    `no_long_act_flips_drawn_cells_faster_than_a_frame_by_her_clock`, ignored with those numbers.
    **Resolved (T1, 2026-10-07):** the user exempts it (not her doing, slow and steady, the
    periodic-motion principle); the exemption is scoped to the dial's and the sky's own change
    (only at a paint where her clock's reading changed; her model, key and other looks the same,
    the changed cells in the footprint of the clock or window that stepped and none in her box),
    and the run, with one across the sky's day to dusk, is in the gate.
  - *The golden driver's order within a moment.* It tells her of a key press and cues the stage
    before it advances her, where the shell advances her first; it paints at no ~10 Hz snapshot
    redraw during playback. **Resolved (T1, 2026-10-07):** the driver advances her first, as the
    shell does (`the_golden_driver_advances_her_before_input_like_the_shell`); no golden moved.
    The snapshot redraws stay unmodelled.
  - *The drawn exemptions don't combine* (new in T1's review). A dial or sky step on the same
    100 ms paint as another exempt change (a blink, a look up at the chat) is exempt by neither arm
    and fails as a flip; in ASCII the sky's cells inside her box (her leaning at the sill) count as
    hers. Neither comes up today; the door batch moving pieces could. Open (plan.md).
  - *Her lines, plain ASCII only* (12d). **Resolved (T3, 2026-10-07):** the user allows a few
    narrow marks ("Realistically none of us use a CJK locale; wide-character support is useful
    mostly for rare subtitles and filenames"). The allowlist is `NARROW`, … ♪ — – ’ ‘ “ ” ·, one
    cell each under the renderer's non-CJK width, and `line!` refuses any other at compile time.
  - *A busy client's `school_out`* (step 1's hand-off; the user left it to the orchestrator).
    **Resolved (T3, 2026-10-07):** her empty home rains out whenever she doesn't come in after all.
- Measured: [phase5c/baseline.md](phase5c/baseline.md), "Step 12c".

## Implemented (2026-10-07)

**Phase 5c is implemented** (steps 1–13, 2026-10-05 to 2026-10-07). Its record (what each step built, the
shipped numbers and band, the deviations, the user's decisions and what is left open) is in
[docs/plan.md](../../plan.md), Phase 38, "Phase 5c — stillness (done 2026-10-07)"; the closing numbers are
[phase5c/baseline.md](phase5c/baseline.md), "Census pass (step 13)". The door batch comes next
(plan.md, "Next: the door batch").
