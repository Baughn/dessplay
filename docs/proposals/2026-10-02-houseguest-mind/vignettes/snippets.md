# Houseguest vignette art: chopsticks, droop face, sata andagi (approved and wired in, 2026-10-03; see plan.md Phase 38, phase 5a)

Last updated: 2026-10-03

This is the art for phase 5a's chopstick ritual (#40) and sata andagi
(#41), as described in `phase5a-design.md` D7. It is drawn and on a model
sheet, but nothing constructs it at runtime yet. The commit adds SVG parts,
ASCII rows, a pose, a face and gallery tests. It adds no `Use`, splice or
script. `worktree.diff` in this directory is the code diff (art and tests
only).

## Review sheets (this directory)

All three sheets are drawn over `#1e2127`, the style study's dark terminal.
The outline colour is the client's `#1d1714`.

- `vignettes-1x.png`: the true on-screen size (9 × 19 px cells).
- `vignettes-1x-nn3x.png`: the same pixels at 3×, nearest-neighbour. Use this to judge what actually reads at 1×.
- `vignettes-3x.png`: a native 3× render. Use this to judge the drawing itself.

Rows, left to right:

1. **Held parts at 4× her scale:** chopsticks joined, split clean, split bad; andagi whole, bitten; then the existing melon bread for comparison.
2. **At the desk** (the desk's `Back` layer behind her, her box on desk column 7, as `homework` is placed): `homework(0)` for reference, then the chopsticks joined (Vacant), split clean (Happy), split bad (Droop).
3. **Faces:** standing Vacant, Blink, Droop; side-on Vacant, Blink, Droop.
4. **Eating at the open fridge:** the melon bread (frame 0) for reference, then the andagi whole (frame 0) and bitten (frame 1).

To regenerate:

```sh
HOUSEGUEST_VIGNETTES=/dir cargo test -p dessplay --lib vignette_sheet -- --ignored
```

## What was added

### SVG parts (`art/osaka.svg`)

| id | what |
|---|---|
| `face-droop` | Frontal let-down face. Each eye keeps its full open-eye size and outline. The top of each eye is a heavy lid in one hard skin-shade tone (`#e3b49c`), with a thick lid edge sagging toward the outer corner. The irises sit low under the lids, and the small mouth turns down. It reads distinctly from Blink and Vacant at 1×. |
| `p-face-droop` | The same face in profile. It keeps the open eye's shape and its top line, with the upper half lidded in the same shade. The thick lid edge sags toward the back, and the iris sits low under it. The mouth turns down. |
| `chopsticks` | Waribashi still joined: one light-wood bar with a groove down the middle and the tips already parted. Drawn upright, with the head end up and the grip at (0, 0). |
| `chopsticks-clean` | Two matched sticks fanned into a V (±11°). Each grip is at (∓2.2, 2), where her two hands go. |
| `chopsticks-bad` | The same V, but uneven. The far stick keeps a ragged chunk of the other's head, and the near stick is torn short with a splintered top. |
| `andagi` | Sata andagi, within the melon bread's radius (~7): a lumpy golden-brown ball (`#c98535`), one hard shade underneath (`#9c5e25`), and a burst top showing the pale crumb (`#f6d48a`) above a jagged crust line. |
| `andagi-bitten` | The same, bitten on the +x side like `melon-bread-bitten`, with crumb showing in the bite. |

Wood is `#ecd29e` with a `#a8824e` groove. All outlines use `currentColor`.

### Rust (`art.rs`, `sprite.rs`)

- **`Face::Droop`** (`sprite.rs`), with glyphs `['u', '_', 'u']`, so the standing face is `(u_u)`. It maps to the new `Expression::Droop` → `face-droop`.
- **`Pose::Chopsticks(u8)`** (`sprite.rs`): frame 0 joined, 1 clean, 2 bad. Its rig is `Rig::chopsticks(frame, expression)`.
- **`Rig::chopsticks`** (`art.rs`) sits her on the homework seat (stool, legs, `shift −16`, `bob −4`) in profile.
  - Its arm angles are solved so the hands land on the stick grips.
  - The hold leans forward into the gap between her face and the desk lamp's shade.
  - Joined: `(72, 86)` at 30°, tilt 8. Clean: `(72, 86)` at 22°, tilt −4 (head up). Bad: `(69, 92)` at 40°, tilt 10 (hands sag, head bowed).
  - The rig takes the face from the caller and never overrides it. The script should pass Happy for the clean split and Droop for the bad one.
- **`Rig::eating_food(frame, [whole, bitten], expression)`** (`art.rs`) is the food parameter. `Rig::eating` calls it with the melon bread ids.
- **Eating grip fixed (review round 1).** The old rig aimed her near arm at `(78, 74)` with a −118° elbow, which put her hand at about `(58, 64)`: up by her cheek, in her hair. The food, meanwhile, floated at `(83, 63)`. This affected the melon bread as well as the andagi, because both use the same rig.
  - The food now sits at her mouth: `(80, 61)` whole, and `(81, 62)` on the bite, following the bite's head tilt.
  - The near-arm angles (`(−97.2, −29.3)` whole, `(−98.5, −23.9)` bite) are solved so the hand lands on the food's lower back edge. The hand is drawn over the food, so it visibly grips it.
  - **This changes how `Pose::Eat` renders for the melon bread too.** The drawing of the bread is unchanged; only her arm and where she holds it moved. No test or golden changed: the full nextest suite passes as is. The kitty image cache will simply draw the new frames.
  - The bite notch still faces away from her (+x), as on the existing melon bread. That's the cartoon convention, which keeps the notch visible in silhouette.
- **Tests:** `poses()` gains the droop face (standing and side-on), the three chopstick frames and the two andagi eat frames, so `every_pose_renders_something_inside_the_box` covers them. `sprite.rs`'s `ALL` gains the three chopstick frames. `vignette_sheet` (ignored) writes the review PNGs.

### ASCII rows (`sprite.rs`, `CHOPSTICKS`)

Each frame facing right, then facing left. The top row is blank, as in `HOMEWORK`.

```
joined    clean     bad

( ._)     ( ^_)     ( u_)
 |V|I      |V|v      |V|y
_/ \      _/ \      _/ \

(_. )     (_^ )     (_u )
I|V|      v|V|      y|V|
 / \_      / \_      / \_
```

- The item in front of her chest is `I` (one bar: the joined pair), `v` (a matched pair) and `y` (lopsided: one long, one short). All three survive `mirror()` unchanged. The `V` collar stays the only `V`, so the ribbon colour isn't spread onto the chopsticks.
- It's a profile pose, so each frame bakes in its own face (`^` for the clean split, `u` for the bad one), as the other profile rows do.
- The standing droop face is `(u_u)`. Alternatives, if `u_u` reads too cute: `=_=` (tired/unimpressed) or `;_;` (too teary, I think).
- **Eat needs no new ASCII.** `" |Vo "` is already a round food, so it serves the andagi too.

## Wiring it in (phase 5a, after approval)

1. **Remove the review-only allows.** `Pose::Chopsticks` and `Face::Droop` each carry `#[cfg_attr(not(test), allow(dead_code))]` with a comment, because nothing constructs them outside tests yet. Delete both once the chopstick script constructs them.
2. **Chopstick script (before homework).** It's a `Splice` with keys `(Pose::Chopsticks(0), Face::Vacant, None)`, then either:
   - `(Pose::Chopsticks(1), Face::Happy, Some(Bubble::Sparkle))`, or
   - `(Pose::Chopsticks(2), Face::Droop, Some(Bubble::Say("Hold 'em by the ends!")))`.

   This line is 21 characters, so it fits the 24-character bubble.

   **Approved by the user:** after the bad-split line, a further key on the same frame with `Some(Bubble::Dots)` ("..."), so she trails off. This is wiring only; no art is needed.
3. **Andagi coda.** D7 has `Pose::Eat` gaining the food (`Eat { frame, food }`). Map it in `for_pose` to `Rig::eating_food(frame, ids, expression)`:
   - melon bread → `["melon-bread", "melon-bread-bitten"]`
   - andagi → `["andagi", "andagi-bitten"]`

   A small `Food` enum with an `ids()` method keeps the strings in one place. Introduce it when the first `Food::Andagi` is constructed, so the lib build never has an unconstructed variant. `sprite.rs` rows ignore the food.
4. **Image budget.** Each new `(Pose, Face)` is a new cached image: three chopstick frames, two andagi frames, plus Droop wherever else it's used.

## Sparkle glyph (item 4, proposal only)

There is no effect sprite, so the clean split's sparkle is a speech-bubble glyph. `Bubble::text` draws one char per cell with no width handling, and every existing bubble is ASCII. So I propose:

- **`Bubble::Sparkle => "*'*"`**: two stars and a glint, which reads as a twinkle in a bubble and is three cells like `"..."`.
- Plain `"*"` is the minimal alternative.
- `"✧"` (U+2727) is narrow in Ghostty and kitty, but it would be the first non-ASCII bubble. Avoid double-width emoji (`✨`), which would break the bubble's cell count.

## Design calls for the user

- **New pose vs. a hold variant.** I made it a new `Pose::Chopsticks(u8)` rather than a `Rig.hold` swap on `Homework`. Script keys are `(Pose, Face, Bubble)`, and the ASCII sprite is chosen by `Pose`. A hold-only variant can't be addressed by a key and would have no ASCII rows. Internally the rig is the homework seat plus a hold and re-aimed arms, so the cost is one variant, one rig fn and three ASCII rows. `Pose::Homework` keeps its frame numbering.
- **Where the sticks go.** At the desk, the lamp's shade sits at her eye level a cell in front of her face. Upright sticks either hide her face or land on the yellow shade, where light wood vanishes. So they lean forward at 22–40° into the gap below the shade, at chest height. Her chibi arms reach about 26 units, so they can't be held further out.
- **What reads at 1×.** In the 1× pixels the three chopstick states are carried mostly by shape and her face: one bar, an even V, then an uneven V with her head bowed and the droop face. The ragged chunk and splinters are only legible from 2× up. The andagi reads at 1× as a golden-brown ball against the yellow melon bread, and its crack shows from 2×.
- **Droop strength.** The frontal droop is clearly glum at 1×, while the profile droop is subtler. The approved `Dots` bubble after the line carries the rest.

## Review round 1 (2026-10-03)

The user approved the sheet with two fixes, both now made:

1. **Eating.** The food hovered in front of her while her hand was up in her hair. This was the rig, so the melon bread had it too. Both are fixed; see "Eating grip fixed" above.
2. **Profile droop.** It looked "a bit like a tiny monster" because the squint left a bare patch of skin. The first version hid the top of the eye under a skin-coloured wedge, which left an unshaded band between the bangs and the lowered lid, and a shrunken eye.
   - Both droop faces now keep the open eye's full shape and outline.
   - They draw the lid inside it in a skin-shade tone.
   - The frontal one had the same wedge and was reworked the same way.
