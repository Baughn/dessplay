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
    /// Show the subtitle pane.
    pub subtitle_pane: bool,
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
            subtitle_pane: false,
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
            subtitle_pane: storage
                .setting("subtitle_pane")?
                .map(|value| parse_bool("subtitle_pane", &value))
                .transpose()?
                .unwrap_or(defaults.subtitle_pane),
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
        storage.set_setting(
            "subtitle_pane",
            Some(if self.subtitle_pane { "true" } else { "false" }),
        )?;
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
            subtitle_pane: true,
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
    fn corrupt_values_error_instead_of_defaulting() {
        let storage = Storage::open_in_memory().unwrap();
        storage.set_setting("player", Some("winamp")).unwrap();
        assert!(storage.load_settings().is_err());
    }
}
