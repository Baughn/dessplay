# Houseguest style study: sticker (bold-outline flat chibi)

Last updated: 2026-10-01

A clean-room style study for Osaka's line art and her furniture, painted
with the `easel` CLI. The only reference was `docs/ayumu.webp`; the existing
SVGs in `dessplay/src/ui/houseguest/art/` were deliberately not shown to the
painters. Four styles were tried (hand-placed pixel art, this one, minimal
ink brush with spot colour, soft gouache/pointillist). Each was painted,
reviewed by a separate art-director agent at 1× over a dark terminal, then
revised. **This one was chosen** as reading best at the size she's actually
drawn. Nothing here is wired into the client yet: the rig draws SVG parts,
so adopting the style means redrawing those parts to match these targets.

## The brief

Chibi "sticker" style: a big head (roughly 40–45% of her height), thick
uniform dark outlines (about 1.5–2 px at 1×, the dominant feature), flat
cel colours with at most one hard-edged shade tone per material, no
gradients, no texture. Clean vector-like shapes; anti-aliased edges are
fine. The silhouette must read at 45 px wide.

Canvas sizes are the true on-screen size at 1× (a Ghostty cell is 9×19 px;
18×38 on HiDPI):

| piece | pose / content | cells | 1× px |
|---|---|---|---|
| `stand` | frontal, vacant stare, arms relaxed | 5×5 | 45×95 |
| `walk` | profile facing right, mid-stride | 5×4 | 45×76 |
| `sit` | on the floor hugging her knees | 5×4 | 45×76 |
| `tv` | small early-2000s CRT on a low stand | 6×4 | 54×76 |
| `sofa` | two-seater, side-on | 9×3 | 81×57 |
| `bed` | single bed, side-on, pillow and blanket | 10×3 | 90×57 |

## Files

- `<piece>.easel/`: the easel documents (continue with `easel apply`, re-export
  with `easel export <doc> -o out.png --scale N`). Each has a hidden
  `preview-bg` layer (#1e2127) for judging over a dark terminal.
- `<piece>@1x.png`, `<piece>@2x.png`: transparent exports at 1× and 2×.
- `<piece>@1x-dark.png`, `<piece>@4x-dark.png`: previews over the dark
  background.
- `sheet-sticker-1x.png`, `sheet-sticker-3x.png`: all six side by side.
- `comparison.png`: all four styles on one sheet, top to bottom pixel,
  sticker, ink, gouache; each band shows the six pieces at 2× above 1×.
- `*.txt`, `*.gen`, `pal.txt`, `furn-pal.txt`, `pp.py`, `exp.sh`: the
  painter's scripts and palettes (the `.txt` files are easel passes;
  `exp.sh` re-exports).

## Open issue: pose box sizes

The critic's top finding, unresolved because it's a design call: with the
walk and sit boxes 5×4 against stand's 5×5, she either shrinks when she
starts walking or her body is squashed. The reviser kept the head the same
size in all three poses (~35 px) and took the difference out of the torso,
skirt and legs. A 5×5 walk box would remove the rest. Every other style's
critic reached the same conclusion. Also check that the sofa and bed fit
her sitting and lying in this style before adopting it.

## Painter (first pass)

**Style as executed:** Bold-outline flat chibi (sticker / cel): big head (~43% of height on stand), 1.6 px uniform warm-dark outlines on every shape, flat cel fills with at most one hard-edged shade tone per material, no gradients or texture, smooth vector curves.

**Palette:** `#3e2a2c`, `#fcdcc4`, `#eab89c`, `#5b4135`, `#432e26`, `#ea8278`, `#cc6660`, `#fbf8f4`, `#d9d5df`, `#cf3540`, `#82343c`, `#622229`, `#d3d3de`, `#8c5030`, `#683a22`, `#6e4430`, `#3a2018`, `#f0c040`, `#b37b4b`, `#8c5a33`, `#5a3622`, `#6f9e90`, `#557d72`, `#d2cdc4`, `#a9a399`, `#3d6a82`, `#6aa3bd`, `#b4e0ee`, `#86a8d8`, `#6587bb`, `#40434b`, `#7fe08a`, `#1e2127`

The only reference used was ref/ayumu.png. I did not open any project source, the existing SVGs or any snapshots.

Contact sheet of all six 1x dark previews: sheet-sticker-1x.png, with a 3x nearest-neighbour enlargement in sheet-sticker-3x.png, both in the sticker dir.

How it was drawn: each part is a flat fill plus a 1.6 px outline stroke (fine detail uses 0.9 px), painted back to front, so overlapping parts get cel-style lines between them. Limbs are a 5.6-7.2 px outline-coloured stroke with a narrower fill-coloured stroke on top, which gives a clean outlined tube.

Outline colour is #3e2a2c. I lightened it from a first #2a1b1e because that version disappeared against the #1e2127 terminal background, and the outer line is the main feature of this style. I tried a white die-cut sticker rim and dropped it, because it would read as a cut-out when she sits on the sofa or bed.

What carries the likeness at 1x is the face: huge round eyes (white, brown iris, dark pupil, catchlight, heavy upper lid), uneven bangs with one strand hanging between the eyes, a straight hair curtain down each cheek, and a tiny smile. Stand and sit use exactly the same head script (stand-head.txt), so the face matches in both. The uniform is reduced to: white V sailor collar with a red stripe, red cord with a yellow toggle, salmon top, white cuffs with a red stripe, maroon skirt with pleat lines and one shaded panel, white knee socks and brown loafers. Walk shows the square collar flap on her back below the long hair. Far-side limbs use the shade tone for depth.

Furniture (same outline and flat-shade rules):
- TV: silver-grey CRT with a glowing blue screen and a hard-edged glare, knobs and speaker grille, rabbit-ear antenna, on a wooden cabinet holding a VCR.
- Sofa: sage green, rolled arms, two back and two seat cushions. Seat top is about 25 px above the floor; the backrest reaches her chest when she sits on it.
- Bed: wooden headboard and footboard, white mattress and pillow, blue blanket with a folded-back sheet, tucked behind the footboard.

Known trade-offs:
1. Walk's head is about 33 px tall against about 39 px in stand and sit (about 0.9 scale with shorter legs), so she visibly shrinks when the sprite changes from stand to walk. I did this to fit the 76 px box without making her over 50% head; keeping the 39 px head would have made the walk frame much more chibi than the others.
2. In sit, the knees and shins read as a pale block at 1x. It's acceptable, but it's the piece I'd improve next.
3. In walk, the near (back-swinging) arm swings only a little so it doesn't cover the collar flap, which still shows only as a white patch below the hair.

Easel limitations I hit:
- There is no rounded-rectangle shape. pp.py expands rrect(x,y,w,h,r) into a 28-point poly, so tv.txt, sofa.txt and bed.txt must go through pp.py (into the .gen files) to regenerate.
- There is no dilate or outline operation, hence the per-shape stroke approach.
- poly fills are even-odd, so reversed vertex order in the bangs quietly left a skin-coloured band above them. It's fixed and stand was re-exported.
- Colours and shapes share one variable namespace: a screen shape named $scr overwrote the $scr colour.
- Walk has a hidden hmask layer that is only used to clip the hair shading.

Source scripts in the dir: pal.txt and furn-pal.txt (palettes), stand-body.txt, stand-top.txt, stand-head.txt, walk-1.txt, walk-2.txt, sit-body.txt, tv.txt, sofa.txt, bed.txt and exp.sh (export helper). Every 1x and 2x export has a transparent corner pixel, and the figure's alpha has no halo. The preview-bg layer (#1e2127) is hidden and sits at the bottom of every doc.

## Art director critique

**Style fidelity:** Mostly on-style for flat cel. Fills are flat with one hard-edged shade per material and no gradients or texture, the shapes are clean and vector-like, the head is about 42% of height in stand, and the uniform is reduced to its iconic parts (white V collar with red stripe, toggle, salmon top, maroon skirt, white socks, loafers).

The style's defining feature, a thick dark outline that dominates the silhouette, is effectively lost over the target background. Measured values: outline #3e2a2c is OKLCH L 0.31, the #1e2127 background is L 0.247, and the hair is L 0.40. The outer contour is invisible at 1x, so the sticker look survives only through the interior lines (arm against torso, collar, skirt, furniture panels). The rejected white die-cut rim was the right call. The fix is to raise the values of the fills that form the outer silhouette, mainly the hair (to about L 0.45), so the fill edge does the outline's job.

The eyes are drawn at higher detail than the pixel budget allows (outline ring, lid and lashes in about 7x9 px). That pushes them toward a noisy semi-realistic look instead of the simple big-round-eye sticker read the style calls for.

The furniture executes the style most cleanly: the TV and sofa are textbook flat sticker art.

**Consistency:** The furniture is the most consistent part. TV, sofa and bed share the #3e2a2c outline, the 1.6 px stroke and the one-hard-shade rule, and they sit with her as one set. The sofa fits her sit sprite well. The TV's height is plausible. The bed is too short for her standing scale (68 px of interior against her 91 px) and needs a curled lying pose or a longer box.

The figure poses are not consistent with each other. Walk is drawn at about 0.78 of stand's height (71 px against 91 px, head about 33 against 39 px), so she visibly shrinks by a whole terminal row when she starts walking. That is the most visible flaw in the set. Hair length differs between poses too: stand and sit hair ends about 11 px below the shoulder, walk hair about 4 px below.

Stand and sit share an identical head, which is good, but both have the same capsule silhouette problem. Walk's head has a different hair-to-face balance (mostly back hair), so the three don't read as the same model sheet. Scale between her and the furniture is fine for the TV and sofa and wrong for the bed.

Transparency is clean on all six exports: corners and edge midpoints are alpha 0, and the alpha views show no halo or stray pixels. The only edge case is the sofa's AA fringe touching x=0 and x=80.

**Top fixes:**

1. Unify the figure scale across poses. Walk is 71 px tall against stand's 91 px (head 33 vs 39 px), so she shrinks a whole terminal row when she starts walking. If the 76 px walk box is fixed, the orchestrator must give walk a 95 px box or redraw stand and sit at walk's scale; don't ship mixed scales.
2. Break the capsule silhouette in stand and sit: pull the arms in from x 6.4-38.6 to about x 9.5-35.5, and end the back hair at y~49-50 with an inward taper, so the head (x 7.5-37.5) overhangs the shoulders by 2-3 px with dark background showing between hair tip and sleeve.
3. Simplify the eyes for 1x: a white ellipse about 6x7, a dark-brown iris about 4x5, a 1x1 catchlight and a single 1 px top lid line, with no outer ring and no lashes. That restores the huge white Osaka eyes, which currently resolve to brown blobs in grey smudges.
4. Rebalance the walk head: move the back-hair boundary from x~7 to x~11-12, widen the face skin to about x 24 to the front edge, clip the bangs at x~33, and add a nose bump at (35,27-28) and a chin at (32,33). Rebuild the torso as a rectangle (x 18-29, y 37-50) with a distinct outlined near arm.
5. Fix the sit knees: two round caps of radius about 4.5 at (19,50) and (26,50) closing the gap that shows the cord, face-skin colour with one hard shade on the outer shin, and arms that wrap across the shins at y~57-59 instead of hoops.
6. Fix the bed's levels: one continuous mattress top at y~30 from x 10 to 78, the blanket 2 px proud on top of it, the side rail continuous at y 41-45, and the upright white fold bar (x 30-36, y 25-45) replaced by a flush band. Escalate the bed length: 68 px of interior can't hold a 91 px girl.
7. Raise the hair value to about #6e4e3c (L~0.45 from 0.40) so the outer silhouette separates from the #1e2127 background now that the #3e2a2c outline (L 0.31) disappears into it.
8. Minor: lighten the TV antenna base (x 20-30, y 10-13) so the ears attach visibly, and inset the sofa 1 px so its outline doesn't sit on canvas columns 0 and 80.

### stand (reads at 1×: yes)

Problems:

- The silhouette is a pill with legs (see `easel view --alpha`). The head/hair blob $bh spans x 7.5-37.5 and the arms $la/$ra span x 6.4-38.6, so the head is no wider than shoulders plus arms. The back hair ($bh, down to y~54) runs straight into the arms (down to y~64), so the outer contour is one unbroken vertical from y~10 to y~66. There is no head-on-body step, which is the main silhouette cue of chibi style. On the 1x sheet she reads as a brown tombstone with a face.
- The eyes fail at 1x (face crop x 13-20 and 24-31, y 25-34). The outline ring, heavy lid and lash marks eat the sclera, so the whites come out as greys (#9a9a9a-ish) plus one white catchlight pixel. At 1x each eye is a brown blob in a grey smudge, not the huge white round Osaka eye.
- The plain hair crown is too tall: flat brown with no shade from the head top (y~2) to the bangs (y~21), about 30x19 px. The eye line sits at y~29, about 70% down the head, against about 55% in the reference, so she looks helmeted and long-skulled.
- The outer outline (#3e2a2c, OKLCH L 0.31) is nearly invisible against the #1e2127 background (L 0.247). Hair is L 0.40, so the hair fill does the edge work and only dimly.
- Legs (x 16-19 and 25-28) show only about 3 px of bare skin (y 74-77) above the socks (y 79-86). That's acceptable but stubby, and the skirt (x 8-37, y 66-73) is a plain rectangle with pleat lines that are barely visible at 1x.
- Minor: the cuffs read pink rather than white with a red stripe (y 61-63).

Fixes:

- Narrow the body so the head overhangs it. Move the arms in to about x 10-14 and 31-35 ($la outer edge from 6.4 to about 9.5, $ra from 38.6 to about 35.5) and the torso to about x 13-32. Keep the head blob at x 7.5-37.5. That gives a 2-3 px step on each side at shoulder height (y~43).
- End the back hair above the arm line with a visible notch: pull $bh's bottom points from y 53-54.5 to about y 49-50, and taper the outer corners inward (6,53 to 8,49; 39,53 to 37,49) so dark background shows between hair tip and sleeve at about (7,51) and (38,51).
- Redraw each eye for the pixel budget, with no outer ring: a white ellipse about 6x7 (left centred about (16.5,29.5), right about (27.5,29.5)), a dark-brown iris about 4x5 in the lower-inner part, a 1x1 white catchlight at the iris top-outer corner, and a single 1 px dark lid line along the top only. No lashes.
- Shorten the crown. Either move the head top from y 2.2 to about y 5 (and drop the 3 px to the legs, giving more thigh skin above the socks), or add one hard-edged lighter hair-shine band across the crown at about y 9-11 to break up the slab.
- Lift the hair value to about #6e4e3c (L~0.45) so the fill edge separates from the background. Keep the internal #3e2a2c lines.
- Flare the skirt hem 2 px past the waist on each side (waist x 10-35 to hem x 8-37 is fine, but bevel the corners), and add 2-3 more pleat lines in the darker shade, 1 px wide, at x 12, 18, 27 and 33.
- Give the cuffs a pure #fbf8f4 white with a single 1 px red line, instead of a blended pink.

### walk (reads at 1×: no)

Problems:

- She shrinks from stand to walk. Walk spans y~2-73 (71 px) against stand's y~2-93 (91 px), about 20 px, a full terminal row, every time she starts walking. The head drops from about 39 px to about 33 px. In motion this is the most visible flaw in the set.
- The profile face is too small. Skin covers only about x 27-34, y 18-34 (7 px wide), while the back-of-head hair fills x~7-27 (20 px). At 1x it reads as a big brown hair mushroom with a sliver of face, not as a right-facing profile.
- Bang strands poke past the face front at about x 35-36, y 21-24, and the profile has no nose or chin shape, so the face's front edge reads as ragged hair.
- The torso is a triangle, narrow at the top (x 21-27 at y 38) and wide at the hem (x 15-30 at y 50). With the white collar-flap strip at x 13-17, y 41-47 and the near arm hardly separated from the torso, the top reads as a cape or satchel rather than a sailor top with an arm.
- Hair length doesn't match stand: here the back hair ends about 4 px below the shoulder (y~41, shoulder ~37), in stand about 11 px below (y~54, shoulder ~43).
- The gait itself is fine: the near (light-sock) leg goes forward at x 24-30 and the far (shade-sock) leg back at x 12-17, which is correct opposition with the near arm swinging back, and the legs read as mid-stride.

Fixes:

- Match the figure scale to stand. If the 45x76 box is fixed, the orchestrator has to resolve it: either walk gets a 45x95 box with the same 39 px head and stand's leg length, or stand (and sit) are redrawn at walk's scale. Don't ship them at different scales.
- Rebalance the head: move the back-hair boundary forward from x~7-8 to about x 11-12, and widen the face so skin runs from about x 24 to the front edge, about 10 px of face. Keep the eye at about x 29-33.
- Clip the bangs to the face front at about x 33. Add a 1-2 px nose bump at about (35,27-28), a chin point at about (32,33), and a 1 px mouth at about (32,31).
- Draw the torso as a rounded rectangle about x 18-29, y 37-50. Draw the near arm as its own outlined tube from the shoulder (21,39) back to the hand at about (15,50), crossing in front of the collar flap, with a white cuff. Shrink the flap to a 4x4 white square with a 1 px red line at about x 17-21, y 38-42, sitting right under the hair tip.
- Keep the back hair's end consistent with whatever stand ends up with: about 6-7 px below the shoulder after stand's hair fix.

### sit (reads at 1×: no)

Problems:

- The silhouette is the same pill as stand (see the alpha view). The head, hair and arm loops are all about 32 px wide, and the skirt fan widens it further at the base, so the outline is a tombstone.
- The knees don't read as knees. The shins are two pale, flat-topped rectangles at x 15-21 and 23-29, y 49-59, with the red cord and yellow toggle showing in the gap at x 22, y 50-58. At 1x the pair reads as a white vest or bib with a cardigan opening, not knees pulled up to the chest (the painter's "pale block").
- The shin skin (about #ffdcc0) is so light that shins and socks (y 64-68) merge into one pale column.
- The arms are large salmon hoops (x 7-14 and 30-37, y 45-62) that read like ear-flaps or a lifebuoy rather than arms wrapped around the shins. The hands (x 19-25, y 61-63) are 2-px specks.
- The skirt fan (x 3-14 and 30-42, y 60-75) reads as a dark red cape behind her. That's acceptable, but it's the widest part of the sprite, so the bottom-heavy blob gets heavier.
- Back hair runs down beside the arms to about y 60, the same capsule-forming problem as stand.

Fixes:

- Make the knees round caps: two circles of radius about 4.5 at (19,50) and (26,50), touching at x~22.5 so the gap closes and the cord/toggle no longer show between them. The shins taper from there down to the socks at y 63.
- Use the face skin colour for the shins, with one hard shade tone (about #e3ae94) on the outer 1-2 px of each shin and a crescent shade under each knee cap, so the knees separate from each other and from the white socks.
- Turn the arms into tubes that wrap in front: from each shoulder (about (13,45) and (32,45)) down the outside of the shins, then horizontally across the front of the shins at about y 57-59, with the white cuffs meeting at the centre about x 20-25. Drop the hoop shape and keep the arms' outer edge at x≥10 and x≤35.
- Shorten the back hair to end at about y 49-50 with the same notch as stand, so the head steps out over the arms.
- Narrow the skirt fan to about x 7-38 and give it a lighter maroon top edge, so it reads as cloth on the floor rather than a cape.

### tv (reads at 1×: yes)

Problems:

- The antenna base dome (about x 20-30, y 10-13) is dark grey with almost no contrast against the background, so at 1x the rabbit ears look like they float.
- Minor: the VCR body (x 11-41, y 60-66) is a dark slot inside the dark cabinet, and only the green LED makes it read. Acceptable.
- Scale is plausible: the CRT is about 37 px wide against her 45, and the cabinet top (y~50, about 26 px above the floor) sits around her skirt hem when standing.

Fixes:

- Lighten the antenna base to the CRT's shade grey (about #a8a39c), or put it on the CRT top as a 10x3 px lighter cap at about x 20-30, y 11-13, so the ears visibly attach.
- Optional: lift the VCR body to about #4a4e58 with a 1 px lighter top edge at y 60 so it separates from the cabinet's interior.

### sofa (reads at 1×: yes)

Problems:

- It reads instantly as a two-seat sofa, and the colour and line weight match the set.
- The outer arms touch the canvas edge: AA fringe at x=0 and x=80 (alpha 0.25) over y~28-46, with no safety margin when tiled next to other cells.
- Scale works: the interior between the arms (about x 13-67, 54 px) fits her 45 px sit sprite. The seat top is at y~30 (about 26 px above the floor), and on it the backrest top (y~6-8) reaches about her collar.

Fixes:

- Inset the whole sofa 1 px horizontally (arms from x 1-80 to 2-79) so the outline doesn't sit on the canvas edge.
- No other changes needed.

### bed (reads at 1×: yes)

Problems:

- The mattress and blanket are at different levels. On the pillow side the white mattress top is at y~30, with a wooden rail visible underneath (x 11-29, y 37-45). The blue blanket starts higher, at y~27, and drops straight to y~45 with no rail under it (x 37-78). So the bed reads as two stacked objects, a pillow bench and a blue box, not one mattress under one blanket.
- The folded-back sheet (x 30-36, y 25-45) is a tall white upright bar that rises above the blanket. At 1x it reads as a standing bolster or a second pillow on end, not a turned-down sheet.
- There's a stray light-grey sliver under the fold at about x 29-37, y 44-46, and a 1 px grey line under the blanket edge.
- Scale: the interior from headboard to footboard is about x 10-78, 68 px, but she stands 91 px tall (71 in walk), so at stand scale she can't lie in it.

Fixes:

- Run one continuous mattress top line from x 10 to x 78 at y~30. Make the blanket a slab over the mattress from x~31 to 78: top at y~29 (2 px proud of the mattress), draping down to y~41. Run the wooden side rail continuously under everything from x 10 to 78 at y 41-45.
- Replace the upright fold with a 3-4 px-wide white band laid along the blanket's head end (x 31-34), flush with the blanket top at y 29 rather than rising above it, plus a 1 px white lip along the blanket's top edge if more sheet is wanted.
- Remove the grey sliver at about x 29-37, y 44-46.
- Escalate the length to the orchestrator: if the 90 px box is fixed, the lying pose has to be curled or knees-up and fit within about 66 px from the pillow (x 12) to the footboard (x 78). Otherwise the bed needs about 100 px of interior, an 11-12 cell box.

## Revision (final state)

**Style as executed:** Bold-outline flat chibi (sticker / cel). The head is about 40% of standing height and the same size (about 35 px) in all three poses. Every shape has a 1.6 px warm-dark outline. Fills are flat with at most one hard-edged shade tone per material, with no gradients or texture. The hair is lifted to #6e4e3c so the fill edge holds the silhouette against a dark terminal. Eyes are simplified for 1x: a big white oval, dark iris, 1 px catchlight and a top lid line only.

**Palette:** `#3e2a2c`, `#fcdcc4`, `#eab89c`, `#6e4e3c`, `#553a2d`, `#8c6a52`, `#4a2e22`, `#1e100a`, `#ffffff`, `#ea8278`, `#cc6660`, `#fbf8f4`, `#d9d5df`, `#cf3540`, `#82343c`, `#622229`, `#9a4650`, `#d3d3de`, `#8c5030`, `#683a22`, `#f0c040`, `#b37b4b`, `#8c5a33`, `#5a3622`, `#6f9e90`, `#557d72`, `#d2cdc4`, `#a9a399`, `#3d6a82`, `#6aa3bd`, `#b4e0ee`, `#86a8d8`, `#6587bb`, `#4a4e58`, `#62666f`, `#1c1d22`, `#7fe08a`, `#1e2127`

Revision pass on the existing six documents. Each doc was revised with `easel amend` on its existing commits, so every doc keeps its commit structure, and all six were re-exported. The only reference was ref/ayumu.png; I opened no project source, SVGs or snapshots. A contact sheet of the six 1x dark previews is at sheet-sticker-1x.png, with a nearest-neighbour 3x copy at sheet-sticker-3x.png. Every 1x/2x export has alpha 0 at all four corners. Column 0 is clear on all six. On the sofa the last column is also clear, and walk no longer touches row 0 (its bbox now starts at y 2).

SCALE: ESCALATED TO THE ORCHESTRATOR
- I kept the spec box sizes and unified the head instead. The head is about 35 px from crown to chin in all three poses: stand y 5-40, sit y 5-40, walk y 2.7-35.4. The face no longer shrinks.
- Bboxes are now stand y 4-94 (90 px) against walk y 2-76 (74 px). The walk head is the same size as stand, so the remaining difference of about 15 px is in torso, skirt and legs only:
  - walk torso 13 px vs 15 in stand
  - walk skirt 10 px vs 14.6
  - visible leg 15 px (on the stride) vs 21
- A 45x95 walk box (5 rows, the same as stand) would remove the rest. With the 76 px box, walk is still one terminal row shorter than stand.

PALETTE (commit 1 of the three figure docs)
- Hair #5b4135 → #6e4e3c (L about 0.45) so the outer silhouette separates from #1e2127.
- hairS → #553a2d. New hairL #8c6a52 for one hard crown-shine band.
- Iris #6e4430 → #4a2e22 and pupil → #1e100a. Without this, the iris would have matched the new hair colour, which the critique did not flag.
- New skirtL #9a4650 for the sit skirt's top edge.

STAND
- Crown top lowered from y 2.2 to 5 and the crown-shine band added. The eye line is now about 60% down the head (was about 70%).
- Back hair ends at y about 50, tapered inward (x 9-36 at the tip).
- Torso narrowed to x 13.6-31.4. Arms are now outlined tubes with outer edges at about x 10.1 and 34.9, so the hair (x 7.2-37.8) overhangs by about 3 px each side and dark background shows below the hair tips. The alpha view shows a head-on-body step.
- Eyes redrawn for 1x:
  - white sclera 7x9 with no outer ring (sampled #fbf8f4, against about #9a9a9a before)
  - dark iris placed low and inward, a small pupil, a 1x1 catchlight, and a single top lid line; no lashes
  - I kept the sclera at 7x9 rather than the suggested 6x7, so a full-pixel white column survives anti-aliasing.
- Skirt flared to a hem of x 10.6-34.4 with bevelled corners and five 1 px darker pleats. I dropped the shaded panel.
- Thigh skin is now about 6 px above the socks (was 3).
- Cuffs are pure #fbf8f4 with one red line.

SIT (reads_at_1x was false)
- Uses the same back hair, torso, collar and head script as stand, so the silhouette now steps in.
- Knees: I deviated from the suggested fix. I tried two r 4.3 circles with crescent shading, and at 1x, framed by the white collar V, they read as a bust. I replaced them with two capsules in face-skin colour, one per leg, with round knee tops at y about 49. They touch at x 22.5 with a 1 px dark line between them, have a hard skinS shade on each outer shin, and cover the cord and toggle.
- Arms are now tubes that run from the shoulder down the outside of the knees and across the front of the shins at y about 58, with outer edges at about x 10 and 35. White cuffs meet small hands at the centre. I dropped the red cuff stripes here because at 1x they read as belt-buckle noise.
- Skirt narrowed to x 7.6-37.4, with a lighter maroon top edge and two fold lines.

WALK (reads_at_1x was false)
- Head is the same size as stand (see SCALE).
- Back hair starts at x 11-12 and ends at y about 46, about 7 px below the shoulder (y about 39), matching stand.
- Face skin runs from x about 24.5 to the front edge (about 12 px), with a nose bump at x 37.4, a chin and a 1 px mouth.
- Bangs and the side curtain are clipped behind the forehead. The eye is a tall white oval with the iris toward the front, a catchlight and a lid line.
- Torso rebuilt as a rounded box (x 18-29.6, y 37.4-51).
- Near arm is its own outlined tube swinging back across the collar flap, with a white cuff, red line and hand. Far arm swings forward behind the body in the shade tone.
- The collar flap (white with a red L-stripe) sits on the back and peeks out below the hair tip.
- The gait is unchanged.

TV
- Antenna base is now in the case's shade grey with a lighter top band, so the ears attach visibly.
- VCR lifted to #4a4e58 with a 1 px #62666f top edge.

SOFA
- Arms inset to x 2.5-78.5, so columns 0 and 80 are fully transparent.

BED
- One continuous mattress (y 29.6-41.2, x 10-80) and one continuous side rail (y 41-46).
- The blanket is a slab from x 31 to 82, 2 px proud of the mattress top, draping to y 41.6.
- The upright white fold bar is replaced by a flush 3.6 px white turned-down band at the blanket's head end.
- The grey sliver is gone.
- Length is unchanged and escalated: about 68 px of interior against her 90 px standing height (74 in walk). A 90 px box needs a curled lying pose; a straight lie needs about 100 px of interior, an 11-12 cell box.

EASEL LESSONS
- The bed's grey sliver came from `fill ... clip=fig`. The clip covers everything already painted on the layer, including the rail, not just the intended shape. Explicit rects fixed it.
- There is no rounded-rectangle shape; tv/sofa/bed still go .txt → pp.py → .gen → amend.
- ImageMagick's -splice with gravity corrupted the first contact sheet. Building it with per-image -extent fixed it.

Source scripts in the dir: pal.txt, furn-pal.txt, stand-body.txt, stand-top.txt, stand-head.txt (shared by stand and sit), sit-body.txt, walk-1.txt, walk-2.txt, tv/sofa/bed .txt and .gen, plus exp.sh.
