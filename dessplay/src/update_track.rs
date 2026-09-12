//! Launcher-owned configuration. The path is a runtime capability, never stored
//! in SQLite or synced; only install.sh (or an explicit flag) supplies it.
use std::{fs, io::Write, path::PathBuf};

/// The two published bookmarks on the same linear history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateTrack {
    /// Every feature and fix as it lands.
    #[default]
    Master,
    /// Advances for protocol changes and critical fixes.
    Stable,
}

impl UpdateTrack {
    /// Bookmark name and on-disk spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Stable => "stable",
        }
    }

    /// Cycle the settings choice.
    pub fn next(self) -> Self {
        match self {
            Self::Master => Self::Stable,
            Self::Stable => Self::Master,
        }
    }
}

/// Runtime permission to edit one launcher file, plus its working value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherTrack {
    path: PathBuf,
    /// Selected bookmark for the next launcher invocation.
    pub track: UpdateTrack,
}

impl LauncherTrack {
    /// Read the supplied file; only an absent file defaults to master.
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let track = match fs::read_to_string(&path) {
            Ok(value) => match value.trim_end_matches('\n') {
                "master" => UpdateTrack::Master,
                "stable" => UpdateTrack::Stable,
                _ => {
                    return Err(format!(
                        "invalid update track in {} (expected master or stable)",
                        path.display()
                    ));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => UpdateTrack::Master,
            Err(error) => return Err(format!("reading update track {}: {error}", path.display())),
        };
        Ok(Self { path, track })
    }

    /// Unrelated settings saves must not rewrite the launcher's choice. Use the
    /// last accepted settings as the baseline, including after first-run setup.
    pub fn save_change(previous: Option<&Self>, next: Option<&Self>) -> Result<(), String> {
        let Some(next) = next.filter(|next| Some(*next) != previous) else {
            return Ok(());
        };
        next.save()
            .map_err(|error| format!("saving update track {}: {error}", next.path.display()))?;
        tracing::info!(
            track = next.track.label(),
            "update track saved; applies on next launcher run"
        );
        Ok(())
    }

    fn save(&self) -> std::io::Result<()> {
        // A sibling temporary file + rename never exposes a partially written
        // track to a concurrent launcher. Bare relative filenames use cwd.
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        writeln!(file, "{}", self.track.label())?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn absent_defaults_to_master_and_changes_round_trip_both_ways() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("with ' quotes/update-track");
        let mut previous = LauncherTrack::load(path.clone()).unwrap();
        assert_eq!(previous.track, UpdateTrack::Master);
        LauncherTrack::save_change(Some(&previous), Some(&previous)).unwrap();
        assert!(
            !path.exists(),
            "unrelated save must not create a track file"
        );
        for _ in 0..4 {
            let mut next = previous.clone();
            next.track = next.track.next();
            LauncherTrack::save_change(Some(&previous), Some(&next)).unwrap();
            assert_eq!(LauncherTrack::load(path.clone()).unwrap(), next);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!("{}\n", next.track.label())
            );
            previous = next;
        }
    }

    #[test]
    fn invalid_or_unreadable_files_and_failed_saves_report_errors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("track");
        for value in [
            "",
            "release",
            "stable master",
            " stable",
            "master\r\n",
            "$(exit 1)",
        ] {
            fs::write(&path, value).unwrap();
            assert!(LauncherTrack::load(path.clone()).is_err());
        }
        fs::remove_file(&path).unwrap();
        let mut next = LauncherTrack::load(path.clone()).unwrap();
        next.track = UpdateTrack::Stable;
        fs::create_dir(&path).unwrap();
        assert!(LauncherTrack::load(path).is_err());
        assert!(LauncherTrack::save_change(None, Some(&next)).is_err());
    }

    #[test]
    fn launcher_capability_is_not_persisted_in_database() {
        let temp = tempfile::tempdir().unwrap();
        let settings = crate::config::Settings {
            launcher_track: Some(LauncherTrack::load(temp.path().join("track")).unwrap()),
            ..Default::default()
        };
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        storage.save_settings(&settings).unwrap();
        assert!(storage.load_settings().unwrap().launcher_track.is_none());
    }
}
