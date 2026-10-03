# Phase 5a design critique: mechanics, sibling sites, determinism (D1–D3, D5–D6)

All line numbers are from the code as it stands now (osaka.rs unless noted). Where they differ from map.md, the code wins.

## Blocker

### B1. D1's `first_due` rule contradicts commit 1's "goldens unchanged"
D1 says `first_due` "returns the next key end (or the next bob frame), never a coarser frame". Commit 1 says goldens and seed 7 stay unchanged. Both cannot hold.

**The evidence chain:**
- The golden `drive` paints whenever `guest.advance(now)` is true (tests/golden.rs:114, 139).
- `advance` returns `tick`'s `changed` (mod.rs:647-676).
- `tick` sets `changed = true` on every due that fires (osaka.rs:1587-1627).
- Today a Use wakes only on the `USE_FRAME_MS` grid from `since`: `first_due` at 1485 and the `fire` re-arm at 1782-1783.

**What a key-end wakeup does:**
- Every key boundary off the 1400 ms grid adds a wakeup, so a frame is painted and a trace line is added.
- Off-grid boundaries today: Homework `len/2` and `len*3/4` (977), Watch 2/5 and 3/5 (985-986), Snack 1500, Pet 7/10, Crumple 4/5 (1000), Unpack 3/5 (1009).
- Worse, `arrived_within(&chats, now, step)` (golden.rs:172/230/274) makes chat arrival depend on the step sequence. An extra step moves when she notices chat, so behaviour changes, not just the trace.
- Concrete case: `golden_stage_room` cues Shopping at 170 s (golden.rs:167). A watch lasts 20-45 s, so its 2/5 boundary is almost never a multiple of 1400.

**The other half, independent of goldens:** if keys with a still pose stop waking on the grid, two things break:
- **The TV freezes.** `next_tick` has no TV wakeup. The channel frame `(now-since)/CHANNEL_FRAME_MS` (mod.rs:1027-1033) only advances when something repaints, and the Use grid is that something. During a watch (pose `Host`, no bob) the snow would freeze for 20-45 s.
- **The grievance is felt late.** The felt check `at >= from + GRIEVANCE_MS` (1747-1764) runs only on wakeups, and the overlay's end is never repainted.

**Fix:**
- In commit 1, `first_due` for Use is `min(next grid frame from the body start, until)`, exactly as today.
- Add key ends to that `min` only in a commit that re-records goldens anyway (commit 3).
- The grid wakeup never goes away, for any key.
- Fold `fire`'s duplicated re-arm (1782-1783) into `self.act_due = self.first_due(at)`, as Lift already does (1813-1815). Then there is one site.

## Major

### M1. `Span::Share` must be a cumulative end, not a summed duration
D1 says the player "generalises door_beat (cumulative ends)", and `door_beat` sums durations. D5 writes Shopping as `Share(2,5)` followed by `Share(1,5)`. Summed, the boundaries round differently from today's code:
- **Homework**, len = 30003: `len/2 + len/4` = 15001 + 7500 = 22501, but `len*3/4` = 22502 (977).
- **Watch**, len = 20004: `len*2/5 + len/5` = 8001 + 4000 = 12001, but `len*3/5` = 12002 (986).

Crumple and Unpack also use a **strict** `>`: `elapsed > length*4/5` (1000) and `> length*3/5` (1009). With end-exclusive keys, the frame at exactly `len*4/5` changes. A Crumple of length 5250 or 5251 puts that boundary at 4200, which is a grid frame. An Unpack of length 4667 or 4668 puts it at 2800, also a grid frame.

**Fix:** use `Span::{Ms(abs), Upto(n,d) /* end = body*n/d */, Until(fn(u64)->u64), Rest}`, all as cumulative ends, with the player clamping each end to the body length. Lint that the `Ms` ends are at most the shortest body, including trial lengths (`TRIAL_USE_MS` = 3500-5000, line 304).

### M2. Bob phase must be relative to the body start, not the key start
Today's frame is `elapsed/USE_FRAME_MS % 2` with `elapsed` measured from `since` (959).
- Snack's Eat key starts at 1500.
- Crumple and Unpack's `ToeTouch(frame)` runs across both of their keys.

A bob measured from the key start shifts every frame painted off the grid: speech ends, chat steps, pending ops. **Fix:** `Bob` is defined as relative to the body start.

### M3. With a prelude, three sites still measure from `since`
These must all measure from `body_start = since + before.len`:
- **The grievance timing.** `grievance_from(at, quiet)` at 2269 runs at `start_job` time, which is the prelude's start. The grievance overlay would cover the chopsticks bubbles, and its timing relative to the body would differ with and without the splice. That makes "splices never change what they wrap" false by construction.
- **The credit.** `credit_done`'s `span(*since, since + whole)` (2406) would credit prelude time as body.
- **The overlay offset.** `acting`'s `from.saturating_sub(since)` (4127).

**Fix:** derive `body_start` once and use it at all three sites, plus the grid in B1.

### M4. `Play` cannot hold what `appearance(now)` needs
`Play { own, branch, before, after, drawn: [u8;2] }` with `Spliced { splice, len }` is missing:
- the bought item that `Say::Pitch` reads (`advert` is going away);
- the chopsticks branch on the Before splice;
- the andagi count (5-7) on the After splice. Only `len` records it, so the player can't tell how many keys to play.

As drawn, appearance cannot be pure. **Fix:** `Spliced { splice, len, branch: u8 }` and `Play { …, bought: Option<Furniture> }`. Alternatively, make the Shopping script id carry the item.

### M5. `Act::SpaceOut { until }` has no `since`, and has three construction sites
A riddle script on SpaceOut needs elapsed time, and the act has no `since`. It is built in three places:
- `muse` (1534);
- Swap-back (1900, 1.5-3 s);
- `plan` (2916).

**Fix:** give it `SpaceOut { since, until, play: Option<Play> }`. Swap-back and plain spacing-out get `None`. `first_due` for SpaceOut stays `until` when there is no play.

### M6. The splice property, as specified, cannot pass at Guest level
"The same visit with them off … the needs after each use are the same" fails for two reasons:
- After the first wrapped use, the two runs diverge in timing. Chat arrives during different acts and later decisions differ, so there is no matching "each wrapped use" to compare.
- `choose_next` applies `needs.pass(at - decided)` (2702). A coda delays the next decision, so the needs differ even for the first use.

**Fix:** state it as a unit property over `start_job`. With the same `(seat, chances, rng state, at)`, forced on versus off:
- the rng state afterwards is identical;
- `whole`, the body length, the purchase and the events are equal;
- `appearance(body_start + t)` is equal for every t in the body;
- `credit_done(body_start + x)` gives the same share.

A Guest-level run compares only up to the end of the first wrapped use.

### M7. `Lines` budget and salt
`pick` gates on `self.said.len() >= LINE_BUDGET` (mind.rs:626). Pooled door lines and musings stored in `said` would use up the beat budget, which contradicts D4's "budget stays for beat lines only".

`below("which-line", …)` (mind.rs:639) also has a fixed salt of 0.

**Fix:** store `(pool, line, at)`, count the budget over beat pools only, and salt both the `"line"` and `"which-line"` rolls by pool. The beat pool gets id 0 so beat picks keep their rolls.

### M8. `ChatMark.asks` as a level breaks arrival detection
Arrival is detected as `mark != view.chat_mark` (mod.rs:1273). `ChatMark`'s own doc says history compaction changes the mark too (idle.rs:20-21).

With `asks` set as "the newest message ends in `?`":
- A compaction while the newest line is a question reads as a question, so she answers it.
- "Newest synced or IRC" is undefined when both sources have lines. app.rs:642-646 builds the mark statelessly from `view.chat.last()` and `irc_log.len()`.

**Fix:** use per-source flags, `synced_asks` and `irc_asks` (each the level of that source's last line). The guest decides "asks" only when the counter for that same source advanced.

### M9. The Answer path in `look()` skips `watch_x`
`look()` sets `watch_until`, clears `hopping`, drops a `mine` heading and resets the pending ops before the `props()` check (3587-3603). It sets `watch_x` only afterwards (3606).

An Answer that returns "before props()" therefore leaves `watch_x` stale. When the coda ends, `choose_next`'s "watching chat" Stand (2681-2685) faces the old x.

**Fix:** in the Answer branch, set `watch_x` and `facing = toward(chat_x)`. Keep the pending reset above it (mischief still goes back at once). Also say whether a post-coda watch is intended; `watch_until` is now + 15 s.

### M10. LampOff and FridgeOpen are not drawn in ASCII
ASCII mode renders only `Cat`, `CatBiting` and the TV screen (mod.rs:2326-2337). `state_parts` (art.rs:886-887) is line art only.

So D3's lint "every `Prop` in a script is drawn in both modes" fails on day one for Sleep and Snack. #70 (lamp off before bed) is also invisible in ASCII.

**Fix:** either add ASCII glyphs for both in commit 3, which re-records anyway, or scope the lint to props that have an ASCII form and record that #70 shows only in line art.

## Minor

- **`Key.hold` is unenforceable.** `Act::props(&self)` (697) has no `now` and classifies by variant, so a per-key hold is dead data. Drop it in 5a, or lint `hold == the host act's stays`.
- **D8 cueing a splice or riddle needs a forced slot.** The stage path is `place` + `pursue` (stage.rs:296/316/341), with no decision in between. A chance roll from stale whims won't reliably fire. Add `Osaka.cued: Option<SpliceId|ScriptId>` and consume it in `start_job`/`muse`.
- **The initial `self.whims` needs a defined value without a mind draw,** for example `Whims(mind.0 ^ SALT)`. `arrive_for_errand` (3804) to poke to `through_door` (1732) to the door arm says the door line before any decision has run. A constant would give the same line on every errand visit.
- **"An act that starts again can't splice twice" is only half true.** A use resumed after an interrupt goes through a new decision and rolls again. That's fine, but the doc should say so.
- **`Posed::Host` is defined only for Watch** (the sofa `Lounge` / `Sit` pose at 959-961). Lint that `Host` appears only in scripts hosted on Watch, or define a per-Use base pose.
- **The door can now be silent.** A pool of about 5 lines with a 10-minute cooldown, plus `pick`'s chance gate, allows a door exit with no line, where `THROUGH` (1717) was unconditional. Accept it explicitly or give the door pool a 1/1 chance.
- **`DOOR` doesn't fit `Key`.** `door_beat`'s `beat.ms.max(gap)` and its `door`/`her`/`there` fields don't map onto `Key`. Share a generic cumulative player, `at<T>(&[T], elapsed, end_of)`, rather than forcing `DoorBeat` into `Key`.
- **#70 on trial sits.** A trial Sleep lasts 3.5-5 s, so the 2 s lamp-on key makes the lamp flicker off. Skip the lamp key on trials.
- **Andagi ends 5-10 s after the eating.** `credit_done` serves at `until`, after the coda. The design accepts this, but it means a coda delays the need relief.
- **`HomeEvent::Used(id)` is pushed at `start_job`** (2276-2280), so a prelude interrupt still counts a made piece as used. It is moot for 5a, since no made piece is wrapped, but it should move to the body start if that ever changes.
- **`use_span` callers see the whole act, including splices:** the Crumple scrap stage (mod.rs:2056-2070), census `doing`, and the tests at tests.rs:5288 and 5295, which assert the grievance lands on the grid from `since`. All are safe under the "no splice wraps Crumple/Unpack" lint and because Lounge has no splice. Note it in the doc.
- **Splice scripts must use `Ms` spans only.** A share would scale with the wrapped body. Lint it.

## Sound (one line each)
- **D5 purchase:** it stays in `start_job` before `rng.range`, filtered on `!trying` and no grievance (2272-2282); unchanged and correct.
- **D2 events:** Unpack and Crumple fire at `until` (1765-1780). Correct given the no-wrap lint.
- **Grievance overlay:** staying on the Use is fine once its timing is relative to the body start (M3).
- **`recheck`, `lost_seat`, `seat`, `at_job` / `hands_row`:** all correct through a prelude or coda, since the spot and seat are the same and `SeatGone` in a coda credits the body in full.
- **Shopping TV:** `Prop::Tv(Shopping)` across the whole watch keeps today's hashed screen.

---

## Can every `use_look` arm be expressed as keys with zero extra draws and the same `appearance(now)`?
**Yes**, provided three things hold: spans are cumulative (M1), bob is relative to the body start (M2), and the wakeup grid stays (B1).

| Use | Keys (cumulative ends, body-relative) |
|---|---|
| Lounge | `Rest`: Lounge, Vacant, no bubble |
| Nap | `Rest`: Bob(Nap, 1400), Blink, Zzz |
| Sleep | `Rest`: Bob(Sleep, 1400), Blink, Zzz, `Prop::LampOff` (the 2 s lamp-on key comes in commit 3) |
| Homework | `Upto(1,2)` Bob(Homework, 1400) Vacant; `Upto(3,4)` Still(Homework(2)) Blink Dots; `Rest` Still(Homework(3)) Blink Zzz |
| Watch, plain | `Rest`: Host, Curious, `Tv(Snow)` |
| Watch, shopping | `Upto(2,5)` Host Curious Ooh; `Upto(3,5)` Host Happy `Pitch(bought)`; `Rest` Host Curious, all `Tv(Shopping)` |
| Read | `Rest`: Bob(Read, 1400), Vacant |
| Snack | `Ms(1500)` Still(Side) Curious `FridgeOpen`; `Rest` Bob(Eat, 1400) Happy |
| Pet | `Upto(7,10)` Still(Pet(0)) Happy Hum; `Rest` Still(Pet(1)) Surprised "Ow!" `CatBiting` |
| Crumple | `Until(\|l\| l*4/5+1)` Bob(ToeTouch) Happy SCRUNCH; `Rest` Bob(ToeTouch) Happy THERE |
| Unpack | `Until(\|l\| l*3/5+1)` Bob(ToeTouch) Happy, no bubble; `Rest` Bob(ToeTouch) Happy Ooh |

- **The grievance overlay** stays a post-pass over whichever key is playing: same pose, Curious face, grievance line, `from` relative to the body start.
- **Draws:** `own` is `Use::script()`, or `Shopping` when `advert` is set. No draw is added or removed.
- **Pet's `bite_at`** is exactly `Upto(7,10)`, so `Until` is needed only for the strict `>` arms.
- **Snack's `FRIDGE_OPEN_MS`** is `Ms(1500)`. A trial Snack (at least 3500 ms) is unaffected.
- **Timing:** see B1. The fridge closes and the bite shows on the next grid frame, as today.

## Paths to `start_job` and the door, and the whims each sees
**`start_job`** is reached only from `fire`'s Walk arm (2011); the test fixture at 4421 also calls it directly. A Walk with `Then::Job` is set from four kinds of place:
1. **`go_to` / `plan` inside `choose_next`**, including hops (each landing re-decides), the heading continuation and leftover. These see fresh whims.
2. **Tear's end calling `pursue(Crumple)`** (1878), with no decision in between. It sees the whims of the decision that started the build. That is harmless: Crumple can't be spliced and has no branch.
3. **Stage `direct` calling `place` + `pursue`** (stage.rs:296/316/341). It sees whatever whims were last set. At a cue right after arrival, that is the initial value.
4. **`lift()` (stage)** starts a Lift job, not a Use. Trial sits come via SetDown, Stand and `decide` (1812-1834), so they see a fresh decision.

**The door arm** (1705-1720): a Door act is set by `plan` (2940), leftover (3091), `find_rest` and `head_for_errand` (both in a decision), Poke's end (1732, using the errand decision's whims), `settle` (3899, 3929: stale), `errand()` redirecting a door or an Away, and `arrive_for_errand` (initial whims). The line is said before `decide`, so it always uses the previous decision's whims.

**Verdict:** deterministic on every path, because `Whims(self.mind.next())` is assigned at the top of `choose_next`, before every reflex return (2661). The whims are fresh except at stage cues, Tear to Crumple, and the initial value; fix those with the minors above.

## Act equality
Nothing compares `Act` values with `==`. Only `matches!` is used (1542, 1611, 2606, 3481), `recheck` compares spots, and `Then` and `Job` equality is unused against acts. `Act` derives `PartialEq, Eq`, so `Play` just needs `Clone, Copy, Debug, PartialEq, Eq` holding ids (`ScriptId`, `SpliceId`, `Furniture`, `u8`). Don't store `&'static Script`: its fn pointers would make the derived equality compare addresses. `act_name` is the Debug prefix, so it is unchanged.

## Body, prelude and coda credit: every site
- **`credit_done` (2406):** measure from the body start. A prelude interrupt then gives 0, the coda gives at least 1.0 (clamped), and trials are unaffected (no splices).
- **`whole`:** equals the body length when not a trial.
- **Trial sits:** keep the midpoint `whole`.
- **`lost_seat` and `recheck` (`Stays::Job`):** both fine.
- **`use_span` users:** `piece_state` is replaced by `prop`; the scrap stage at mod.rs:2056 is covered by the lint; census `doing` counts splice time as furniture time.
- **Unpack and Crumple events at `until`:** covered by the lint.
- **Restful beauty credit:** follows `done`.

## `Chat::Answer` against every chat path
- **`look()`:** M9. Use `OnChat::Look`, so `Back` and `Landed` never apply.
- **`ChatPassing`:** an Answer checked before the restful test answers even over text. The next `recheck` (`Restless`) interrupts anyway, which is acceptable.
- **Grumbling:** it hides speech, but a coda never overlaps the grievance window, which is in the body.
- **The resident's chat avoidance** (2838) only affects choosing, so it is unaffected.
- **Facing after the answer:** it stays turned to the chat for the rest of the coda. Specify whether she turns back; that needs a stored `answered_at`, not a change to `appearance`.

## Is `Prop` enough for today's `piece_state` (mod.rs:2199-2222)?
Yes, at most one Osaka-driven state at a time, with four caveats:
1. **Key the prop by Furniture kind** and apply it to every shown piece of that kind. The lamp is not the seat, and the TV's `Looks.tv` covers all TVs.
2. **Keep `CatBiting` under the `if cat` guard,** with the home `Cat` state as the fallback.
3. **`Prop::Tv` carries a channel without a frame,** and `prop(now)` resolves the frame from the use's `since` at 400 ms. The ASCII snow glyphs (mod.rs:2332-2333) depend on the frame and are hashed.
4. **The ASCII gap** in M10.

5b may need a small Vec for simultaneous props, such as the TV on while the lamp is off.