# Phase 5c baseline: how much she moves today

**Measured 2026-10-05/06**, step 3 (D1) of [phase5c-design.md](../phase5c-design.md), on the commit
that adds this file (behaviour as at `f29c482`, after D0a and D0b). Release builds, all runs
deterministic: the fed census run twice gave identical tables. Paths are relative to
`dessplay/src/ui/houseguest/`.

**The band's baseline is now [After the shorter watch](#after-the-shorter-watch-step-5-9-minute-visits)**
(step 5: the 5 s watch, 9-minute visits, at the end). The other tables are the 15 s watch (the fed
afternoon's over 15 minutes).

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
  (most of the home and resident 4–10 below quiet); now it's −3 to +3. The restarts barely move
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
