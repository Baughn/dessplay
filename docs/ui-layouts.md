# Terminal layout authoring

The layout migration is in progress. Currently the shared settings/List-entry
forms, their semantic label/value/annotation rows, category tabs and notes, the F11 log viewer's
controls, dropdowns, log viewport, and footer, application pane composition, chat/input/suggestions,
attachments, rich message variants, recent-chat projection, separate subtitle rows, Users, Playlist, all Series modes,
file/episode browsers, AniDB/Nyaa searches, local-copy offers, confirmations,
name editing, changelog entries, status text, keybindings, health metrics and progress, and the roguelike frame, game, recovery, document, equipment, and ending pages
use local templates. Other pane interiors still use their existing presentation adapters. See [the implementation tracker](plan.md#phase-36-runtime-editable-display-layouts)
for the remaining work; exporting defaults does not yet expose every pane.

## Files and recovery

`dessplay layout init [PATH]` exports the editable defaults. Existing files
are kept, including files whose contents differ from the defaults.
`dessplay layout check [PATH]` compiles the complete candidate without starting
a terminal, player, network connection, or storage session.

The default directory is `dirs::config_dir()/dessplay/ui` (on macOS, normally
`~/Library/Application Support/dessplay/ui`). `--layout-dir PATH` overrides
it. `--builtin-layout` starts with embedded definitions. XML files in
`templates/` replace templates by name; missing templates fall back to the
binary. `style.css` follows embedded CSS in the cascade. Template files are
read in filename order; duplicate names within the custom bundle are errors.

Changes are watched, including rename-based editor saves, and debounced for
200 ms. Relative directory arguments are normalized for event matching. The
watch survives creation or replacement of the override directory. Compilation
happens on a worker thread. A complete valid candidate
is installed at a frame boundary; an invalid candidate leaves the last good
layout installed. Startup initially uses the embedded layout. Files and
layout state never enter the synchronized replica.

F12 opens layout tools even above another modal. It shows the directory,
latest compiler diagnostic, template identities, matched declarations, and
arranged bounds from templates rendered in this session. Up/Down scroll;
`r` reloads and resumes custom layouts; `b` selects bundled layouts for this
session; `d` resets drag sizes for the active layout; Esc closes the tools.
This view always renders from embedded definitions. Opening it cancels
pointer grabs and roguelike recovery, while retaining editors, modal state,
and held text selections. Drag proportions are stored in the local database,
keyed by layout source, content revision, and stable split ID. Successful file
changes clear that source's drag sizes, including changes discovered on restart.
Old pane percentages are imported once into the bundled named splits.
Ordinary settings saves and layout edits never write over each other.

## Contract version 1

```xml
<templates version="1">
  <template name="form">
    <column id="form" title="title">
      <slot name="header" />
      <scroll><slot name="body" /></scroll>
      <slot name="notes" />
      <slot name="save" />
    </column>
  </template>
</templates>
```

Unknown versions, elements, attributes, bindings, properties, and values
are errors. The parser accepts XML comments, escaped attribute values, and
an optional XML declaration. Text comes only from bindings, never literal
markup or message contents. DTDs, external resources, scripts, expressions,
and processing instructions are unsupported.

`row`, `column`, `grid`, and `box` compose children. `scroll` declares a
clipped scroll viewport whose controller supplies the visible contents.
`overlay` declares a separate paint layer aligned to the bottom of its
parent content, inset one cell horizontally. Its dimensions and margins
are styleable; `placement="center"` centers the allocated box with a one-cell
inset on all sides. `placement="modal"` centers a full dialog within the entry
viewport: percentages floor to whole cells and minimum sizes yield to the
viewport. Nested overlays paint after their ordinary content; later sibling
overlays cover earlier ones. Primitive slots participate in this same order.
`placement="after"` attaches a popup below its parent control
and lets it escape that control's clip while staying inside the entry viewport. `placement="before"` attaches above its parent and may paint
on the entry border, for measured captions containing an editor cursor. Capture remains in controllers. `text bind="field"` paints typed text;
`slot name="field"` reserves a primitive's content rectangle. Containers accept
`title` and `title-bottom` bindings for terminal frame captions.
`if="field"` tests a named boolean. `id`, `class`, and `style` supply selector
identity, whitespace-separated classes, and inline declarations. `title`
names a text binding placed on the enclosing border.

Reusable helpers use `<use template="helper-name"/>`. Helpers are named
templates and inherit the caller's binding contract. Unknown references,
cycles, duplicate expanded IDs, and repeated controller slots are errors.
Unused names outside the entry-template schema are errors, to catch typos.
Form collections use stable controller keys and only instantiate visible
rows. Small semantic lists also support XML-authored keyed repetition; direct
image bindings remain outstanding.
Chat recent-history projections remain explicit controller-owned views.

`flow` combines text, rich, and nested flow children into shared measured lines.
`separator=" "` supplies the text between nonempty visible children (one space
by default); nested flows let metric groups use different separators. Wrapping,
alignment, indentation, and box allocation belong to the outer flow. Inline
children accept text colors/emphasis/custom properties and `display: none`;
box properties on inline children are errors. `rich bind="field"` accepts typed
styled spans and opaque existing controller actions. Message data never becomes
XML. The `rich-text` entry exposes a rich `body` field. Source character ranges
and action identities are emitted from the measured paint fragments.

`repeat bind="list"` contains one item root, which may use a helper template.
Its children use the list's typed item contract rather than the enclosing
view's fields. Rust supplies nonempty unique keys; reordering a list preserves
those identities, and `:selected` follows the selected item. IDs inside an item
are local for CSS matching and qualified by list/key in arranged records.
Repetition has no expressions or markup generated from message text.

```xml
<repeat bind="tabs" style="flex-direction: row; gap: 1ch">
  <flow><text bind="label"/><text bind="missing" if="invalid"/></flow>
</repeat>
```

The `form` contract exposes lists `tabs` (`form-tab` items) and `note-items`
(`form-note` items). The default header delegates to `form-tabs`, whose list is
`tabs`; the note region delegates to `form-notes`, whose list is `notes`.
`form-tab` exposes `open`, `label`, `missing`, `close` text and `invalid` boolean;
`form-note` exposes plain `body`. Edit these helpers to restyle the fields, or
put a repeat directly in the main form to move the list. The default header
reserves one line; change `#form-header` height when arranging tabs vertically.
Controller-owned editors and Save/footer reservation retain their existing
behavior. Repeats eagerly arrange small lists (at most 1,024 items each and
65,536 layout nodes per entry). Long history and browser collections continue
using the virtualized row renderer.

Current entry templates and bindings:

| Template | Text | Slots | Booleans |
| --- | --- | --- | --- |
| `form` | `title` | `header`, `body`, `notes`, `save`, `editor`, `error` | `editing`, `invalid` |
| `form-row` | `label`, `value`, `annotation` | none | `labelled`, `annotated` |
| `log` | `title`, `session-label`, `startup`, `app-label`, `app-level`, `other-label`, `other-level`, `picker-end`, `footer`, `unavailable` | `body`, `app-options`, `other-options` | `choose-app`, `choose-other`, `has-unavailable`, `available` |
| `log-option` | `label` | none | none |
| `layout-tools` | `title`, `body` | none | none |

The form's editor frame/error layout is authored in its overlay template;
the text-editor primitive owns only the text and cursor. Log controls and their
attached dropdowns are authored together; option rows retain controller keys.
Moving either control moves its dropdown anchor without changing which logging
scope it edits.
Form body rows remain navigable while scrolled offscreen. Model row identities
preserve selection across data changes. Secret values are masked before
presentation; the inspector never prints binding contents.

## CSS subset currently implemented

Selectors: element, universal `*`, `.class`, `#id`, compound selectors,
descendant and child `>`, comma lists, and `:focus`, `:selected`, `:disabled`.
Specificity and source order apply; inline declarations follow stylesheet
declarations. Custom properties inherit and `var(--name, fallback)` supports
fallbacks. Undefined and cyclic variables are errors, including variables
used only under an ancestor's focused/selected/disabled state.

Properties:

| Group | Supported properties / values |
| --- | --- |
| Display | `display: flex/grid/none`, `flex-direction: row/column` |
| Flex | `flex-grow`, `flex-shrink`, `flex-basis`; no wrapping |
| Dimensions | `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height` |
| Spacing | `margin`, `padding` (one to four values), `gap`, `row-gap`, `column-gap` |
| Alignment | `align-items`, `align-self`: start/end/center/stretch; `justify-content`: start/end/center/space-between/space-around |
| Grid | `grid-template-columns`, `grid-template-rows`, `grid-column`, `grid-row` |
| Color | `color`, `background-color`, `border-color`: #RGB, #RRGGBB, default, black/red/green/yellow/blue/magenta/cyan/white/gray/darkgray |
| Borders | `border`, `border-top`, `border-right`, `border-bottom`, `border-left`: `0/none/1/1ch/1lh`; single-cell solid edges |
| Text | `font-weight: normal/bold`, `font-style: normal/italic`, `text-decoration: none/underline`, `text-align: left/center/right`, `white-space: normal/nowrap/pre`, `text-overflow: clip/ellipsis`, terminal `hanging-indent: Nch` |

Lengths use `ch`, `lh`, percentages, `0`, and `auto` where meaningful.
Terminal allocation treats a length unit as a cell in its allocation axis.
There are no pixel units, negative lengths, wrapping flex rows, `calc`,
at-rules, `!important`, or browser positioning. Grid tracks support fixed
lengths, percentages, `auto`, intrinsic unwrapped `max-content`, `fr`, and
`minmax(min, max)`. Placement uses positive lines and `span N`, optionally
separated by `/`. Grid repetition is not supported.

A component root sits in a single-cell viewport grid. Auto dimensions stretch
within that viewport; explicit dimensions and margins retain their CSS meaning.
Natural row heights include outer margins, while margin cells are not hit targets.
Horizontal allocation uses unwrapped intrinsic widths. Shared edges snap to
cells; the renderer freezes widths before measuring wrapped heights. The
same measured fragments paint text and retain source offsets. Parent clips
bound all painting, including original border edges of oversized children.
The form footer reservation remains an explicit measured-content policy;
row labels/values/annotations have their own boxes. Path values retain their
ends when clipped. The legacy theme adapter still runs during migration,
but preserves explicit backgrounds and maps RGB text to the finite terminal
palette in limited mode. Image regions bypass this conversion.

## File-only examples

Move a pane by moving its `slot` in `templates/app.xml`. For example, swapping
`<slot id="users" name="users" />` and `<slot id="series" name="series" />`
changes their positions and Tab order. Hide a pane with `#users { display: none; }`.
Focus stays on a surviving pane or moves to the next visible pane; F12 remains
available even with every pane hidden. Hidden controllers keep their drafts
and selections. A scroll viewport's offscreen rows remain navigable.

Resizable rows and columns declare `resizable="true"` and an `id`; all direct
children require stable IDs. Handles follow the actual adjacent boundaries,
including gaps. Dragging trades space between that pair and preserves other
children, with a ten-percent container share minimum where it fits. A resizable
container must use flex layout. CSS grid composition remains available without
drag handles. `app` bindings are the slots `chat`, `subtitles`, `series`, `users`,
`playlist`, `health`, `status`, `keybar` and boolean `separate-subtitles`.

The roguelike frame and game page are in `templates/rogue.xml`. `rogue` has
`title`, `footer`, `notices`, `error`, a `body` slot, and `has-notices`/`has-error`
booleans. `rogue-game` exposes `map`, `sidebar`, and `journal` slots, objective,
weapon and reach text, and individual labeled depth, blood, breath, pain,
bleeding, supply, nutrition, and gold fields. `wide` is true at 80 content cells;
removing that condition makes the sidebar available at smaller widths.

To put the sidebar on the left, move its slot before the map slot and change
its margin to `#rogue-sidebar { margin: 0 1ch 0 0; }`. The default map minimum
and bounded journal height preserve the existing layout. Sidebar wound/threat
summaries remain bounded measured content; moving the sidebar does not affect
expedition or recovery state. `rogue-recovery` exposes title/footer/phase,
individual label/value fields for blood, breath, nutrition, bleed, pain, linen,
splints, and food, `linen-used`/`splints-used`/`food-used` annotations, and a
bounded `wounds` slot. Its metric flows can be reordered or restyled separately.

`rogue-inspection` supplies `heading`, optional `note` (`has-note`), and a `body`
slot. `rogue-equipment-row` has `description`; `rogue-document` has plain `body`;
`rogue-epitaph` has `heading`, `summary`, and `actions`. Condition-row composition
and bounded wound/threat children still need the remaining semantic migration.

Built-in semantic color variables are `--surface`, `--text`, `--muted`,
`--accent`, `--danger`, and `--dungeon-border`. The last follows the existing
injury effect policy. Authored custom-property declarations override semantic
values; CSS `border-color` can also override the frame directly. Colors include
the six `lightred`/`lightgreen`/`lightyellow`/`lightblue`/`lightmagenta`/`lightcyan`
terminal names alongside the colors listed above.

Users and Playlist composition and rows live in `templates/collections.xml`.
`users` and `playlist` expose `title` and a `body` slot. `user-row` fields are
`name`, `status`, `last-seen`, and `holders`; booleans are `online`, `offline`,
and `seeders`. Seeder summaries are read-only. `playlist-row` fields are
`marker`, `title`, `download`, `temporary`, and `watch`; booleans are `entry`,
`show-download`, and `show-temporary`. The synthetic Add New row uses the same
template with `entry` false.

Move the watch column before the title using only the row template:

```xml
<template name="playlist-row">
  <row id="playlist-row">
    <text class="playlist-watch" bind="watch" if="entry" />
    <text class="playlist-marker" bind="marker" />
    <text class="playlist-title" bind="title" />
    <text class="playlist-download" bind="download" if="show-download" />
    <text class="playlist-temporary" bind="temporary" if="show-temporary" />
  </row>
</template>
```

The watch column receives an intrinsic width from the widest unpadded watch
label; CSS can override it. The default title minimum reserves six cells plus
the two-cell marker before less important columns clip at the viewport edge.
Rows measure their height after width allocation. For example,
`.playlist-title { white-space: normal; }` wraps titles, and row padding adds
space without changing which episode a click selects. Only rows around the
viewport become layout trees; measurements cache by semantic key, presentation,
style revision, and width. Keyboard selection remains controller-owned and
available for offscreen items.

Chat composition lives in `templates/chat.xml`. `chat` has slots `log`,
`suggestions`, `input`, title bindings `title` and `input-title`, and boolean
`suggesting`. `chat-suggestion` exposes `signature` and `help`. `recent-chat`
is an explicit read-only projection with slot `log` and text `title`; it does
not own a second editor or affect chat scroll state. `subtitles` has slot `body`
and text `title`; `subtitle-row` exposes `timestamp`, `speaker`, `body` and boolean
`named`. Subtitle keys survive progressive cue updates.

`chat-attachment` exposes intrinsic slots `timestamp-gutter` and `image`. The
bundled gutter measures the timestamp plus its separator. The image slot's
border, padding, and margins count toward the one-third-of-log-height budget.
An unusable interior leaves the plain link. Frame rows are not selectable.
Scrolling crops the original fitted frame and pixels, preserving the encoding
size; text-theme conversion never recolors the pixels. Modals and recovery
views still suppress terminal graphics.

For example, these file-only changes restore a flush-left, borderless image:

```css
#timestamp-gutter { display: none; }
#chat-attachment-image { border: 0; }
```

Change continuation indentation independently of message prefixes:

```css
#chat-log-frame, #recent-chat-frame { hanging-indent: 4ch; }
```

`hanging-indent` is inherited and accepts a cell length. The default chat value
is two cells; general template text defaults to zero. Source anchors preserve
scrolled-back context when the width or layout changes and when new messages
arrive. The `chat-message` template exposes `timestamp`, `origin`,
`action-marker`, `sender`, and `sender-delimiter` text, `external`, `action`,
and `named` booleans, and rich `body`. The `chat-separator` template exposes
`label`. All message variants use these fields; user text stays literal.

A `prefix` may be the first child of an outer `flow`. Its text stays on the
first line; the body wraps through the remaining cells and continues at the
authored hanging indent. If the first body glyph is too wide for the remainder,
it starts on the next line when wrapping is enabled. Move the prefix fields
into a separate row to put sender and timestamp above the body.
Each rich binding appears once per entry; a secondary view uses an explicit
read-only projection. Rich source ranges exclude spoiler animation marks,
so selections and spoiler clicks use the same character positions as painting.

Restyle a form in `style.css`:

```css
#form { background-color: #18202a; color: #e0e8ef; padding: 0 1ch; }
.form-label { width: 20ch; font-weight: bold; }
.form-annotation { color: #a5bdce; font-style: italic; }
```

Move annotations before values by changing only `templates/form-row.xml`:

```xml
<templates version="1">
  <template name="form-row">
    <row class="form-row">
      <text class="form-label" bind="label" if="labelled" />
      <text class="form-annotation" bind="annotation" if="annotated" />
      <text class="form-value" bind="value" />
    </row>
  </template>
</templates>
```

Hide form notes with `#form > slot` only if you intend to hide all direct
slots; selectors cannot match binding names. For one slot, give it an ID in
the template and use `#form-notes { display: none; }`.

The parser caps source files at 1 MiB, XML nesting at 64, expanded templates
at 4096 nodes, and conditional style validation and variable expansion at
bounded work budgets. An over-budget candidate is rejected rather than
blocking or allocating without bounds on the UI thread.

## Series rows

`series` exposes `title`, `filter-label`, rich `filter`, boolean `filter-visible`,
and a `body` slot. Its caption is an attached flow, so the filter cursor keeps
its measured cell when the pane moves. `series-row` exposes `title`, `year`,
`marker`, `count`, `unavailable`, `name`, `nero`, `episode`, `available`, and
`watchers`; booleans are `franchise`, `heading`, `entry`, `has-year`, `has-nero`,
and `unlinked`. Move these text elements to reorder columns or put them on
separate rows. Group headings and entries retain controller identities across
reload, wrapping, and changes in the List's ordering.

## Browsers, searches, and small dialogs

Browser and search definitions are in `templates/collections.xml`.
`file-browser` exposes `title`, `filter-label`, rich `filter`, boolean
`filter-visible`, and slot `body`. `file-row` exposes `marker`, `name`, and
boolean `entry`, including directory, file, parent, selection, and note rows.
`episode-browser` exposes `title` and slot `body`. `episode-row` exposes
`marker`, `episode`, `filename`, `holders`, `gutter`, `title`, `count`, `note`;
booleans are `file`, `has-episode`, `child`, `season`, `branch`, and `empty`.
The default episode row gives filenames priority over holders when narrow.
Change its flex rules to allocate a reserved holder column instead.

`anidb-search` and `nyaa-search` expose `title`, `message`, `editor`/`body`
slots, and `editing`, `has-message`, `has-results` booleans. Both reuse the
`search-dialog` helper. `anidb-result` exposes `title`, `matched`, `series`,
and boolean `alias`. `nyaa-result` exposes `filename`, `title`, `size`,
`seeders`, `stage`, `progress`, and booleans `alias`, `result`, `active`.
Search editors, results, in-progress work, and errors retain controller state
across reload. Search result fields can become columns or separate rows.

`confirm-dialog` exposes `title`, `prompt`, `yes`, `no`; `name-dialog` exposes
`title`, `note`, and slot `editor`; `copy-dialog` exposes `title`, `filename`,
`note`, and slot `body`. `copy-row` exposes `filename` and `evidence`.
Their capture and keyboard actions stay in the existing modal controllers.

## Changelog, status, and keybindings

`changelog` exposes `title`, `ok`, and slot `body`. `changelog-row` exposes
`date`, `bullet`, `category`, `body`, and booleans `day`, `entry`, `categorized`.
Its prefix and continuation indentation use the shared flow service. The
controller retains an item/source anchor across width and layout changes;
only the visible document window and rows needed to navigate are measured.

`templates/chrome.xml` owns `status` and `keybar`. Status fields are `marker`,
`state`, `blockers`, `now-label`, `title`, with `blocked` and `has-title`
booleans. Keybar has a keyed `bindings` list. Its `keybinding` item exposes
`separator`, `key`, `label`, and boolean `separated`; changing their layout
never changes the keymap. Health/progress composition remains under migration.

### Health and form chrome

`health` supplies `progress`, `middle`, and `metrics` slots. Their default
priority policy measures unwrapped `health-metrics`, reserves at least four
useful cells plus the authored middle padding for a suggestion, and gives the
remaining width to `health-progress`. Progress truncates before the suggestion
disappears. Metrics may clip when the whole row is too narrow. Rearrange the
slots in `health` or fields in their templates to change the composition.

`health-metrics` repeats keyed `metrics` through `health-metric`. Each item
supplies `label`, `value`, `separator`, `up-marker`, `down-marker`, `upload`,
`download`; conditions are `ordinary`, `bandwidth`, `suffix`, `separated`.
`health-progress` exposes `open`, `fill`, `close`, `elapsed`, `slash`, `duration`
and boolean `available`. The fill is the terminal progress primitive; it
contains only fill cells. `health-middle` supplies text `body` and slot
`marquee`, with `suggestion` and `animation` conditions. Warning suggestions
keep precedence over marquee animation. The marquee's moving slice remains an
intrinsic primitive inside the authored padding and clip.

`settings-form` and `list-edit-form` wrap the shared `form` in modal overlays.
The default `.form-modal` is 70% by 70%; the List editor uses 60% by 60%.
Change those rules to resize the outer dialog. Header and notes may shrink;
the body yields before the fixed Save footer. The Save slot renders
`form-save`: `save-label`, `save-needs`, `save-hint`, `save-blocked`, and
`:selected`. The error slot renders `form-error` with text `error`.
Editors, validation, and Save behavior remain controller-owned.

Form rows supply raw action labels in `value`; `action`, `open`, and `close`
let templates control their enclosing brackets. Secret values remain masked
before they enter presentation data.
