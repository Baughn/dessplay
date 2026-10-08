# Critique: her character and the art (door batch working design)

Lens: what the user will **see**, checked against the approved sheet B (`art/wall-door-1x-nn3x.png`,
`art/snippets.md`) and plan.md Phase 38 "Next: the door batch". Paths are relative to
`dessplay/src/ui/houseguest/` unless noted. Checked as claimed: the art is in the tree as dead code
(art.rs:1119/1121 `WallDoor`, :1146 `flap_plate`, :1221 `render_wall_door`; sprite.rs:478/479 `WALL_DOOR`);
`first_due` is at osaka.rs:2861; `Pose::Side` exists (sprite.rs:425); the door pose is fixed `Pose::Stand` at
osaka.rs:9857-9861.

## Blocker

1. **D8 (beats) / Step 8 sites: if beat 6 draws the door, the visit never ends.** D8 says her external door draws
   `Shut{away: true}` in the gap beat 6 ("unlike the door in space"). `Osaka::gone_out` (osaka.rs:5296) is
   `leaving.filter(|_| self.hidden(now) && self.door(now).is_none())`, and `door()` (osaka.rs:9067) returns the
   beat's door. If `wall_beat`'s door feeds `door()`, `gone_out` never fires, `out_by_door` (mod.rs:1869) never runs,
   and she never reaches `State::Away`: the visit's frame hangs with her hidden, and no Away door appears.
   `osaka.rs:13588` asserts `gone_out == Some(School)` at the gap. map.md:1763 flags this; the design doesn't.
   **Amend:** `gone_out` tests the beat itself (beat 6, not yet `there`), not `door().is_none()`; keep "what is
   drawn" (`wall_beat`) apart from "is she through" (`hidden`/`gone_out`). Add osaka.rs:5296 and its test to step 8's
   sites (and step 4's, where `Through::Home` lands). Test: `gone_out` is `Some` at beat 6 of a `Through::Home` door
   while the cue is drawn, both modes.

## Major

2. **D8 "beats 6-9 `Shut{away: true}`" contradicts the approved table and sheet.** snippets.md:186-189: 6 and 7
   `Shut{away}`, **8 `Ajar`, 9 `Open`**; band 4 shows ajar, open, then her peek. As written, the door stays shut
   through 7-9 and jumps to `Open` as she peeks: the opening is lost. Step 8 test 3 ("gap beats 6-9") repeats it.
   The slippers: snippets.md:195 says "until she's back in (beat 10)... vanish as she steps into them", but `Ajar`
   and `Open` carry no `away` (art.rs:1125-1130) and band 4 shows no slippers once the door opens. **Amend:** D8
   and step 8 test 3 say "beats 6-7 `Shut{away: true}`, 8 `Ajar`, 9 `Open`; the slippers go as the door opens
   (beat 8)", matching the sheet. Fix the snippets' line in step 10.

3. **D8: when the shut door appears and goes during a visit is unspecified; by default it pops in beside her.**
   Today a door shows only during `Act::Door` (`Osaka::door`, osaka.rs:9067; `draw` mod.rs:3033). The approved
   sheet (band 2, frames 1-2; snippets.md:179 "Job::Leave walk: `Shut` (the door shows as she arrives)") shows the
   door in the wall **while she walks up**, and band 4's last frame shows it still there as she walks off. D8's
   draw paths read only `Act::Door`, so the door pops into the wall at beat 0 right next to her, and vanishes the
   instant beat 12 ends, beside her again: two eye-catching pops per crossing (stillness), and neither matches the
   sheet. brief.md:408 decided "shows only while she's out or going through", which leaves the walk open.
   **Amend:** her external door is drawn `Shut` from her set-off (`leaving.is_some()`, or a `Job::Leave` walk or
   heading with `shift == Going`) so its one appearance coincides with her line; after beat 12 it stays until her
   box no longer meets the space (or for one walk step), then goes. Add the site (`draw_art`, `draw`, the
   `Figure`/door query) and a test: no frame in which the door appears or goes while her box meets the space,
   except at her set-off line.

4. **D7 trigger: "on the first frame she's in sight" collides with her arrival lines.** `Osaka::say` hushes
   whatever is showing (osaka.rs:3030-3055). Coming home, the door's end calls `home_from`/`come_home`
   (osaka.rs:9487-9570), which say "I'm home!"/"Tadaima!". Her first in-sight frame of a Return or Dash is beat 10,
   half through the doorway (or, at the fallback, emerging face-on): "Can't get to the door!" would show there,
   then be cut by "I'm home!" ~1 s later (or cut it). On an idle arrival InChat collides with her hello. Phase 4
   grievances never speak on a first frame or over another line (`grievance_from`, osaka.rs:1655, waits for
   `quiet`; `GRIEVANCE_MS` holds two frames). **Amend:** feel on first sight, but **say** at the first moment she's
   quiet after her arrival line (`morning_until`, or her first `choose_next` once out of the door), standing, for
   `GRIEVANCE_MS`; `grumbling()` covers it so a look-up doesn't step on it (osaka.rs look_at). Say a row's line only
   if a key of it was actually felt (a `Why::Protected` fallback arrival has none). Test: no frame shows two of her
   lines' text, and "Can't get to the door!" shows whole after "I'm home!", both modes.

5. **D7: a space that yields for height is blamed on furniture.** `kept` needs `raw.rows >= HEIGHT`; map.md:1068
   puts that at H ≳ 23 in the bundled layout, so below it the space always yields and every return is through the
   fallback (`fallback_in`). DoorClear judges `space.rect` "kept or not", with `rect` computed from the raw extent
   regardless of height (D1). On a short terminal any piece at the wall end of the door's strip is DoorClear-broken:
   she says "Can't get to the door!" on every return (a false line: the room is too short, not blocked) and, if
   Industrious, moves a piece that wasn't the cause; the space still yields after. **Amend:** DoorClear is broken
   only where `raw.rows >= HEIGHT` (the space yields *because of pieces*); split `Why::Yield` into `Short` and
   `Pieces` and set `fallback_in` only for `Pieces`. Test (H 18-22, both modes): nothing DoorClear-broken, no line,
   no repair.

6. **D7: the on-sight lines nag every visit.** InChat (and DoorClear on a fallback return) is felt "the first time
   she's in sight in a visit" and its line said once a visit. Visits are many a day (each idle visit, the return
   from school, each dash, the work return), and a Lazy Osaka never mends (step 6 test 7), so "Not in the
   chat..." would open nearly every visit indefinitely: extra fuss, the opposite of stillness. Phase 4's
   grievances at least need her to use the piece. **Amend:** speak the on-sight line only on a visit whose mood
   would mend (`to_mend` > 0), or at most once per game day per key (a cooled pool line, as `lines.pick` cools);
   still *feel* it every visit so a mend can follow. Test: a Lazy day-long run says each on-sight line at most once
   a game day.

7. **D4 step 3: the fallback door hops with pane text while she's out.** The space's rule (decided list, line 57:
   text never moves her door, it hides it) is not applied to `Set::Floor`: the strict fallback requires "her box
   calm" (design.md line 190), so it is recomputed per frame against text. While she's out for hours, a Users or
   Playlist row appearing or going under the fallback spot moves the face-on door across the floor, and she comes
   home somewhere else. That is the movement-draws-attention failure in exactly the yield path older homes live in.
   **Amend:** the fallback is chosen from pieces, platforms, protected and the chat only (never text); over text it
   is hidden that frame, as the space's door is. Test: `the_fallback_door_never_moves_for_text` (text written into
   and out of its box, the spot identical, both modes).

8. **D5/D8: text at the wall will hide the Away cue for most of the time she's out.** The bundled stylesheet puts
   right-aligned text at the pane's last interior column: `.playlist-download { width: 4ch; text-align: right }`,
   `.playlist-watch` after it (`dessplay/src/ui/layout/assets/style.css:36-37`). Wherever the playlist's rows reach
   the bottom four rows, the shut door's own column `to-1` meets text, and D5 hides the whole door. If her pieces'
   strip is Playlist (the chooser prefers it, D2), the cue the user asked for ("there's really no other way to know
   she *is* out") is mostly invisible. Step 3's exit defers measuring this to after the fact; it is predictable
   now. **Amend:** put it to the user before step 2 saves a wall, with a recommendation: either (a) the chooser
   tie-breaks among edge walls toward one whose `to-1` column is calm on `chatty_ui` (Users' names are left-aligned
   with a right margin, style.css:39), or (b) the shut door's two columns may stand over text while she's out (the
   slippers still cropped), as the flap's wall column already does.

9. **D8 vs visits: the door over text in a visit is hidden, so she opens an invisible door.** The Away rule hides
   the door over text (D5), and step 8 test 5 makes `Hidden::check` fail any door cell over text. But during a
   visit she walks to her spot and goes through even with text there (decided: "going out isn't blocked by
   text"). With the door hidden she stands at the wall, mimes the beats, and walks clipped into the wall with no
   door: it looks broken. `Hidden::check` already allows her image over text in passing for `HIDDEN_MS` = 10 s
   (tests.rs:549-556). **Amend:** during a visit's beats 0-5 and 7-12 (≈ 3.5 s) the door's image is drawn over text
   in passing, as hers is; only the long states (Away, and a work gap's beat 6, 60-180 s) hide over text. State it
   in D8 and in step 8 test 5's exemption.

10. **D9: a delivery can slide through her slippers or her legs.** Deliveries run in `furnish` during a visit
    (mod.rs:3322, :3352), including while she's out at work in beat 6 (the Away cue drawn: slippers at `to-3 ..=
    to-2`) and while she stands in the space for her door's beats. D9 slides the parcel from the wall to `w-7`
    across the whole space, "with her too, if she stands in the space", and the slide's `Shut{flap: lean(..)}`
    says nothing of `away`. Either the slippers vanish for the slide or the box drives through them; with her in
    the space it passes through her. **Amend:** a delivery waits (as the clock's already waits on `visit.flap`,
    mod.rs:3345) while her external door's act runs or her box meets the space; test `no_parcel_slides_through_her
    _or_her_slippers`.

11. **D8 thirds are three times her walking speed.** Beat 2 (700 ms) and beat 10 (600 ms) move her 2 columns per
    third: about 2 columns per 230 ms, against her walk's 1 column per 333 ms (`WALK_MS`, osaka.rs:386). She would lurch through the doorway at ~3× her pace, right after a calm walk: a change of pace
    draws the eye. **Amend:** for her external door, step her through at her walking pace: beat 2 and 10 lengthened
    to 4 × `WALK_MS` (≈ 1.33 s), `d` advancing a column per step with the walk frames cycling (`Walk(d % 4)`),
    `first_due` waking per step; or keep thirds with each third ≥ 2 × `WALK_MS`. Re-count the stillness census for
    the extra wakes.

## Minor

12. **D8 beat 10-12: face-on, side-on, face-on at the door.** Table: beat 10's last third `Stand` (face-on),
    11-12 `Side` facing the room, then `home_from` sets `Act::Stand` (face-on) for "I'm home!" (osaka.rs:9500-9510).
    Three turns in ~1 s, each a change that draws the eye. **Amend:** hold `Side` facing the room from beat 10's
    last third through 12 and let the line's `Stand` be the one turn to face the viewer (or `Stand` through 10-12).
    The sheet's band 4 frame 5 is one review frame, not a beat requirement.

13. **D6 work return: her leeks pop in after the door.** `come_home` (osaka.rs:9554) sets `Act::Home`, drawn
    `Pose::Carry` (osaka.rs:9845-9847), only once the door has ended; beats 10-12 draw her empty-handed (`Walk`,
    `Stand`, `Side`). **Amend:** for a `Through::Home` door with `shift` set, beats 10-12 use `Pose::Carry(frame)`
    so she steps in with her shopping.

14. **D6 school: turning to the chat, then away.** `go_out` faces the chat while she's watching it
    (osaka.rs:5267-5270) and says her line; under D6 the walk then turns her toward the door at once: a turn and
    a turn back in one beat. **Amend:** face the chat only when she is already at her spot (the door opens at
    once, as today); otherwise say the line facing her way to the door.

15. **D6: a route with no way there means two doors.** `go_to` with no route (osaka.rs:8051-8060) takes a door in
    space to the spot, so she goes through a face-on door to reach her side-on door: a door to a door. **Amend:**
    with no route to the spot, go out by the door in space where she stands (`Through::Space`, gap `MAX`), and her
    Away door stands at the space; one door, and still never left standing at her feet.

16. **D6 "she gets up": no mechanism named.** Test 1 ("she gets up, says one OFF line, walks") and the design.md
    edit ("gets up first") imply a step out of the bed; the night's `wake` steps beside the piece
    (`beside`, osaka.rs:258, used at 5326-5334), the school cut has none (map.md:20). **Amend:** say which: the
    ordinary end of a use (she walks out of the bed's image, as after any nap: no new beat), or `beside` first,
    as `wake` does. Recommend the former for stillness; write the test to match.

17. **D4/D8: the fallback door's look is unspecified.** `Set::Floor` is face-on (`Look::Door`). While she's out
    there: today's closed face-on door, no slippers? In a work gap at the fallback, today's beat 6 draws nothing,
    so the door vanishes for 1-3 minutes. **Amend:** say it: the face-on fallback is the closed `Look::Door` while
    she's out (no cue, a known difference) and stands through a work gap's beat 6, so "a door standing" always
    means she's out, in both looks. On terminals under ~23 rows (map.md:1068) this is the only door the user sees;
    step 8's CHANGELOG line should say "where there's room".

18. **D8 sky: the hedge stays in daylight.** `wd-beyond` is "a sky-to-pale-gold daylight gradient over a green
    hedge" (snippets.md:113); D8 derives only the gradients per `Sky`. At Dusk/Evening/Night (a late work return,
    an errand) a sunlit hedge under a night sky reads wrong. **Amend:** tint the hedge per `Sky` too (darker, cooler
    at Evening/Night, as the window's skies darken, art.rs:1430-1434), and include it in the regenerated sheet.
    Unfed → `Day` matches the window's `Plain` (art.rs:1269): fine.

19. **D8 ASCII: the thirds leave a sliver.** At `d = 4` her box is `to-1 ..= to+3`, so ASCII keeps one column of
    her glyphs at `to-1`. That's what snippets.md:166-167 approved, and ASCII is dev-only, but one column of
    glyphs reads as noise. **Amend (optional):** in ASCII, `d = 2` keeps her clipped, `d = 4` hides her.

20. **D7 line: "Not in the chat..."** She has no line about panes or the chat (grep of `line!` finds none); the
    other grievances are about the room, not the UI (rules.rs:63-110). **Amend:** a line in her voice about why,
    e.g. "Everyone reads here..." or "People are talking here...", then put it to the user with the sheet (the
    design already lists it as open).

21. **D6 work: she sets off without a word.** `go_to_work` (osaka.rs:9463) says nothing. She now walks to the same
    door as for school, silently, in the afternoon. Optional, for the user: an OFF-pool line ("I'm off!") as she
    sets off for work, latched like school's. Not a defect; the walk alone also reads.
