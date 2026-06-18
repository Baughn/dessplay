//! Typed client settings, persisted in the settings table.
//!
//! Defaults fill any missing key, so a fresh database loads cleanly and
//! new settings can be added without a migration. Command-line flags and
//! environment variables override these at runtime (wired up in Phase 5)
//! but are never written back.

use std::time::Duration;

use crate::storage::{Result, Storage, StorageError};

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

/// Render a retention window for display: whole days, else whole hours,
/// else minutes, else seconds (whichever divides cleanly first).
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
    plural(secs, "second")
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
        }
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
