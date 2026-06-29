//! File logging for the interactive client, split into one file per
//! "biblical" day (the 09:00-local boundary the chat pane uses for its
//! day separators — design.md, System Messages).
//!
//! Splitting per day keeps trimming cheap: old data is removed by
//! deleting whole files, never by rewriting the active log. At startup
//! [`trim_old_logs`] deletes day-files older than a week and migrates
//! away the legacy unitary `dessplay.log`; [`BiblicalDailyWriter`] then
//! writes today's file and rolls to a new one when the boundary passes
//! mid-session.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{Duration, Local, NaiveDate};
use tracing_subscriber::fmt::MakeWriter;

use crate::timeutil::biblical_date;

/// Prefix of a dated log file (`dessplay-2026-06-29.log`).
const LOG_PREFIX: &str = "dessplay-";
/// Suffix of a dated log file.
const LOG_SUFFIX: &str = ".log";
/// The pre-rotation unitary log file, deleted on first run under the
/// daily scheme.
const LEGACY_LOG: &str = "dessplay.log";
/// `chrono` format of the date embedded in a log filename.
const DATE_FMT: &str = "%Y-%m-%d";

/// The directory interactive-mode logs live in
/// (`$XDG_DATA_HOME/dessplay`), created if absent. `None` if the
/// platform data dir can't be resolved.
pub fn log_dir() -> Option<PathBuf> {
    let dir = dirs::data_dir()?.join("dessplay");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Filename of the day-file for `date`, e.g. `dessplay-2026-06-29.log`.
fn log_file_name(date: NaiveDate) -> String {
    format!("{LOG_PREFIX}{}{LOG_SUFFIX}", date.format(DATE_FMT))
}

/// The biblical date of a day-file's name, or `None` for the legacy
/// `dessplay.log` and any unrelated filename.
fn parse_log_date(name: &str) -> Option<NaiveDate> {
    let middle = name.strip_prefix(LOG_PREFIX)?.strip_suffix(LOG_SUFFIX)?;
    NaiveDate::parse_from_str(middle, DATE_FMT).ok()
}

/// Today's biblical date on the local clock.
pub fn today_biblical() -> NaiveDate {
    let now = Local::now();
    biblical_date(now.timestamp_millis().max(0) as u64).unwrap_or_else(|| now.date_naive())
}

/// Path of the day-file for `today`'s biblical date under `dir` (used to
/// point the user at the active log on a fatal error).
pub fn current_log_path(dir: &Path) -> PathBuf {
    dir.join(log_file_name(today_biblical()))
}

/// Delete day-files in `dir` whose biblical date is older than
/// `keep_days` before `today`, and remove the legacy unitary
/// `dessplay.log` if present. `today` is passed in (never read from the
/// clock) so this is deterministic and testable. Best-effort: a missing
/// directory or an unremovable file is logged and skipped, never fatal.
pub fn trim_old_logs(dir: &Path, today: NaiveDate, keep_days: i64) {
    let cutoff = today - Duration::days(keep_days);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("could not read log dir {}: {err}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let expired = parse_log_date(name).is_some_and(|date| date < cutoff);
        if (name == LEGACY_LOG || expired)
            && let Err(err) = std::fs::remove_file(entry.path())
        {
            tracing::warn!("could not remove old log {name}: {err}");
        }
    }
}

/// A [`MakeWriter`] that appends to a per-biblical-day file under a
/// directory, opening a new file when the 09:00-local boundary passes.
/// Synchronous (like the `Mutex<File>` it replaces): each log event
/// holds the state lock for the duration of its write.
pub struct BiblicalDailyWriter {
    dir: PathBuf,
    /// The currently-open day-file and the biblical date it is for.
    /// `None` until the first write (or after an open failure).
    state: Mutex<Option<(NaiveDate, File)>>,
}

impl BiblicalDailyWriter {
    /// A writer appending to day-files under `dir`.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            state: Mutex::new(None),
        }
    }

    /// Ensure the held file is for today's biblical date, rolling to a
    /// fresh file at the boundary. Best-effort: if the date can't be
    /// computed or the file can't be opened, the held file is left as is
    /// (the guard then discards writes, like `io::sink`).
    fn roll(&self, state: &mut Option<(NaiveDate, File)>) {
        let Some(today) = biblical_date(Local::now().timestamp_millis().max(0) as u64) else {
            return;
        };
        let stale = match state.as_ref() {
            Some((date, _)) => *date != today,
            None => true,
        };
        if !stale {
            return;
        }
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(log_file_name(today)))
        {
            *state = Some((today, file));
        }
    }
}

/// Per-event writer handle: holds the [`BiblicalDailyWriter`] state lock
/// and delegates writes to the open day-file.
pub struct DailyWriterGuard<'a> {
    state: MutexGuard<'a, Option<(NaiveDate, File)>>,
}

impl io::Write for DailyWriterGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.state.as_mut() {
            Some((_, file)) => file.write(buf),
            // No open file (no date / open failed): discard, like io::sink,
            // so logging never blocks or errors the TUI.
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.state.as_mut() {
            Some((_, file)) => file.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for BiblicalDailyWriter {
    type Writer = DailyWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        // Poison-tolerant: a panic mid-write must not wedge all logging.
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.roll(&mut state);
        DailyWriterGuard { state }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::io::Write;

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn file_name_and_parse_round_trip() {
        let d = date(2026, 6, 29);
        assert_eq!(log_file_name(d), "dessplay-2026-06-29.log");
        assert_eq!(parse_log_date("dessplay-2026-06-29.log"), Some(d));
    }

    #[test]
    fn parse_rejects_non_day_files() {
        assert_eq!(parse_log_date(LEGACY_LOG), None);
        assert_eq!(parse_log_date("notes.txt"), None);
        assert_eq!(parse_log_date("dessplay-bad.log"), None);
        assert_eq!(parse_log_date("dessplay-2026-13-40.log"), None);
    }

    #[test]
    fn trim_keeps_window_and_drops_old_and_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let today = date(2026, 6, 29);
        let make = |name: &str| std::fs::write(dir.path().join(name), b"x").unwrap();

        make(&log_file_name(today)); // today
        make(&log_file_name(today - Duration::days(6))); // in window
        make(&log_file_name(today - Duration::days(7))); // boundary: kept
        make(&log_file_name(today - Duration::days(8))); // expired
        make(LEGACY_LOG); // legacy: removed
        make("notes.txt"); // unrelated: untouched

        trim_old_logs(dir.path(), today, 7);

        let exists = |name: &str| dir.path().join(name).exists();
        assert!(exists(&log_file_name(today)));
        assert!(exists(&log_file_name(today - Duration::days(6))));
        assert!(exists(&log_file_name(today - Duration::days(7))));
        assert!(!exists(&log_file_name(today - Duration::days(8))));
        assert!(!exists(LEGACY_LOG));
        assert!(exists("notes.txt"));
    }

    #[test]
    fn trim_missing_dir_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        // Must not panic on a non-existent directory.
        trim_old_logs(&missing, date(2026, 6, 29), 7);
    }

    #[test]
    fn writer_appends_to_todays_file() {
        let dir = tempfile::tempdir().unwrap();
        let writer = BiblicalDailyWriter::new(dir.path().to_path_buf());
        writer.make_writer().write_all(b"hello\n").unwrap();
        writer.make_writer().write_all(b"world\n").unwrap();

        let path = dir.path().join(log_file_name(today_biblical()));
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "hello\nworld\n");
    }
}
