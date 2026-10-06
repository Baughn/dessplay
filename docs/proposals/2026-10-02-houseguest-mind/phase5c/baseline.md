# Phase 5c baseline: how much she moves today

**Measured 2026-10-05/06**, step 3 (D1) of [phase5c-design.md](../phase5c-design.md), on the commit
that adds this file (behaviour as at `f29c482`, after D0a and D0b). Release builds, all runs
deterministic: the fed census run twice gave identical tables. Paths are relative to
`dessplay/src/ui/houseguest/`.

**The band's baseline is now step 8b's c0** ([Step 8b: the retune, stopped](#step-8b-the-retune-stopped-2026-10-06):
step 7's state with step 8's fixes, each chat line at its own time, 9-minute visits). Before it,
[After the shorter watch](#after-the-shorter-watch-step-5-9-minute-visits) (step 5). The older
tables are the 15 s watch (the fed afternoon's over 15 minutes).

**Step 10a measured the stage after "more makeshift"** (the lever 8b named), levers still neutral:
[Step 10a: more makeshift, measured](#step-10a-more-makeshift-measured-2026-10-06). No cell left its
band, but the stage regressed on two of the user's criteria: its industrious ÷ lazy spread fell
from 1.50 to 1.17 (the rule is ≥ 1.6), and its lazy share rose about 3 points, further over its
ceiling of 17.

**Step 8's tuning stopped short of the band:** [Step 8: tuning stopped](#step-8-tuning-stopped-2026-10-06)
has every move tried, and why M3's cap and the levers can't both be met in every room. **So did
step 8b's**, to the user's per-mood band: [Step 8b](#step-8b-the-retune-stopped-2026-10-06) has the
runs, and why the stage's short trips hold it at its floor under a per-mood cap.

Re-measured after the step's review, which tightened the definitions below. Her arrival is now a
set-off (the warm-up's rate rises by about 0.2 a minute). A heave is only the beat she steps back (the
stage's share falls by about 0.2 points). Lines that came while she was in a door or aloft now count as
cuts (the stage's restarts from moving rise from 0.22–0.29 to 0.41–0.43 a minute). Shares and set-offs
after the warm-up are otherwise unchanged.

## What is measured

All of it is one class of each act, `census_moves` in `osaka.rs` (wildcard-free, so a new act
doesn't compile until it's classed): *off* (what she sets off on: Walk, Out, Climb, Clamber, Door),
*on* (moving that carries on from setting off: Peer, Fall, Dazed, Away), *heave* (a pull's step
back with the line), or *still*. The motion, the set-offs and the chat cuts all read it.

- **In sight:** visiting, not asleep for the night, not out of sight (`Osaka::hidden`: Away off the
  screen, a door's hidden beats). `Osaka::census_motion` returns `None` both when she's still and
  when she's out of sight, so the census leaves out `hidden` before counting still time.
- **Moving** (`Osaka::census_motion`): bodies *walk* (Walk, Out), *climb* (Climb, Clamber), *fall*
  (Peer, Fall, Dazed), *door* (its seen beats), *text* (a pull's heave: the beat she has just
  stepped back with the line, `heaving && offset > gap`; the bracing beat after it is still). Swap,
  Sneeze and PutBack move text, not her: still, printed apart. Exercise and LieFront are still,
  printed apart.
- **Purpose** (`Osaka::census_purpose`), first that holds: a heading's or a `Walk { then: Job }`'s job
  kind (*to text*: Pull, Swap, Build; *to seat*: Use; *to home*: Lift, SetDown); *work*; *routine*;
  *dash*; *errand*; otherwise the chain tag: the last decision's method, or what set her going
  without one (*arrival*, *accident* for the floor gone under her, *errand* for the door out after
  an errand). The tag reads *wander* (walk/along), *travel* (travel/link, travel/door), *off text*,
  or the raw method. Two departures from D1 and B4:
  - The climb, the daze after a hop's drop and **the walk back in from off the screen keep the
    hop's tag**. So a trip around the screen's edge reads *travel* to its end, not *return*.
    Her shift, routine and leaving still win by priority.
  - A walk to a job that a re-anchor (`Osaka::settle`) turned into a plain walk reads as its chain.
- **Set-off** (`Osaka::count_set_off`, bumped in `set`, pinned by
  `the_census_counts_setting_off_not_moving_on`): the start of a walk, climb or door that either comes
  from a body not on a trip (standing, looking at the chat, pulling: anything *still* or *heave*) or
  is the first such start of a decision. So a second start in one decision counts only from a still
  body. Moving on within a trip without a decision (walk to a pole then the climb, off the edge and
  back in, a door's far side) and falling are not set-offs. **Her arrival's act is one** (a walk in,
  or a door for an errand, her routine home or a dash home, each for that purpose), though no `set`
  starts it. In the fed census it falls in the warm-up. Real paths are pinned by
  `a_hop_is_one_set_off` and `her_arrival_is_a_set_off_for_what_she_came_for`.
- **Set-offs a minute in sight** count every set-off she makes, wherever (her log), over her minutes
  in sight. That's the same in the fed, unfed and day censuses. Only what the very step that ends a
  visit logs is lost (the census reads her logs after every step).
- **Chat cuts and restarts** (`Osaka::chat_cuts`, pinned by
  `the_census_counts_her_restarts_after_a_chat_line`): a line that stops her logs what it cut. That's
  what she was moving for if she was on a trip, read before the line lets go of where she was
  heading, or *still*. A line that comes while she's out of sight, in a door or aloft is owed. It stops
  her when she next decides and watches, logged as a look at that moment (for what she was on when it
  came). If bed or school comes first, or the watch has run out by then, it lapses and isn't logged.
  A **restart** is the first set-off after a cut with nothing but the look and her watch between. An
  owed glance in between drops it, so restarts are a slight undercount.
- **Warm-up:** the first 3 minutes (`WARM_MS`) are printed apart. Every share and rate below is from
  minute 3 on.
- **σ:** the standard deviation of the pooled 4-seed mean over disjoint seed sets. The fed census has
  20 sets × 4 seeds = 80 visits a cell. The unfed census has only 4 sets × 4 (16 visits), so its σ is
  rough. σ should scale as about 1/√N.
- **Setups.**
  - *Fed afternoon* (`fed_afternoon`, the band's own setup, B4): Tuesday 13:00, 15 real minutes
    (90 game minutes, all Afternoon slot); `clock_sent`; no shopping; the stage's TV held back on
    order; pieces placed by an unfed arrival in the same drawing mode; mood forced after the arrival
    paint. B4's "props unchanged" is checked as **the same pieces, none boxed, `ordered` unchanged**:
    where they stand may change, because the census home starts broken on purpose and putting it
    right is hers to do. The literal check failed on the home (seeds 0, 1 and 3, Ordinary, ASCII).
  - *Unfed* (`visit_census`, 5a-comparable): 30 minutes, 16 seeds, shopping on.
  - Chat is either quiet or at the room's census cadence (stage 45 s, home 90 s, resident 60 s).
- **Gate checks** on the census itself: `the_fed_afternoon_tallies_add_up` runs a 4-minute fed
  afternoon on the stage and in the home in both modes. It checks that her time adds up, that the
  warm-up splits at minute 3, that the set-offs and chat cuts in the table are her logs', and that
  out of sight is counted. Each mutant the review named fails it or a unit test.

Commands:

```text
cargo test --release -p dessplay --lib fed_afternoon_census -- --ignored --nocapture
CENSUS_SETS=1 cargo test --release -p dessplay --lib fed_afternoon_census -- --ignored --nocapture   # the cost table
CENSUS_MOODS=1 CENSUS_MODES=both CENSUS_CHAT=both \
  cargo test --release -p dessplay --lib visit_census -- --ignored --nocapture
cargo test --release -p dessplay --lib day_census -- --ignored --nocapture
```

The forced mood held. Every fed run (3840 of them) asserts at every step that she's visiting the same
visit (decisions only grow) in the forced mood. At the end it asserts that her pieces are the same,
none boxed, and nothing is on order beyond the held-back TV. All passed. The unfed `arrive_drawn` now
`expect`s Visiting before forcing the mood, and never failed.

## Finding: the home and resident census rooms don't depend on drawing mode

In `furnished_room` and `resident_room`, ASCII and line art give identical runs, seed for seed, both
fed and unfed. The cause was traced, and it's that **neither room has text she can reach.** The home has no
text. The resident's ten scattered lines sit at rows 3–21, columns 2–41, while its floors are rows
12–13 (columns 52–97) and 26. At arrival, in both modes, both rooms have the same platforms, no
pulls, swaps, builds or loose glyphs, and no standing spot that's unrestful. So line art has nothing
to change there. The resident's "to text 0.0", "off text 0.00" and swap "–" rows below are this, not
a finding about her. (`resident_room`'s doc said "text in reach", which was wrong; it now says so.)
The stage is the only census room with reachable text. There it has 168 pulls and 184 swaps in ASCII
against 26 and 36 in line art, and 88 unrestful standing spots in line art against none in ASCII.

Line art is **11–17 points stiller** on the stage, fed, and 5–11 unfed. That's mostly because she
walks to text about half as much: 9–15 points against 21–27 in ASCII, and set-offs to text
1.6–2.2 a minute against 2.5–4.1. The fed setup asserts that the guest has a picker exactly when
line art is asked for.

Step 2 consequences:
- the band's home and resident cells in line art are the ASCII cells;
- **no census room tests line-art terrain over text except the stage.** The resident can't see
  text-locality levers (M5), so the line-art band needs a text-filled home or resident, its text at
  her floors' heights (memory note: "fill panes with text where terrain matters").

Below, home and resident rows are given once for both modes.

## Fed afternoon (the band's setup)

Moving % in sight, σ of a 4-seed mean (20 sets), the least and most a single visit had, and set-offs
a minute in sight. "Cap at target" is the step-2 formula, today's rate × (target ÷ today's share) ×
1.15, with targets Lazy 15, Ordinary/Dreamy 22.5, Industrious 30. Its slack is still to be checked
against σ.

| Room | Mood | Chat | Mode | Moving % | σ (N=4) | a visit | Set-offs/min | σ | Warm-up % | Warm-up set-offs/min | Cap at target |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | ascii | 43.2 | 8.32 | 14.9–71.1 | 6.31 | 0.93 | 65.5 | 4.87 | 3.78 |
| stage | Ordinary | quiet | line art | **29.3** | 4.82 | 5.2–58.8 | **4.81** | 0.58 | 58.7 | 5.97 | 4.25 |
| stage | Ordinary | chat/45s | ascii | 39.5 | 4.52 | 15.9–57.7 | 4.54 | 0.57 | 55.7 | 4.42 | 2.97 |
| stage | Ordinary | chat/45s | line art | **27.0** | 4.96 | 11.2–57.1 | **4.21** | 0.29 | 52.3 | 5.29 | 4.03 |
| stage | Lazy | quiet | ascii | 37.3 | 8.59 | 10.5–67.5 | 4.57 | 0.60 | 61.3 | 4.77 | 2.11 |
| stage | Lazy | quiet | line art | **20.9** | 4.83 | 3.7–61.8 | **4.09** | 0.39 | 51.1 | 5.48 | 3.38 |
| stage | Lazy | chat/45s | ascii | 35.2 | 6.74 | 12.0–60.6 | 4.52 | 0.38 | 54.7 | 4.60 | 2.22 |
| stage | Lazy | chat/45s | line art | **19.9** | 3.98 | 6.0–50.7 | **3.99** | 0.39 | 49.3 | 5.24 | 3.46 |
| stage | Industrious | quiet | ascii | 44.8 | 6.93 | 26.6–75.8 | 6.58 | 1.22 | 68.2 | 4.89 | 5.07 |
| stage | Industrious | quiet | line art | **33.3** | 5.61 | 15.9–58.7 | **4.97** | 0.52 | 62.1 | 6.14 | 5.15 |
| stage | Industrious | chat/45s | ascii | 40.6 | 4.66 | 19.2–58.9 | 4.81 | 0.52 | 54.5 | 4.62 | 4.09 |
| stage | Industrious | chat/45s | line art | **29.2** | 5.70 | 13.2–52.8 | **4.45** | 0.37 | 54.0 | 5.28 | 5.26 |
| stage | Dreamy | quiet | ascii | 42.4 | 7.30 | 12.3–74.3 | 5.71 | 1.06 | 64.4 | 4.82 | 3.48 |
| stage | Dreamy | quiet | line art | **27.0** | 4.27 | 8.9–56.6 | **4.76** | 0.47 | 55.4 | 5.71 | 4.56 |
| stage | Dreamy | chat/45s | ascii | 38.4 | 5.33 | 11.3–60.9 | 4.64 | 0.59 | 53.4 | 4.54 | 3.13 |
| stage | Dreamy | chat/45s | line art | **21.4** | 3.51 | 8.4–43.1 | **3.99** | 0.28 | 50.0 | 5.25 | 4.82 |
| home | Ordinary | quiet | both | **46.8** | 3.99 | 31.6–62.8 | **2.12** | 0.13 | 57.8 | 2.40 | 1.17 |
| home | Ordinary | chat/90s | both | **42.2** | 2.89 | 21.9–56.3 | **2.27** | 0.11 | 53.8 | 2.45 | 1.39 |
| home | Lazy | quiet | both | **34.8** | 3.24 | 22.3–52.4 | **1.54** | 0.13 | 50.9 | 2.12 | 0.76 |
| home | Lazy | chat/90s | both | **34.9** | 2.82 | 20.8–47.0 | **1.79** | 0.10 | 51.5 | 2.23 | 0.88 |
| home | Industrious | quiet | both | **51.4** | 4.87 | 27.9–65.6 | **2.41** | 0.12 | 59.3 | 2.56 | 1.62 |
| home | Industrious | chat/90s | both | **47.6** | 2.68 | 37.0–60.4 | **2.56** | 0.17 | 55.5 | 2.54 | 1.86 |
| home | Dreamy | quiet | both | **44.2** | 3.70 | 27.9–66.1 | **2.04** | 0.09 | 55.9 | 2.41 | 1.19 |
| home | Dreamy | chat/90s | both | **39.5** | 2.16 | 27.8–53.4 | **2.20** | 0.09 | 52.3 | 2.33 | 1.44 |
| resident | Ordinary | quiet | both | **37.1** | 3.76 | 23.1–52.5 | **2.63** | 0.12 | 47.1 | 3.24 | 1.83 |
| resident | Ordinary | chat/60s | both | **30.0** | 2.55 | 19.5–46.2 | **2.62** | 0.12 | 42.9 | 3.14 | 2.26 |
| resident | Lazy | quiet | both | **33.1** | 3.60 | 18.6–49.8 | **2.29** | 0.11 | 41.6 | 2.69 | 1.19 |
| resident | Lazy | chat/60s | both | **28.7** | 3.95 | 14.7–52.3 | **2.39** | 0.15 | 39.2 | 2.67 | 1.44 |
| resident | Industrious | quiet | both | **41.9** | 2.87 | 27.0–56.7 | **2.78** | 0.09 | 47.9 | 3.40 | 2.29 |
| resident | Industrious | chat/60s | both | **34.7** | 2.96 | 18.0–47.1 | **2.67** | 0.20 | 45.1 | 3.50 | 2.65 |
| resident | Dreamy | quiet | both | **34.7** | 2.64 | 20.3–50.3 | **2.54** | 0.14 | 45.8 | 3.05 | 1.89 |
| resident | Dreamy | chat/60s | both | **28.0** | 3.82 | 16.8–44.7 | **2.50** | 0.10 | 41.6 | 3.10 | 2.31 |

Read for step 2:
- **Every home and resident cell is over its ceiling,** quiet and chatty: lazy is 29–35 against 17,
  ordinary and dreamy 28–47 against 26, industrious 35–51 against 31.
- **The line-art stage is already near or inside the band** (lazy 20–21, ordinary 27–29, dreamy
  21–27, industrious 29–33).
- **Its set-off rate is the binding constraint there:** 4.0–5.0 a minute, against caps of 3.4–5.3.
  Lazy is 4.0–4.1 against 3.4–3.5.
- **Mood barely spreads today.** Industrious ÷ lazy is 1.2–1.6 in the stage and 1.2–1.5 in the home
  and resident, against the ≥ 1.6 starting value of B5.
- **Chat lowers the share** in most cells, by 0–7 points (more in the resident). The watch time it
  adds is still time in sight.
- **σ of a 4-seed mean is 2.2–5.7 points in line art.** That's too wide for 3σ thresholds at N = 4.
  - At N = 16 it would be about 1.1–2.9 (by 1/√N), so 3σ is about 3–9 points.
  - The stage is the noisiest. A single stage visit spans 4–62%.
- **The warm-up is far busier** than the steady state (39–68% moving, 2.1–6.1 set-offs a minute with her arrival's): B5's warm-up cut matters.

### By purpose and body (fed, line art; % of time in sight)

| Room | Mood | Chat | to seat | to text | to home | travel | wander | off text | pull (text) | other | walk | climb | door | fall |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | 5.1 | 12.6 | 0.0 | 6.9 | 3.9 | 0.7 | 0.2 | 0.0 | 17.4 | 6.5 | 5.1 | 0.2 |
| stage | Ordinary | chat/45s | 3.6 | 13.8 | 0.0 | 5.2 | 3.1 | 0.7 | 0.2 | 0.3 | 16.8 | 5.6 | 4.0 | 0.3 |
| stage | Lazy | quiet | 5.2 | 9.1 | 0.0 | 3.9 | 2.1 | 0.5 | 0.1 | 0.0 | 11.6 | 4.1 | 5.0 | 0.1 |
| stage | Lazy | chat/45s | 4.5 | 9.7 | 0.0 | 3.0 | 1.8 | 0.4 | 0.2 | 0.3 | 11.5 | 3.9 | 4.2 | 0.1 |
| stage | Industrious | quiet | 3.2 | 14.5 | 0.0 | 9.6 | 5.4 | 0.6 | 0.1 | 0.0 | 20.1 | 7.8 | 4.9 | 0.3 |
| stage | Industrious | chat/45s | 2.2 | 15.0 | 0.0 | 6.9 | 3.7 | 0.8 | 0.3 | 0.2 | 18.0 | 6.7 | 3.9 | 0.3 |
| stage | Dreamy | quiet | 5.3 | 11.2 | 0.0 | 6.2 | 3.4 | 0.8 | 0.2 | 0.0 | 15.4 | 5.5 | 5.8 | 0.1 |
| stage | Dreamy | chat/45s | 3.2 | 10.2 | 0.0 | 4.5 | 2.4 | 0.5 | 0.3 | 0.2 | 12.4 | 4.4 | 4.1 | 0.2 |
| home | Ordinary | quiet | 26.6 | 0.0 | 2.0 | 10.6 | 7.7 | 0.0 | 0.0 | 0.0 | 44.2 | 2.3 | 0.0 | 0.2 |
| home | Ordinary | chat/90s | 24.1 | 0.0 | 1.9 | 8.8 | 7.4 | 0.0 | 0.0 | 0.0 | 40.1 | 1.9 | 0.0 | 0.2 |
| home | Lazy | quiet | 24.7 | 0.0 | 0.0 | 5.7 | 4.4 | 0.0 | 0.0 | 0.0 | 33.4 | 1.3 | 0.0 | 0.1 |
| home | Lazy | chat/90s | 25.2 | 0.0 | 0.0 | 5.1 | 4.5 | 0.0 | 0.0 | 0.0 | 33.7 | 1.1 | 0.0 | 0.1 |
| home | Industrious | quiet | 24.5 | 0.0 | 5.4 | 11.9 | 9.6 | 0.0 | 0.0 | 0.0 | 48.4 | 2.7 | 0.0 | 0.3 |
| home | Industrious | chat/90s | 21.8 | 0.0 | 5.4 | 11.3 | 9.1 | 0.0 | 0.0 | 0.1 | 45.2 | 2.1 | 0.0 | 0.3 |
| home | Dreamy | quiet | 25.5 | 0.0 | 2.2 | 9.2 | 7.2 | 0.0 | 0.0 | 0.0 | 41.9 | 2.1 | 0.0 | 0.2 |
| home | Dreamy | chat/90s | 23.3 | 0.0 | 2.0 | 7.4 | 6.7 | 0.0 | 0.0 | 0.1 | 37.7 | 1.6 | 0.0 | 0.2 |
| resident | Ordinary | quiet | 18.4 | 0.0 | 1.5 | 11.3 | 5.9 | 0.0 | 0.0 | 0.0 | 31.3 | 5.5 | 0.0 | 0.3 |
| resident | Ordinary | chat/60s | 14.6 | 0.0 | 1.7 | 8.8 | 4.9 | 0.0 | 0.0 | 0.1 | 25.0 | 4.7 | 0.0 | 0.2 |
| resident | Lazy | quiet | 22.2 | 0.0 | 0.0 | 7.1 | 3.8 | 0.0 | 0.0 | 0.0 | 26.7 | 6.2 | 0.0 | 0.2 |
| resident | Lazy | chat/60s | 20.2 | 0.0 | 0.0 | 5.2 | 3.3 | 0.0 | 0.0 | 0.0 | 22.9 | 5.6 | 0.0 | 0.1 |
| resident | Industrious | quiet | 19.1 | 0.0 | 1.2 | 14.8 | 6.8 | 0.0 | 0.0 | 0.0 | 35.0 | 6.5 | 0.0 | 0.3 |
| resident | Industrious | chat/60s | 15.6 | 0.0 | 1.1 | 11.5 | 6.4 | 0.0 | 0.0 | 0.1 | 29.2 | 5.2 | 0.0 | 0.3 |
| resident | Dreamy | quiet | 17.4 | 0.0 | 1.7 | 10.6 | 5.0 | 0.0 | 0.0 | 0.0 | 28.7 | 5.7 | 0.0 | 0.3 |
| resident | Dreamy | chat/60s | 14.0 | 0.0 | 1.9 | 7.6 | 4.4 | 0.0 | 0.0 | 0.1 | 23.7 | 4.1 | 0.0 | 0.2 |

"Other" is the raw chain tags (heading/hop, heading/mine, use/made, accident), each ≤ 0.3. "Travel"
includes the climb at the end of a hop, and the door and climb columns are mostly travel and
off-floor jobs.

- **In the home and resident, walking to a seat is roughly half to two thirds of all moving**
  (14–27 points; 45–71% of it). The biggest levers there are fewer, longer uses and nearer seats, as
  the map predicted.
- **On the stage, walking to text is the biggest single part** (9–15 points). Most of it is walks to
  swap letters (Mischief), then walks to pull.

**To jobs, by want** (chat cadence, line art; % in sight). The Build want shows as "to text/Use(…)",
a walk to text torn for a made piece:

- stage: Swap 5.1–9.5, Pull 3.3–5.4, Lounge 1.9–2.9, Nap 0.3–1.3, to text for a made sofa or bed
  ≤ 0.6.
- home: Lounge 4.8–6.9, Watch 3.6–5.5, Read 3.6–5.1, Snack 3.1–4.8, Arrange 0–5.4 (industrious
  highest), Sleep 1.1–3.9, Homework 0.5–2.7, Nap 0.1–2.3.
- resident: Watch 6.8–9.7, Lounge 5.1–7.9, Nap 0.8–4.1, Arrange 0–1.9.

### Set-offs a minute in sight, by purpose (fed, line art), and chat

| Room | Mood | Chat | to seat | to text | to home | wander | travel | off text | doors (any purpose) | lines that stopped her still, /min | lines that cut a move, /min | set-offs right after one, from still | from moving |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | 1.05 | 2.07 | 0.00 | 0.84 | 0.70 | 0.15 | 1.05 | – | – | – | – |
| stage | Ordinary | chat/45s | 0.87 | 1.96 | 0.00 | 0.67 | 0.54 | 0.17 | 0.83 | 0.80 | 0.61 | 0.55 | 0.43 |
| stage | Lazy | quiet | 1.40 | 1.57 | 0.00 | 0.53 | 0.46 | 0.13 | 1.04 | – | – | – | – |
| stage | Lazy | chat/45s | 1.41 | 1.66 | 0.00 | 0.47 | 0.33 | 0.11 | 0.86 | 0.91 | 0.49 | 0.76 | 0.41 |
| stage | Industrious | quiet | 0.63 | 2.21 | 0.00 | 1.07 | 0.91 | 0.15 | 1.02 | – | – | – | – |
| stage | Industrious | chat/45s | 0.53 | 2.20 | 0.00 | 0.83 | 0.70 | 0.18 | 0.82 | 0.76 | 0.67 | 0.48 | 0.43 |
| stage | Dreamy | quiet | 1.19 | 2.02 | 0.00 | 0.76 | 0.62 | 0.17 | 1.20 | – | – | – | – |
| stage | Dreamy | chat/45s | 0.98 | 1.78 | 0.00 | 0.61 | 0.49 | 0.14 | 0.85 | 0.85 | 0.56 | 0.55 | 0.42 |
| home | Ordinary | quiet | 1.25 | 0.00 | 0.17 | 0.40 | 0.30 | 0.00 | 0.00 | – | – | – | – |
| home | Ordinary | chat/90s | 1.36 | 0.00 | 0.18 | 0.42 | 0.31 | 0.00 | 0.00 | 0.36 | 0.37 | 0.25 | 0.27 |
| home | Lazy | quiet | 1.15 | 0.00 | 0.00 | 0.23 | 0.16 | 0.00 | 0.00 | – | – | – | – |
| home | Lazy | chat/90s | 1.36 | 0.00 | 0.00 | 0.26 | 0.17 | 0.00 | 0.00 | 0.45 | 0.27 | 0.39 | 0.22 |
| home | Industrious | quiet | 1.20 | 0.00 | 0.40 | 0.47 | 0.34 | 0.00 | 0.00 | – | – | – | – |
| home | Industrious | chat/90s | 1.28 | 0.00 | 0.43 | 0.49 | 0.37 | 0.00 | 0.00 | 0.29 | 0.44 | 0.20 | 0.26 |
| home | Dreamy | quiet | 1.23 | 0.00 | 0.17 | 0.36 | 0.27 | 0.00 | 0.00 | – | – | – | – |
| home | Dreamy | chat/90s | 1.38 | 0.00 | 0.19 | 0.38 | 0.25 | 0.00 | 0.00 | 0.35 | 0.38 | 0.24 | 0.27 |
| resident | Ordinary | quiet | 1.48 | 0.00 | 0.13 | 0.59 | 0.43 | 0.00 | 0.00 | – | – | – | – |
| resident | Ordinary | chat/60s | 1.47 | 0.00 | 0.17 | 0.56 | 0.42 | 0.00 | 0.00 | 0.61 | 0.46 | 0.39 | 0.28 |
| resident | Lazy | quiet | 1.64 | 0.00 | 0.00 | 0.38 | 0.27 | 0.00 | 0.00 | – | – | – | – |
| resident | Lazy | chat/60s | 1.78 | 0.00 | 0.00 | 0.36 | 0.25 | 0.00 | 0.00 | 0.67 | 0.38 | 0.55 | 0.30 |
| resident | Industrious | quiet | 1.44 | 0.00 | 0.09 | 0.70 | 0.55 | 0.00 | 0.00 | – | – | – | – |
| resident | Industrious | chat/60s | 1.38 | 0.00 | 0.09 | 0.69 | 0.51 | 0.00 | 0.00 | 0.60 | 0.48 | 0.34 | 0.24 |
| resident | Dreamy | quiet | 1.46 | 0.00 | 0.15 | 0.52 | 0.41 | 0.00 | 0.00 | – | – | – | – |
| resident | Dreamy | chat/60s | 1.46 | 0.00 | 0.17 | 0.50 | 0.37 | 0.00 | 0.00 | 0.62 | 0.45 | 0.40 | 0.29 |

**Restarts after a watch** (minor 7):
- Of the chat lines that stop her while she's still, 57–87% are followed straight away by a
  set-off (on the stage, 0.48–0.76 a minute).
- Lines that cut a move come 0.27–0.67 times a minute. After one, she sets off again 0.22–0.43 times a
  minute (the stage 0.41–0.43). That counts the lines that came while she was in a door, aloft or off
  the screen and stopped her once she was back: they're most of the stage's rise from the first
  baseline's 0.22–0.29.
- Together these are 18–36% of all set-offs at the census cadences. B1 (looking up
  in place) and the 5 s watch go after the "from still" column.
- "Passing" cuts (chat over text, no watch) are included in both columns. They're ≤ 0.06 a minute.

**Doors** happen only on the stage: 0.8–1.2 a minute, a fifth to a quarter of her set-offs there.

### Still but busy, and bubbles (fed, line art; minor 8)

| Room | Mood | Chat | exercise % | exercise begun /min | lie front % | swap % | sneeze % | bubbles /min |
|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | 6.7 | 0.63 | 4.4 | 1.9 | 1.0 | 4.68 |
| stage | Ordinary | chat/45s | 5.2 | 0.52 | 2.4 | 1.0 | 0.3 | 5.20 |
| stage | Lazy | quiet | 1.8 | 0.17 | 2.7 | 1.5 | 0.6 | 3.71 |
| stage | Lazy | chat/45s | 1.0 | 0.11 | 1.5 | 1.0 | 0.2 | 5.08 |
| stage | Industrious | quiet | 11.3 | 1.05 | 7.3 | 2.2 | 1.4 | 5.26 |
| stage | Industrious | chat/45s | 8.4 | 0.86 | 2.1 | 1.2 | 0.4 | 5.46 |
| stage | Dreamy | quiet | 3.7 | 0.36 | 3.2 | 1.8 | 0.8 | 4.66 |
| stage | Dreamy | chat/45s | 3.1 | 0.31 | 2.7 | 1.2 | 0.3 | 5.57 |
| home | Ordinary | quiet | 4.7 | 0.45 | – | – | – | 1.34 |
| home | Ordinary | chat/90s | 4.0 | 0.40 | – | – | – | 2.48 |
| home | Lazy | quiet | 1.2 | 0.12 | – | – | – | 1.07 |
| home | Lazy | chat/90s | 0.9 | 0.09 | – | – | – | 2.37 |
| home | Industrious | quiet | 6.6 | 0.60 | – | – | – | 1.55 |
| home | Industrious | chat/90s | 6.2 | 0.58 | – | – | – | 2.54 |
| home | Dreamy | quiet | 3.5 | 0.33 | 0.1 | – | – | 1.61 |
| home | Dreamy | chat/90s | 2.7 | 0.27 | 0.0 | – | – | 2.79 |
| resident | Ordinary | quiet | 6.9 | 0.63 | 0.2 | – | 0.1 | 1.14 |
| resident | Ordinary | chat/60s | 5.9 | 0.59 | 0.1 | – | 0.1 | 2.86 |
| resident | Lazy | quiet | 1.9 | 0.19 | 0.5 | – | 0.2 | 0.91 |
| resident | Lazy | chat/60s | 1.3 | 0.14 | 0.2 | – | 0.1 | 2.61 |
| resident | Industrious | quiet | 9.5 | 0.85 | 0.2 | – | 0.1 | 1.10 |
| resident | Industrious | chat/60s | 8.6 | 0.82 | 0.0 | – | 0.0 | 2.84 |
| resident | Dreamy | quiet | 4.1 | 0.39 | 1.4 | – | 0.1 | 1.41 |
| resident | Dreamy | chat/60s | 3.4 | 0.35 | 1.0 | – | 0.1 | 3.12 |

- PutBack is ≤ 0.05% everywhere.
- Bubbles count each new bubble over her in sight (a speech line, `!`, `?`, Zzz, a heart...),
  sampled at the census's ≤ 1 s steps.
- A chat line adds about 1.0–1.7 onsets a minute in the home and resident (its `!` and `?`). The
  stage's own mischief lines keep it at 3.7–5.6 either way.
- Out of sight is 1.6–5.5% of a fed visit. Asleep is 0.

**Exercise is minor 8's watch item.** If tuning raises exercise more than 3 points over this table
(per cell), the numbers go back to the user before pinning.

## Unfed (`visit_census`, 30 min, 16 seeds, 5a-comparable)

| Room | Mood | Chat | Mode | Moving % | σ (4×4) | a visit | Set-offs/min | σ | Warm-up % | Warm-up set-offs/min |
|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | ascii | 31.3 | 5.91 | 12.4–60.8 | 5.70 | 1.09 | 63.9 | 5.03 |
| stage | Ordinary | quiet | line art | 20.2 | 2.07 | 6.8–38.5 | 3.76 | 0.33 | 46.1 | 5.48 |
| stage | Ordinary | chat/45s | ascii | 30.0 | 4.11 | 20.0–40.5 | 5.55 | 1.04 | 53.4 | 4.44 |
| stage | Ordinary | chat/45s | line art | 22.5 | 2.40 | 13.7–33.4 | 4.05 | 0.31 | 49.4 | 5.12 |
| stage | Lazy | quiet | ascii | 25.4 | 4.96 | 9.4–64.2 | 4.13 | 0.32 | 58.6 | 4.68 |
| stage | Lazy | quiet | line art | 15.9 | 2.32 | 5.7–38.1 | 3.03 | 0.44 | 42.4 | 4.81 |
| stage | Lazy | chat/45s | ascii | 26.5 | 8.72 | 12.0–46.4 | 4.72 | 0.79 | 53.4 | 4.54 |
| stage | Lazy | chat/45s | line art | 17.9 | 3.50 | 9.2–49.9 | 4.21 | 0.66 | 43.4 | 4.96 |
| stage | Industrious | quiet | ascii | 38.4 | 3.99 | 22.2–77.5 | 7.92 | 1.65 | 68.5 | 5.29 |
| stage | Industrious | quiet | line art | 30.1 | 6.84 | 9.1–51.0 | 3.82 | 0.14 | 57.4 | 5.97 |
| stage | Industrious | chat/45s | ascii | 32.4 | 3.92 | 24.0–42.5 | 5.78 | 0.62 | 55.2 | 4.30 |
| stage | Industrious | chat/45s | line art | 27.3 | 3.44 | 19.0–39.6 | 4.16 | 0.90 | 49.2 | 5.03 |
| stage | Dreamy | quiet | ascii | 30.8 | 4.72 | 14.0–57.7 | 5.76 | 1.69 | 62.4 | 5.20 |
| stage | Dreamy | quiet | line art | 21.3 | 4.35 | 6.2–32.6 | 4.03 | 1.18 | 50.8 | 5.40 |
| stage | Dreamy | chat/45s | ascii | 28.7 | 5.15 | 13.5–49.3 | 5.15 | 0.76 | 54.2 | 4.16 |
| stage | Dreamy | chat/45s | line art | 19.0 | 2.52 | 11.3–33.0 | 3.74 | 0.32 | 44.0 | 5.00 |
| home | Ordinary | quiet | both | 33.5 | 1.27 | 25.7–40.6 | 1.47 | 0.05 | 50.3 | 2.20 |
| home | Ordinary | chat/90s | both | 37.8 | 1.43 | 29.4–48.2 | 1.84 | 0.05 | 48.3 | 2.30 |
| home | Lazy | quiet | both | 29.8 | 2.95 | 20.4–38.0 | 1.35 | 0.03 | 40.0 | 1.87 |
| home | Lazy | chat/90s | both | 33.8 | 2.70 | 24.7–43.3 | 1.72 | 0.06 | 46.1 | 2.07 |
| home | Industrious | quiet | both | 42.1 | 2.12 | 30.9–52.3 | 1.89 | 0.13 | 58.3 | 2.44 |
| home | Industrious | chat/90s | both | 40.9 | 1.13 | 32.2–45.9 | 2.17 | 0.09 | 52.6 | 2.45 |
| home | Dreamy | quiet | both | 35.7 | 1.34 | 28.9–44.7 | 1.52 | 0.10 | 42.1 | 1.95 |
| home | Dreamy | chat/90s | both | 35.1 | 2.21 | 26.5–40.6 | 1.82 | 0.05 | 46.6 | 2.07 |
| resident | Ordinary | quiet | both | 35.2 | 3.19 | 23.1–46.9 | 2.33 | 0.06 | 44.6 | 2.98 |
| resident | Ordinary | chat/60s | both | 30.4 | 1.05 | 24.7–36.5 | 2.37 | 0.08 | 43.5 | 3.01 |
| resident | Lazy | quiet | both | 33.2 | 2.00 | 21.0–44.2 | 2.16 | 0.09 | 40.6 | 2.62 |
| resident | Lazy | chat/60s | both | 31.2 | 2.43 | 22.0–40.4 | 2.29 | 0.02 | 37.0 | 2.67 |
| resident | Industrious | quiet | both | 39.5 | 1.77 | 28.7–53.1 | 2.52 | 0.15 | 53.2 | 3.23 |
| resident | Industrious | chat/60s | both | 35.0 | 1.63 | 25.4–44.1 | 2.54 | 0.08 | 46.2 | 3.28 |
| resident | Dreamy | quiet | both | 32.1 | 4.59 | 23.4–46.5 | 2.17 | 0.11 | 47.0 | 3.01 |
| resident | Dreamy | chat/60s | both | 28.5 | 2.60 | 22.0–36.6 | 2.34 | 0.05 | 40.9 | 3.00 |

Fed and unfed compared:
- **Unfed is 1–13 points stiller in the home,** likely because Sleepy fills in 15 minutes unfed
  (map G5: more Sleep uses). The resident is about the same either way (−2.5 to +2.6).
- **On the stage, unfed is 2–9 points stiller in line art and 7–12 in ASCII.**
- **Unfed home and resident set-offs carry a constant 0.04 a minute of "work"** (her shift's
  set-off). Fed, her job is closed on a school day.
- **Unfed purposes look like fed ones.** In the home and resident, to seat is 14–25 points and
  travel 4–13. On the stage, to text is 7–25 points, more in ASCII.
- The bare-room (stage) numbers for step 9 come from here, not a fed week (map G4).

## Day census (fed week, ASCII, by her day's mood)

The band's quantity in a fed week: here, awake and in sight, not on a dash home. Three seeds × two
dates per room.

| Room | Mood | minutes | Moving % | Set-offs/min | Biggest purposes (%) |
|---|---|---|---|---|---|
| stage | Ordinary | 2427 | 37.2 | 4.04 | to seat 15.0, to text 13.2, travel 5.4, wander 3.2 |
| stage | Lazy | 1328 | 34.4 | 3.38 | to seat 15.0, to text 13.4, travel 3.4, wander 2.4 |
| stage | Industrious | 520 | 30.9 | 4.64 | to seat 10.9, to text 9.9, travel 5.7, wander 4.3 |
| stage | Dreamy | 465 | 29.7 | 5.35 | to text 13.2, to seat 9.2, travel 4.0, wander 3.1 |
| home | Ordinary | 2529 | 40.6 | 1.93 | to seat 26.8, travel 7.7, wander 5.7 |
| home | Lazy | 1372 | 36.4 | 1.75 | to seat 27.2, travel 4.9, wander 4.1 |
| home | Industrious | 539 | 44.3 | 2.07 | to seat 26.8, travel 9.8, wander 7.5 |
| home | Dreamy | 501 | 36.7 | 1.90 | to seat 24.6, travel 6.4, wander 5.1 |
| resident | Ordinary | 2528 | 37.6 | 2.38 | to seat 25.4, travel 7.7, wander 4.2 |
| resident | Lazy | 1372 | 37.8 | 2.25 | to seat 30.5, travel 4.2, wander 3.0 |
| resident | Industrious | 544 | 39.0 | 2.45 | to seat 25.5, travel 8.4, wander 5.0 |
| resident | Dreamy | 500 | 30.7 | 2.30 | to seat 20.8, travel 5.9, wander 3.6 |

- **The day census is thin per mood.** Lazy, industrious and dreamy each have one or two game days a
  seed. Its industrious and dreamy rows don't separate from ordinary, and don't follow the fed
  afternoon's ordering. It's the hand-checked acceptance of minor 10, not a band reading.
- **The "stage" here gains a TV and shops during its week:** "to seat" is 15 points there, against
  3–5 on the fed afternoon's bare stage.
- **The slot lines now print moving by purpose.** The Asleep and Away slot lines (a dash, her way to
  bed or out) are excluded from any band reading.
- The week totals for moving (stage 35, home 39, resident 37) match 5b's 35/40/37.

## Cost

Release, `CENSUS_SETS=1 … fed_afternoon_census` (4 visits a cell, so 4 threads at once, no
contention), each visit's wall time including its unfed placing arrival. Cost per sim-minute:

| Room | ASCII | Line art |
|---|---|---|
| stage | 27–38 ms | 36–44 ms |
| home | 13–18 ms | 27–38 ms |
| resident | 12–14 ms | 20–24 ms |

- The default run puts a visit on each of this machine's 32 hardware threads, and the cost per visit
  roughly doubles (line-art stage 71–86 ms). Its cost column is not this table. `CENSUS_THREADS`
  caps the threads.
- Whole runs: the fed census (3840 visits × 15 min) takes about 2 minutes, the unfed census with
  all knobs about 1.5 minutes, the day census about 1.75 minutes.
- **Gate budget at these costs.** A line-art band cell of N = 8 × 15 min (8 × 12 in sight after the
  warm-up) costs about 5 s (stage), 4 s (home) and 2.5 s (resident) of CPU, per chat condition.
- The gate runs the dev profile (opt-level 2), which is in the same range.

## After the shorter watch (step 5, 9-minute visits)

**Measured 2026-10-06**, on step 5's commit (`WATCH_MS` 15 → 5 s, `90a98239`), at the band's own
length: 9-minute visits (6 after the warm-up), line art, 20 sets × 4 seeds = 80 visits a cell, release.
These rows are `tests/band.rs`'s `BASELINE` (moving %, σ, set-offs, σ), with `BASELINE_MINUTES = 9`,
so the band's √(time) factor is 1. To part the watch's effect from the shorter visits, the same census
was also run with `WATCH_MS` put back to 15 s (the "15 s watch" columns and the brackets).

```text
CENSUS_MINUTES=9 CENSUS_MODES=line \
  cargo test --release -p dessplay --lib fed_afternoon_census -- --ignored --nocapture
```

"Cuts/min" are the chat lines that stopped her (looks and passes), a minute in sight; "restarts" the
set-offs right after one (from still and from moving together).

| Room | Mood | Chat | Moving % | σ (N=4) | a visit | Set-offs/min | σ | Warm-up % | Warm-up set-offs/min | 15 s watch: moving % | set-offs/min | Exercise % (15 s) | Cuts/min | Restarts/min (15 s) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | **32.5** | 6.98 | 4.8–72.8 | **4.96** | 0.67 | 58.7 | 5.97 | 32.5 | 4.96 | 6.7 (6.7) | – | – |
| stage | Ordinary | chat/45s | **30.7** | 7.26 | 10.0–73.2 | **5.32** | 0.57 | 56.4 | 6.37 | 30.0 | 4.50 | 5.6 (4.7) | 1.41 | 0.97 (1.01) |
| stage | Lazy | quiet | **21.8** | 6.55 | 2.3–72.1 | **4.19** | 0.30 | 51.1 | 5.48 | 21.8 | 4.19 | 1.8 (1.8) | – | – |
| stage | Lazy | chat/45s | **24.6** | 6.89 | 5.3–76.9 | **4.74** | 0.41 | 52.4 | 6.09 | 23.7 | 4.19 | 1.2 (1.0) | 1.41 | 1.13 (1.17) |
| stage | Industrious | quiet | **34.9** | 6.93 | 12.8–64.1 | **5.11** | 0.51 | 62.1 | 6.14 | 34.9 | 5.11 | 11.2 (11.2) | – | – |
| stage | Industrious | chat/45s | **35.3** | 5.60 | 17.4–77.0 | **5.66** | 0.56 | 60.5 | 6.34 | 32.4 | 4.66 | 10.2 (7.4) | 1.39 | 0.83 (0.97) |
| stage | Dreamy | quiet | **27.8** | 5.25 | 9.6–75.2 | **4.86** | 0.43 | 55.4 | 5.71 | 27.8 | 4.86 | 3.6 (3.6) | – | – |
| stage | Dreamy | chat/45s | **29.5** | 6.27 | 9.9–65.3 | **5.24** | 0.66 | 58.5 | 6.26 | 25.4 | 4.25 | 2.9 (2.8) | 1.41 | 0.92 (1.02) |
| home | Ordinary | quiet | **48.4** | 5.16 | 20.3–76.0 | **2.33** | 0.20 | 57.8 | 2.40 | 48.4 | 2.33 | 4.7 (4.7) | – | – |
| home | Ordinary | chat/90s | **48.6** | 5.68 | 20.2–75.0 | **2.67** | 0.20 | 56.2 | 2.53 | 42.6 | 2.46 | 4.4 (3.9) | 0.75 | 0.49 (0.53) |
| home | Lazy | quiet | **35.8** | 4.65 | 15.8–57.6 | **1.51** | 0.18 | 50.9 | 2.12 | 35.8 | 1.51 | 1.3 (1.3) | – | – |
| home | Lazy | chat/90s | **39.0** | 3.66 | 20.4–56.5 | **2.06** | 0.16 | 52.6 | 2.34 | 35.3 | 1.80 | 1.0 (1.0) | 0.78 | 0.58 (0.61) |
| home | Industrious | quiet | **53.4** | 6.26 | 22.5–73.2 | **2.72** | 0.26 | 59.3 | 2.56 | 53.4 | 2.72 | 6.6 (6.6) | – | – |
| home | Industrious | chat/90s | **54.0** | 4.42 | 28.3–76.9 | **3.08** | 0.30 | 58.4 | 2.76 | 47.9 | 2.84 | 6.8 (6.0) | 0.77 | 0.48 (0.47) |
| home | Dreamy | quiet | **46.1** | 4.11 | 26.6–71.2 | **2.27** | 0.17 | 55.9 | 2.41 | 46.1 | 2.27 | 3.6 (3.6) | – | – |
| home | Dreamy | chat/90s | **47.7** | 4.35 | 28.3–65.5 | **2.61** | 0.21 | 54.8 | 2.49 | 40.3 | 2.40 | 2.6 (2.6) | 0.77 | 0.53 (0.50) |
| resident | Ordinary | quiet | **38.8** | 4.97 | 17.9–58.3 | **2.79** | 0.20 | 47.1 | 3.24 | 38.8 | 2.79 | 6.8 (6.8) | – | – |
| resident | Ordinary | chat/60s | **37.8** | 5.89 | 10.0–61.2 | **3.14** | 0.24 | 47.8 | 3.53 | 31.0 | 2.79 | 6.8 (5.8) | 1.10 | 0.69 (0.68) |
| resident | Lazy | quiet | **32.3** | 3.71 | 12.8–51.0 | **2.31** | 0.12 | 41.6 | 2.69 | 32.3 | 2.31 | 2.1 (2.1) | – | – |
| resident | Lazy | chat/60s | **35.0** | 5.48 | 13.0–58.3 | **2.78** | 0.17 | 43.6 | 2.98 | 28.4 | 2.36 | 1.3 (1.6) | 1.04 | 0.80 (0.83) |
| resident | Industrious | quiet | **43.9** | 5.43 | 23.4–65.9 | **2.92** | 0.16 | 47.9 | 3.40 | 43.9 | 2.92 | 9.1 (9.1) | – | – |
| resident | Industrious | chat/60s | **41.2** | 3.51 | 23.7–59.7 | **3.20** | 0.21 | 49.9 | 3.70 | 34.1 | 2.72 | 9.1 (8.4) | 1.10 | 0.64 (0.56) |
| resident | Dreamy | quiet | **36.6** | 4.65 | 17.6–56.4 | **2.70** | 0.22 | 45.8 | 3.05 | 36.6 | 2.70 | 4.1 (4.1) | – | – |
| resident | Dreamy | chat/60s | **35.1** | 4.00 | 16.3–58.3 | **3.01** | 0.20 | 45.3 | 3.39 | 29.2 | 2.61 | 3.6 (3.5) | 1.10 | 0.66 (0.68) |

What moved:
- **Quiet cells are identical** to the 15 s watch's, number for number, as they must be: no line
  comes there. Every change below is the chat cells'.
- **Chat cells move more**: +0.7 to +4.1 points on the stage, +3.7 to +7.4 in the home, +5.9 to +7.1
  in the resident. Set-offs rise by 0.55–1.00 a minute on the stage, 0.21–0.26 in the home and
  0.35–0.48 in the resident. That's feas m1's prediction: the 10 s a line no longer holds her standing
  go to fresh rolls, often a walk. Until step 6 (she looks up in place) lets a still act run on under
  a look, chat adds movement in the home and resident (home, lazy: 2.06 set-offs a minute against
  1.51 quiet): expected, not a regression.
- **Chat no longer lowers the share.** Chat minus quiet was −10 to +2 points with the 15 s watch
  (most of the home and resident 4–10 below quiet); now it's −2.7 to +3.2. The restarts barely move
  (0.48–1.13 a minute, against 0.47–1.17): each line still cuts what it cuts. What changed is the
  stand that followed.
- **Exercise** in place rises a little with chat (the stage's industrious 7.4 → 10.2%; 8.4% in the
  15-minute baseline above): at most +1.8 over that table, inside minor 8's 3 points.
- **The shorter visits (15 → 9 minutes, the same watch) are busier**, the quiet cells say: the stage
  +0.8 to +3.2 points, the home +1.0 to +2.0, the resident −0.8 to +2.0 (minutes 3–9 against 3–15). σ
  of a 4-visit mean is 3.5–7.3 points, against 2.2–5.7 over 15 minutes, about the √2 that half the time
  in sight predicts (6 minutes against 12).
- **Every home and resident cell is still over its ceiling** (lazy 32–39 against 17, ordinary and
  dreamy 35–49 against 26, industrious 41–54 against 31). The line-art stage is now over its band in
  every cell, by less (lazy 22–25 against 17, ordinary and dreamy 28–33 against 26, industrious 35
  against 31). Industrious ÷ lazy is 1.4–1.6 on the stage, 1.4–1.5 in the home, 1.2–1.4 in the
  resident.
- **The band tests on these rows** (still ignored): at the gate's 2 seeds, 10 of 15 fail (every home
  and resident cell, and the stage's lazy and ordinary on the set-off cap); at full strength
  (`CENSUS_BAND_SEEDS=190`, release, `--profile band`, 5.6 minutes wall) all 15 fail, every cell over
  its band and its cap, and each room's spread under 1.6 (stage 1.46, home 1.47, resident 1.23). The
  most visits a band line asks for is now 190 (the stage's lazy with chat; 187 before), and the
  stage's spread 356.

## After looking up in place (step 6)

**Measured 2026-10-06**, on step 6 with its review fixes (change `pzwwwpsnxyzv`: in a still act she
looks up where she is; a doze stirs, and sets no watch; Setsubun's beans and a dash home are cut as
before; at her desk she keeps facing it; her look waits for a grievance; what she's saying shows over
the look), the same census as step 5's (9-minute visits, line art, 20 sets × 4 seeds, release).
"Before the fixes" is the same census on step 6 as first built (before its review), kept for
comparison. `tests/band.rs`'s `BASELINE` stays step 5's rows; these are for comparison. "Cuts/min"
are the lines logged as cuts (an in-place look is one that cut nothing, "still"; a stir isn't logged,
as at night, so part of the drop is stirs no longer counted); "of them still" the looks and passes
that cut no trip, in place or standing.

```text
CENSUS_MINUTES=9 CENSUS_MODES=line \
  cargo test --release -p dessplay --lib fed_afternoon_census -- --ignored --nocapture
```

| Room | Mood | Chat | Moving % | σ (N=4) | a visit | Set-offs/min | σ | Before the fixes: moving % | set-offs/min | Step 5: moving % | set-offs/min | Exercise % (step 5) | Cuts/min (step 5) | of them still | Restarts/min (step 5) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | **32.5** | 6.98 | 4.8–72.8 | **4.96** | 0.67 | 32.5 | 4.96 | 32.5 | 4.96 | 6.7 (6.7) | – | – | – |
| stage | Ordinary | chat/45s | **28.7** | 7.28 | 4.4–68.8 | **4.92** | 0.55 | 28.9 | 4.92 | 30.7 | 5.32 | 6.4 (5.6) | 1.23 (1.41) | 0.74 | 0.64 (0.97) |
| stage | Lazy | quiet | **21.8** | 6.55 | 2.3–72.1 | **4.19** | 0.30 | 21.8 | 4.19 | 21.8 | 4.19 | 1.8 (1.8) | – | – | – |
| stage | Lazy | chat/45s | **23.7** | 7.95 | 4.6–72.5 | **4.19** | 0.48 | 23.7 | 4.16 | 24.6 | 4.74 | 1.3 (1.2) | 0.97 (1.41) | 0.52 | 0.53 (1.13) |
| stage | Industrious | quiet | **34.9** | 6.93 | 12.8–64.1 | **5.11** | 0.51 | 34.9 | 5.11 | 34.9 | 5.11 | 11.2 (11.2) | – | – | – |
| stage | Industrious | chat/45s | **35.7** | 7.39 | 15.4–77.0 | **5.59** | 0.71 | 35.6 | 5.57 | 35.3 | 5.66 | 10.0 (10.2) | 1.33 (1.39) | 0.73 | 0.68 (0.83) |
| stage | Dreamy | quiet | **27.8** | 5.25 | 9.6–75.2 | **4.86** | 0.43 | 27.8 | 4.86 | 27.8 | 4.86 | 3.6 (3.6) | – | – | – |
| stage | Dreamy | chat/45s | **28.2** | 7.95 | 5.7–66.1 | **4.92** | 0.55 | 28.5 | 4.94 | 29.5 | 5.24 | 3.1 (2.9) | 1.23 (1.41) | 0.73 | 0.56 (0.92) |
| home | Ordinary | quiet | **48.4** | 5.16 | 20.3–76.0 | **2.33** | 0.20 | 48.4 | 2.33 | 48.4 | 2.33 | 4.7 (4.7) | – | – | – |
| home | Ordinary | chat/90s | **44.9** | 6.20 | 16.3–75.0 | **2.47** | 0.27 | 44.2 | 2.46 | 48.6 | 2.67 | 4.6 (4.4) | 0.66 (0.75) | 0.27 | 0.30 (0.49) |
| home | Lazy | quiet | **35.8** | 4.65 | 15.8–57.6 | **1.51** | 0.18 | 35.8 | 1.51 | 35.8 | 1.51 | 1.3 (1.3) | – | – | – |
| home | Lazy | chat/90s | **34.0** | 4.73 | 11.6–59.4 | **1.68** | 0.21 | 33.8 | 1.68 | 39.0 | 2.06 | 1.1 (1.0) | 0.52 (0.78) | 0.22 | 0.26 (0.58) |
| home | Industrious | quiet | **53.4** | 6.26 | 22.5–73.2 | **2.72** | 0.26 | 53.4 | 2.72 | 53.4 | 2.72 | 6.6 (6.6) | – | – | – |
| home | Industrious | chat/90s | **51.7** | 4.27 | 24.2–75.9 | **2.86** | 0.31 | 51.8 | 2.85 | 54.0 | 3.08 | 6.4 (6.8) | 0.73 (0.77) | 0.26 | 0.34 (0.48) |
| home | Dreamy | quiet | **46.1** | 4.11 | 26.6–71.2 | **2.27** | 0.17 | 46.1 | 2.27 | 46.1 | 2.27 | 3.6 (3.6) | – | – | – |
| home | Dreamy | chat/90s | **42.4** | 5.17 | 22.0–63.1 | **2.38** | 0.16 | 42.3 | 2.37 | 47.7 | 2.61 | 2.9 (2.6) | 0.65 (0.77) | 0.25 | 0.32 (0.53) |
| resident | Ordinary | quiet | **38.8** | 4.97 | 17.9–58.3 | **2.79** | 0.20 | 38.8 | 2.79 | 38.8 | 2.79 | 6.8 (6.8) | – | – | – |
| resident | Ordinary | chat/60s | **35.6** | 3.32 | 14.9–59.5 | **2.99** | 0.17 | 35.7 | 3.00 | 37.8 | 3.14 | 6.3 (6.8) | 1.02 (1.10) | 0.54 | 0.39 (0.69) |
| resident | Lazy | quiet | **32.3** | 3.71 | 12.8–51.0 | **2.31** | 0.12 | 32.3 | 2.31 | 32.3 | 2.31 | 2.1 (2.1) | – | – | – |
| resident | Lazy | chat/60s | **30.6** | 4.10 | 12.3–53.5 | **2.61** | 0.15 | 30.5 | 2.59 | 35.0 | 2.78 | 1.6 (1.3) | 0.90 (1.04) | 0.46 | 0.35 (0.80) |
| resident | Industrious | quiet | **43.9** | 5.43 | 23.4–65.9 | **2.92** | 0.16 | 43.9 | 2.92 | 43.9 | 2.92 | 9.1 (9.1) | – | – | – |
| resident | Industrious | chat/60s | **38.6** | 4.21 | 11.8–58.3 | **3.03** | 0.18 | 38.6 | 3.02 | 41.2 | 3.20 | 9.1 (9.1) | 1.08 (1.10) | 0.56 | 0.37 (0.64) |
| resident | Dreamy | quiet | **36.6** | 4.65 | 17.6–56.4 | **2.70** | 0.22 | 36.6 | 2.70 | 36.6 | 2.70 | 4.1 (4.1) | – | – | – |
| resident | Dreamy | chat/60s | **32.5** | 4.30 | 13.1–53.7 | **2.80** | 0.23 | 32.9 | 2.82 | 35.1 | 3.01 | 3.7 (3.6) | 0.99 (1.10) | 0.53 | 0.38 (0.66) |

What moved:
- **Quiet cells are identical**, number for number: no line comes there.
- **The review fixes move the chat cells by at most 0.7 points** (home, ordinary: 44.2 → 44.9) and
  set-offs by at most 0.03 a minute, inside a 4-visit σ of 3.3–8.0 points.
- **Chat now lowers the share, as intended.** Chat minus quiet is −1.7 to −5.3 points in the home
  and resident (every cell below quiet), and −3.8 to +1.9 on the stage; at step 5 it was −2.7 to
  +3.2. Against step 5 the chat cells fall by 2.2–5.3 points in the home and resident (most in the
  home, dreamy), and by −0.4 to 2.0 on the stage.
- **Restarts after a line roughly halve** (stage 0.53–0.68 a minute against 0.83–1.13; home
  0.26–0.34 against 0.48–0.58; resident 0.35–0.39 against 0.64–0.80): a still act no longer ends at
  a line. Set-offs fall by 0.07–0.55 a minute; chat still adds a few over quiet (home, lazy: 1.68
  against 1.51; the restarts left are lines that cut a walk or a standing moment, which chat cuts as
  before).
- **Exercise** is within ±0.8 points of step 5's, inside minor 8's 3 points.
- **Every home and resident cell is still over its ceiling**, and the line-art stage over its band;
  the levers (steps 7–8) are what bring them in.


## Step 8: tuning stopped (2026-10-06)

**Measured 2026-10-06** on step 7 (the levers landed neutral), the same census as step 6's
(`CENSUS_MINUTES=9 CENSUS_MODES=line fed_afternoon_census`, 20 sets × 4 seeds, release), one run
per move in synthesis M5's order. **Stopped by the stop rule**: no setting of M5's levers puts
every cell inside its band and under M3's set-off cap, because the cap and the levers pull the
rooms apart (below). None of these values ships: the commit that adds this section ships the
slow blink and, from its review, the credit fix (move 1) alone; the levers stay
`Stillness::NEUTRAL`. The values tried are listed here in full.

### The moves, in order

Each column adds its move to the one before; cells are moving % in sight / set-offs a minute.

1. **credit**: needs rise to the credit's moment before it lands (`rise_to` at the top of
   `serve`, step 2's hand-off).
2. **needs, bases**: `rise_ms` Restless 90 → 300 s, Tidy 60 → 240 s, Mischief 4 → 8 min; Walk
   14 → 9, Travel 10 → 7 (M4).
3. **near**: `Stillness.near` on (pull, swap and pick_build on her floor first; seats weighted).
4. **lengths**, half way from today's to D4's list: SpaceOut 13–37 s, Sit 20–57, LieBack 27–80,
   SitDoze 27–80, Gaze 7–17, Lounge 27–60, Nap 45–105, Watch 32–82, Read 30–65, Homework 37–75
   (LieFront, Sleep and the homework slot's 120–240 s unchanged).
5. **space out, stand**: SpaceOut `own_sake: true`; a chosen Stand 3–8 s (was 2–5).
6. **linger, settle, sessions**: `Stillness::STARTING` (linger Lazy 1.5, Ordinary 1.0, Dreamy
   1.0, Industrious 0.7; settle Lazy 0.6, Ordinary 0.35, Dreamy 0.35, Industrious 0.15; sitting
   dozes half the time; musings Dreamy 2–4, Ordinary 1–3, Lazy 0–2, Industrious 0–1; nod-off by
   mood).
7. **5 + idle shorter, tidy 150 s**: the floor's still acts back toward today's (SpaceOut 10–28 s,
   Sit 15–40, LieBack and SitDoze 20–60, Gaze 6–14), Tidy 240 → 150 s, Mischief 8 → 6 min, to
   bring the stage and resident up.

| Room | Mood | Chat | step 7 | 1. credit | 2. needs, bases | 3. near | 4. lengths | 5. space out, stand | 6. linger, settle, sessions | 7. idle shorter, tidy 150 s | Band | M3 cap |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | 32.5 / 4.96 | 30.6 / 4.87 | 24.5 / 4.02 | 18.5 / 3.71 | 13.7 / 2.69 | 13.2 / 2.35 | 10.7 / 1.99 | 10.8 / 2.11 | 16–26 | 3.43 |
| stage | Ordinary | chat | 28.7 / 4.92 | 29.4 / 5.15 | 24.6 / 4.28 | 16.0 / 3.80 | 12.9 / 2.83 | 10.2 / 2.43 | 10.5 / 2.19 | 9.8 / 2.28 | 16–26 | 3.90 |
| stage | Lazy | quiet | 21.8 / 4.19 | 21.7 / 4.18 | 22.4 / 3.40 | 12.3 / 2.83 | 11.7 / 2.07 | 11.4 / 1.94 | 6.2 / 1.06 | 6.9 / 1.20 | 11–17 | 2.88 |
| stage | Lazy | chat | 23.7 / 4.19 | 24.8 / 4.42 | 21.6 / 3.54 | 13.5 / 3.23 | 12.2 / 2.32 | 10.6 / 2.29 | 6.2 / 1.29 | 6.7 / 1.46 | 11–17 | 2.89 |
| stage | Industrious | quiet | 34.9 / 5.11 | 33.0 / 5.07 | 23.7 / 4.52 | 19.0 / 3.80 | 15.1 / 2.92 | 13.3 / 2.59 | 14.9 / 2.73 | 14.4 / 2.89 | 20–31 | 4.39 |
| stage | Industrious | chat | 35.7 / 5.59 | 36.5 / 5.49 | 26.3 / 4.84 | 16.3 / 3.85 | 14.8 / 3.26 | 12.5 / 2.78 | 12.6 / 2.86 | 12.6 / 2.92 | 20–31 | 4.81 |
| stage | Dreamy | quiet | 27.8 / 4.86 | 27.2 / 4.69 | 20.2 / 3.81 | 15.4 / 3.30 | 13.0 / 2.38 | 11.7 / 2.21 | 10.5 / 1.86 | 9.6 / 1.93 | 16–26 | 3.93 |
| stage | Dreamy | chat | 28.2 / 4.92 | 30.2 / 5.02 | 21.4 / 3.95 | 12.4 / 3.34 | 10.0 / 2.65 | 9.4 / 2.34 | 8.9 / 1.99 | 8.7 / 2.03 | 16–26 | 4.00 |
| home | Ordinary | quiet | 48.4 / 2.33 | 48.0 / 2.28 | 34.9 / 1.78 | 34.9 / 1.79 | 25.4 / 1.53 | 25.6 / 1.52 | 21.8 / 1.36 | 23.3 / 1.46 | 16–26 | 1.08 |
| home | Ordinary | chat | 44.9 / 2.47 | 43.1 / 2.41 | 31.5 / 1.94 | 31.1 / 1.92 | 25.8 / 1.68 | 24.8 / 1.66 | 22.3 / 1.50 | 22.7 / 1.51 | 16–26 | 1.24 |
| home | Lazy | quiet | 35.8 / 1.51 | 35.7 / 1.55 | 24.5 / 1.26 | 24.8 / 1.26 | 18.5 / 0.97 | 17.9 / 0.97 | 10.4 / 0.57 | 10.9 / 0.60 | 11–17 | 0.63 |
| home | Lazy | chat | 34.0 / 1.68 | 33.4 / 1.73 | 24.0 / 1.42 | 24.1 / 1.42 | 19.7 / 1.11 | 19.0 / 1.10 | 11.1 / 0.69 | 11.3 / 0.71 | 11–17 | 0.79 |
| home | Industrious | quiet | 53.4 / 2.72 | 52.8 / 2.73 | 38.3 / 2.36 | 38.4 / 2.35 | 33.4 / 2.05 | 31.8 / 2.00 | 34.6 / 2.13 | 35.9 / 2.24 | 20–31 | 1.53 |
| home | Industrious | chat | 51.7 / 2.86 | 51.4 / 2.96 | 37.1 / 2.57 | 37.0 / 2.59 | 30.4 / 2.25 | 27.8 / 2.11 | 31.3 / 2.32 | 33.6 / 2.41 | 20–31 | 1.71 |
| home | Dreamy | quiet | 46.1 / 2.27 | 46.2 / 2.29 | 30.1 / 1.75 | 30.0 / 1.73 | 23.7 / 1.49 | 23.6 / 1.46 | 19.7 / 1.29 | 21.0 / 1.31 | 16–26 | 1.11 |
| home | Dreamy | chat | 42.4 / 2.38 | 42.4 / 2.36 | 30.1 / 1.99 | 29.9 / 1.96 | 23.3 / 1.62 | 21.8 / 1.57 | 18.8 / 1.39 | 19.9 / 1.45 | 16–26 | 1.23 |
| resident | Ordinary | quiet | 38.8 / 2.79 | 38.8 / 2.82 | 23.3 / 2.32 | 22.7 / 2.33 | 16.8 / 1.73 | 15.8 / 1.67 | 14.0 / 1.48 | 15.0 / 1.51 | 16–26 | 1.62 |
| resident | Ordinary | chat | 35.6 / 2.99 | 34.8 / 3.04 | 22.0 / 2.49 | 21.2 / 2.52 | 16.1 / 1.84 | 14.3 / 1.82 | 13.2 / 1.66 | 14.1 / 1.69 | 16–26 | 1.87 |
| resident | Lazy | quiet | 32.3 / 2.31 | 33.8 / 2.45 | 25.6 / 2.14 | 25.3 / 2.14 | 14.0 / 1.24 | 13.3 / 1.18 | 7.8 / 0.64 | 8.3 / 0.66 | 11–17 | 1.07 |
| resident | Lazy | chat | 30.6 / 2.61 | 32.1 / 2.60 | 24.4 / 2.27 | 24.2 / 2.26 | 15.1 / 1.48 | 13.4 / 1.39 | 6.9 / 0.70 | 7.3 / 0.73 | 11–17 | 1.19 |
| resident | Industrious | quiet | 43.9 / 2.92 | 41.5 / 2.84 | 25.6 / 2.32 | 23.1 / 2.30 | 19.6 / 1.77 | 18.4 / 1.67 | 20.1 / 1.71 | 20.1 / 1.73 | 20–31 | 2.00 |
| resident | Industrious | chat | 38.6 / 3.03 | 38.2 / 3.02 | 23.4 / 2.49 | 22.1 / 2.52 | 17.5 / 1.90 | 17.2 / 1.80 | 18.1 / 1.89 | 17.7 / 1.89 | 20–31 | 2.33 |
| resident | Dreamy | quiet | 36.6 / 2.70 | 36.6 / 2.64 | 20.1 / 2.30 | 19.9 / 2.31 | 14.0 / 1.64 | 12.8 / 1.58 | 11.4 / 1.36 | 11.7 / 1.39 | 16–26 | 1.66 |
| resident | Dreamy | chat | 32.5 / 2.80 | 31.7 / 2.82 | 18.5 / 2.40 | 17.8 / 2.42 | 12.5 / 1.73 | 12.0 / 1.69 | 10.7 / 1.48 | 11.8 / 1.54 | 16–26 | 1.93 |

| stage industrious ÷ lazy | 1.55 | 1.49 | 1.14 | 1.37 | 1.25 | 1.17 | 2.22 | 1.99 |
| home industrious ÷ lazy | 1.51 | 1.51 | 1.55 | 1.54 | 1.67 | 1.62 | 3.07 | 3.13 |
| resident industrious ÷ lazy | 1.31 | 1.21 | 0.98 | 0.91 | 1.27 | 1.33 | 2.60 | 2.42 |

### Why it stops

**M3's cap freezes each room's moving seconds per set-off at the baseline's**, so it caps the
share: the most a cell can move under its cap is the cap × the seconds a set-off moves her now.
The levers shorten her trips without any walk getting faster: once she no longer wanders off and
travels (Restless 300 s, Walk 9), what's left is the short walk from where she is to a seat or a
line (home: a set-off moved her 12.5 s at the baseline and 9.6 s now; resident 8.3 → 6.0; stage
3.9 → 3.1; the ratio hardly moves between moves 3 and 7). In move 7:

| Room | Mood | Chat | Moving % | Set-offs/min | M3 cap | s moving a set-off (baseline) | most % under the cap | Band | Fits? |
|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | 10.8 | 2.11 | 3.43 | 3.1 (3.9) | 17.6 | 16–26 | yes |
| stage | Ordinary | chat | 9.8 | 2.28 | 3.90 | 2.6 (3.5) | 16.8 | 16–26 | yes |
| stage | Lazy | quiet | 6.9 | 1.20 | 2.88 | 3.5 (3.1) | 16.6 | 11–17 | yes |
| stage | Lazy | chat | 6.7 | 1.46 | 2.89 | 2.8 (3.1) | 13.3 | 11–17 | yes |
| stage | Industrious | quiet | 14.4 | 2.89 | 4.39 | 3.0 (4.1) | 21.9 | 20–31 | yes |
| stage | Industrious | chat | 12.6 | 2.92 | 4.81 | 2.6 (3.7) | 20.8 | 20–31 | yes |
| stage | Dreamy | quiet | 9.6 | 1.93 | 3.93 | 3.0 (3.4) | 19.6 | 16–26 | yes |
| stage | Dreamy | chat | 8.7 | 2.03 | 4.00 | 2.6 (3.4) | 17.1 | 16–26 | yes |
| home | Ordinary | quiet | 23.3 | 1.46 | 1.08 | 9.6 (12.5) | 17.3 | 16–26 | yes |
| home | Ordinary | chat | 22.7 | 1.51 | 1.24 | 9.0 (10.9) | 18.6 | 16–26 | yes |
| home | Lazy | quiet | 10.9 | 0.60 | 0.63 | 10.9 (14.2) | 11.5 | 11–17 | barely |
| home | Lazy | chat | 11.3 | 0.71 | 0.79 | 9.5 (11.4) | 12.6 | 11–17 | yes |
| home | Industrious | quiet | 35.9 | 2.24 | 1.53 | 9.6 (11.8) | 24.5 | 20–31 | yes |
| home | Industrious | chat | 33.6 | 2.41 | 1.71 | 8.4 (10.5) | 23.9 | 20–31 | yes |
| home | Dreamy | quiet | 21.0 | 1.31 | 1.11 | 9.6 (12.2) | 17.8 | 16–26 | yes |
| home | Dreamy | chat | 19.9 | 1.45 | 1.23 | 8.2 (11.0) | 16.9 | 16–26 | yes |
| resident | Ordinary | quiet | 15.0 | 1.51 | 1.62 | 6.0 (8.3) | 16.1 | 16–26 | barely |
| resident | Ordinary | chat | 14.1 | 1.69 | 1.87 | 5.0 (7.2) | 15.6 | 16–26 | no |
| resident | Lazy | quiet | 8.3 | 0.66 | 1.07 | 7.5 (8.4) | 13.5 | 11–17 | yes |
| resident | Lazy | chat | 7.3 | 0.73 | 1.19 | 6.0 (7.6) | 11.9 | 11–17 | yes |
| resident | Industrious | quiet | 20.1 | 1.73 | 2.00 | 7.0 (9.0) | 23.2 | 20–31 | yes |
| resident | Industrious | chat | 17.7 | 1.89 | 2.33 | 5.6 (7.7) | 21.8 | 20–31 | yes |
| resident | Dreamy | quiet | 11.7 | 1.39 | 1.66 | 5.1 (8.1) | 14.0 | 16–26 | no |
| resident | Dreamy | chat | 11.8 | 1.54 | 1.93 | 4.6 (7.0) | 14.8 | 16–26 | no |

- **The rooms need opposite moves.** The stage's ordinary cell must set off about 3.3 times a
  minute to clear 16%, the home's at most 1.08: three to one, from one shared table of lengths.
  At today's lengths (move 3) the ratio was 2.1, and longer still acts lower it (the stage's
  rate falls faster: its uses are made pieces close to her, with the rest of its set-offs short
  hops to text). The stage and resident are time-limited now, not need-limited: Tidy 150 s and
  Mischief 6 min (move 7) moved the stage's to-text share by 0.1 point.
- **Cells with no window at all** (the most under the cap at or below the floor): resident
  dreamy (14.0, 14.8 against 16), resident ordinary at its chat cadence (15.6). Barely: home lazy
  quiet (11.5 against 11), resident ordinary quiet (16.1 against 16).
- **What did land:** the spread between moods (industrious ÷ lazy 2.0 / 3.1 / 2.4 against 1.6),
  from move 6 alone; and exercise fell in every cell (industrious stage by 8 points), inside
  minor 8's +3.
- **Not levers:** seat nearness off moved nothing (her seats are mostly one of a kind); the
  credit fix moved every cell by 2 points or less.

### What else the attempt showed

- **The band's guard** (`band_visits_outlast_her_longest_still_act_twice`): with Lazy's linger
  1.5 a day's sleep is 270 s, so `BAND_MINUTES` must be 12 (3 + 2 × 4.5) for any landing of
  `STARTING`'s lingering; the Lounge + Nap settle chain at move 4's lengths is 247 s.
- **The credit fix** landed alone in step 8's review (decisions.md, "An easing lands on her
  needs as they are"), `a_credit_eases_her_needs_as_they_are` un-ignored. With every lever on
  (move 7) `her_needs_shape_long_visits` passed over its four visits; with the fix alone it
  failed there, as step 2 found. Measured rather than retuned: the four visits doze 0.7× as much
  late, twelve 1.3×, forty 2× (34 of 40 more late), so the test now runs twelve.
- **Chat lines in the census and band harnesses** arrive at the first step that crosses them,
  so a wake that changes only how she looks (the slow blink's) moves the chat cells a little
  (home dreamy chat 2.34 → 2.26 set-offs a minute; stage dreamy chat 31.1 / 4.27 → 30.8 /
  4.53). Stepping to the next line's own time fixes that, but changes the driver `BASELINE`
  was measured with; it belongs at the start of the retune, with everything re-measured.
- `sleepiness_draws_her_to_lie_down` fails at Walk 9 / Travel 7 with Restless 300 s (it reads the
  bases); `a_restless_osaka_mostly_moves` passes.

### For the user

The cap is M3's (step 4): the baseline's rate × target ÷ baseline share, so a lever that only
shortens walks fails it. These levers shorten walks by keeping her near her things, not by
gaming the measure. The choice is the user's:
- recompute the cap from the tuned seconds per set-off (or restore D2's × 1.15 slack, which
  alone isn't enough for the resident's dreamy cells);
- or keep the cap, and accept per-room floors below the band where the room has her things close
  (the resident, the stage), or a mood lever beyond lengths and settling;
- or judge set-offs alone (onsets are what draw the eye) and the share only as a ceiling.

## Step 8b: the retune, stopped (2026-10-06)

**Measured 2026-10-06**, to the user's round-3 band (per-mood caps in every room and chat
condition: lazy ≤ 1.5 set-offs a minute in sight, ordinary and dreamy ≤ 2.25, industrious ≤ 3.0;
share ceilings 17 / 26 / 31, floors 5 / 8 / 12; industrious ÷ lazy ≥ 1.6 a room). **Stopped by the
stop rule, again**, one level down from step 8's: under a per-mood cap the stage's short trips put
the most it can move at about its floor, while the home's industrious cell is over its ceiling,
and every lever in M5's list moves both rooms of a mood the same way. Nothing of the tuning ships:
the commit that adds this section ships the band's new rule (still ignored), `BASELINE` re-measured
at the new driver, and watching taken out of lingering (inert while the levers are neutral).

### First: chat lines at their own time

Step 8's hand-off, done first and committed alone: the census and band harnesses now step to each
chat line's own moment (`Room::step_from`, used by both drivers), instead of delivering it at the
first step that crosses it, whose length her wakes set. Re-measured on step 7's state (the levers
neutral, with step 8's blink and credit fix), the band's 9-minute line-art census, 20 sets × 4
seeds:

```text
CENSUS_MINUTES=9 CENSUS_MODES=line \
  cargo test --release -p dessplay --lib fed_afternoon_census -- --ignored --nocapture
```

Quiet cells are identical to step 8's "1. credit" column (no line comes there); chat cells move by
at most 0.5 points and 0.07 set-offs a minute (mean 0.10 points). These rows (c0) are now
`tests/band.rs`'s `BASELINE`.

| Room | Mood | Chat | Moving % | σ (N=4) | a visit | Set-offs/min | σ | Step 8's credit column (old driver) |
|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | **30.6** | 6.47 | 2.9–72.4 | **4.87** | 0.59 | 30.6 / 4.87 |
| stage | Ordinary | chat | **29.4** | 7.87 | 4.4–74.5 | **5.14** | 0.49 | 29.4 / 5.15 |
| stage | Lazy | quiet | **21.7** | 7.23 | 2.3–70.5 | **4.18** | 0.42 | 21.7 / 4.18 |
| stage | Lazy | chat | **24.8** | 8.00 | 5.0–67.2 | **4.41** | 0.43 | 24.8 / 4.42 |
| stage | Industrious | quiet | **33.0** | 6.06 | 10.3–69.4 | **5.07** | 0.47 | 33.0 / 5.07 |
| stage | Industrious | chat | **36.7** | 7.16 | 14.1–80.2 | **5.45** | 0.62 | 36.5 / 5.49 |
| stage | Dreamy | quiet | **27.2** | 6.51 | 6.9–75.2 | **4.69** | 0.55 | 27.2 / 4.69 |
| stage | Dreamy | chat | **30.5** | 7.06 | 5.7–71.2 | **4.95** | 0.66 | 30.2 / 5.02 |
| home | Ordinary | quiet | **48.0** | 5.99 | 22.0–69.8 | **2.28** | 0.18 | 48.0 / 2.28 |
| home | Ordinary | chat | **43.4** | 5.40 | 16.4–66.0 | **2.42** | 0.21 | 43.1 / 2.41 |
| home | Lazy | quiet | **35.7** | 4.47 | 11.8–57.6 | **1.55** | 0.21 | 35.7 / 1.55 |
| home | Lazy | chat | **33.9** | 5.83 | 14.5–60.4 | **1.74** | 0.22 | 33.4 / 1.73 |
| home | Industrious | quiet | **52.8** | 4.42 | 28.0–75.7 | **2.73** | 0.25 | 52.8 / 2.73 |
| home | Industrious | chat | **51.4** | 5.05 | 31.2–75.0 | **2.96** | 0.25 | 51.4 / 2.96 |
| home | Dreamy | quiet | **46.2** | 4.31 | 18.2–68.0 | **2.29** | 0.16 | 46.2 / 2.29 |
| home | Dreamy | chat | **42.1** | 6.02 | 22.5–65.4 | **2.35** | 0.20 | 42.4 / 2.36 |
| resident | Ordinary | quiet | **38.8** | 3.14 | 17.9–55.7 | **2.82** | 0.17 | 38.8 / 2.82 |
| resident | Ordinary | chat | **35.3** | 5.34 | 14.5–53.0 | **3.03** | 0.28 | 34.8 / 3.04 |
| resident | Lazy | quiet | **33.8** | 4.15 | 8.9–56.0 | **2.45** | 0.17 | 33.8 / 2.45 |
| resident | Lazy | chat | **31.9** | 4.46 | 2.7–56.2 | **2.58** | 0.17 | 32.1 / 2.60 |
| resident | Industrious | quiet | **41.5** | 4.26 | 14.0–63.9 | **2.84** | 0.15 | 41.5 / 2.84 |
| resident | Industrious | chat | **38.3** | 4.05 | 15.6–53.0 | **3.02** | 0.22 | 38.2 / 3.02 |
| resident | Dreamy | quiet | **36.6** | 5.71 | 13.7–61.6 | **2.64** | 0.22 | 36.6 / 2.64 |
| resident | Dreamy | chat | **31.8** | 3.69 | 15.9–53.5 | **2.81** | 0.21 | 31.7 / 2.82 |

### The runs

Each run is the fed census at 12-minute visits (`BAND_MINUTES` 12 once lingering lands: a lazy
day's sleep lingered is 270 s), line art, 20 sets × 4 seeds, release, at the new driver.
- **c1: step 8's move 7**, the starting point: Restless 300 s, Tidy 150 s, Mischief 6 min; Walk 9,
  Travel 7; SpaceOut `own_sake`, a chosen Stand 3–8 s; SpaceOut 10–28 s, Sit 15–40, LieBack and
  SitDoze 20–60, Gaze 6–14, Lounge 27–60, Nap 45–105, Watch 32–82, Read 30–65, Homework 37–75;
  `Stillness::STARTING` (linger, settling, sessions, nod-off, near), **watching out of lingering**
  (D7: a lazy day would watch animated snow ×1.5 longer).
- **c2: c1 + industrious restless ×1.5 → ×1.2**, a diagnostic: is restlessness the home's
  industrious excess (wander and travel are ~9% of her time in sight there against ~3% on the
  stage)?
- **c3: c1 + Tidy 150 → 90 s and industrious linger 0.7 → 0.85**: Tidy only rises with text on
  offer, so it's a stage-only lever (the home's and resident's cells are identical to c1's,
  number for number); the linger brings the home's industrious cell in.

Cells are moving % in sight / set-offs a minute in sight; **bold** is outside the band or over the
cap. "Most % under the cap" is the cap × the cell's own share ÷ rate: how much she could move at
that cell's seconds per set-off without passing the cap.

| Room | Mood | Chat | step 7 at the new driver (c0, 9 min) | step 8's move 7 (old driver, 9 min) | c1: move 7, watching unlingered, 12 min | c2: c1 + industrious restless ×1.2 | c3: c1 + tidy 90 s, industrious linger 0.85 | Band | Cap | Most % under the cap (c1 / c3) |
|---|---|---|---|---|---|---|---|---|---|---|
| stage | Ordinary | quiet | 30.6 / 4.87 | 10.8 / 2.11 | 9.1 / 1.94 | 9.1 / 1.94 | 8.7 / 1.88 | 8–26 | 2.25 | 10.6 / 10.4 |
| stage | Ordinary | chat | 29.4 / 5.14 | 9.8 / 2.28 | 8.7 / 2.08 | 8.7 / 2.08 | 10.7 / 2.14 | 8–26 | 2.25 | 9.4 / 11.2 |
| stage | Lazy | quiet | 21.7 / 4.18 | 6.9 / 1.20 | 6.2 / 1.17 | 6.2 / 1.17 | 6.7 / 1.16 | 5–17 | 1.5 | 7.9 / 8.7 |
| stage | Lazy | chat | 24.8 / 4.41 | 6.7 / 1.46 | 6.2 / 1.33 | 6.2 / 1.33 | 5.3 / 1.31 | 5–17 | 1.5 | 7.0 / 6.1 |
| stage | Industrious | quiet | 33.0 / 5.07 | 14.4 / 2.89 | 13.5 / 2.78 | 12.3 / 2.63 | 12.4 / 2.49 | 12–31 | 3.0 | 14.6 / 14.9 |
| stage | Industrious | chat | 36.7 / 5.45 | 12.6 / 2.92 | **11.7 / 2.76** | 12.4 / 2.74 | **10.0 / 2.74** | 12–31 | 3.0 | 12.7 / 10.9 |
| stage | Dreamy | quiet | 27.2 / 4.69 | 9.6 / 1.93 | **7.8 / 1.73** | **7.8 / 1.73** | 9.0 / 1.76 | 8–26 | 2.25 | 10.1 / 11.5 |
| stage | Dreamy | chat | 30.5 / 4.95 | 8.7 / 2.03 | 8.3 / 1.92 | 8.3 / 1.92 | **7.6 / 1.81** | 8–26 | 2.25 | 9.7 / 9.4 |
| home | Ordinary | quiet | 48.0 / 2.28 | 23.3 / 1.46 | 22.3 / 1.31 | 22.3 / 1.31 | 22.3 / 1.31 | 8–26 | 2.25 | 38.3 / 38.3 |
| home | Ordinary | chat | 43.4 / 2.42 | 22.7 / 1.51 | 22.1 / 1.45 | 22.1 / 1.45 | 22.1 / 1.45 | 8–26 | 2.25 | 34.3 / 34.3 |
| home | Lazy | quiet | 35.7 / 1.55 | 10.9 / 0.60 | 11.5 / 0.63 | 11.5 / 0.63 | 11.5 / 0.63 | 5–17 | 1.5 | 27.4 / 27.4 |
| home | Lazy | chat | 33.9 / 1.74 | 11.3 / 0.71 | 11.9 / 0.71 | 11.9 / 0.71 | 11.9 / 0.71 | 5–17 | 1.5 | 25.1 / 25.1 |
| home | Industrious | quiet | 52.8 / 2.73 | 35.9 / 2.24 | **33.1 / 2.00** | **33.2 / 2.01** | 30.4 / 1.85 | 12–31 | 3.0 | 49.7 / 49.3 |
| home | Industrious | chat | 51.4 / 2.96 | 33.6 / 2.41 | 30.1 / 2.17 | 29.2 / 2.13 | 29.7 / 2.09 | 12–31 | 3.0 | 41.6 / 42.6 |
| home | Dreamy | quiet | 46.2 / 2.29 | 21.0 / 1.31 | 20.3 / 1.21 | 20.3 / 1.21 | 20.3 / 1.21 | 8–26 | 2.25 | 37.7 / 37.7 |
| home | Dreamy | chat | 42.1 / 2.35 | 19.9 / 1.45 | 19.2 / 1.32 | 19.2 / 1.32 | 19.2 / 1.32 | 8–26 | 2.25 | 32.7 / 32.7 |
| resident | Ordinary | quiet | 38.8 / 2.82 | 15.0 / 1.51 | 13.6 / 1.35 | 13.6 / 1.35 | 13.6 / 1.35 | 8–26 | 2.25 | 22.7 / 22.7 |
| resident | Ordinary | chat | 35.3 / 3.03 | 14.1 / 1.69 | 13.2 / 1.51 | 13.2 / 1.51 | 13.2 / 1.51 | 8–26 | 2.25 | 19.7 / 19.7 |
| resident | Lazy | quiet | 33.8 / 2.45 | 8.3 / 0.66 | 8.1 / 0.68 | 8.1 / 0.68 | 8.1 / 0.68 | 5–17 | 1.5 | 17.9 / 17.9 |
| resident | Lazy | chat | 31.9 / 2.58 | 7.3 / 0.73 | 8.5 / 0.79 | 8.5 / 0.79 | 8.5 / 0.79 | 5–17 | 1.5 | 16.1 / 16.1 |
| resident | Industrious | quiet | 41.5 / 2.84 | 20.1 / 1.73 | 18.1 / 1.62 | 16.9 / 1.57 | 18.2 / 1.57 | 12–31 | 3.0 | 33.5 / 34.8 |
| resident | Industrious | chat | 38.3 / 3.02 | 17.7 / 1.89 | 17.2 / 1.77 | 15.8 / 1.70 | 15.4 / 1.67 | 12–31 | 3.0 | 29.2 / 27.7 |
| resident | Dreamy | quiet | 36.6 / 2.64 | 11.7 / 1.39 | 10.2 / 1.21 | 10.2 / 1.21 | 10.2 / 1.21 | 8–26 | 2.25 | 19.0 / 19.0 |
| resident | Dreamy | chat | 31.8 / 2.81 | 11.8 / 1.54 | 10.6 / 1.36 | 10.6 / 1.36 | 10.6 / 1.36 | 8–26 | 2.25 | 17.5 / 17.5 |

Industrious ÷ lazy moving, both chat conditions pooled:

| Ratio | c0 | step 8's move 7 | c1 | c2 | c3 |
|---|---|---|---|---|---|
| stage industrious ÷ lazy | 1.50 | | 2.03 | 1.99 | 1.87 |
| home industrious ÷ lazy | 1.50 | | 2.70 | 2.67 | 2.57 |
| resident industrious ÷ lazy | 1.21 | | 2.13 | 1.97 | 2.02 |

### Why it stops

- **The pooled σ** of an 80-visit cell is about 0.22 × its 4-visit σ; c1's (not tabled) are
  2.2–4.6 on the stage and 2.1–4.3 in the home, so the pooled σ is about 0.5–1.0 point in
  either. So c1's home industrious quiet (33.1 against 31) is out
  by over 2σ, a real excess; its stage cells at 7.8 and 11.7 are at their floors within noise.
- **Under a per-mood cap the stage's share is bounded by its trips.** The most a cell can move is
  its cap × the seconds a set-off moves her. On the stage that's 2.2–3.5 s (a hop to a line on her
  own floor, a few steps to the piece she made), so its windows from the floor to that bound are:
  lazy 5 to 6–9, ordinary and dreamy 8 to 9–11.5, industrious 12 to 14–15 quiet and 11–14 with
  chat. The middle of the band is out of reach with any lever that keeps her trips short, and
  "not at the floor" leaves a point or two. In the home a set-off moves her 8–11 s (the resident
  5–7), so the cap there binds nothing (the most under it is 25–50%).
- **Industrious can't hold both rooms away from the stage's floor.** Home ÷ stage within
  industrious is 2.35–2.97 across the runs (c1 33.1 ÷ 13.5 = 2.45 quiet, 30.1 ÷ 11.7 = 2.57 chat;
  c2 2.70 / 2.35; c3 2.45 / 2.97; step 8's move 7 2.49 / 2.67), against the band's own
  31 ÷ 12 = 2.58. Some runs fall under 2.58 (c2's chat holds both, 29.2 and 12.4), so it isn't that
  no setting holds both; it's that at these ratios a home under 31 leaves the stage at 10.4–13.2,
  at or within about a point of its floor of 12, and the tuning may not aim a cell at its floor.
  Every lever that moved the stage moved the home with it: industrious linger 0.85 (c3) brought the
  home in (30.4 / 29.7) by taking the stage's chat cell to 10.0. The one stage-only lever tried,
  Tidy 90 s, moved the stage only within noise (she's time-limited there, not tidy-limited).
  Restless ×1.2 (c2) moved the home's industrious cell within noise (+0.1 quiet, 33.1 → 33.2;
  −0.9 with chat, 30.1 → 29.2): the home's excess isn't restlessness. Its industrious time walking
  is to a seat (19%: a snack 5.1, lounging 4.6, watching 3.9, reading 3.1) and to arrange her home
  (5.4%, against 2.2% ordinary: `home_acts`, three a visit industrious, the user's call).
- **At full strength** a stage cell aimed inside its window would need N in the many hundreds to a
  few thousand for its bounds to be the band itself (c1's stage dreamy chat, 0.3 over its floor at
  a 4-visit σ of 4.1: N ≈ 6800).
- **What did land, again:** the home and resident inside the band in every cell from c1 on, apart
  from the home's industrious; set-offs under every cap; industrious ÷ lazy 1.87–2.70 a room.

### For the user

A room-selective lever is needed, or the rule changes for the stage. In the right direction:
- **Longer trips on the stage**: `near` off for text (pulls and swaps from other floors: longer
  trips, and more legs, so more set-offs too; it undoes M6's "pulls stay on her floor"); or wait
  for step 10a's "more makeshift while she owns no real piece" (making ×3 is stage-only and takes
  her across her floor for the text), measuring the stage after it before retuning.
- **Less industrious moving in the home** by 3–5 points without touching the stage: `home_acts`
  three → two industrious (its excess over ordinary is 3.2 points), or something in the home's
  seat trips (a snack's walk to the fridge is 5.1% industrious, 3.1% ordinary).
- **A wider stage window**: lower floors on the stage (it has her made things close by), or the
  user's third option from step 8: judge the stage by set-offs alone, the share only as a ceiling.
- Or ship c3's values as they are (every home and resident cell in, the stage at its floors and
  near its caps), un-ignoring only the home and resident cells.

Step 8's hand-offs that come with any tuning carry forward unchanged: `SPACE_OUT_MS` lengthens in
the commit that switches the shipped `Stillness` on; `BAND_MINUTES` 12 with lingering;
`sleepiness_draws_her_to_lie_down` restated at Walk 9 / Travel 7 / Restless 300 s; the riddle
test per mood (or over daydreams with a musing); the minor-1 restatements; the 15-minute fed, unfed
and day censuses re-measured (nothing she does changed in 8b, so none was re-run).

## Step 10a: more makeshift, measured (2026-10-06)

**Measured 2026-10-06** on step 10a's commit (`3030d69e`): homework and a book on the floor where no
desk or bookshelf stands, the paper desk, making ×3 while she owns no real piece of the kind (×3 more
for the desk while her back aches), cross-legged watching. The levers are still
`Stillness::NEUTRAL`, so this is 8b's c0 plus step 10a. The band at full strength:

```text
CENSUS_BAND_SEEDS=190 cargo nextest run -p dessplay --release --profile band \
  --run-ignored all -E 'test(/houseguest::tests::band::/)' --success-output final
```

190 visits a cell (9 minutes, line art), 6.0 minutes' wall. Moving % in sight / set-offs a minute in
sight, quiet | chat, with c0 (`BASELINE`, 80 visits) after each.

| Room | Mood | Quiet (10a) | Quiet (c0) | Chat (10a) | Chat (c0) | Band | Cap |
|---|---|---|---|---|---|---|---|
| stage | Lazy | 24.9 / 3.98 | 21.7 / 4.18 | 25.3 / 4.04 | 24.8 / 4.41 | 5–17 | 1.5 |
| stage | Ordinary | 27.7 / 4.33 | 30.6 / 4.87 | 28.5 / 4.49 | 29.4 / 5.14 | 8–26 | 2.25 |
| stage | Dreamy | 26.1 / 4.29 | 27.2 / 4.69 | 27.3 / 4.51 | 30.5 / 4.95 | 8–26 | 2.25 |
| stage | Industrious | 29.3 / 4.65 | 33.0 / 5.07 | 29.4 / 4.83 | 36.7 / 5.45 | 12–31 | 3.0 |
| home | Lazy | 34.1 / 1.52 | 35.7 / 1.55 | 33.2 / 1.70 | 33.9 / 1.74 | 5–17 | 1.5 |
| home | Ordinary | 47.3 / 2.28 | 48.0 / 2.28 | 44.0 / 2.44 | 43.4 / 2.42 | 8–26 | 2.25 |
| home | Dreamy | 45.5 / 2.25 | 46.2 / 2.29 | 42.5 / 2.41 | 42.1 / 2.35 | 8–26 | 2.25 |
| home | Industrious | 53.3 / 2.76 | 52.8 / 2.73 | 52.1 / 2.91 | 51.4 / 2.96 | 12–31 | 3.0 |
| resident | Lazy | 33.0 / 2.25 | 33.8 / 2.45 | 30.9 / 2.37 | 31.9 / 2.58 | 5–17 | 1.5 |
| resident | Ordinary | 38.4 / 2.66 | 38.8 / 2.82 | 34.1 / 2.80 | 35.3 / 3.03 | 8–26 | 2.25 |
| resident | Dreamy | 35.4 / 2.56 | 36.6 / 2.64 | 32.2 / 2.72 | 31.8 / 2.81 | 8–26 | 2.25 |
| resident | Industrious | 42.5 / 2.74 | 41.5 / 2.84 | 37.3 / 2.88 | 38.3 / 3.02 | 12–31 | 3.0 |

Industrious ÷ lazy (both chats pooled): stage 1.17, home 1.56, resident 1.25 (c0: 1.50, 1.50, 1.21).

- **No cell is in its band but the stage's industrious share** (now 29.3 / 29.4, under 31; its
  set-offs are still over the cap). None left the band: none was in it before.
- **The stage moved most.** Its set-offs fell 0.2–0.6 a minute in every cell, its share 1–7 points
  in all but lazy, which rose about 3 points quiet (about 2 pooled σ: possibly real, possibly
  noise). The new still acts (homework and a book on the floor) draw her in a bare room, and the
  paper desk is one more piece to make and use. The share didn't rise toward the band's middle, so
  making ×3 is not the room-selective lever 8b hoped for: it took set-offs down with the share,
  not trips longer.
- **The home is within noise** (it has a desk and a bookshelf, so nothing new binds there but
  watching cross-legged, which moves nothing); the resident fell a point or so, set-offs 0.1–0.2.
- **The spread fell on the stage** (1.50 → 1.17): the floor acts draw a lazy Osaka as much as an
  industrious one with the levers neutral (lingering and settling, the mood levers, aren't on).
  **This is a regression on two of the user's criteria, not a neutral result:** the spread moved
  further from ≥ 1.6, and the stage's lazy share (24.9 quiet, c0 21.7) further over its 17 ceiling.
  Homework on the floor doesn't linger (its nod-off moves by mood instead, as at the desk), so with
  the levers neutral nothing in it separates the moods. No re-tune while the band is ignored, but
  8b's open stage question now has to recover this spread as well as reach the band.


## Step 10b: a borrowed line, measured (2026-10-06)

**Measured 2026-10-06** on step 10b's tree (before its commit): with no bookshelf and a line that
lends a strip, a read is a borrowed strip one decision in three (reading on her back the rest). The
levers are still `Stillness::NEUTRAL`. The band at full strength, as for 10a (190 visits a cell, 9
minutes, line art, about 6 minutes' wall). Moving % in sight / set-offs a minute in sight, with
step 10a's after each.

| Room | Mood | Quiet (10b) | Quiet (10a) | Chat (10b) | Chat (10a) | Band | Cap |
|---|---|---|---|---|---|---|---|
| stage | Lazy | 24.8 / 4.03 | 24.9 / 3.98 | 25.0 / 4.07 | 25.3 / 4.04 | 5–17 | 1.5 |
| stage | Ordinary | 27.0 / 4.40 | 27.7 / 4.33 | 28.7 / 4.54 | 28.5 / 4.49 | 8–26 | 2.25 |
| stage | Dreamy | 25.0 / 4.27 | 26.1 / 4.29 | 27.6 / 4.53 | 27.3 / 4.51 | 8–26 | 2.25 |
| stage | Industrious | 29.5 / 4.60 | 29.3 / 4.65 | 27.9 / 4.77 | 29.4 / 4.83 | 12–31 | 3.0 |
| home | every mood | as 10a | | as 10a | | | |
| resident | every mood | as 10a | | as 10a | | | |

Industrious ÷ lazy (both chats pooled): stage 1.15 (10a 1.17), home 1.56, resident 1.25.

- **The home and the resident are byte-for-byte 10a's numbers**: the home has a bookshelf, and the
  band's resident room has no line in her reach at all (`resident_room`: its text lies above her).
- **The stage is within noise** (every cell within about 1.5 points and 0.07 set-offs a minute of
  10a, under one pooled σ but the industrious chat share, 1.5 points down). Its dreamy quiet share
  (25.0) now reads inside its band, a noise-level move across the 26 ceiling. The borrow adds a walk
  where the book was read on the spot, but it is a third of a want that was seldom her pick; the
  set-off counts didn't move.
- No cell left its band (none was in it but the stage's industrious share, still in). Nothing
  re-tuned.
- **Its review fixes** (her grip held from taking hold to letting go; a key press judged by where
  the text is; the stage's placement putting the strip back the same frame) left every line of the
  band at full strength identical to the digit (re-run, 190 visits a cell): they move single frames,
  never what she chooses, in these rooms.


## Step 11: the window, measured (2026-10-06)

**Measured 2026-10-06** on step 11's tree (before its commit): the window hung low (bottom row the
floor − 2), leaning on its sill for 60–180 s with up to three musings on the sky, base 8 → 3. The
levers are still `Stillness::NEUTRAL` (no settling in: sitting in front of the window and watching
the clouds are built but inert).

**The band at full strength** (`CENSUS_BAND_SEEDS=190`, as for 10a): every line identical to step
10b's, digit for digit. None of the band's rooms owns a window.

**The day census** (`day_census`, 6 runs a room, a game week each; the home is the census home with a
clock and a window):

| | Before (step 10b) | Step 11 (base 3) | Base 6 (measured, not shipped) |
|---|---|---|---|
| Look-outs, home, 42 game days | 683 (about 16 a day) | 14 (one in three days) | 82 (about two a day) |
| Afternoon hour glances, home | 4 | 2 | 4 |
| Afternoon hour glances, stage / resident | 2 / 9 | 2 / 9 | 2 / 9 |

The design aimed at "a few a game day"; its base 3 gives one in three days, since a want at 3 seldom
makes the top four. The glance stays starved by its own gates (decisions.md, "Her window is a long
daydream").

**Printed beside the band** (new `printed_rooms`, `CENSUS_ROOMS=printed CENSUS_MINUTES=9
CENSUS_MODES=line fed_afternoon_census`, 20 sets × 4 visits): moving % in sight (σ of a 4-visit mean)
/ set-offs a minute in sight.

| Room | Mood | Quiet | Chat |
|---|---|---|---|
| home + window | Ordinary | 48.1 (6.33) / 2.28 | 43.0 (5.58) / 2.38 |
| home + window | Lazy | 35.7 (4.47) / 1.55 | 33.9 (5.73) / 1.73 |
| home + window | Industrious | 52.9 (4.83) / 2.65 | 51.2 (4.93) / 2.89 |
| home + window | Dreamy | 46.0 (4.71) / 2.28 | 42.3 (6.18) / 2.30 |
| home, TV only | Ordinary | 50.2 (3.80) / 1.86 | 47.5 (3.66) / 2.09 |
| home, TV only | Lazy | 45.2 (4.60) / 1.72 | 43.0 (4.39) / 1.93 |
| home, TV only | Industrious | 52.4 (2.89) / 1.97 | 50.0 (3.38) / 2.20 |
| home, TV only | Dreamy | 46.9 (3.36) / 1.70 | 42.1 (3.73) / 1.87 |
| resident, text low | Ordinary | 44.2 (3.92) / 3.19 | 42.2 (3.56) / 3.30 |
| resident, text low | Lazy | 39.4 (3.20) / 2.77 | 38.0 (3.40) / 2.91 |
| resident, text low | Industrious | 48.7 (2.59) / 3.32 | 44.6 (3.26) / 3.38 |
| resident, text low | Dreamy | 42.2 (2.20) / 3.07 | 39.7 (3.26) / 3.33 |

- **The windowed home reads as the band's home** (within noise in every cell): at base 3 no
  afternoon look-out shows in 640 visits' "where her time went"; the window adds nothing to her
  afternoon until its base or the tuning gives it room.
- **The TV-only home** moves about as much as the furnished one, with fewer set-offs (1.7–2.2 a
  minute against 1.5–2.9), so a set-off moves her longer there; lazy reads 45 (the furnished 34).
- **The resident with text low** (its chat's last ten rows, ending at one column, beside her floor):
  she pulls there (to text 11.8% in sight ordinary quiet), which adds 6–7 points and about 0.5
  set-offs a minute to the band's resident in every mood; industrious ÷ lazy is 1.21 pooled.
