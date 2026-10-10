# Her front door in play: shots for review

Last updated: 2026-10-10 (the door batch, step 10 and its review)

Real game frames from the built code, in line art at 9 × 19 px cells (the
picker the tests use), on the bundled layout at 100 × 30 (one at 100 × 20).
The ignored test `door_shots` (`dessplay/src/ui/houseguest/tests/away.rs`,
end of file) paints a guest through each scene, at most 50 ms apart, and
writes them:

```text
HOUSEGUEST_DOOR_SHOTS=$PWD/docs/proposals/2026-10-02-houseguest-mind/door/shots \
  cargo nextest run -p dessplay --run-ignored only -E 'test(door_shots)'
```

How they're drawn: the frame's text and lines are rendered as a terminal
would (lines as her images redraw them, text in the vendored DejaVu Sans, each
glyph squeezed into its cell (two for a wide one) as a monospace font would
set it, a glyph the font lacks as its alien block), and her images are
composed over their cells exactly as the game encodes them
(`Graphics::take_shots`; the test checks every shot carried her images). The
contact sheets crop each frame to 36 × 11 cells round her door; each tile is
labelled with what it shows (a door beat is `osaka.rs`'s `wall_beat` table:
0-5 going out, 6-7 the gap, 8-12 coming in; "walk k" is her last leg to her
door, a shot every 1.5 s, not a hop toward it nor a walk she finishes first).

Her home in each: her sofa and TV in Users, her bed and lamp in Playlist
(the parcel scene: the sofa alone, so the parcel takes her door's strip).
Her door stands in Users' right wall, the screen's edge.

| File | What |
|---|---|
| `school-morning.png` | A school morning (Tuesday, fed). From 08:10 she's visiting; at 08:15 she says she's off, walks to her door, steps through it a column at a time (cut at the wall's line, the door's front post over her), and the door shuts; her empty home stands with her slippers before the door and a card on the knob. Then 12:45: the door opens, she steps in, it shuts, and she says she's home. |
| `out-day.png` | That morning's empty home, the whole screen: her door shut in Users' right wall with the Away cue. |
| `school-scene-dusk.png` | The stage's school scene cued at 18:00 (a 5 s gap): the same beats, the doorway under the dusk sky (orange to violet over the hedge). She's cued as she drops in for the first time ("Nice to meet you."), so the stage sets her down on the floor below her door's (the y 17 ledge, whose middle is nearer her door than her door's own floor's middle) and she walks left to its end to climb up first (about 7 s, not shown); the walk tiles start on her door's floor, from the crop's left edge (walk 0: only her line shows). |
| `parcel.png` | A parcel through her door's flap: the flap riding up on the parcel as it slides out (beats 0-1), the parcel clear of the doorway (2), settling (3), at rest just past her door's space. |
| `short-terminal.png` | A 100 × 20 terminal while she's out: no space keeps at the wall, so her door stands face-on at the fallback (`Fallback::Short`, checked by the test), clear of every piece and never across a pane's side wall (│); its top row stands on the Playlist pane's top border, over its title, and its feet on the bottom border. Her door's image derezzes the title text it covers (the face-on door stands over text, as decided; the rule bars only a plain side wall above its floor row). |

Notes for the eye:
- A shot is what the guest painted; the Kitty placeholders under an image are
  drawn as the canvas's background.
- The shut door shows no sky; only the open and ajar doorway and the flap's
  hole do.
