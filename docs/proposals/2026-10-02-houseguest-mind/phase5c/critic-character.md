# Phase 5c critique: character and attention

Critic KEY "character", 2026-10-05, against `phase5c-design.md` and HEAD `cabf5d1`. Paths are
relative to `dessplay/src/ui/houseguest/` unless they start with `docs/`. Every code claim was read
at the cited line. The lens: "watching her is kind of fun", and the eye is caught by change
(set-offs, pose flips, frame flips, bubbles), not by time spent.

## Verdict

The design's skeleton is right: mood acts through lengths and settling, the set-off cap closes the
"walk faster" loophole, and the pre-commits are class fixes. But as written it **adds change where it
means to remove it**. The biggest case: every chat line during a seated or lying act still stands
her up and lays her back down (D3). Three more: the tuning makes hops and doors the top answer to
restlessness, several of the lengthened "still" acts flicker (LieFront's kicks, the TV's static,
a frozen "ooh"), and daydream sessions make the lazy Osaka the chattiest one. One blocker, seven
majors, nine minors. All are fixable inside the user's decisions.

## Blocker

### B1. The look at chat stands her up, so every "resume" costs two big pose changes

- **What's wrong.** The look is a standing side view: `Act::Look` draws `Pose::Side` with `!` then
  `?` (osaka.rs:7297-7304), and `interrupt` replaces the act (osaka.rs:6508-6524). With D3 a chat
  line during Lounge, Nap, LieBack, Sit, Read, Watch or Homework goes: lying or seated → standing
  side-on (4 s) → standing (osaka.rs:7149-7158, the watch tail) → lying or seated again. Going
  between lying (1-2 rows) and standing (4 rows) is the biggest change her sprite makes. D4 makes
  exactly these acts 2-3× longer, so they catch more lines: at the stage's 45 s cadence
  (tests/census.rs:54) a lazy LieBack of 60-180 s pops up 1-4 times. Each pop is the eye-catching change
  this phase exists to remove. A lively chat that lasts past D3's 30 s window then sends her off
  afresh, often on a walk.
- **The repo already reacts in place three times.** The andagi answer turns her toward the chat,
  says its line and plays on (osaka.rs:6462-6470, drawn at 7200-7204). The grievance overlay puts a
  face and bubble on the running use (osaka.rs:7205-7210). The night's `stir` says "mm" over a
  turned frame (osaka.rs:4169-4173, 7130-7145). `Rig::for_pose` takes pose and face independently
  (art.rs:1309-1354; `sprite::cells` likewise, sprite.rs:328), so this needs no new art.
- **Amendment (how Q1 is built, not a challenge to it).** For a seated or lying still act, "she
  looks, then resumes" happens **on her current pose**. She turns toward the chat where the pose has
  a facing. She shows `Surprised` + `!` for `SURPRISED_MS`, then `Curious` + `?` until `LOOK_MS`,
  then holds a plain watch until `watch_until`. The act's clock keeps running, and nothing is cut.
  This still meets design.md:1393-1395 ("stops, turns toward the chat pane with a `!` then a `?`").
  Dozes (LieBack, Nap, day Sleep) get the night's stir instead, or a drowsy variant: `Blink` and
  "Mm?".
- **What this simplifies.** D3's `Resume` machinery is only needed for the standing still acts
  (SpaceOut, Gaze), which already look in a standing pose. The 30 s, 8 s and seat-still-stands
  preconditions, the credit scaling for seated acts, and the D5 conflict (m2) all go away. ASCII
  `LIE_BACK` has no face cell (map G13), so there it's bubble only. That's acceptable: the user's
  client draws line art.
- **For the user to decide knowingly.** If they want a visible stand-up for *awake* seated acts,
  that's their call. It should be chosen, not inherited from today's code.

## Major

### M1. The watch reflex comes before settling in, so a line near an act's end stands her up

- The reflex `at < watch_until → set(Act::Stand)` runs at osaka.rs:5495-5503, before the `leftover` call
  (osaka.rs:5627) and so before D4's settle-in. It replaces `self.act`. Settle-in needs the ended act
  to still be `self.act`.
- Result: a still act that ends on its own within 5 s of any chat line never settles in. She stands
  up side-on instead, and her next choice is a fresh roll. At a 45 s cadence that's about 1 natural
  end in 9.
- **Amendment.** For a still act that ended on its own, evaluate settle-in (and resume) before the
  watch reflex, and let the watch happen in the settled pose (as B1). Alternatively, the watch
  reflex keeps a seated or lying pose instead of `Act::Stand`. Test: a line 2 s before a Lounge's
  natural end still settles into Nap (at the settle odds).

### M2. After the tuning, Travel outscores Walk: hops and doors become the top answer to restlessness

- Walk goes 14 → 9 (design D4) and Travel stays at 10 (brain.rs:691-692). Score is
  `base × (FLOOR + need² × quality × amount × WEIGHT)` (brain.rs:792-816), with `quality` 1.0 for
  Restless (brain.rs:344).
- At Restless 1: **Walk 9 × 0.9 = 8.1, Travel 10 × 0.9 = 9.0** (the ratio holds at any level).
  Climbs, falls and doors are the most eye-catching onsets she has. On the stage, doors alone are
  10% of time (map G7).
- The set-off cap counts a door the same as a six-cell walk.
- **Amendment.** Travel's base moves with Walk and stays below it (e.g. 10 → 6). Put it in step
  3c's order of moves before the Walk base. Count doors (visible) apart in the baseline, and weigh a
  door ×2 in the set-off cap, or give doors a cap of their own per room.

### M3. Restlessness drifts toward exercise, which the band counts as still

- Jacks and ToeTouch are base 6, serving Restless 0.5 (brain.rs:685). Walk is 9 × 0.4 after the
  tuning. At Restless 1 they score 6.6 each, against 8.1 for Walk (12.6 today). So exercise's share
  of restless answers grows from about half to about 60%.
- Jacks flips frames every 450 ms with a Count bubble (osaka.rs:1229-1237, 1244).
- The user has settled that exercise counts as still. The design's job is to make sure the drift is
  seen, not hidden.
- **Amendment.** `phase5c/baseline.md` and the post-3c re-measure print exercise share **and
  exercise starts per in-sight minute**, per room × mood. Name a trigger: if exercise share rises
  more than about 3 points over the baseline, the numbers go back to the user before the band is
  pinned.

### M4. Lengthened "still" acts that flicker

- **LieFront** kicks every 500 ms with a Hum bubble (osaka.rs:1232, 1243). The design doubles its
  base to 20-50 s and scales it by linger: up to 75 s of 2 Hz kicking when lazy.
- **Watch** shows TV static the whole time: the snow is reshuffled every `CHANNEL_FRAME_MS` = 400 ms
  (script.rs:91; art.rs:1134-1137; spans at osaka.rs:7714-7718). The design takes it to 45-120 s
  × linger, so up to 3 minutes of 2.5 Hz static. In a TV home this would be the most eye-catching
  thing on screen.
- **Gaze** holds `Ooh` for the whole act (osaka.rs:1247). At 10-25 s × 1.5, a frozen "ooh" reads as
  stuck, not still.
- **Amendments.**
  - LieFront: either kept out of linger at today's length, or its kicks taper to a held frame after
    about 8 s ("absorbed").
  - The TV: snow only as it switches on and while she surfs, then a held programme picture. New art
    is fine; `Channel::ColourBars` is already a static picture (art.rs:1124-1129). Or a two-frame
    programme at `USE_FRAME_MS` or slower.
  - Gaze: `Ooh` for its first 3 s, then `Curious` with no bubble.
  - Add a test that no act longer than 30 s flips any of her cells faster than `USE_FRAME_MS`
    after its first 10 s, TV included.

### M5. Daydream sessions: the lazy Osaka muses most, rolls repeat, the pool runs dry

- **The mood is inverted.** `n` drawn 1-3 then multiplied by linger and rounded gives Lazy up to 5
  musings (3 × 1.5 = 4.5 rounds up) and Dreamy up to 4. Dreamy should muse most. Lazy should mostly
  stare.
- **Every musing gets the same roll.** `muse` rolls with the decision's fixed `Whims`
  (osaka.rs:2153-2240). `chance` is a pure hash of label and salt (mind.rs:57-68). So every musing in
  one session gets the same riddle-or-not answer: a session is all riddles or none.
  `Whims::series(label, k)` (mind.rs:74-76) is the existing fix, which sleep-talk already uses.
- **The pool runs dry.** `MUSINGS` has 8 lines (mind.rs:673-687) with a 10-minute cooldown
  (mind.rs:878, `pick` at 1062-1082). Two or three sessions use them all. After that the viewer has
  heard every musing in ten minutes, and the sessions go quiet.
- **Amendments.**
  - Musing count by mood: Dreamy 2-4, Ordinary 1-3, Lazy 0-2, Industrious 0-1.
  - Musing `k` rolls with `whims.series("daydream", k)`.
  - A small pool of **trains of thought**: 2-3 lines written together and said in order. The existing
    "Black spots on white?" / "Or white on black..." pair is the template; "That cloud's a bun." →
    "...a melon bun." → "I'm hungry." is another. A linked train is funnier than unrelated lines, and
    uses up fewer of them.
  - A global floor of about 12 s between any two lines she starts herself, whatever the source.
  - The censuses print **bubble onsets per in-sight minute** beside set-offs, so stillness can't be
    bought with chatter.

### M6. Linger shortens the industrious Osaka's homework

- `Mood::linger` gives Industrious 0.7 and Lazy 1.5 on Homework as on rest acts. An industrious
  Osaka should keep at her homework longest; a lazy one should give up on it soonest.
- Desk homework already nods off: `Dots` at ½, `Zzz` with `Homework(3)` at ¾ (osaka.rs:7703-7711).
- **Amendment.** Linger applies to the rest acts only (Sit, LieBack, Gaze, SpaceOut, Lounge, Nap,
  Watch, Read, LookOut). For homework, the mood moves the **nod-off point** instead: Lazy ⅓,
  Ordinary ½, Industrious ⅚, or never. Her mood then shows with no extra motion. Floor homework gets
  the same arc, ending in the face-down doze the art sheet already lists.

### M7. Nearer wander targets make walks shorter, not rarer

- `walk` is uniform over her floor today (mind.rs:253-263). The weight `1/(1+|dx|/8)` shortens
  wanders without lowering set-offs: it's the time-share gaming that map G6 warned about. The
  design's own cap will reject it.
- It also turns ambles into 3-8-cell shuffles, which read as fidgeting.
- **Amendment.** Drop the distance weight for `walk`. Keep it for `place`, `pull` and `swap`, where a
  near job replaces a far one, and often a door or a climb.

## Minor

- **m1. The watch tail.** After the 4 s look, a 1 s plain `Stand` (`Side`, no bubble) runs to
  `watch_until` (osaka.rs:5495-5503, 7149-7158), then she resumes: a pointless extra frame change.
  If B1 isn't taken, extend the look to `watch_until` when less than 2 s would be left.
- **m2. D3 and D5 contradict each other on Read.** D3 resumes a cut Read; D5 mends a cut borrowed
  strip at once. B1 removes the conflict. Otherwise, exclude `use/borrow` from resume.
- **m3. Where she reads a borrowed line.** She should read **beside the tear**, so sliding it back
  costs no second walk (one set-off per read; HG #72, docs/proposals/2026-09-28-houseguest.md:560-561). The line should come from her own floor, nearest
  first, like the pull and swap preference.
- **m4. Floor homework's frames and line.** State its frame period: `USE_FRAME_MS`, as desk homework
  bobs (osaka.rs:1161, 7704), not LieFront's 500 ms. "My back..." should come once a visit, when
  `ached` is first set; after that it ends quietly. Otherwise a bare room's homework slot (×3)
  repeats the line every time.
- **m5. Cloud-watching.** Specify it as a held pose: period 0, eyes open (LieBack otherwise flips
  every 1400 ms, osaka.rs:1231, 1242). Give it one static thought bubble and at most one line. A
  small new bubble with a cloud in it is cheap art, and ties back to "That cloud's a bun."
- **m6. Sky musings.** `LOOK_OUT_LINES` has two lines a sky (three by day; script.rs:1703-1715). A Day
  sky lasts about 100 real minutes. With up to three musings a session, the new pool needs four or
  more per sky, or some can be trains (M5).
  - Optional payoff: once a session, the window's bird moves when she says "A bird!" One onset, and
    it's explained.
  - A lazy settle at the window could be dozing with her head on her arms on the sill.
- **m7. A lazy Osaka spacing out on her feet.** SpaceOut is drawn standing with `Dots`
  (osaka.rs:7236). At 20-60 s × 1.5 that's 90 s on her feet. Where her spot is restful, a lazy
  session could start seated (`Sit` + `Dots`), which also shortens the chain to LieBack.
- **m8. Held poses never blink.** Only `Stand` blinks (osaka.rs:2299, 2721-2723, every 4-9 s). Sit,
  Lounge, cross-legged and a long Gaze hold one frame for minutes, which reads as frozen. Proposal:
  a 150 ms blink every 6-12 s on held poses, the smallest sign she's alive. Ask the user first: it
  moves the goldens.
- **m9. Settle-in chains always go the same way.** SpaceOut → Sit → LieBack every time will show
  itself over a two-hour lazy session. A sitting doze on the art sheet (head nodding over her knees;
  map G13 says there's none) would give Sit a second branch: Lazy or Ordinary dozes sitting, Dreamy
  lies back to cloud-watch.

## Right as designed (keep)

- D0a's `tend_fades` and D0b's credit-class test. Both are class fixes with tests that fail first.
- Q3's set-off cap beside the share. The fed afternoon in both chat conditions, with the mood
  asserted, not set silently.
- Mood through lengths and settling in, with no walking factor (`sane_factor` stands).
- Settle-in binds in place only, after `leftover` and the routine, is credited as its own want, and
  doesn't enter `recent`.
- Resume credit scaled by `left ÷ whole`, and the rule (design.md + decisions.md) that a
  continuation skips the roll.
- Pull and swap on her own floor first: the stage's doors and hops (map G7).
- LookOut rarer and longer, with 5b's D4 (Gaze × window) dropped. The window becomes a place, not a
  twitch.
- `ached` → the paper desk: a visible cause and effect ("my back...", then she builds one), the kind
  of thing that makes watching her fun. Its build burst buys long stillness at the desk afterwards.
- Explicit `MAKES` arms for the desk (no sofa fallback) and the "...my desk." loss line.
- Cross-legged TV as art only; the binding already works (map §8).
