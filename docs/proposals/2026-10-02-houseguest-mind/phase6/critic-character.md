# Critique: her character and watchability (phase 6a working design)

Lens: what a viewer of an idle overlay will **see** of 6a, at terminal scale and at a glance, judged against "watching
her is kind of fun", her character (spacey, slow, earnest) and 5c's stillness rule (movement draws the eye; tune for
stillness). Checked against plan.md "Phase 6 — structures (brief, 2026-10-10)" (not relitigated), the approved gremlin
sheet (`phase6/art/gremlin-1x-nn3x.png`, `snippets.md`), and the code at `ad7f4af7`. Paths are relative to
`dessplay/src/ui/houseguest/` unless noted. Checked as claimed: `BUBBLE_CHARS` = 24 (mod.rs:18) and `line!` asserts it
at compile time (mod.rs:102-114); `Pose::Stretch` is front-on, arms up, `bob: -2` (art.rs:283) and its activity look is
`Face::Blink` with the `"nnn~"` bubble (osaka.rs:2153, 2210); `Pose::Gaze` is in profile with a small head tilt
(art.rs:318), looked as `Face::Curious` ("looking up, a small o", sprite.rs:275) with `"ooh"` for its first 3 s
(osaka.rs:2154, 2175); `Act::Lift`/`Act::SetDown` draw `Pose::ToeTouch`, bent to the floor (osaka.rs:11246-11249);
`Act::Admire` already exists (Stand, `Face::Pleased`, `"hehe"`, osaka.rs:11363) and follows a crumple (osaka.rs:4832)
and a finished pull (7467); `use_duration` (osaka.rs:1780): Unpack 4-6 s, LookOut 60-180 s, Lounge 27-60 s, and
`SPACE_OUT_MS` 10-28 s (osaka.rs:555); `on_sight` (mod.rs:3642) owes a line wherever she is in sight, gated only by
`said`/`today`; a prop's `facing` is saved per piece (ledger.rs:480); `advert` (mod.rs:194) and `CATALOGUE`
(mod.rs:169, 10 pieces after the TV); the bundled layout at 100×30 measured off `door/shots/out-day.png`: The List 9
rows, Users 8, Playlist 8 (panes take H − 5 rows, split 34/33/33 by style.css:14-16).

**Stillness, in one paragraph.** The design respects 5c: building is a 6 s still act once a game day, twice in a
shelf's life; the gaze up is a still use reached by a short walk; nothing she puts up pushes a hung piece; and a
shelf's evidence (half up, finished, the gremlin on it) is **persistent state**, which is exactly what an
occasionally-glanced overlay needs. The risk in 6a is not movement but talk (finding 2) and legibility (3-5).

**Her lines.** All four approved lines fit: `"A shelf! Up high!"` 17, `"Aww, a little gremlin!"` 22, `"It wants a
shelf..."` 19, `"Up it goes..."` 13, ASCII only; overflow is unrepresentable (`line!`). Every line suggested below was
counted and fits. They're in her voice (the trailing `...`, the earnest exclamations of her other pitches,
room.rs:184-343).

## Major

1. **D3 / D11 / step 3: who will ever see a shelf.** Shelves want the *top* of a pane; the app's text lives at the
   top of a pane. In a 9-row pane (7 inside) a low shelf hangs on interior row 3 from the top (`f - 5`), its
   `LOW_ABOVE` rows are interior rows 1-2: the very rows the pane's first three entries are drawn on. A high shelf in
   a 13-row pane (11 inside) likewise needs interior rows 1-7. Only panes taller than that leave text any rows of its
   own above the shelf. With the bundled split (panes = H − 5, 34/33/33), by my arithmetic (rounding unconfirmed):
   The List takes a low shelf from H = 30, Users and Playlist from about H = 32; a high shelf from about H = 43 (List)
   and 45 (Users, Playlist). But **content** decides more than geometry: The List's entries carry name, episode,
   availability and watchers across the row (`templates/collections.xml:61-65`), and any real group has more than
   seven franchises, so The List (the stage room's only candidate, D11) will almost never hold one in use; the
   Playlist's right-aligned download and watch columns (style.css:36-37) and long titles make it a weak host; **Users**
   (short left-aligned names, collections.xml:84-90) is the realistic host, and only at about 32 rows or more. So at
   the design's reference 100×30 a real user sees no shelf, ever; two rows taller, probably one in Users.
   **Amend:** step 3's measurement reports, per terminal height (30, 32, 35, 40, 50) and with typical content (3-5
   users, ~10 playlist rows, ~20 franchises: `chatty_ui` or a fuller fixture), which pane takes a shelf and which
   height, not geometry alone; the design says plainly that Users is the realistic host; and ask the user what size
   their and their friends' terminals usually are, so the census rooms (`shelf_room`, `low_shelf_room`) are rooms
   that occur.

2. **D7 / D9: a gremlin that can never get its shelf nags forever.** The kit is sold only `while shelf_room` (D7),
   but the gremlin sells in the decor tier with no such gate (D9), right after the poster. On a terminal where no
   shelf can go (100×30 with a populated List, finding 1), `High(Gremlin)` is broken for good, `on_sight` owes
   `"It wants a shelf..."` once a game day (every ~4 real hours at 6×) whenever her mood would mend, the wish
   names a kit the channel never sells, and *nesting* rises every visit for a mend that cannot exist. It's the door
   critique's #6 class (on-sight nagging), worse, because no repair is possible. The proposal itself had the gremlin
   "on a shelf she built, once shelves exist" (2026-10-02-houseguest-mind.md:713), and the sheet's `snippets.md:126`
   ("gated on shelves existing"). **Amend** (keeping the user's "the gremlin may stand on a floor"): sell the gremlin
   only on a frame with `shelf_room` (as the kit), so the story it starts can finish; and `High` is felt only while
   a shelf stands or `shelf_room` holds (a resize that takes the room away doesn't leave her grieving). Have
   `shelf_census` count her `"It wants a shelf..."` lines between the gremlin's delivery and it standing on the shelf.
   Test: `no_gremlin_grievance_where_no_shelf_can_go` (a home with the gremlin on the floor, every pane full of text,
   a day-long fed run, both modes: the line never said, `Arrange` never offered for it).

3. **D7 / D6 / D9: a high shelf is put up, and loaded, from well out of her reach.** Her box is rows `f-4 ..= f-1`;
   `Pose::Stretch` keeps her hands inside it, so her fingertips top out at row `f-4`. A low shelf (`f - 5`) is just
   over them: that build reads. A high shelf (`f - 7` at its lowest) has two empty rows, ~40 px, between her raised
   hands and it, and she "puts it up" there. Worse, the gremlin's carry (D6, "she lifts and sets down standing on the
   floor under it") draws `Act::SetDown` as `Pose::ToeTouch`, **bent to the floor**, while the figurine appears on a
   shelf three rows over her head. A viewer sees a magic trick, and it undercuts 6b's premise (she needs a ladder
   because she can't reach up there; yet she put a shelf and a gremlin there). **Amend:** (a) `SetDown`/`Lift` whose
   target strip is a shelf draw `Pose::Stretch` (arms up, `Face::Curious`), never `ToeTouch`, at both heights; (b)
   for a high shelf, put to the user before step 5 how a stage and a set-down reach it, with the cheap options: a
   little hop (her image drawn a row up on alternate beats, `"Hup! Hup!"` 9, cyclic, in place, a few seconds twice in
   a shelf's life and once a carry: far inside the band), or let it be her magic and say so in decisions.md; (c) for
   6b's brief, not 6a: standing on the kit's own box to reach is the ladder's natural prequel. Test (with (a)):
   `she_reaches_up_to_set_a_piece_on_her_shelf` (the set-down's pose on a shelf is never `ToeTouch`, both modes).

4. **D3 / D4: a shelf among the UI's text reads as the UI's.** The per-cell test asks only that the run's own cells
   (and the rows above, at choice) be blank, so a run may hang in the blank middle of rows holding text: between a
   user's name and the next column, or in a playlist row between the title and its right-aligned percentage. In the
   unfocused-border grey (`SHELF_INK`), ten `─` inline with a table's text read as a rule, a separator, or a render
   glitch, not as a shelf she put up; with Users the realistic host (finding 1) this is the common case, not the
   corner. The conceit ("structures from the UI's own lines") lands only if the shelf stands apart from the UI's
   tables. **Amend:** the per-cell test extends a margin of 2 blank cells past each end of the run (the run's row
   and the decor's rows); and at choice, prefer (score first) spots whose run row holds no text anywhere in the pane
   interior, falling back to the margin rule. Test (property, in `read_ground_takes_only_blank_free_cells`'s
   frames): every shown run has 2 blank cells beyond each end on its row.

5. **D8: the gaze up reads as her idle daydream, aimed at nothing.** `Use::Admire` is `Pose::Gaze` held, no line:
   the same pose and face (`Face::Curious`) as the idle `Gaze` she does anywhere (minus its `"ooh"`), so nothing on
   screen says *admiring her shelf*. And its first spot is "under its middle": `Gaze` is in profile with a small head
   tilt, so from directly under the gremlin she looks sideways at the wall, not up at it. **Amend:** spots beside it
   first (her box's near edge one or two columns past the piece's, still under the run), facing it, so her profile
   points at it; a bubble that says *mine* at its start: `"hehe"` (`Bubble::Hehe`, as `Act::Admire` has) for 2-3 s,
   then `Face::Curious` held; and a small cooled pool of lines (idea 17). Test: `she_gazes_up_at_what_is_on_her_shelf`
   asserts the spot is beside, facing it, and the opening bubble.

## Minor

6. **D7: the kit vanishes into the air when she unpacks it.** On `Unpacked(ShelfKit)` the prop goes and the shelf
   comes at `built: 0`; as I read D3/D4 the drawn run is the built part, so nothing is drawn at 0. The viewer sees
   her open a box (`"ooh"`), the box disappear, and nothing in its place, possibly for hours (`Want::Build` must win
   a roll; D7's "half up that day" is not guaranteed). For every other piece, unpacking shows the piece. **Amend**,
   either: (a) chain the first stage onto the unpack, as a crumple chains to `Act::Admire` and then a use
   (osaka.rs:4826-4832): she unpacks, walks to the spot, `"Up it goes..."`, the first half appears, all in one
   sitting; or (b) draw `built: 0` as a marked outline (`┈` light dashes in `SHELF_INK`, pencil marks on the wall:
   not a ledge of hers anyway in 6a; dashed glyphs aren't redrawn by her line art, graphics.rs:307-309, harmless
   while her box never meets a shelf, a 6b note). (a) is less code and the stronger beat.

7. **D7: the build looks like an idle stretch.** `Act::PutUp` holds `Pose::Stretch`; its existing look is eyes shut
   (`Face::Blink`) with `"nnn~"`, the morning stretch. With `"Up it goes..."` showing ~2 s (`speech_ms`,
   osaka.rs:574) and then 4 s of a plain stretch, the shelf pops at the end with the cause off screen. **Amend:**
   specify the act's look: `Face::Curious` (looking up), no `"nnn~"`; and if cheap, write the stage's cells over its
   6 s (a cell a second from the wall end) rather than all at the end, so the line grows under her hands. A one-off
   change tied to her act draws the eye to *her*, which is the point; it happens twice in a shelf's life.

8. **D9: `"It wants a shelf..."` has no referent, and outlives its meaning.** `on_sight` owes it wherever she is in
   sight; said in Playlist with the gremlin on Users' floor, "it" points at nothing a viewer can find. And after the
   shelf is finished, on a day her mood won't carry the gremlin up, she says it with a shelf standing right there.
   **Amend:** owe the line only while she stands in the gremlin's room (`strip.room()` of hers and its), and say it
   turned to face it (as a grievance faces its piece); after a shelf is up, say it only on a visit she'll mend, so
   the carry follows the line (the door batch's `to_mend` term already holds it; state that it does). A second line
   for the shelf-is-up case ("Almost, little guy..." 21) is the user's call, not a requirement.

9. **D8 / Census: the gaze up should be as long and as local as 5c's still acts.** 8-16 s is short beside
   spacing out (10-28 s), lounging (27-60 s) or looking out (60-180 s); a walk then a short gaze then a walk raises
   set-offs a minute, the band's other cap. And nothing restricts it to her own floor: "a short walk on her own floor"
   (D8) is an expectation, not a rule, so a shelf in Users can call her up a climb from Playlist (climbs are the
   eye-catching movement). **Amend:** 20-45 s, letting 5c's settling-in apply; offer it only when she stands on the
   shelf's room floor; leave the clock out of `Use::Admire` (her 5b clock glance already looks at it; a mantel clock
   gazed at for 30 s reads as clock-watching); and run the window's look-out count (the user's 2-6 a game day) in
   `shelf_room`'s census, since Admire's *daydreams* term competes with `LookOut`.

10. **D3: a high shelf right over a hung piece reads as its frame.** Rise 7 puts the run on the row just above the
    wall lane, so over a poster or the clock it touches their top edge: the poster looks hung from it, or framed.
    **Amend:** among lefts at a rise, prefer runs that clear every laid hung piece's columns, else take rise 8 there.

11. **D6: a lone gremlin should stand mid-shelf.** In practice only the gremlin goes up (the plant and the clock only
    by a rare repair), so a 10-wide run holds a 2-wide figurine wherever the search's packing puts it, often at an
    end. A mantel ornament stands in the middle. **Amend:** the repair's candidate order on a shelf tries the run's
    middle first.

12. **D11: the user's review path can't show 6a.** The stage room is 100×30 and its List is the only candidate; the
    example "says so when the terminal is too short". The door batch ended with real frames for review
    (`door/shots/`); 6a has none planned. **Amend:** the three shelf scenes run on a taller stage frame (one that
    takes a high shelf in Users) whatever the user's terminal, and step 8 writes review shots as the door batch's
    `door_shots` did: the kit unpacked, each stage, the half shelf, the carry up, the gaze up, a low shelf at 32 rows
    and a high one at 45.

13. **D8 naming: `Use::Admire` beside `Act::Admire`.** `Act::Admire` is her pleased beat after making or pulling
    something (osaka.rs:4832, 7467, 11363); a use of the same name for a different, long, still act will confuse
    every trace and review. **Amend:** name the use for what she does: `Use::GazeUp` (`gaze up` in logs and stage).

## Ideas (fun per line)

14. **The payoff beat, one line of code each.** `HomeEvent::Built` and a set-down on a shelf end in silence. Reuse
    `Act::Admire` (Stand, pleased, `"hehe"`, 1.2-2 s) after each stage, with `THERE` (`"There!"`, osaka.rs:569)
    after the last; and after setting a piece on her shelf, step straight into a short gaze up at it. The arc's
    climax (shelf finished, gremlin up, she beams at it) then happens in one sitting, the one moment most worth
    catching.

15. **Make the wish visible.** When the kit comes from the wishlist, its pitch says why: `"A shelf for the
    gremlin!"` (24, tight) or `"A shelf! For him!"` (17). Otherwise the wishlist's whole effect is invisible: a viewer
    can't tell a wished kit from a catalogue one. One `Spec`-adjacent line and a branch in the pitch.

16. **The flat-pack gag.** Unpacking a kit is Osaka meeting assembly instructions: the kit's unpack ends on `"Which
    way's up?"` (15) instead of `"ooh"` (a per-item bubble in `UNPACK`'s last key, script.rs:2225-2238). Very her,
    one line.

17. **Lines for gazing up at him** (cooled, as `lines.pick` cools; one at most per gaze, after the `"hehe"`): `"Hi,
    little guy."` 15, `"You stay up there."` 18, `"Behave, okay?"` 13, `"He's watchin' me..."` 19. And novelty: the
    gaze's weight raised for a game day or two after something goes up on the shelf, then back to its base, so she
    stares most when the user is most likely to notice the new thing.

18. **The gremlin's own tiny life.** A prop's `facing` is saved (ledger.rs:480) and the gremlin is drawn mirrored
    for free (`symmetric: false`, `drawn_facing`). Rarely (a rarity roll, phase 5b's `rarity`, while she's asleep or
    out), flip his facing; at her next gaze up, `"?"` then `"Did you move...?"` (16). No new art, one ledger write, a
    change made while she isn't looking: the viewer who happens to catch him turning gets the best moment in the
    phase, and Osaka half-believing it is exactly her. (Cheaper variant: turn him to face her when a gaze starts.)

19. **Shelf brackets from the UI's own lines.** A run drawn `┬────────┬` reads as a shelf on brackets, not a stray
    border segment, still in the UI's glyphs (`┬` is redrawn by her line art, graphics.rs:333); a half-built shelf
    shows one bracket at its wall end (`┬────`), which says *unfinished* by itself. The sheet shows a plain line, so
    it's the user's call. Caveat for 6b: `┬` has a down stroke, so the terrain must keep it no pole and no link
    (`find_links`), as D5's flag already does for the shelf's cells.
