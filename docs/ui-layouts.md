# Terminal layout authoring

The layout migration is in progress. Currently the shared settings/List-entry
forms, their semantic label/value/annotation rows, the F11 log viewer's
header/body/footer, application pane composition, chat/input/suggestions,
attachments, recent-chat projection, and separate subtitle rows use local
templates. Message rich-text composition and other pane interiors still use
their existing presentation adapters. See [the implementation tracker](plan.md#phase-36-runtime-editable-display-layouts)
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
200 ms. Compilation happens on a worker thread. A complete valid candidate
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
are styleable; capture remains in controllers. `text bind="field"` paints typed text;
`slot name="field"` reserves a primitive's content rectangle.
`if="field"` tests a named boolean. `id`, `class`, and `style` supply selector
identity, whitespace-separated classes, and inline declarations. `title`
names a text binding placed on the enclosing border.

Reusable helpers use `<use template="helper-name"/>`. Helpers are named
templates and inherit the caller's binding contract. Unknown references,
cycles, duplicate expanded IDs, and repeated controller slots are errors.
Unused names outside the entry-template schema are errors, to catch typos.
Form collections use stable controller keys and only instantiate visible
rows. XML-authored keyed repetition, rich-text/image bindings, and explicit
read-only projections are not implemented yet.

Current entry templates and bindings:

| Template | Text | Slots | Booleans |
| --- | --- | --- | --- |
| `form` | `title` | `header`, `body`, `notes`, `save`, `editor`, `error` | `editing`, `invalid` |
| `form-row` | `label`, `value`, `annotation` | none | `labelled`, `annotated` |
| `log` | `title` | `header`, `body`, `footer`, `dropdown` | `choosing` |
| `layout-tools` | `title`, `body` | none | none |

The form's editor frame/error layout is authored in its overlay template;
the text-editor primitive owns only the text and cursor. The log dropdown
is still controller-rendered; its slot/boolean reserve names for migration.
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
| Borders | `border: 0/none/1/1ch/1lh`; single-cell solid frame |
| Text | `font-weight: normal/bold`, `font-style: normal/italic`, `text-decoration: none/underline`, `text-align: left/center/right`, `white-space: normal/nowrap/pre`, `text-overflow: clip/ellipsis`, terminal `hanging-indent: Nch` |

Lengths use `ch`, `lh`, percentages, `0`, and `auto` where meaningful.
Terminal allocation treats a length unit as a cell in its allocation axis.
There are no pixel units, negative lengths, wrapping flex rows, `calc`,
at-rules, `!important`, or browser positioning. Grid tracks support fixed
lengths, percentages, `auto`, intrinsic unwrapped `max-content`, `fr`, and
`minmax(min, max)`. Placement uses positive lines and `span N`, optionally
separated by `/`. Grid repetition is not supported.

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
arrive. Rich message prefix fields are not yet template-authored.

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
