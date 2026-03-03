//! Declarative UI specification.
//!
//! `ViewSpec` is a platform-agnostic tree describing the entire screen.
//! A pure `view()` function builds it from application state; a renderer
//! consumes it to produce terminal or web output.  See `docs/ui-architecture.md`.

// ---------------------------------------------------------------------------
// Top-level spec
// ---------------------------------------------------------------------------

/// Complete description of one frame of the UI.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub struct ViewSpec {
    /// The base layout tree (panes and splits).
    pub base: LayoutNode,
    /// Modal overlays rendered on top, in stack order (last = topmost).
    pub modals: Vec<ModalSpec>,
    /// Status bar at the bottom (keybinding hints).
    pub status_bar: Option<StatusBarSpec>,
}

// ---------------------------------------------------------------------------
// Layout tree
// ---------------------------------------------------------------------------

/// Spatial arrangement node.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub enum LayoutNode {
    /// Horizontal split: left | right at `ratio` (0.0..1.0 = fraction for left).
    HSplit {
        left: Box<LayoutNode>,
        right: Box<LayoutNode>,
        ratio: f32,
    },
    /// Vertical split: top / bottom at `ratio` (fraction for top).
    VSplit {
        top: Box<LayoutNode>,
        bottom: Box<LayoutNode>,
        ratio: f32,
    },
    /// A leaf pane with content and keybindings.
    Pane(PaneSpec),
    /// Fixed-height spacer (e.g. player status bar).
    Spacer {
        height: u16,
        content: ContentKind,
    },
}

// ---------------------------------------------------------------------------
// Pane
// ---------------------------------------------------------------------------

/// Unique pane identifier for focus routing.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Chat,
    ChatInput,
    Users,
    Playlist,
    RecentSeries,
    PlayerStatus,
}

/// A bordered, optionally focusable content region.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub struct PaneSpec {
    pub id: PaneId,
    pub title: Vec<StyledSpan>,
    pub focused: bool,
    pub content: ContentKind,
    pub bindings: Vec<Keybinding>,
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------

/// An overlay rendered on top of the base layout.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub struct ModalSpec {
    pub title: String,
    /// Width as a fraction of the terminal (0.0..1.0).
    pub width_pct: f32,
    /// Height as a fraction of the terminal (0.0..1.0).
    pub height_pct: f32,
    pub content: ContentKind,
    pub bindings: Vec<Keybinding>,
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

/// Semantic content within a pane or modal.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub enum ContentKind {
    /// Scrollable log of styled lines (chat, system messages).
    TextLog {
        lines: Vec<Vec<StyledSpan>>,
        /// Number of lines scrolled back from the bottom.
        scroll_back: usize,
    },
    /// List of items, one is selected.
    SelectableList {
        items: Vec<Vec<StyledSpan>>,
        selected: usize,
        /// Offset so the renderer can scroll the viewport.
        scroll_offset: usize,
        /// Secondary highlight index (e.g. now-playing in playlist).
        highlighted: Option<usize>,
    },
    /// Single-line text input with cursor.
    TextInput {
        text: String,
        cursor_pos: usize,
        placeholder: String,
    },
    /// Progress bar with label.
    ProgressBar {
        fraction: f64,
        label: String,
    },
    /// Vertical stack of sub-contents (composite pane).
    Composite {
        children: Vec<ContentKind>,
    },
    /// Form with labelled fields (settings screen).
    Form {
        fields: Vec<FormField>,
        focused_field: usize,
        /// Optional alert banner at the top.
        alert: Option<String>,
        /// Validation hint at the bottom.
        hint: Option<Vec<StyledSpan>>,
    },
    /// Nothing to show.
    Empty,
}

/// A field in a form.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub kind: FormFieldKind,
    pub error: Option<String>,
}

/// The kind of form field.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub enum FormFieldKind {
    /// Editable text.
    Text { value: String },
    /// Masked text (password).
    Masked { value: String },
    /// Toggle between fixed choices.
    Toggle { value: String, choices: Vec<String> },
    /// List of paths (media roots).
    PathList {
        paths: Vec<PathListEntry>,
        selected: usize,
    },
}

/// An entry in a path list form field.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub struct PathListEntry {
    pub path: String,
    /// Whether this is the download target (topmost root).
    pub is_download_target: bool,
}

// ---------------------------------------------------------------------------
// Styled text
// ---------------------------------------------------------------------------

/// A span of text with semantic styling.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub text: String,
    pub color: SemanticColor,
    pub bold: bool,
}

impl StyledSpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: SemanticColor::Default,
            bold: false,
        }
    }

    pub fn colored(text: impl Into<String>, color: SemanticColor) -> Self {
        Self {
            text: text.into(),
            color,
            bold: false,
        }
    }

    pub fn bold(text: impl Into<String>, color: SemanticColor) -> Self {
        Self {
            text: text.into(),
            color,
            bold: true,
        }
    }
}

/// Semantic colors — renderers map these to platform-appropriate values.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticColor {
    /// Default foreground.
    Default,
    /// User is ready (green).
    Ready,
    /// User is paused (red).
    Paused,
    /// User is not watching (gray).
    NotWatching,
    /// File is downloading (blue).
    Downloading,
    /// File is missing (red, distinct from Paused in context).
    Missing,
    /// Muted/secondary text (gray, e.g. played items).
    Muted,
    /// Accent color (blue, e.g. download target).
    Accent,
    /// Focused border/highlight (yellow or bright).
    Focused,
    /// Error/warning text (red).
    Error,
    /// System message (dim cyan).
    System,
    /// Chat username.
    Username,
}

// ---------------------------------------------------------------------------
// Keybindings
// ---------------------------------------------------------------------------

/// A keybinding declaration: key combo → action + display label.
#[derive(Debug, Clone)]
pub struct Keybinding {
    pub key: KeyCombo,
    pub label: &'static str,
    pub action: Action,
    /// If true, show in the keybinding bar.
    pub show_in_bar: bool,
}

/// A key or key combination.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCombo {
    /// A plain key press.
    Plain(Key),
    /// Ctrl + key.
    Ctrl(Key),
    /// Shift + key.
    Shift(Key),
}

/// Individual key values.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// A flat action enum covering all user-triggerable operations.
///
/// Produced by keybinding resolution; consumed by the runner to mutate
/// `UiState` and/or `AppState`.  Lives in `dessplay-core` so that the
/// `ViewSpec` types (which reference `Action`) are platform-independent.
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // --- Global ---
    Quit,
    CycleFocus,
    OpenSettings,

    // --- Chat ---
    SendChat,
    ClearInput,
    InsertChar(char),
    DeleteBack,
    DeleteForward,
    DeleteWordBack,
    CursorLeft,
    CursorRight,
    CursorWordLeft,
    CursorWordRight,
    CursorHome,
    CursorEnd,
    ScrollChatUp,
    ScrollChatDown,
    ScrollChatPageUp,
    ScrollChatPageDown,

    // --- Playlist ---
    PlaylistSelectUp,
    PlaylistSelectDown,
    PlaylistPageUp,
    PlaylistPageDown,
    PlaylistMoveUp,
    PlaylistMoveDown,
    PlaylistRemove,
    OpenFileBrowser,
    ManualMapFile,
    AssignMetadata,

    // --- Series Pane ---
    RecentSelectUp,
    RecentSelectDown,
    RecentPageUp,
    RecentPageDown,
    RecentSeriesSelect,
    SeriesToggleMode,
    SeriesToggleSort,

    // --- Episode Browser ---
    EpisodeBrowserUp,
    EpisodeBrowserDown,
    EpisodeBrowserSelect,
    EpisodeBrowserBack,

    // --- Settings ---
    SettingsNextField,
    SettingsPrevField,
    SettingsSave,
    SettingsInsertChar(char),
    SettingsDeleteBack,
    SettingsDeleteWordBack,
    SettingsTogglePlayer,
    SettingsToggleReadyOnStartup,
    SettingsAddMediaRoot,
    SettingsRemoveMediaRoot,
    SettingsMoveRootUp,
    SettingsMoveRootDown,
    SettingsMediaRootUp,
    SettingsMediaRootDown,
    SettingsMediaRootPageUp,
    SettingsMediaRootPageDown,
    SettingsCancel,

    // --- File Browser ---
    FileBrowserUp,
    FileBrowserDown,
    FileBrowserPageUp,
    FileBrowserPageDown,
    FileBrowserSelect,
    FileBrowserBack,
    FileBrowserSelectDir,

    // --- Connecting ---
    CancelConnect,

    // --- TOFU Warning ---
    TofuAccept,
    TofuReject,

    // --- Metadata Assignment ---
    MetadataSelectUp,
    MetadataSelectDown,
    MetadataPageUp,
    MetadataPageDown,
    MetadataConfirmSeries,
    MetadataInsertChar(char),
    MetadataDeleteBack,
    MetadataConfirmEpisode,
    MetadataCancel,

    // --- Now Playing ---
    SetNowPlaying,
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

/// The keybinding bar at the bottom of the screen.
#[derive(Debug, Clone)]
pub struct StatusBarSpec {
    /// (key label, action label) pairs to display.
    pub bindings: Vec<(String, &'static str)>,
}

// ---------------------------------------------------------------------------
// Manual Arbitrary impls for types containing &'static str
// ---------------------------------------------------------------------------

#[cfg(feature = "fuzzing")]
impl<'a> arbitrary::Arbitrary<'a> for Keybinding {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            key: u.arbitrary()?,
            label: "",
            action: u.arbitrary()?,
            show_in_bar: u.arbitrary()?,
        })
    }
}

#[cfg(feature = "fuzzing")]
impl<'a> arbitrary::Arbitrary<'a> for StatusBarSpec {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let bindings: Vec<String> = u.arbitrary()?;
        Ok(Self {
            bindings: bindings.into_iter().map(|s| (s, "")).collect(),
        })
    }
}
