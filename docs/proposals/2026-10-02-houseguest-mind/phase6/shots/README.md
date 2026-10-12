# Her shelf and her gremlin in play: shots for review

Last updated: 2026-10-12 (phase 6a, step 9)

Real game frames from the built code, in line art at 9 × 19 px cells (the
picker the tests use), on the client's own layout at 191 × 46 (the user's
terminal) with an evening's content (`fuller_ui`: forty chat lines, four
users, ten playlist entries, twenty franchises in The List); the low shelf on
the census home's screen (100 × 20, two 9-row panes). The ignored test
`shelf_shots` (`dessplay/src/ui/houseguest/tests/shelf_shots.rs`) paints a
guest through each scene, at most 50 ms apart, and writes them:

```text
HOUSEGUEST_SHELF_SHOTS=$PWD/docs/proposals/2026-10-02-houseguest-mind/phase6/shots \
  cargo test --release -p dessplay --lib shelf_shots -- --ignored --nocapture
```

(About two minutes in release; nextest's default profile kills it at 60 s.)
They're drawn as the door batch's are (`door/shots/README.md`): the frame's
text and lines as a terminal would render them, her images composed over
their cells exactly as the game encodes them (the test checks every shot
carried her images). The contact sheets crop each frame to 44 × 15 cells
round her (her floor second from the bottom, so a high shelf over it and
what stands on it show), round her door for a parcel's slide, and round his
parcel in `his-parcel.png`; each tile is labelled with what it shows, and a
label is shot once (the first frame it names).

Her home in the kit scenes: everything the channel sold before her kit
(`tests/kit.rs`'s `all_but_the_kit`): her sofa, TV, window and gremlin in
Users, and her other pieces in the other panes. Her shelf goes up in Users,
her gremlin's pane (he stands on its floor), high: Users has 11 rows inside
at 191 × 46, so rise 7 exactly.

| File | What |
|---|---|
| `kit.png` | Her kit through her door's flap (Monday 16:00, fed), her unpacking it ("Which way's up?": a flat pack), the first stage straight after, chained ("Up it goes...", arms up), the hop as the frame takes it (one beat of `Pose::Jack(1)`, shot on the first frame that shows the stage), her pleased beat ("hehe"), and the half shelf: its bracket and the half of its run nearer its wall (`┬────`). |
| `stage-2.png` | The second stage (its game day's pace forgotten and the stage offered alone, standing in for the next morning: the shelf census puts it at about 07:00 the next day), its hop (shot on the first frame that shows the whole run: the frame that takes a stage was written before it was up) and her beat, finished (`┬────────┬`); then, as her finished shelf shows, she feels her gremlin wanting up there and says so ("It wants a shelf...", in his room, facing him). |
| `carry-up.png` | Industrious, she mends it: she lifts him ("Hup!"), carries him to under her shelf, sets him on it arms up, the hop (a piece set up on a high shelf), and her pleased beat in place of "There!". |
| `gaze-up.png` | Her gremlin centred on a high shelf of hers (Saturday 14:00, a home whose day's draw turns him), before and after he turned round (`Guest::gremlin_turns`, as when she sleeps or is out); her next gaze up at him opens puzzled ("?", then "Did you move...?"), and her later gazes open as ever ("hehe") and, one in two, a line of hers to him a few seconds in ("He's watchin' me..." here). Her gaze up is offered alone so it comes in minutes, not once a game day. |
| `high-191x46.png` | The whole screen: a high shelf where the stage puts one on the user's terminal with her gremlin waiting on her living room's floor (Users, with her sofa and TV: his pane, as in play; on a row the users list leaves clear), her plant and gremlin on it (the census's `wide_shelf_room`, seed 0); she isn't in sight on this frame (an unfed arrival's first frame). |
| `low-shelf.png` | The whole screen: a low shelf (rise 5, just over her head) on the census home's pane where her sofa is and her gremlin waited (as in play), her gremlin on it (her plant, 3 tall, has no third clear row over a low run there), her poster hung beside the shelf's block and her window in the right pane (the census's `low_shelf_room`). |
| `his-parcel.png` | Her gremlin delivered (ordered, her shelf up): his parcel through her flap, at rest, her unpacking it ("ooh"), and him out of his box on the floor. His parcel is the 5 × 2 parcel scaled into his 2 × 2 footprint: a tiny box (the open question below). |

Notes for the eye:
- A carried piece isn't drawn in her arms in line art (as before 6a: the
  sofa she carries in passing shows the same), so "carrying" tiles show her
  alone.
- The gremlin turned is the approved art mirrored; at this scale the turn
  reads mostly in his ears and grin.
- **Open, for the user**: his parcel (a tiny box for a tiny figurine, or a
  box of its own for a 2-wide piece?).
