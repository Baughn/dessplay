# Phase 6 code map: her life around structures (shopping, decor, organising, persistence, the mind)

Last updated: 2026-10-10, against master `60727649`. Read-only map for planning phase 6
("Structures from the UI's own lines", proposal row 6). The terrain side (`read_ground`, poles,
ledges, climb conditions, covering) is in `map-terrain.md` beside this file. Paths are relative
to `dessplay/src/ui/houseguest/` unless they start with `docs/`. **Guess:** marks a design
choice or inference, not a finding.

## 1. The shopping channel and deliveries

| Item | Where | Note |
|---|---|---|
| `CATALOGUE` | mod.rs:166-180 | Sofa, Bed, Desk, Lamp, Bookshelf, Fridge, CatBed, Window, Plant, Poster. "The window is sold after the cat bed" is only this array's order. The TV isn't in it; the Clock isn't either (a gift). |
| `SHOP_EVERY = 3`, `FIRST_TV_VISIT = 2` | mod.rs:181-184 | Cadence is in **visits** (`ledger.visits`), not game days. |
| `advert(ledger, shop_now, needs)` | mod.rs:188-208 | The whole sell decision: due (`visits >= bought_on + 3`, or the stage's `shop_now`), idle (`ordered.is_none() && !home.boxed()`), owns a TV. It picks the first unowned decor item when `needs.pressing(Beauty)`, else the first unowned furniture (each falls back to the other). **No rarity or pity gate**: rarity.rs gates `ScriptId`s only (rarity.rs:1-20). |
| `Chances.advert` | osaka.rs:49-50; computed mod.rs:2955, 3008 | Recomputed every frame from the ledger and her needs. |
| Purchase commits at the channel's start | osaka.rs:5438-5475 | `bought = chances.advert` if a plain watch (no grievance due, not trying a spot, not a surf cue, not the first sunrise). It pushes `HomeEvent::Bought(item)` at once. |
| Recording it | mod.rs:3671-3690 (`record`) | `Bought` sets `ordered = Some(item)`, `bought_on = visits`, clears `shop_now`. `record` returns "changed", so the guest saves at once (decisions.md:3299). |
| `HomeEvent` enum | osaka.rs:406-432 | Bought, Unpacked, Crumpled, Used, SetDown, Calendar, Seen, Grieved. A stage (`Assembled`) would be one more arm here and in `record`. |
| First TV ordered for her | mod.rs:4086-4092 | `bought_on = visits - 1`, so it arrives the same visit. |
| Delivery | mod.rs:4093-4155 (`furnish`) | It waits for `awake` (not tucked in, not a dash, `osaka.awake`), `parcel_may_come`, `home.doorstep(...)` (room.rs:2419) and `clear_of_her`, then `home.add(prop)`. Sets `visit.flap`, `ordered = None`, says "A parcel!". Why a delivery waits is traced once per change (`ParcelWait`). |
| The flap through her door | room.rs:648-661 (`Flap { door: bool }`); door batch step 9 | It slides past her door's space in four beats. A wall flap is the fallback. design.md:1787-1841; decisions.md:5891. |
| Wall clock: a one-time non-catalogue gift | mod.rs:4156-4190; `Ledger.clock_sent` | The precedent for a delivery outside the catalogue. It never touches `ordered`/`bought_on`. |
| Boxed and unpacking | `Prop.boxed` (room.rs:663-680); `Shown::uses` → `[Unpack]` while boxed (room.rs:1437-1443) | Unpack is 4-6 s (osaka.rs:1787). At its end `HomeEvent::Unpacked` (osaka.rs:4820-4823) → `Home::unbox` (room.rs:1646) flips `boxed` only. "A parcel stays boxed, across visits, until she finishes unpacking it" (design.md:1839-1841). |
| `Want::Use(Unpack)` | brain.rs (def: base 40, `own_sake`, in_chat) | "Nothing comes close." Unpacking is **not** a home act (not counted in `home_acts`). |
| "Wishlist" | mod.rs:1508-1513 `Guest::wishlist()` | **Stage only**: the first `Furniture::ALL` item she doesn't own. Its one caller is `examples/houseguest.rs:260` (key `f` → `give`). Nothing of hers is a wishlist. The proposal names "the wishlist feeds the channel" only in row 6 (proposal:616) and never defines it. |
| Stage hooks | mod.rs:1490-1506 (`send_parcel`, `shop`), mod.rs:1517-1545 (`cue`) | `send_parcel` walks `[Tv] + CATALOGUE`. |
| Tests pinning catalogue order | tests.rs:5220 `the_channel_sells_decor_when_her_room_feels_bare` (asserts Bed then Plant), tests.rs:5574 `her_home_fills_up_over_visits`, tests.rs:5433, 5998, 11925 | A new catalogue entry shifts them. |

**What selling a stepladder or a gremlin would need.**
- Every channel type is `Furniture`: `advert` → `Option<Furniture>`, `Ledger.ordered: Option<Furniture>`, `HomeEvent::Bought(Furniture)`, `Spec.pitch` (what she says buying it), `doorstep(..., item)` → `Prop`. The cheapest path makes each a `Furniture` kind with a `Spec` row.
- The gremlin fits that path directly: decor, `beauty` > 0, `Offer::Decor`.
- **Guess:** the stepladder needs a kind whose box unpacks into something that isn't a `Prop`. Today `Unpacked` only flips `boxed`, so "assemble into a structure" is a new transition: the Prop is consumed into a `Structure`, or it stays a Prop with a stage.
- Gating: "sold when needed" (a broken `Connected`, a shelf wanted for the gremlin) would feed `advert` the way `needs.pressing(Beauty)` does today. That is the natural hook for "the wishlist feeds the channel" (**guess**).
- `ordered` is one slot, so a ladder competes with furniture and decor for it.
- `from_json` drops an `ordered` value it can't read (ledger.rs:225-228). An older build loses an ordered ladder or gremlin and keeps the rest of the record.

## 2. Decor and beauty

| Item | Where | Note |
|---|---|---|
| `Furniture` enum, `ALL` (12), `legacy()` | room.rs:20-47, 50-95 | `legacy()` covers the first eight kinds, which older builds know. Decor kinds return false. |
| `Spec` struct | room.rs:141-178 | name, pitch, footprint, ascii, ink, uses, offers, sit, comfort, `hang: Option<u16>`, `beauty: f64`, symmetric. |
| Decor rows | PLANT room.rs:296 (3×3, beauty 1), POSTER room.rs:310 (4×2, hang 4, beauty 1), CLOCK room.rs:325 (3×2, hang 4, beauty 0.5, symmetric, `Offer::Decor`), WINDOW room.rs:341 (4×2, hang 1, beauty 0, `Use::LookOut`, offers none). The furniture rows run from SOFA at room.rs:182. | `decor()` is `offers.contains(Decor)` (room.rs:70). So the clock counts as decor; the window doesn't. |
| `Offer::Decor` | room.rs:523-536 | "no room is named by it". |
| `beauty_at(shown, nooks, (x,y))` | mod.rs:229-245 | Sums `spec().beauty` of unboxed, non-made pieces on **the strip whose floor she stands on** (`raw_strips`). Off any strip it's 0, so a shelf's decor wouldn't count for the room below as written. |
| Beauty need | brain.rs:46-48, `Rising.plain` brain.rs:376-379; set from `chances.beauty_here <= 0` osaka.rs:3353 | It rises over 20 min only in a plain room. Rest and use there ease it by share × `beauty_here.min(1)` (osaka.rs:7475-7583). No want answers it directly. Needs start fresh each visit (beauty 0.3). |
| `Need::pressing` | brain.rs:475-479 | Ties count. Phase 4 found that needs clamp at 1, so decor sells on a tie (plan.md:2478-2480). |
| Gremlin | — | **None anywhere**: no code, no SVG, no art. It appears only in the proposal (lines 51, 259, 480, 616, 713) and plan.md:2430 ("the gremlin waits for shelves"). |
| SVG art | `art/props.svg` (`<g id="plant">`, `poster`, `clock`, `window-sky-*`, …), included at art.rs:13 | `parts(prop, layer)` art.rs:1373-1395 maps a kind to its groups. `prop_frame` art.rs:1902 sizes the frame from the footprint (20×42 units a cell). Parcel art is `art/delivery.svg`, scaled by footprint (`art::lean` art.rs:1347). |
| Art process | docs/proposals/…/decor/snippets.md (plant, poster); `phase5b/art/`, `phase5c/art/`, `vignettes/`, `door/art/` | Drawn in a throwaway worktree in the sticker style (docs/houseguest-style/README.md), sometimes by a `gen.py`. Shown as review sheets (1x, 3x, nn3x PNGs) from an ignored `art.rs` test keyed by env var (`HOUSEGUEST_CATALOGUE` art.rs:3258, `HOUSEGUEST_CLOCK` 2452, `HOUSEGUEST_VIGNETTES` 2048, …). `snippets.md` and `worktree.diff` are kept. Wired in only after the user approves ("model sheet first", phase5a-design.md:148-157; phase4-design.md:346). |
| Adding a kind: the compiler's checklist (wildcard-free matches, from Plant/Poster/Clock) | room.rs `ALL`, `legacy`, `spec`, new `Spec` const; mod.rs:166 `CATALOGUE`; ledger.rs:575-590 `RoomKind::of`; scrap.rs:~125-140 (`Made::of`); art.rs:1373-1395 `parts`; art.rs:1555-1578 piece states (if animated); props.svg group | Plus the ledger tests that pin kind lists (ledger.rs:1345-1392 `an_older_build_skips_the_clock_and_the_window`, `OLDEST_KINDS`). |

## 3. Organising (rules, wants, the cap, visits)

| Item | Where | Note |
|---|---|---|
| `Rule` enum | rules.rs:19-48 | Faces, Near, AgainstWall, Apart, Belongs, DoorClear, InChat. `Apart`'s doc says "a room is a strip, for now" (rules.rs:30). |
| `RULES` table (8 rows) | rules.rs:74-126 | Each `RuleRow { rule, felt_on: &[Use], grievance }`. Apart's row is Bed/Tv, felt on Sleep, "Too noisy to sleep...". DoorClear and InChat are felt on `ANY_USE` and on sight. |
| Judged on layout, never on the frame | rules.rs:1-8; `broken(laid, home, keep)` rules.rs:207 | `Connected` would need the live terrain's links, or a counted trigger (door trips). Today no rule reads terrain. **Guess:** it's a counter-triggered "felt" event, like DoorClear's bump-on-sight, not a layout judgement. |
| `kind(row)` labels | rules.rs:191-203 | Wildcard-free over `Rule`. |
| `Repair` | rules.rs:409-428 | `{ key, piece: Furniture, to: Placement, at: Shown, cost, tier }`. Move-shaped. A partition repair (raise a structure) doesn't fit. `search` (rules.rs:905), `check`/`check_again` (1053, 1061) and `Trials::ties` (1110) all assume a piece moves. `Repair::label` has an irrefutable `Strip::Bottom` (rules.rs:433). |
| Search budget | `CANDIDATES = 2_000` rules.rs:470; `REPAIRS = 3`, `SETTLED_BEHIND = 3` | Runs at paint, throttled (plan.md:2449-2453). |
| Felt rules | `Felt { key, on, let_go }` osaka.rs:2223-2230; `Osaka.felt` osaka.rs:2439 | "Each rule is felt once a visit, forgotten when she leaves" (design.md:2141-2152). |
| Home-acts cap | `Mood::home_acts` brain.rs:207-215 (Lazy 0, Ordinary/Dreamy 1, Industrious 3); `Osaka.home_acts` osaka.rs:2457 | Incremented once per episode at the first set-down taken (osaka.rs:9411-9415). Checked in `to_mend` (9639-9654) and `aloud_owed` (9214). |
| `Want::Arrange` | brain.rs:538-540; def `row(6.0, [(Nesting, 1.0)])` brain.rs:806-809 | Offered only while a felt rule is broken and the cap has room. Methods `ARRANGE` = use-it, carry, lift (mind.rs:274-278; guards mind.rs:650-720). |
| Pocket carry | `HomeEvent::SetDown` → `visit.set_down` → judged in `furnish` mod.rs:4033-4084 (`judge_move`) | The commit happens at paint. Success sets strip, anchor, facing, `settled = true`, `at`. |
| Trials | `Trials` rules.rs:1083-1160; keep roll in `arrange_next` osaka.rs:9500ff | Ties within `TIE_CELLS = 4`. |
| Nesting | brain.rs:43-45; `Rising.grieved` | Rises only while a felt rule is still broken. |
| Decision buckets | `Bucket { Reflex, Owed, Continuation, Normal }` osaka.rs:1136-1142 | The proposal's rule 4: "earlier visits never take the continuation bucket". A `Partial` ladder reaches her only through the normal roll (proposal:346). |

**What "a visit" means today.**
- A visit is `State::Visiting(Box<Visit>)` (mod.rs:886-966, `State` mod.rs:1052-1068). It is created by `begin_visit` (mod.rs:3237-3330) with a **new `Osaka`**.
- `Kind::Normal` increments `ledger.visits`; `Kind::Dash` doesn't (mod.rs:1121-1131). A dash still home at 12:45 becomes Normal and counts (mod.rs:2171-2179).
- Everything on `Osaka` is per visit: `felt`, `home_acts`, `episode`, needs, `lines`.
- `begin_day` (osaka.rs:6339-6373) starts a new game day inside a visit. It resets mood, rares, lines, `worked`, needs-by-clock and the lamp. It does **not** reset `felt` or `home_acts`.
- Per-game-day state lives on the `Guest`, in memory only, never saved: `rares_today`, `meal_today`, `night_today`, `lines_today`, and `said_aloud` (the DoorClear/InChat "once a game day", mod.rs:3716-3721).
- **The visit's length changed with 5b.**
  - A non-resident visit lasts from the idle gate to local input: minutes.
  - A resident's (the default) lasts from the 12:45 return to the 08:15 departure. Away at school ends a visit (phase5b-design.md:152-161), so a school day adds a visit.
  - On days off there is no Away, so **one resident visit can span a whole weekend**. Bedtime and the night are acts inside the visit. Work is a door trip inside it (`Shift`).
  - So "one repair a visit", "three door trips in a visit" and "`Partial{n}` waits for the next visit" sit on a unit that now ranges from minutes to about 2.5 game days.
  - The mood is the game day's (`day_seed`), so the cap's mood can change mid-visit while `home_acts` carries on.

**Counting door trips between rooms.**
- A trip through a door in space is `Act::Door { since, to: Through, gap }` (osaka.rs:760-764). `Through::Space(spot)` is a door in space; `Through::Home(DoorSpot)` is her external door (osaka.rs:1321-1342), so `Through::Home` is never between rooms.
- `through_door(to, at)` (osaka.rs:10354) is the one constructor of a `Through::Space` door. Its callers:
  - osaka.rs:4782, in `fire`: off the chat log's text after an errand's poke, "the way she came: by door";
  - osaka.rs:8810: the `Bind::Door(spot)` arm of binding a decision (from `mind::door_away`);
  - osaka.rs:10576, in `find_rest` (10553): to a calm spot `elsewhere` when she has no rest where she is;
  - osaka.rs:10606, in `head_for_errand` (10583): to the accordion when it isn't a short walk away;
  - osaka.rs:9182 (`door_to`: heading for a job with no route, from `go_to` around 9150-9170);
  - `mind::door_away` (mind.rs:440, the `travel/door` method), which binds `Bind::Door`.
- She takes a door only when `route(terrain, here, there)` is `None`, so a door trip means no terrain link exists. That is what `Connected` would detect.
- **Guess:** a counter keyed by (from strip, to strip), the strip of a spot found via `raw_strips` as `beauty_at` does. Whether it lives per visit on `Osaka` (like `home_acts`) or per game day on `Guest` (like `said_aloud`) is open. Ways out by her routine (school, work, the stage) also go through doors in space and should be excluded.

## 4. Multi-stage and persisted work

| What persists | Where | Note |
|---|---|---|
| A boxed piece | `Prop.boxed`, ledger `props[].boxed` | Binary. It waits across visits. |
| An order | `Ledger.ordered`, `bought_on` | One slot. |
| Unsettled deliveries | `Prop.settled` ↔ ledger `unsettled` (written only when non-empty) | A phase-4 top-level field. |
| Her door's wall | `Home.door` ↔ ledger `door` | Door batch. |
| A one-time gift | `clock_sent` | |
| Staged work that does **not** persist | `Scrap.stage` 0..`STAGES = 4` (scrap.rs:22-35). Progress derives from the use's span each paint (mod.rs:4684-4699). `Crumpled` commits only at the end (osaka.rs:4824-4831 → mod.rs:3691-3699). | A made piece lasts one visit. This is the visual precedent for ladder stages. `Assembled` needs a per-stage `HomeEvent` through `record` so the ledger saves at once. |

**The ledger** (ledger.rs):
- `VERSION = 1` (ledger.rs:56), key `houseguest_ledger` (ledger.rs:59).
- `Ledger` fields (ledger.rs:62-100): master_seed, visits, home, ordered, bought_on, clock, idle_min, rare_at, legend_at, calendar_on, seen, clock_sent.
- Write: `to_json` (ledger.rs:250-306) builds `Saved` (struct ledger.rs:593-630). Each newer field is `skip_serializing_if` default, so a record without it reads as before.
- Read: `from_json` (ledger.rs:164-248) goes through `Raw` (ledger.rs:651-690), where every field is `#[serde(default)]` and the newer ones are `Option<serde_json::Value>`, read leniently. Unknown top-level fields are ignored (no `deny_unknown_fields`). An older build therefore ignores a `structures` field, and drops it if it saves.
- Kinds: unknown pieces are skipped entry by entry. An unknown strip in `anchors` falls back to the room's pane: `an_unknown_strip_falls_back_to_the_rooms_pane` (ledger.rs:859-868) **already round-trips `{"Shelf":3}`**. An unknown `door` reads as None.
- Goldens: `the_record_as_written` (ledger.rs:773-790) pins the exact JSON. dump.rs:500-562 pins the `--dump` houseguest JSON. Each field has its own round-trip, garbage and older-build tests (ledger.rs:872-1392; helper `as_an_older_build_reads`).
- `--dump`: `Ledger::summary` (ledger.rs:320-386) → `Summary` (struct ledger.rs:391-430). `Owned.strip` is `format!("{:?}")`.
- **Adding `structures`** (by the pattern of `door` and `seen`):
  - a `Ledger` field, plus `Saved` (skip if empty) and `Raw` (`Option<Value>`, lenient, entry by entry);
  - a `Summary` field and the dump.rs golden;
  - round-trip, garbage-skipped and older-build-reads-past tests;
  - an update to the module doc (ledger.rs:1-46) and to design.md "Her record" (design.md:2641-2670), which lists what's kept and what an older build loses.
- **`Strip` is a one-variant enum**, `Strip::Bottom(Nook)` (room.rs:626-632). Five irrefutable `let Strip::Bottom(nook) = …` patterns break when `Shelf` is added: ledger.rs:205, 365, 553; room.rs:2321; rules.rs:433. Counts of `Strip::Bottom`: room.rs 50, rules.rs 17, ledger.rs 8, mod.rs 2, osaka.rs 2, tests.rs 19.
- `raw_strips(nooks)` (room.rs:805) makes strips from panes only. `beauty_at`, `clock_on`, `Home::extents`, door and rules all go through it, so a shelf strip changes its inputs (it would need the structures).

## 5. Methods and wants

| Item | Where | Note |
|---|---|---|
| `Want` enum and `ALL` (29) | brain.rs:518-541, 649-679 | A new want (say `Build`) goes in both. |
| `Want::def` (`DesireDef { base, serves, own_sake, factors }`) | brain.rs:683-811 | Arrange is `row(6.0, [(Nesting, 1.0)])`. The `Factor` doc: below 1 deletes a want, because `choose` keeps the top four (brain.rs:543-557, 849). |
| `all_lists_every_want` | brain.rs:1144-1170 | A kind index per variant (Arrange = 10). |
| `every_want_has_a_sane_row` | brain.rs:1210 | |
| Method table | `Method`/`m()` mind.rs:247-258; `methods(want)` mind.rs:298-317; `bind` mind.rs:416-428 | A `Bind::Job` must stand on a floor. `Bind` is at mind.rs:199-213. |
| `Osaka::credit_path(want, method)` | osaka.rs:7647-7720 | Wildcard-free. Every new want/method pair needs a path. Lint: `every_want_that_serves_has_a_credit_path` tests/credit.rs:19. |
| Stage-cue lint | `every_want_can_be_cued` tests.rs:10580-10638 | Exhaustive over `Want::ALL`. Each want maps to `Scene`s in `Scene::ALL` (stage.rs:170, 58 scenes). Also `every_script_can_be_cued` (10679) and `every_splice_can_be_cued` (10731). |
| `Scene` | stage.rs:28-169; labels stage.rs:~260-290; `furniture()` stage.rs:296-310; `cue()` 319; `Guest::cue` mod.rs:1517 | `Scene::Arrange` sets `arranging`, so the stage turns the sofa away (`stage_arrange` in `furnish`). |
| `Chances` | osaka.rs:29-89 | The only view of the frame that the mind has. Any new offer (a ladder to assemble, a broken `Connected`, a shelf wanted) must be a field here, as `advert`, `broken`, `repairs`, `lift_at`, `judged` and `beauty_here` are. Built in mod.rs around 2955/3008. |
| Adding a `Use` (say assembling): the compiler's checklist | room.rs:409-490 (`Use` enum, `ALL`, `inside`, `asks_room`, `lingers`); script.rs:525, 642, 744, 1453, 2465, 2512, 2546 (Unpack's arms; `UNPACK` keys at script.rs:2225); osaka.rs:1787 (duration table, `Unpack => (4_000, 6_000)`); stage.rs:296 `Scene::furniture()`; brain.rs `Want::ALL`, `def`; osaka.rs:7647 `credit_path`; tests.rs:10580 cue lint | Every use is a phase-5a `Script` (`Key`s and prelude/body/coda, phase5a-design.md), so "three stages of ~6 s" is authored there. `Crumple` is the closer precedent for the look: a use whose progress shows across its span (mod.rs:4684-4699). (Unpack's own keys, script.rs:2225, not read in detail.) |
| Arrange's offer path | `to_mend` → `chances.repairs` → `mind::lift` | It continues via `arrange_next` (osaka.rs:9508ff) in the Continuation bucket within a visit. |
| Censuses | tests/census.rs:1107 `visit_census`, 2310 `day_census`; tests.rs:10868 `sofa_census` | Phases record census rows in plan.md. The band and stillness gates (tests/band.rs, tests/stillness.rs) bound how much she moves. |

## 6. The rules from the docs

**design.md (Houseguest, from line 1339; "Last updated 2026-10-10").** Line numbers mark where each bullet starts.

| Topic | design.md | Rules (paraphrased) |
|---|---|---|
| Overlay, never covers | 1360-1369 | A post-render overlay. Never covers the input line, the Player Status, the keybinding bar or inline images (+1 margin). Never writes a protocol-image cell. A wide glyph is protected whole. |
| Terrain from the frame | 1365-1373 | Horizontal box glyphs with 4 open rows above are floors. Vertical borders are climbable. Ledge ends drop. Clamber over dividers. Edge step-outs. |
| Door in space | 1374-1384 | Used only with no route. It stays within her box and covers no terrain. |
| Her own overlays after every read | design.md:2619-2622 (the accordion's shake "painted after anything of hers reads the frame"); the rule is decisions.md:2263 | The proposal makes structures **the one exception**: they are terrain by design, written before `Terrain::read` (proposal:503). That needs a decisions.md amendment. |
| Choosing; beauty; mood | 1550-1600 | Ten needs. Beauty rises over 20 min only in a plain room and is eased by rest or use in a pretty room. Mood is the game day's. Day-carried state is in memory only. |
| Getting furniture (channel, parcels) | 1787-1841 | TV on visit 2. Then on a watch ≥3 visits after the last purchase, with nothing on order or boxed, she buys the next piece in the fixed order, decor first while beauty is most pressing, the moment the channel comes on. The clock is a gift. A parcel comes once she's up and in sight, through her door's flap if her door's space is kept, else a wall flap. It waits for her acts and her path. It lands where she can stand to unpack it and fit to use it, never in the chat or her door's space. It stays boxed across visits until unpacked. |
| Furniture; **Strips and anchors** | 1842 (bullet); 1850 (sub-anchor) | A strip is a quiet pane's bottom border (Users, Playlist, List; never chat). Anchor = wall side + offset. Pieces pack in anchor order. They show only on unbroken lines over blank cells, else the closet. A gone strip moves its pieces together to the first strip that holds them. |
| **No new piece in the chat pane** | 1869 (same bullet) | Nothing of hers is newly put in the chat or her door's space: deliveries, gifts, repairs, moves, or anything they push. Makeshift pieces are exempt from the chat half only. |
| **The closet for the chat** | 1882 | A piece in the chat that no room would take is hidden for that frame, never moved. |
| **Her door's space** | 1897 | One wall, chosen from the panes (screen edge first) and saved. 6 columns × her height × the floor row are kept. The floor lane packs beside it. Hung pieces keep out but for poster and clock. It yields when keeping it would cost a piece. |
| **Wall pieces** | 1925 | Hung 4 rows up in a wall lane; the window 2 rows up. Only the floor lane moves a room. |
| **Rooms from contents** | 1957 (Using her furniture follows at 1969) | Rooms are strips. Role is the first table row met: living (screen+seat, no bed), bedroom (bed, no screen or fridge), study (desk), kitchen (fridge, no bed), else den. A placed piece and its floor are solid to text. |
| The rules of her home | 2101-2152 | The table. Judged on layout. Felt on a use, two frames; DoorClear and InChat also on sight. Each felt once a visit and forgotten with it. Nesting rises only while a felt rule is broken. Cap by mood 0/1/1/3 a visit. |
| Putting her home right | 2153-2211 | A satisfied rule never moves anything. The cheapest move that breaks nothing; no den made; nothing spoilt; the kept door space stays kept; ≤2,000 ways. The pocket carry. Three tries. Let go → "Oh well..." and the rule stays this visit. |
| Trials | 2212-2235 | |
| Makeshift furniture | 2236-2292 | |
| Her clock / routine / Away / night | 2293-2467 | |
| Window and wall clock | 2525-2574 | |
| Resident | 2575-2607 | The focused pane is protected. Its contents rain out and its furniture goes to the closet. |
| Her record | 2641-2670 | Version 1. What older builds keep and lose. "Pieces, rooms or strips a build doesn't know are skipped." |

**decisions.md entries** (line, date, title):

| Line | Date | Title |
|---|---|---|
| 1899 | 2026-09-28 | Houseguest is a post-render overlay |
| 1921 | 2026-09-28 | Houseguest terrain comes from the rendered frame |
| 1994 | 2026-09-28 | Houseguest furniture is pane-relative and leaves with her |
| 2049 | 2026-09-28 | Houseguest has a door in space |
| 2085 | 2026-09-29 | Houseguest shops, and what she buys comes boxed (a purchase commits at the channel's start) |
| 2104 | 2026-09-29 | Houseguest keeps a local record |
| 2227 | 2026-10-01 | Houseguest makes furniture of torn-off text |
| **2263** | **2026-10-01** | **Houseguest reads the frame she stands on** ("her own overlays … are painted after every read"; one definition of her image, `terrain::image`) |
| 2305 | 2026-10-01 | Houseguest rechecks where she rests |
| 2536 | 2026-10-02 | Her home stands on strips, and its rooms are what's in them |
| 2613 | 2026-10-03 | She feels what's wrong with her home, once a visit |
| 2663 | 2026-10-03 | She puts her home right, with the piece in her pocket |
| 2746 | 2026-10-03 | She tries a piece in a spot or two |
| 2813 | 2026-10-03 | Pieces hang on the wall |
| 2865 | 2026-10-03 | A bare room wants decor |
| 2975 | 2026-10-03 | A room is spoilt only by what it forbids |
| 3299 | 2026-10-04 | Her record: events at once, time in batches, and on exit |
| 3334 | 2026-10-04 | Away at school ends the visit at her door |
| 3401 | 2026-10-04 | The day is the unit |
| 3530 | 2026-10-05 | A parcel comes only when she's up and can stand to unpack it |
| 3555 | 2026-10-04 | Her window and her wall clock (the window sold after the cat bed) |
| 5086 | 2026-10-07 | Where she unpacks a parcel is judged with it there |
| 5322 | 2026-10-08 | Her door has its own space at the screen's edge |
| 5460 | 2026-10-08 | She walks to her door; her line is latched apart from `leaving` |
| 5520 | 2026-10-08 | Work goes out by her door |
| 5581 | 2026-10-08 | No furniture in the chat pane |
| 5643 | 2026-10-09 | She feels her door blocked and a piece in the chat on sight |
| 5744 | 2026-10-09 | Makeshift pieces may stand in the chat pane |
| 5771 | 2026-10-09 | Her open door follows a resize in the frame it's drawn in |
| 5812 | 2026-10-08 | Her door is drawn side-on |
| 5891 | 2026-10-10 | Parcels come in through her door's flap |
| 5941 | 2026-10-10 | A way out in space is clear of her pieces (`osaka::Clear`) |

## 7. CHANGELOG.md

The format is `## YYYY-MM-DD`, newest first; `- Category: text`; continuations indented two spaces; append only. Entries are worded for the player. Examples:

```
## 2026-10-10

- Added: parcels come in through the flap in Osaka's front door.

## 2026-10-09

- Added: Osaka moves furniture out of her door's way, or out of the chat
  pane, herself ("Can't get to the door!").
```

## Open questions (for the phase-6 design)

1. **What unit is "a visit" for phase 6?** Should door trips for `Connected`, "one build stage a visit" and `Partial` waiting "for the next visit" be counted per visit (on `Osaka`, like `home_acts`) or per game day (on `Guest`, like `said_aloud`, in memory)? A resident visit runs from 12:45 to 08:15, and over a whole weekend. `begin_day` resets neither `felt` nor `home_acts`.
2. **Rooms ≠ strips.** A partition splits one strip into two rooms. Today `role_of`, `Home::rooms`, `Apart`, `beauty_at`, the anchors (from a strip's walls) and the repair tiers all treat a room as a strip. Is a partition a new wall inside the strip, so that anchors count from it, or a room boundary only? (The biggest model question at the room/terrain seam; see map-terrain.md.)
3. **The partition repair isn't a move.** `Repair { piece, to }` and `search`, `check` and `Trials` are move-shaped. Weighing "raise a partition" against "carry the bed" needs a sum type, or a parallel search.
4. **`Connected` isn't a layout rule.** Rules are judged on layout and never read terrain. Is it a `RULES` row at all, or a counter-triggered "felt" (like DoorClear's bump), with the build as its repair? (**Guess:** the latter.)
5. **Ladder: Prop or structure?** It arrives boxed as a `Furniture` parcel on a strip (that's all the channel can sell). Assembling it must turn it into a structure between two floors, a transition nothing has today (`Unpacked` only flips `boxed`). Where do `Partial{n}` and `Assembled` live: a stage on `Prop`, or a new `structures` ledger field?
6. **What is "the wishlist"?** Undefined in the proposal. The existing `Guest::wishlist()` is a stage helper. **Guess:** a want for a structure (or a shelf for the gremlin) steers `advert` the way beauty does. Does it bypass `SHOP_EVERY`?
7. **Gremlin on a shelf vs `beauty_at`.** `beauty_at` counts only the strip she stands on. A shelf would be its own strip, so the gremlin wouldn't prettify the room below unless that changes. Can she stand on a shelf (4 clear rows)?
8. **Cadence under the clock.** `SHOP_EVERY = 3` counts visits. With resident visits lasting half a day to a weekend, three visits can mean several real days. Do the extra phase-6 catalogue entries need a different cadence?
9. **Gremlin art.** None exists. It needs a model sheet first in the sticker style (art.rs review test, snippets.md, user approval), plus its boxed parcel. Structures need no SVG by design (proposal:488-492).
10. **Is a build stage a home act?** Unpacking isn't (base 40, `own_sake`), but the proposal's Q10 counts "a repair or a build stage" against the 0–3 cap. Assembly then can't reuse Unpack's want row.
