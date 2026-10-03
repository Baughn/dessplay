## Phase 5a critique: character, fun and scope

### Blocker

**B1. Commit 1's promise that the goldens stay unchanged contradicts D1's `first_due`.** This is outside my lens, but it is verified.
- D1 says `first_due` "returns the next key end … never a coarser frame". Today a Use re-arms only on the 1400 ms grid (osaka.rs:1484-1486, 1786).
- `tick` returns `changed = true` whenever a due passes (osaka.rs:1590-1596). `advance` passes that on (mod.rs:668-674). `drive` hashes a frame on every step where `advance` is true (tests/golden.rs:139-145).
- Several key boundaries fall off that grid: Homework at `length/2`, Snack at `FRIDGE_OPEN_MS` = 1500, Pet at `bite_at` = 7/10, and the shopping channel at 2/5 and 3/5. Each one becomes a new hashed frame, and the switch now shows on time instead of at the next frame. So the "refactor proof" fails by construction.
- **Fix:** in commit 1, plain converted scripts keep the frame-grid due (key ends rounded up to the next `USE_FRAME_MS` multiple). The on-time key-end due switches on in commit 3, which re-records the goldens anyway.
- **Also:** `Span::Share(n, d)` must mean "ends at body×n/d from the body start" (cumulative), not a duration. D5 writes the shopping keys as successive durations, `Share(2,5)` then `Share(1,5)`. At length 20004, `L*2/5 + L/5` = 12001 but `L*3/5` = 12002 (the code uses the latter, osaka.rs:983-989).
- `bite_at` is exactly `length * 7 / 10` (osaka.rs:911-913), so `Share(7,10)` covers Pet, and `Span::Until(fn)` is scope that can go.

### Major

**M1. A grievance buries the chopsticks prelude.**
- `start_job` times the grievance from the act start: `grievance_from(at, quiet)` (osaka.rs:2267-2268). That is at least one frame (1.4 s) in.
- Homework feels the lamp rule ("Too dark in here...", rules.rs:71-78), and an unsettled piece is felt on any use.
- With a 4–5 s chopsticks prelude, she cranes round mid-split and says her grievance over it. D1 says the overlay "overlays whatever key is playing", so this is the design as written.
- **Fix:** time the grievance from the body start (`since + before.len`), consistent with D2's "the body is the use proper".

**M2. Pending speech hides a script's first key, so a riddle can show its punchline with no setup.**
- Speech overrides bubbles (`speech.or(bubble)`, osaka.rs:4093). Three paths speak just before a host starts:
  - the door says `THROUGH` and then `decide`s (osaka.rs:1716-1717);
  - `AH_RIGHT` is said just before a `Job::Use` returns (osaka.rs:3020-3022);
  - `muse` speaks via `self.say` (osaka.rs:1530).
- Effects:
  - A riddle chosen right after a door shows the answer with the question hidden.
  - A bad chopsticks split loses "Hold 'em by the ends!" and leaves an unexplained droop.
- **Fix:** the player starts the keys at `quiet`, as `grievance_from` already does (osaka.rs:941-944). Store the offset in `Play` (and on the SpaceOut host) when the act starts, so `appearance` stays pure.

**M3. Sata andagi as designed won't read as "5–7 times, happier each time".**
- **Same text back to back reads as one bubble.** Keys with identical text and no gap look like one long bubble. The census agrees: `said` counts only a change of text (tests/census.rs:166-175), so D8's "lines said" row would count the andagi once.
  - **Fix:** each key is about 1.9 s of "Sata andagi." (that is `speech_ms`), then a 0.5–0.8 s quiet beat with a small bob or `Hum`. Six keys come to about 15 s.
  - That is twice the snack it follows (6–9 s, osaka.rs:925). The user said "a few times", so 4–6 fits better than the catalogue's 5–7. That count is the design's choice, not the user's.
- **Wrong face in the ramp.** Vacant → Curious → Happy does not read as "happier": Curious is `'o'`, looking up, which reads as a question (sprite.rs:85-99). **Fix:** Vacant → Pleased (`^_^`) → Happy (`^o^`).
- **Her pose and where the andagi comes from are unspecified.** The body ends on melon bread, and an andagi has to appear from somewhere. **Fix:**
  1. Open the coda with a ~1 s `Prop::FridgeOpen` key (already in D3, no new art).
  2. Hold the andagi up through the saying keys: `Still(Eat { frame: 0, food: Andagi })`.
  3. Bite on the last two keys.
- **The answer should look different from the keys.** It has the same text as the keys, so turning toward the chat is the only visual difference. Give it `Happy`, start it only in a quiet gap, and turn back afterwards.
- **Optional:** a question arriving in a roughly 15 s window that happens on 1 snack in 4 will be very rare. If you want it seen, raise the coda's chance in `SpliceCtx` when chat was active in the last minute or two. This is a chance only; the behaviour stays the user's.

**M4. The census can't see two of the four new vignettes.**
- `furnished_room` owns Sofa, TV, Bed, Desk and Bookshelf (tests/census.rs:48-56). The resident owns Sofa and TV. No room has a Fridge or a Lamp.
- My release run of `visit_census` shows no `Use(Snack)` in any room.
- So D8's andagi and lamp-off rows would read 0, and the 1-in-4 andagi chance can't be checked.
- **Fix:** add Fridge and Lamp to the home room, or add a kitchen room. The home's baseline numbers move; re-pin with the reason.

**M5. Channel surfing gets repetitive in the resident's room.**
- Census, choices per visit: resident Watch 16.2% of 77 ≈ 12.5 watches; home 10.5% of 49 ≈ 5.1.
- At 1 in 4 that is about 3 identical surfs a visit for a resident: snow → bars → snow → sunrise → snow every time.
- **Fix:**
  - Keep a per-script cooldown in the same store as lines (D4 already makes `Lines` the one store): no script twice in 10 minutes. This also bounds chopsticks and andagi.
  - Drop the chance to 1 in 5.
  - Add two `branch` variants, for example one that lingers on the sunrise.
- **Make the joke land:** `Ooh` on the sunrise, then `Rest` on snow with `Happy` + `Hum`. She flips past the pretty picture and is content with static.
- **Use the advert's own filter:** surfing should also need `Watch && grievance.is_none() && !trying` (osaka.rs:2272-2274).
  - A trial watch lasts 3.5–5 s (osaka.rs:304) and cuts a surf off.
  - A "Can't see the telly..." grievance overlays the surf.

**M6. The door pool needs a chance gate and better lines.**
- The census heard "Where was I?" 467 times in 16 stage visits, about once a minute (plan.md:2316-2319 says the same). At home and in the resident's room it was 0.
- Five lines with a 10-minute cooldown and no gate go in bursts: five doors talk, then about five are silent, and repeat.
- **Fix:** gate at 1 in 3 through `pick`'s `(n, d)`, which works out to about 8–10 door lines in a 30-minute stage visit.
- Two of the design's example lines don't fit:
  - "Back again!" — she has arrived somewhere new.
  - "Hm? Oh, right." — collides with `AH_RIGHT` "Ah, right!" (mind.rs:597), which the same `decide` can say a moment later.
- Replacement lines are at the end.

### Minor

- **m1. The beat budget trap.** `pick` gates on `self.said.len() >= LINE_BUDGET` (mind.rs:625). If the door and musing pools share `said`, the stage's door lines use up the 8 beat lines within minutes. D4 says the budget is for beats only, so state that beats are counted separately.
- **m2. Riddles need a fixed host and a frequency.**
  - Only the `Here::Muse` binding may carry a riddle. Its SpaceOut is 6–14 s (osaka.rs:1534), which fits 3 + 2.5 s. The SpaceOut after a swap-back is 1.5–3 s (osaka.rs:1900-1903), which doesn't.
  - The design gives no riddle share. Musings are a third of spacing-outs (mind.rs:210-212), and SpaceOut is about 2.2 / 4.8 / 9.2 a visit (home / resident / stage).
  - Proposed: a riddle on 1 musing in 3. That is about one every four home visits, one every two resident visits, and one per stage visit, which suits [U].
  - **Reading the gag:** the catalogue has her answer instantly. When she sets the riddle herself, the instant answer only reads with the timing right:
    - the question with a `Curious` face looking up, as if remembering one she heard;
    - the answer with no gap, `Happy`;
    - then `Hehe` instead of plain dots.
- **m3. Chopsticks spans and the sparkle are unspecified.**
  - Proposed spans: joined ~1 s, split ~0.8 s, result ~2.5 s (that is `speech_ms` of "Hold 'em by the ends!", 21 characters). Branches even.
  - `Bubble` has no sparkle (osaka.rs:1087-1101), and a lone glyph barely reads at her size. Use a single-width `*`, then `Hehe` with `Happy`. Avoid emoji, which are wide.
- **m4. Lamp off (#70).** The silent 2 s `Dots` key is right. Sleep is 14% of home choices (about 7 a visit), so any spoken line there would grate. Sound as designed.
- **m5. design.md text needs amending:**
  - 1389: the chat reaction (`!` then `?`, 15 s watch) needs an andagi `Answer` exception, like the job's "doesn't fetch her".
  - 1377 and 1803: "Where was I?" becomes a pool.
  - 1501: the beat cooldown and budget need restating for pools.
  - 1512: a musing on a third of spacing-outs, some now riddles.
  - 1620: the snack may end in an andagi.
  - 1622: the lamp is dark from 2 s into her sleep.
  - Plus a line for channel surfing.
- **m6. The doors in one trip share a roll.** The whims label for the door line is the same for every door in one trip, because heading hops don't decide anew. So the doors of a trip are all silent or all talk. That's acceptable; to vary it, salt with a per-visit door count.

### Sections that are sound
- **D1 hosts and the grievance overlay:** sound, given M1.
- **D2 Play, body and credit:** sound.
- **D3 Props:** sound. Monochrome `currentColor` colour bars still read as a test card.
- **D5 the purchase commits when the channel comes on:** sound and matches design.md:1542-1547.
- **D6:** only Before/After Snack/Homework rows, and the lint barring Crumple and Unpack, are sound.
- **D7 art on a model sheet first:** sound.
- **D8 stage cues:** sound.
- **Scope creep:** none beyond `Span::Until` (B1).

### Dropping Adverb, and no free-standing `Act::Script`
- **Adverb:** dropping it from 5a is sound, because no 5a vignette needs one. But neither the 5a nor the 5b decision list mentions it, so the proposal's row-5 type (proposal:615, 634) disappears without a record. Record in plan.md that it is deferred and why: moods already set rates.
  - A zero-cost "adverb-lite": put her mood in `SpliceCtx` and let rows scale their chance by mood (dreamy: riddles and andagi ×2; lazy: a bad chopsticks split 2 in 3).
- **No free-standing `Act::Script`:** sound for 5a. Every vignette sits at a piece or on Muse, and SpaceOut gives a riddle the right credit, census group and `Stays::Rest`.
  - 5b's dash-ins (#46, #47) and calendar arrivals are act-level and will need one, or scripts hosted on Out/Away/Home. Protect that now at low cost:
    - keep the player host-agnostic: `at(keys, elapsed, body: Option<u64>)`;
    - lint free-standing scripts to `Ms` spans only, with no `Share`, `Rest` or `Host`;
    - make census `group` exhaustive now (tests/census.rs:267-281 falls back to `"standing"`).

### Riddles (riddle, answer), each line ≤ 24 characters (lengths checked)
1. "Bread ya can't eat?" (19) → "A fryin' pan!" (13)
2. "Which animal's bread?" (21) → "The pan-da!" (11)
3. "What has keys, no locks?" (24) → "A keyboard!" (11)
4. "Has a neck but no head?" (23) → "A bottle!" (9)
5. "All holes, holds water?" (23) → "A sponge!" (9)
6. "Goes up, never down?" (20) → "Yer age!" (8)
7. "What never comes today?" (23) → "Tomorrow!" (9)

### Door lines, each ≤ 24 characters
1. "Where was I?" (12)
2. "Huh? How'd I get here?" (22)
3. "...What was I doin'?" (20)
4. "I forgot what I forgot." (23)
5. "Handy, these doors." (19)

The census numbers above come from my release run of `visit_census`. Its output is in /tmp/claude-1000/-home-svein-dev-dessplay/da610644-cf25-4acd-83f6-9d40e9435e69/scratchpad/census-critic.txt.