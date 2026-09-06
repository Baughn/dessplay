//! Typed client settings, persisted in the settings table.
//!
//! Defaults fill any missing key, so a fresh database loads cleanly and
//! new settings can be added without a migration. Command-line flags and
//! environment variables override these at runtime (wired up in Phase 5)
//! but are never written back.

use std::time::Duration;

use crate::storage::{Result, Storage, StorageError};
use crate::ui::props::{BrowserSort, ListSort, SeriesSort};

/// Which video player to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayerKind {
    /// mpv via JSON IPC (the primary target).
    #[default]
    Mpv,
    /// VLC via Lua TCP script (open scope decision for v2).
    Vlc,
}

impl PlayerKind {
    fn as_str(self) -> &'static str {
        match self {
            PlayerKind::Mpv => "mpv",
            PlayerKind::Vlc => "vlc",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "mpv" => Ok(PlayerKind::Mpv),
            "vlc" => Ok(PlayerKind::Vlc),
            other => Err(StorageError::Corrupt(format!("unknown player {other:?}"))),
        }
    }

    /// Advance the settings-screen placeholder choice.
    pub fn next(self) -> Self {
        match self {
            PlayerKind::Mpv => PlayerKind::Vlc,
            PlayerKind::Vlc => PlayerKind::Mpv,
        }
    }

    /// Human-readable settings-screen label.
    pub fn label(self) -> &'static str {
        match self {
            PlayerKind::Mpv => "mpv",
            PlayerKind::Vlc => "VLC",
        }
    }
}

/// How the local player's subtitle lines are surfaced in the chat pane.
/// Strictly local — never synced (different releases / sub tracks per
/// user are expected).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitleMode {
    /// Don't show subtitles at all.
    #[default]
    Off,
    /// Fold subtitle lines into the chat log, ordered by arrival.
    Intermixed,
    /// Show subtitles in a dedicated pane split off the chat area.
    SeparatePane,
}

impl SubtitleMode {
    fn as_str(self) -> &'static str {
        match self {
            SubtitleMode::Off => "off",
            SubtitleMode::Intermixed => "intermixed",
            SubtitleMode::SeparatePane => "separate",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "off" => Ok(SubtitleMode::Off),
            "intermixed" => Ok(SubtitleMode::Intermixed),
            "separate" => Ok(SubtitleMode::SeparatePane),
            other => Err(StorageError::Corrupt(format!(
                "unknown subtitle_mode {other:?}"
            ))),
        }
    }

    /// Cycle Off -> Intermixed -> SeparatePane -> Off (the F2 order).
    pub fn next(self) -> Self {
        match self {
            SubtitleMode::Off => SubtitleMode::Intermixed,
            SubtitleMode::Intermixed => SubtitleMode::SeparatePane,
            SubtitleMode::SeparatePane => SubtitleMode::Off,
        }
    }

    /// Short label for the keybinding bar / settings row.
    pub fn label(self) -> &'static str {
        match self {
            SubtitleMode::Off => "Off",
            SubtitleMode::Intermixed => "Intermixed",
            SubtitleMode::SeparatePane => "Separate",
        }
    }
}

/// Local presentation of serious roguelike injuries. Never changes simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RoguelikeEffects {
    #[default]
    /// Red injury flashes and restrained decorative corruption.
    Full,
    /// Static injury emphasis without flashing or corruption.
    Reduced,
    /// Disable cosmetic injury effects; mechanical consequences remain.
    Off,
}
impl RoguelikeEffects {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Reduced => "reduced",
            Self::Off => "off",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "full" => Ok(Self::Full),
            "reduced" => Ok(Self::Reduced),
            "off" => Ok(Self::Off),
            other => Err(StorageError::Corrupt(format!(
                "unknown roguelike_effects {other:?}"
            ))),
        }
    }
    /// Cycle through Full, Reduced, and Off.
    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Reduced,
            Self::Reduced => Self::Off,
            Self::Off => Self::Full,
        }
    }
    /// Human-readable label for the settings screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Reduced => "Reduced",
            Self::Off => "Off",
        }
    }
}

/// How the synced marquee line (AI commentary today; the register is
/// deliberately generic) is displayed locally. The register itself is
/// always synced — this only chooses this client's presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarqueeMode {
    /// Scroll one pass across the bottom line's middle slot (the
    /// original behavior).
    #[default]
    Marquee,
    /// Fold each update into the chat log as a dim local line instead
    /// of scrolling.
    Chat,
    /// Don't show marquee updates at all.
    Off,
}

impl MarqueeMode {
    fn as_str(self) -> &'static str {
        match self {
            MarqueeMode::Marquee => "marquee",
            MarqueeMode::Chat => "chat",
            MarqueeMode::Off => "off",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "marquee" => Ok(MarqueeMode::Marquee),
            "chat" => Ok(MarqueeMode::Chat),
            "off" => Ok(MarqueeMode::Off),
            other => Err(StorageError::Corrupt(format!(
                "unknown marquee_mode {other:?}"
            ))),
        }
    }

    /// Cycle Marquee -> Chat -> Off -> Marquee on the settings screen.
    pub fn next(self) -> Self {
        match self {
            MarqueeMode::Marquee => MarqueeMode::Chat,
            MarqueeMode::Chat => MarqueeMode::Off,
            MarqueeMode::Off => MarqueeMode::Marquee,
        }
    }

    /// Human-readable settings-screen label.
    pub fn label(self) -> &'static str {
        match self {
            MarqueeMode::Marquee => "Marquee",
            MarqueeMode::Chat => "In chat",
            MarqueeMode::Off => "Off",
        }
    }
}

/// What to do when a limited-color terminal has more recently active
/// subtitle speakers than the fixed palette can distinguish.
///
/// True-color terminals are not constrained by this preference: they can
/// allocate another distinct speaker color instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitleSpeakerOverflow {
    /// Preserve today's behavior by reusing colors from the fixed palette.
    #[default]
    ReuseColors,
    /// Render subtitle lines without speaker colors while the palette is
    /// over capacity.
    DisableColors,
}

impl SubtitleSpeakerOverflow {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReuseColors => "reuse_colors",
            Self::DisableColors => "disable_colors",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "reuse_colors" => Ok(Self::ReuseColors),
            "disable_colors" => Ok(Self::DisableColors),
            other => Err(StorageError::Corrupt(format!(
                "unknown subtitle_speaker_overflow {other:?}"
            ))),
        }
    }

    /// Cycle the limited-color terminal fallback on the settings screen.
    pub fn next(self) -> Self {
        match self {
            Self::ReuseColors => Self::DisableColors,
            Self::DisableColors => Self::ReuseColors,
        }
    }

    /// Human-readable settings-screen label.
    pub fn label(self) -> &'static str {
        match self {
            Self::ReuseColors => "Reuse colors",
            Self::DisableColors => "Disable colors",
        }
    }
}

/// How long watched cache downloads are kept after their last access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheRetention {
    /// `0`: delete watched downloads at the next eviction pass — the
    /// small-laptop setting.
    AfterWatch,
    /// Keep for this long after last access.
    Keep(Duration),
    /// Never delete — the NAS/seeder setting.
    Infinite,
}

impl Default for CacheRetention {
    fn default() -> Self {
        // A week: next session's episode usually survives, a laptop
        // doesn't fill up.
        CacheRetention::Keep(Duration::from_secs(7 * 24 * 60 * 60))
    }
}

impl CacheRetention {
    fn as_string(self) -> String {
        match self {
            CacheRetention::AfterWatch => "0".into(),
            CacheRetention::Keep(duration) => duration.as_secs().to_string(),
            CacheRetention::Infinite => "infinite".into(),
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "0" => Ok(CacheRetention::AfterWatch),
            "infinite" => Ok(CacheRetention::Infinite),
            secs => secs
                .parse::<u64>()
                .map(|secs| CacheRetention::Keep(Duration::from_secs(secs)))
                .map_err(|_| StorageError::Corrupt(format!("bad cache_retention {value:?}"))),
        }
    }

    /// Cycle through the settings-screen presets:
    /// Delete-when-watched -> 1 day -> 7 days -> 30 days -> Keep forever
    /// -> wrap. A non-preset `Keep` (set out-of-band) advances to the
    /// first preset strictly larger than it, so it rejoins the ladder.
    pub fn next(self) -> Self {
        const DAY: u64 = 24 * 60 * 60;
        match self {
            CacheRetention::AfterWatch => CacheRetention::Keep(Duration::from_secs(DAY)),
            CacheRetention::Keep(d) => {
                let secs = d.as_secs();
                if secs < DAY {
                    CacheRetention::Keep(Duration::from_secs(DAY))
                } else if secs < 7 * DAY {
                    CacheRetention::Keep(Duration::from_secs(7 * DAY))
                } else if secs < 30 * DAY {
                    CacheRetention::Keep(Duration::from_secs(30 * DAY))
                } else {
                    CacheRetention::Infinite
                }
            }
            CacheRetention::Infinite => CacheRetention::AfterWatch,
        }
    }

    /// Human-readable label for the settings row.
    pub fn label(self) -> String {
        match self {
            CacheRetention::AfterWatch => "Delete when watched".into(),
            CacheRetention::Infinite => "Keep forever".into(),
            CacheRetention::Keep(d) => humanize(d),
        }
    }
}

/// Render a duration for display: whole days, else whole hours, else
/// minutes, else seconds (whichever divides cleanly first). Sub-hour
/// values that don't divide into minutes read as a minute+second
/// compound ("4 minutes 30 seconds", the commentary preset) rather
/// than a raw second count.
fn humanize(d: Duration) -> String {
    let secs = d.as_secs();
    let plural = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    if secs == 0 {
        return "0 seconds".into();
    }
    for (size, unit) in [(24 * 60 * 60, "day"), (60 * 60, "hour"), (60, "minute")] {
        if secs.is_multiple_of(size) {
            return plural(secs / size, unit);
        }
    }
    if secs > 60 && secs < 60 * 60 {
        return format!(
            "{} {}",
            plural(secs / 60, "minute"),
            plural(secs % 60, "second")
        );
    }
    plural(secs, "second")
}

/// How often the AI commentary engine speaks. `Off` disables it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommentaryInterval {
    /// Never — the default; the engine does not run.
    #[default]
    Off,
    /// Roughly every this often, gated on playback being active.
    Every(Duration),
}

impl CommentaryInterval {
    fn as_string(self) -> String {
        match self {
            CommentaryInterval::Off => "off".into(),
            CommentaryInterval::Every(d) => d.as_secs().to_string(),
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "off" => Ok(CommentaryInterval::Off),
            secs => secs
                .parse::<u64>()
                .map(|secs| CommentaryInterval::Every(Duration::from_secs(secs)))
                .map_err(|_| StorageError::Corrupt(format!("bad commentary_interval {value:?}"))),
        }
    }

    /// Cycle the settings-screen presets: Off -> 2 min -> 4 min ->
    /// 10 min -> wrap. A non-preset value (set out-of-band) advances to
    /// the first preset strictly larger than it, rejoining the ladder.
    /// The middle preset is 4:00 rather than 5:00 so that, jitter and
    /// request latency included, each request reliably lands inside the
    /// Anthropic prompt cache's 5-minute ephemeral TTL (see the
    /// commentary engine).
    pub fn next(self) -> Self {
        const MIN: u64 = 60;
        match self {
            CommentaryInterval::Off => CommentaryInterval::Every(Duration::from_secs(2 * MIN)),
            CommentaryInterval::Every(d) => {
                let secs = d.as_secs();
                if secs < 2 * MIN {
                    CommentaryInterval::Every(Duration::from_secs(2 * MIN))
                } else if secs < 4 * MIN {
                    CommentaryInterval::Every(Duration::from_secs(4 * MIN))
                } else if secs < 10 * MIN {
                    CommentaryInterval::Every(Duration::from_secs(10 * MIN))
                } else {
                    CommentaryInterval::Off
                }
            }
        }
    }

    /// Human-readable label for the settings row.
    pub fn label(self) -> String {
        match self {
            CommentaryInterval::Off => "Off".into(),
            CommentaryInterval::Every(d) => format!("Every {}", humanize(d)),
        }
    }

    /// The tick period, when enabled.
    pub fn duration(self) -> Option<Duration> {
        match self {
            CommentaryInterval::Off => None,
            CommentaryInterval::Every(d) => Some(d),
        }
    }
}

const UPLOAD_LIMIT_ERROR: &str = "enter `unlimited` or a whole byte rate such as `500 KiB/s`";

/// Parse the upload-rate syntax used by the settings screen.
///
/// A bare integer retains the persisted representation's bytes-per-second
/// meaning. Human-readable input uses binary units and must be a whole number;
/// the multiplication is checked so an oversized rate is rejected.
pub(crate) fn parse_upload_limit(value: &str) -> std::result::Result<Option<u64>, String> {
    let value = value.trim();
    if value == "unlimited" {
        return Ok(None);
    }

    let mut parts = value.split_whitespace();
    let amount = parts.next().ok_or_else(|| UPLOAD_LIMIT_ERROR.to_owned())?;
    let unit = parts.next();
    if parts.next().is_some() || !amount.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(UPLOAD_LIMIT_ERROR.to_owned());
    }

    let multiplier = match unit {
        None | Some("B/s") => 1,
        Some("KiB/s") => 1024,
        Some("MiB/s") => 1024 * 1024,
        Some("GiB/s") => 1024 * 1024 * 1024,
        Some(_) => return Err(UPLOAD_LIMIT_ERROR.to_owned()),
    };
    let amount = amount
        .parse::<u64>()
        .map_err(|_| UPLOAD_LIMIT_ERROR.to_owned())?;
    amount
        .checked_mul(multiplier)
        .map(Some)
        .ok_or_else(|| UPLOAD_LIMIT_ERROR.to_owned())
}

/// Format an upload limit for editing without losing byte-level precision.
pub(crate) fn format_upload_limit(limit: Option<u64>) -> String {
    let Some(bytes_per_second) = limit else {
        return "unlimited".into();
    };

    if bytes_per_second == 0 {
        return "0 B/s".into();
    }

    for (size, unit) in [
        (1024 * 1024 * 1024, "GiB/s"),
        (1024 * 1024, "MiB/s"),
        (1024, "KiB/s"),
    ] {
        if bytes_per_second.is_multiple_of(size) {
            return format!("{} {unit}", bytes_per_second / size);
        }
    }
    format!("{bytes_per_second} B/s")
}

/// All persisted client settings. `username` and `password` are `None`
/// until first-run setup completes — that's the "show the settings
/// screen" signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Self-chosen nickname. Defaults to `$USER` in the settings screen,
    /// but is only persisted once confirmed.
    pub username: Option<String>,
    /// Rendezvous server address.
    pub server: String,
    /// Shared room password, plaintext (see the threat model).
    pub password: Option<String>,
    /// Which player to drive.
    pub player: PlayerKind,
    /// Start as Ready instead of Paused when connecting.
    pub ready_on_startup: bool,
    /// Download-cache retention policy.
    pub cache_retention: CacheRetention,
    /// Upload cap in bytes/sec for serving files to peers; `None` =
    /// unlimited.
    pub upload_limit: Option<u64>,
    /// How local player subtitles are surfaced in the chat pane.
    pub subtitle_mode: SubtitleMode,
    /// Prefix text subtitle cues with their ASS speaker/actor name when one
    /// is available. Local display preference; default false preserves the
    /// spoiler-safe behavior from before speaker names were exposed.
    pub subtitle_speaker_names: bool,
    /// Color separate-pane subtitle lines by ASS speaker (design.md #22).
    /// When false, every separate-pane line renders uniformly dim
    /// regardless of speaker. Default true. Has no effect on Intermixed
    /// mode, which is already uniformly dim.
    pub subtitle_speaker_colors: bool,
    /// Fallback when a limited-color terminal's fixed speaker palette is
    /// smaller than the set of recently active speakers. Defaulting to color
    /// reuse preserves the behavior from before this setting existed.
    pub subtitle_speaker_overflow: SubtitleSpeakerOverflow,
    /// How the synced marquee line is displayed locally: scrolled on the
    /// bottom line (default), folded into the chat log, or hidden.
    pub marquee_mode: MarqueeMode,
    /// Local cosmetic injury presentation for the waiting-room expedition.
    pub roguelike_effects: RoguelikeEffects,
    /// Sort order for the All Series browser mode (toggled with `s`).
    /// Local-only display preference; persisted across sessions.
    pub series_sort: SeriesSort,
    /// Sort order for The List mode (toggled with `s`). Local-only
    /// display preference; persisted across sessions.
    pub list_sort: ListSort,
    /// Sort order for the add/map file browser (design.md #8).
    /// Local-only display preference; persisted across sessions.
    pub file_browser_sort: BrowserSort,
    /// Whether to automatically fetch files from peers (prefetch window
    /// and the missing now-playing file). When false the client never
    /// downloads — it relies on its own library (design.md,
    /// Pre-fetching). Default true.
    pub auto_download: bool,
    /// Put archived downloads in a sanitized series-name subdirectory under
    /// the download root. When false, archive directly into the root.
    /// Default true, preserving the original archive layout.
    pub archive_subdirectory: bool,
    /// Archive a cached download automatically once it is personally
    /// watched (the 85% rule), instead of waiting for an explicit `A`.
    /// Default false: archiving stays the deliberate "keep this in the
    /// library" decision (design.md, Archive).
    pub auto_archive: bool,
    /// Enable the Playlist pane's explicit Nyaa browse import
    /// (design.md, BitTorrent Downloads). When off at startup the
    /// torrent engine is never started. **Disabling applies
    /// immediately** (seeding torrents are removed and pending imports
    /// cancelled — the escape hatch for a saturated uplink); enabling
    /// requires a restart when the engine was off at startup. Default
    /// off.
    pub torrent_enabled: bool,
    /// Bridge our own chat to IRC so the conversation survives the app
    /// being closed (design.md, IRC bridge). Default on.
    pub irc_enabled: bool,
    /// IRC server to bridge to. Default `irc.rizon.net`.
    pub irc_server: String,
    /// Connect to IRC over TLS (port 6697); plaintext (6667) when false.
    /// Default true.
    pub irc_tls: bool,
    /// IRC channel to join. Default `#dess`.
    pub irc_channel: String,
    /// Anthropic API token for the AI commentary engine (design.md, AI
    /// Commentary). Plaintext, like `password` (see the threat model).
    /// Baughn only — nobody else is expected to set this.
    pub anthropic_token: Option<String>,
    /// How often the commentary engine speaks. Off disables it.
    pub commentary_interval: CommentaryInterval,
    /// Pane splitter positions (dragged with the mouse). Local-only.
    pub pane_layout: PaneLayout,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            username: None,
            server: "dessplay.brage.info".into(),
            password: None,
            player: PlayerKind::default(),
            ready_on_startup: false,
            cache_retention: CacheRetention::default(),
            upload_limit: None,
            subtitle_mode: SubtitleMode::default(),
            subtitle_speaker_names: false,
            subtitle_speaker_colors: true,
            subtitle_speaker_overflow: SubtitleSpeakerOverflow::default(),
            marquee_mode: MarqueeMode::default(),
            roguelike_effects: RoguelikeEffects::default(),
            series_sort: SeriesSort::default(),
            list_sort: ListSort::default(),
            file_browser_sort: BrowserSort::default(),
            auto_download: true,
            archive_subdirectory: true,
            auto_archive: false,
            torrent_enabled: false,
            irc_enabled: true,
            irc_server: "irc.rizon.net".into(),
            irc_tls: true,
            irc_channel: "#dess".into(),
            anthropic_token: None,
            commentary_interval: CommentaryInterval::default(),
            pane_layout: PaneLayout::default(),
        }
    }
}

/// Where the pane splitters sit, as whole percentages of the region
/// they divide (design.md, Mouse support: resizable panes). Integer
/// percent rather than a float: a terminal cell is the finest
/// resolution anyway, it keeps `Settings: Eq`, and it survives a text
/// round-trip exactly. Every field is clamped into [`PaneLayout::MIN`,
/// `PaneLayout::MAX`] by `clamped`, and the right column's series +
/// users share is capped so the playlist keeps its minimum too, so a
/// stored layout can never squeeze a pane out of existence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneLayout {
    /// Width of the chat column, as a percentage of the pane area.
    pub chat_width: u8,
    /// Height of the separate subtitle pane, as a percentage of the
    /// chat column (only used in `SubtitleMode::SeparatePane`).
    pub subtitle_height: u8,
    /// Height of the Series pane, as a percentage of the right column.
    pub series_height: u8,
    /// Height of the Users pane, as a percentage of the right column;
    /// the playlist takes the remainder.
    pub users_height: u8,
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self {
            chat_width: 50,
            subtitle_height: 30,
            series_height: 34,
            users_height: 33,
        }
    }
}

impl PaneLayout {
    /// Smallest share any pane may be dragged to.
    pub const MIN: u8 = 10;
    /// Largest share any single pane may be dragged to.
    pub const MAX: u8 = 90;

    /// The same layout with every share forced into range. The users
    /// pane is clamped *after* series so the invariant `series + users
    /// <= 100 - MIN` always holds, whichever field moved.
    pub fn clamped(self) -> Self {
        let chat_width = self.chat_width.clamp(Self::MIN, Self::MAX);
        let subtitle_height = self.subtitle_height.clamp(Self::MIN, Self::MAX);
        let series_height = self.series_height.clamp(Self::MIN, 100 - 2 * Self::MIN);
        let users_height = self
            .users_height
            .clamp(Self::MIN, 100 - Self::MIN - series_height);
        Self {
            chat_width,
            subtitle_height,
            series_height,
            users_height,
        }
    }

    /// The playlist's share of the right column.
    pub fn playlist_height(self) -> u8 {
        100 - self.series_height - self.users_height
    }

    /// `chat,subtitle,series,users` — the persisted form.
    pub fn as_string(self) -> String {
        format!(
            "{},{},{},{}",
            self.chat_width, self.subtitle_height, self.series_height, self.users_height
        )
    }

    /// Parse the persisted form. Values are clamped rather than
    /// rejected — an out-of-range share is a stale layout, not
    /// corruption — but a malformed string is.
    pub fn parse(value: &str) -> Result<Self> {
        let corrupt = || StorageError::Corrupt(format!("bad pane_layout {value:?}"));
        let mut parts = value.split(',').map(|part| part.trim().parse::<u8>());
        let mut next = || parts.next().ok_or_else(corrupt)?.map_err(|_| corrupt());
        let layout = Self {
            chat_width: next()?,
            subtitle_height: next()?,
            series_height: next()?,
            users_height: next()?,
        };
        if parts.next().is_some() {
            return Err(corrupt());
        }
        Ok(layout.clamped())
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(StorageError::Corrupt(format!("bad {key} {other:?}"))),
    }
}

impl Settings {
    /// First-run setup is needed until a username and password exist.
    pub fn needs_setup(&self) -> bool {
        self.username.is_none() || self.password.is_none()
    }

    pub(crate) fn load(storage: &Storage) -> Result<Self> {
        let defaults = Settings::default();
        Ok(Settings {
            username: storage.setting("username")?,
            server: storage.setting("server")?.unwrap_or(defaults.server),
            password: storage.setting("password")?,
            player: storage
                .setting("player")?
                .map(|value| PlayerKind::parse(&value))
                .transpose()?
                .unwrap_or(defaults.player),
            ready_on_startup: storage
                .setting("ready_on_startup")?
                .map(|value| parse_bool("ready_on_startup", &value))
                .transpose()?
                .unwrap_or(defaults.ready_on_startup),
            cache_retention: storage
                .setting("cache_retention")?
                .map(|value| CacheRetention::parse(&value))
                .transpose()?
                .unwrap_or(defaults.cache_retention),
            upload_limit: storage
                .setting("upload_limit")?
                .map(|value| {
                    value
                        .parse::<u64>()
                        .map_err(|_| StorageError::Corrupt(format!("bad upload_limit {value:?}")))
                })
                .transpose()?,
            subtitle_mode: match storage.setting("subtitle_mode")? {
                Some(value) => SubtitleMode::parse(&value)?,
                // Migrate the legacy boolean: an enabled pane becomes
                // SeparatePane, anything else (absent or "false") Off.
                None => match storage.setting("subtitle_pane")? {
                    Some(value) if parse_bool("subtitle_pane", &value)? => {
                        SubtitleMode::SeparatePane
                    }
                    _ => defaults.subtitle_mode,
                },
            },
            subtitle_speaker_names: storage
                .setting("subtitle_speaker_names")?
                .map(|value| parse_bool("subtitle_speaker_names", &value))
                .transpose()?
                .unwrap_or(defaults.subtitle_speaker_names),
            subtitle_speaker_colors: storage
                .setting("subtitle_speaker_colors")?
                .map(|value| parse_bool("subtitle_speaker_colors", &value))
                .transpose()?
                .unwrap_or(defaults.subtitle_speaker_colors),
            subtitle_speaker_overflow: storage
                .setting("subtitle_speaker_overflow")?
                .map(|value| SubtitleSpeakerOverflow::parse(&value))
                .transpose()?
                .unwrap_or(defaults.subtitle_speaker_overflow),
            roguelike_effects: storage
                .setting("roguelike_effects")?
                .map(|value| RoguelikeEffects::parse(&value))
                .transpose()?
                .unwrap_or(defaults.roguelike_effects),
            marquee_mode: storage
                .setting("marquee_mode")?
                .map(|value| MarqueeMode::parse(&value))
                .transpose()?
                .unwrap_or(defaults.marquee_mode),
            series_sort: match storage.setting("series_sort")? {
                Some(value) => SeriesSort::parse(&value).ok_or_else(|| {
                    StorageError::Corrupt(format!("unknown series_sort {value:?}"))
                })?,
                None => defaults.series_sort,
            },
            list_sort: match storage.setting("list_sort")? {
                Some(value) => ListSort::parse(&value)
                    .ok_or_else(|| StorageError::Corrupt(format!("unknown list_sort {value:?}")))?,
                None => defaults.list_sort,
            },
            file_browser_sort: match storage.setting("file_browser_sort")? {
                Some(value) => BrowserSort::parse(&value).ok_or_else(|| {
                    StorageError::Corrupt(format!("unknown file_browser_sort {value:?}"))
                })?,
                None => defaults.file_browser_sort,
            },
            auto_download: storage
                .setting("auto_download")?
                .map(|value| parse_bool("auto_download", &value))
                .transpose()?
                .unwrap_or(defaults.auto_download),
            archive_subdirectory: storage
                .setting("archive_subdirectory")?
                .map(|value| parse_bool("archive_subdirectory", &value))
                .transpose()?
                .unwrap_or(defaults.archive_subdirectory),
            auto_archive: storage
                .setting("auto_archive")?
                .map(|value| parse_bool("auto_archive", &value))
                .transpose()?
                .unwrap_or(defaults.auto_archive),
            torrent_enabled: storage
                .setting("torrent_enabled")?
                .map(|value| parse_bool("torrent_enabled", &value))
                .transpose()?
                .unwrap_or(defaults.torrent_enabled),
            irc_enabled: storage
                .setting("irc_enabled")?
                .map(|value| parse_bool("irc_enabled", &value))
                .transpose()?
                .unwrap_or(defaults.irc_enabled),
            irc_server: storage
                .setting("irc_server")?
                .unwrap_or(defaults.irc_server),
            irc_tls: storage
                .setting("irc_tls")?
                .map(|value| parse_bool("irc_tls", &value))
                .transpose()?
                .unwrap_or(defaults.irc_tls),
            irc_channel: storage
                .setting("irc_channel")?
                .unwrap_or(defaults.irc_channel),
            anthropic_token: storage.setting("anthropic_token")?,
            commentary_interval: storage
                .setting("commentary_interval")?
                .map(|value| CommentaryInterval::parse(&value))
                .transpose()?
                .unwrap_or(defaults.commentary_interval),
            pane_layout: storage
                .setting("pane_layout")?
                .map(|value| PaneLayout::parse(&value))
                .transpose()?
                .unwrap_or(defaults.pane_layout),
        })
    }

    pub(crate) fn save(&self, storage: &Storage) -> Result<()> {
        storage.set_setting("username", self.username.as_deref())?;
        storage.set_setting("server", Some(&self.server))?;
        storage.set_setting("password", self.password.as_deref())?;
        storage.set_setting("player", Some(self.player.as_str()))?;
        storage.set_setting(
            "ready_on_startup",
            Some(if self.ready_on_startup {
                "true"
            } else {
                "false"
            }),
        )?;
        storage.set_setting("cache_retention", Some(&self.cache_retention.as_string()))?;
        storage.set_setting(
            "upload_limit",
            self.upload_limit.map(|limit| limit.to_string()).as_deref(),
        )?;
        storage.set_setting("subtitle_mode", Some(self.subtitle_mode.as_str()))?;
        storage.set_setting(
            "subtitle_speaker_names",
            Some(if self.subtitle_speaker_names {
                "true"
            } else {
                "false"
            }),
        )?;
        storage.set_setting(
            "subtitle_speaker_colors",
            Some(if self.subtitle_speaker_colors {
                "true"
            } else {
                "false"
            }),
        )?;
        storage.set_setting(
            "subtitle_speaker_overflow",
            Some(self.subtitle_speaker_overflow.as_str()),
        )?;
        storage.set_setting("roguelike_effects", Some(self.roguelike_effects.as_str()))?;
        storage.set_setting("marquee_mode", Some(self.marquee_mode.as_str()))?;
        storage.set_setting("series_sort", Some(self.series_sort.as_str()))?;
        storage.set_setting("list_sort", Some(self.list_sort.as_str()))?;
        storage.set_setting("file_browser_sort", Some(self.file_browser_sort.as_str()))?;
        storage.set_setting(
            "auto_download",
            Some(if self.auto_download { "true" } else { "false" }),
        )?;
        storage.set_setting(
            "archive_subdirectory",
            Some(if self.archive_subdirectory {
                "true"
            } else {
                "false"
            }),
        )?;
        storage.set_setting(
            "auto_archive",
            Some(if self.auto_archive { "true" } else { "false" }),
        )?;
        storage.set_setting(
            "torrent_enabled",
            Some(if self.torrent_enabled {
                "true"
            } else {
                "false"
            }),
        )?;
        storage.set_setting(
            "irc_enabled",
            Some(if self.irc_enabled { "true" } else { "false" }),
        )?;
        storage.set_setting("irc_server", Some(&self.irc_server))?;
        storage.set_setting("irc_tls", Some(if self.irc_tls { "true" } else { "false" }))?;
        storage.set_setting("irc_channel", Some(&self.irc_channel))?;
        storage.set_setting("anthropic_token", self.anthropic_token.as_deref())?;
        storage.set_setting(
            "commentary_interval",
            Some(&self.commentary_interval.as_string()),
        )?;
        storage.set_setting("pane_layout", Some(&self.pane_layout.as_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn fresh_database_yields_defaults_and_needs_setup() {
        let storage = Storage::open_in_memory().unwrap();
        let settings = storage.load_settings().unwrap();
        assert_eq!(settings, Settings::default());
        assert!(settings.needs_setup());
        assert_eq!(settings.server, "dessplay.brage.info");
        assert!(settings.archive_subdirectory);
        assert!(!settings.auto_archive);
    }

    #[test]
    fn settings_round_trip() {
        let storage = Storage::open_in_memory().unwrap();
        let settings = Settings {
            username: Some("Baughn".into()),
            server: "localhost".into(),
            password: Some("hunter2".into()),
            player: PlayerKind::Vlc,
            ready_on_startup: true,
            cache_retention: CacheRetention::Infinite,
            upload_limit: Some(1_000_000),
            subtitle_mode: SubtitleMode::Intermixed,
            subtitle_speaker_names: true,
            subtitle_speaker_colors: false,
            subtitle_speaker_overflow: SubtitleSpeakerOverflow::DisableColors,
            marquee_mode: MarqueeMode::Chat,
            roguelike_effects: RoguelikeEffects::Reduced,
            series_sort: SeriesSort::Year,
            list_sort: ListSort::Alphabetical,
            file_browser_sort: BrowserSort::Newest,
            auto_download: false,
            archive_subdirectory: false,
            auto_archive: true,
            torrent_enabled: true,
            irc_enabled: false,
            irc_server: "irc.example.org".into(),
            irc_tls: false,
            irc_channel: "#watchparty".into(),
            anthropic_token: Some("sk-ant-test".into()),
            commentary_interval: CommentaryInterval::Every(Duration::from_secs(300)),
            pane_layout: PaneLayout {
                chat_width: 60,
                subtitle_height: 25,
                series_height: 20,
                users_height: 50,
            },
        };
        storage.save_settings(&settings).unwrap();
        let loaded = storage.load_settings().unwrap();
        assert_eq!(loaded, settings);
        assert!(!loaded.needs_setup());
    }

    #[test]
    fn clearing_optionals_deletes_rows() {
        let storage = Storage::open_in_memory().unwrap();
        let mut settings = Settings {
            username: Some("Baughn".into()),
            password: Some("hunter2".into()),
            upload_limit: Some(5),
            ..Settings::default()
        };
        storage.save_settings(&settings).unwrap();

        settings.username = None;
        settings.upload_limit = None;
        storage.save_settings(&settings).unwrap();
        let loaded = storage.load_settings().unwrap();
        assert_eq!(loaded.username, None);
        assert_eq!(loaded.upload_limit, None);
        assert!(loaded.needs_setup());
    }

    #[test]
    fn commentary_interval_ladder_cycles_and_round_trips() {
        let every = |secs: u64| CommentaryInterval::Every(Duration::from_secs(secs));
        // Off -> 2 min -> 4 min -> 10 min -> Off. The middle preset is
        // 4:00, not 5:00, so a jittered request reliably lands inside
        // the prompt cache's 5-minute ephemeral TTL.
        let mut interval = CommentaryInterval::Off;
        let mut seen = Vec::new();
        for _ in 0..4 {
            interval = interval.next();
            seen.push(interval);
        }
        assert_eq!(
            seen,
            vec![every(120), every(240), every(600), CommentaryInterval::Off]
        );
        // An out-of-band value rejoins the ladder at the next preset up
        // (a stored pre-4:00 "4 min 30 s" or "5 min" advances to 10 min).
        assert_eq!(every(3 * 60).next(), every(240));
        assert_eq!(every(270).next(), every(600));
        assert_eq!(every(300).next(), every(600));
        // Persisted form survives.
        for value in [CommentaryInterval::Off, every(120), every(7 * 60)] {
            assert_eq!(
                CommentaryInterval::parse(&value.as_string()).unwrap(),
                value
            );
        }
        assert_eq!(CommentaryInterval::Off.duration(), None);
        assert_eq!(
            every(120).duration(),
            Some(Duration::from_secs(120)),
            "duration feeds the engine's ticker"
        );
        assert_eq!(every(240).label(), "Every 4 minutes");
    }

    #[test]
    fn upload_limit_accepts_bytes_human_rates_and_unlimited() {
        for (text, expected) in [
            ("unlimited", None),
            ("  unlimited\t", None),
            ("0", Some(0)),
            ("123", Some(123)),
            ("123 B/s", Some(123)),
            ("500 KiB/s", Some(500 * 1024)),
            ("2 MiB/s", Some(2 * 1024 * 1024)),
            ("3 GiB/s", Some(3 * 1024 * 1024 * 1024)),
        ] {
            assert_eq!(parse_upload_limit(text), Ok(expected), "input {text:?}");
        }
        assert_eq!(
            parse_upload_limit(&u64::MAX.to_string()),
            Ok(Some(u64::MAX))
        );
    }

    #[test]
    fn upload_limit_rejects_invalid_fractional_and_overflow_values() {
        for text in [
            "",
            "none",
            "-1",
            "+1",
            "1.5 MiB/s",
            "1 MB/s",
            "1 MiB",
            "1 MiB/s trailing",
            "18446744073709551616",
            "18014398509481984 KiB/s",
        ] {
            assert!(
                parse_upload_limit(text).is_err(),
                "unexpectedly accepted {text:?}"
            );
        }
    }

    #[test]
    fn upload_limit_formatter_uses_exact_binary_units() {
        assert_eq!(format_upload_limit(None), "unlimited");
        assert_eq!(format_upload_limit(Some(0)), "0 B/s");
        assert_eq!(format_upload_limit(Some(513)), "513 B/s");
        assert_eq!(format_upload_limit(Some(500 * 1024)), "500 KiB/s");
        assert_eq!(format_upload_limit(Some(2 * 1024 * 1024)), "2 MiB/s");
        assert_eq!(format_upload_limit(Some(3 * 1024 * 1024 * 1024)), "3 GiB/s");
        assert_eq!(
            format_upload_limit(Some(u64::MAX)),
            format!("{} B/s", u64::MAX)
        );
    }

    proptest::proptest! {
        /// Any four bytes clamp to a layout whose every pane keeps at
        /// least its minimum share, and that layout survives the text
        /// round-trip unchanged.
        #[test]
        fn pane_layout_clamp_invariants_and_round_trip(
            chat_width in proptest::num::u8::ANY,
            subtitle_height in proptest::num::u8::ANY,
            series_height in proptest::num::u8::ANY,
            users_height in proptest::num::u8::ANY,
        ) {
            let layout = PaneLayout { chat_width, subtitle_height, series_height, users_height }.clamped();
            let range = PaneLayout::MIN..=PaneLayout::MAX;
            proptest::prop_assert!(range.contains(&layout.chat_width));
            proptest::prop_assert!(range.contains(&layout.subtitle_height));
            proptest::prop_assert!(range.contains(&layout.series_height));
            proptest::prop_assert!(range.contains(&layout.users_height));
            proptest::prop_assert!(layout.playlist_height() >= PaneLayout::MIN);
            proptest::prop_assert_eq!(layout.clamped(), layout, "clamping is idempotent");
            proptest::prop_assert_eq!(PaneLayout::parse(&layout.as_string()).unwrap(), layout);
        }

        #[test]
        fn upload_limit_format_parse_round_trip(limit in proptest::option::of(proptest::num::u64::ANY)) {
            proptest::prop_assert_eq!(parse_upload_limit(&format_upload_limit(limit)), Ok(limit));
        }
    }

    #[test]
    fn retention_representations() {
        for retention in [
            CacheRetention::AfterWatch,
            CacheRetention::Keep(Duration::from_secs(86_400)),
            CacheRetention::Infinite,
        ] {
            assert_eq!(
                CacheRetention::parse(&retention.as_string()).unwrap(),
                retention
            );
        }
        assert!(CacheRetention::parse("yes please").is_err());
    }

    #[test]
    fn retention_cycle_ladder() {
        let day = |n: u64| CacheRetention::Keep(Duration::from_secs(n * 24 * 60 * 60));
        // The preset ladder wraps.
        assert_eq!(CacheRetention::AfterWatch.next(), day(1));
        assert_eq!(day(1).next(), day(7));
        assert_eq!(day(7).next(), day(30));
        assert_eq!(day(30).next(), CacheRetention::Infinite);
        assert_eq!(CacheRetention::Infinite.next(), CacheRetention::AfterWatch);
        // A non-preset value rejoins at the first strictly-larger preset.
        assert_eq!(
            CacheRetention::Keep(Duration::from_secs(3 * 24 * 60 * 60)).next(),
            day(7)
        );
        assert_eq!(
            CacheRetention::Keep(Duration::from_secs(100 * 24 * 60 * 60)).next(),
            CacheRetention::Infinite
        );
    }

    #[test]
    fn retention_labels() {
        assert_eq!(CacheRetention::AfterWatch.label(), "Delete when watched");
        assert_eq!(CacheRetention::Infinite.label(), "Keep forever");
        assert_eq!(
            CacheRetention::Keep(Duration::from_secs(24 * 60 * 60)).label(),
            "1 day"
        );
        assert_eq!(
            CacheRetention::Keep(Duration::from_secs(7 * 24 * 60 * 60)).label(),
            "7 days"
        );
        assert_eq!(
            CacheRetention::Keep(Duration::from_secs(90 * 60)).label(),
            "90 minutes"
        );
        assert_eq!(
            CacheRetention::Keep(Duration::from_secs(2 * 60 * 60)).label(),
            "2 hours"
        );
    }

    #[test]
    fn corrupt_values_error_instead_of_defaulting() {
        let storage = Storage::open_in_memory().unwrap();
        storage.set_setting("player", Some("winamp")).unwrap();
        assert!(storage.load_settings().is_err());
    }

    #[test]
    fn subtitle_mode_representations() {
        for mode in [
            SubtitleMode::Off,
            SubtitleMode::Intermixed,
            SubtitleMode::SeparatePane,
        ] {
            assert_eq!(SubtitleMode::parse(mode.as_str()).unwrap(), mode);
        }
        assert!(SubtitleMode::parse("sideways").is_err());
    }

    #[test]
    fn subtitle_mode_cycles() {
        assert_eq!(SubtitleMode::Off.next(), SubtitleMode::Intermixed);
        assert_eq!(SubtitleMode::Intermixed.next(), SubtitleMode::SeparatePane);
        assert_eq!(SubtitleMode::SeparatePane.next(), SubtitleMode::Off);
    }

    #[test]
    fn missing_subtitle_speaker_names_preserves_hidden_names() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(!storage.load_settings().unwrap().subtitle_speaker_names);
    }

    #[test]
    fn subtitle_speaker_overflow_representations_and_cycle() {
        for overflow in [
            SubtitleSpeakerOverflow::ReuseColors,
            SubtitleSpeakerOverflow::DisableColors,
        ] {
            assert_eq!(
                SubtitleSpeakerOverflow::parse(overflow.as_str()).unwrap(),
                overflow
            );
        }
        assert!(SubtitleSpeakerOverflow::parse("invent-colors").is_err());
        assert_eq!(
            SubtitleSpeakerOverflow::ReuseColors.next(),
            SubtitleSpeakerOverflow::DisableColors
        );
        assert_eq!(
            SubtitleSpeakerOverflow::DisableColors.next(),
            SubtitleSpeakerOverflow::ReuseColors
        );
    }

    #[test]
    fn roguelike_effects_roundtrip_default_and_cycle() {
        let storage = Storage::open_in_memory().unwrap();
        assert_eq!(
            storage.load_settings().unwrap().roguelike_effects,
            RoguelikeEffects::Full
        );
        for mode in [
            RoguelikeEffects::Full,
            RoguelikeEffects::Reduced,
            RoguelikeEffects::Off,
        ] {
            let settings = Settings {
                roguelike_effects: mode,
                ..Settings::default()
            };
            storage.save_settings(&settings).unwrap();
            assert_eq!(storage.load_settings().unwrap().roguelike_effects, mode);
            assert_eq!(mode.next().next().next(), mode);
        }
        assert!(RoguelikeEffects::parse("wild").is_err());
    }

    #[test]
    fn marquee_mode_representations_and_cycle() {
        for mode in [MarqueeMode::Marquee, MarqueeMode::Chat, MarqueeMode::Off] {
            assert_eq!(MarqueeMode::parse(mode.as_str()).unwrap(), mode);
        }
        assert!(MarqueeMode::parse("sideways").is_err());
        assert_eq!(MarqueeMode::Marquee.next(), MarqueeMode::Chat);
        assert_eq!(MarqueeMode::Chat.next(), MarqueeMode::Off);
        assert_eq!(MarqueeMode::Off.next(), MarqueeMode::Marquee);
        // A missing key keeps the original scrolling behavior.
        let storage = Storage::open_in_memory().unwrap();
        assert_eq!(
            storage.load_settings().unwrap().marquee_mode,
            MarqueeMode::Marquee
        );
    }

    #[test]
    fn missing_subtitle_speaker_overflow_keeps_current_behavior() {
        let storage = Storage::open_in_memory().unwrap();
        assert_eq!(
            storage.load_settings().unwrap().subtitle_speaker_overflow,
            SubtitleSpeakerOverflow::ReuseColors
        );
    }

    #[test]
    fn legacy_subtitle_pane_migrates() {
        // Enabled pane -> SeparatePane.
        let storage = Storage::open_in_memory().unwrap();
        storage.set_setting("subtitle_pane", Some("true")).unwrap();
        assert_eq!(
            storage.load_settings().unwrap().subtitle_mode,
            SubtitleMode::SeparatePane
        );

        // Disabled pane -> Off.
        let storage = Storage::open_in_memory().unwrap();
        storage.set_setting("subtitle_pane", Some("false")).unwrap();
        assert_eq!(
            storage.load_settings().unwrap().subtitle_mode,
            SubtitleMode::Off
        );

        // Neither key present -> default Off.
        let storage = Storage::open_in_memory().unwrap();
        assert_eq!(
            storage.load_settings().unwrap().subtitle_mode,
            SubtitleMode::Off
        );

        // The new key wins over a stale legacy key.
        let storage = Storage::open_in_memory().unwrap();
        storage.set_setting("subtitle_pane", Some("true")).unwrap();
        storage
            .set_setting("subtitle_mode", Some("intermixed"))
            .unwrap();
        assert_eq!(
            storage.load_settings().unwrap().subtitle_mode,
            SubtitleMode::Intermixed
        );
    }
}
