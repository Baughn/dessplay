//! Directory watches over the media roots (design.md, Media Library
//! Scanning). They shorten the wait for a new file; the periodic walk stays
//! the authority, so everything a watch can miss is still found within a
//! scan interval.
//!
//! Every directory a walk reads gets its own non-recursive watch, refreshed
//! after each walk, instead of one recursive watch per root. notify's
//! recursive inotify registration stops at the first directory it cannot
//! watch, and an HDD's mount root always holds an unreadable `lost+found`
//! (the layout watcher's 2026-09-15 bug, decisions.md). Per-directory
//! watches also follow symlinked season folders exactly as the walk does.
//!
//! An event never touches the index itself. Anything that might be a new
//! file wakes the file actor, which walks again once events go quiet.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use notify::Watcher as _;
use notify::event::{AccessKind, AccessMode, EventKind, ModifyKind, RenameMode};
use tokio::sync::mpsc;

/// Identifies what a watch is attached to. inotify watches inodes: a
/// directory deleted and re-created in place, or a filesystem re-mounted
/// on the same path, silently loses its watch. The directory's ctime
/// changes in every such case (and whenever an entry is added, which only
/// costs a redundant re-add), so a changed id means "watch it again".
#[cfg(target_os = "linux")]
pub(super) type DirId = (u64, u64, i64, i64);
/// Other backends (FSEvents) watch paths, which survive re-creation.
#[cfg(not(target_os = "linux"))]
pub(super) type DirId = ();

pub(super) fn dir_id(metadata: &std::fs::Metadata) -> DirId {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = metadata;
    }
}

/// Whether an event might mean a new or finished video file: one created,
/// renamed in, done being written or re-stamped, or a directory created
/// or renamed in (a whole season copied over). Reads (including our own
/// hashing and walking) and the stream of writes while a file grows are
/// not, and neither is anything that isn't a video or a directory
/// (`.part` downloads, subtitles).
pub(super) fn is_discovery(event: &notify::Event) -> bool {
    if event.need_rescan() {
        // The backend dropped events; only a walk can catch up.
        return true;
    }
    let dirs_count = match event.kind {
        EventKind::Create(_) => true,
        // The arriving side of a rename; FSEvents reports either side
        // as `Any`.
        EventKind::Modify(ModifyKind::Name(
            RenameMode::To | RenameMode::Both | RenameMode::Any,
        )) => true,
        // Finished writing, or a downloader stamping the final mtime.
        EventKind::Access(AccessKind::Close(AccessMode::Write))
        | EventKind::Modify(ModifyKind::Metadata(_)) => false,
        _ => return false,
    };
    event
        .paths
        .iter()
        .any(|path| super::is_video_file(path) || (dirs_count && path.is_dir()))
}

/// The set of watched media-root directories.
pub(super) struct LibraryWatch {
    watcher: Option<notify::RecommendedWatcher>,
    watched: HashMap<PathBuf, DirId>,
    warned_limit: bool,
    warned_failure: bool,
}

impl LibraryWatch {
    /// Watch nothing (watching is off, or the backend is unavailable).
    pub(super) fn disabled() -> Self {
        Self {
            watcher: None,
            watched: HashMap::new(),
            warned_limit: false,
            warned_failure: false,
        }
    }

    /// A watcher with no directories yet ([`Self::sync`] adds them).
    /// `wake` gets a signal whenever an event might mean a new file; its
    /// capacity coalesces a burst into one.
    pub(super) fn start(wake: mpsc::Sender<()>) -> Self {
        let handler = move |event: notify::Result<notify::Event>| match event {
            Ok(event) if is_discovery(&event) => {
                tracing::trace!(kind = ?event.kind, paths = ?event.paths, "media-root watch event");
                let _ = wake.try_send(());
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("media-root watch error: {e}"),
        };
        match notify::recommended_watcher(handler) {
            Ok(watcher) => Self {
                watcher: Some(watcher),
                ..Self::disabled()
            },
            Err(e) => {
                tracing::warn!(
                    "cannot watch media roots ({e}); new files appear at the periodic rescan"
                );
                Self::disabled()
            }
        }
    }

    /// Make the watched set exactly `dirs` (a walk's directories): unwatch
    /// what the walk no longer reaches, watch what is new or whose identity
    /// changed. Registration failures are logged once and never fatal; the
    /// periodic walk covers whatever is left unwatched.
    pub(super) fn sync(&mut self, dirs: &[(PathBuf, DirId)]) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        let wanted: HashMap<&Path, DirId> = dirs
            .iter()
            .map(|(path, id)| (path.as_path(), *id))
            .collect();
        let gone: Vec<PathBuf> = self
            .watched
            .keys()
            .filter(|path| !wanted.contains_key(path.as_path()))
            .cloned()
            .collect();
        let fresh: Vec<(&Path, DirId)> = wanted
            .iter()
            .filter(|(path, id)| self.watched.get(**path) != Some(*id))
            .map(|(path, id)| (*path, *id))
            .collect();
        if gone.is_empty() && fresh.is_empty() {
            return;
        }
        // One batch: on FSEvents every separate (un)watch restarts the
        // event stream.
        let mut paths = watcher.paths_mut();
        for path in &gone {
            // The backend may already have dropped a deleted directory.
            let _ = paths.remove(path);
            self.watched.remove(path);
        }
        let mut failed = 0usize;
        let mut first_failure = None;
        let mut limited = false;
        for (path, id) in fresh {
            match paths.add(path, notify::RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.watched.insert(path.to_path_buf(), id);
                }
                Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                    limited = true;
                    break;
                }
                Err(e) => {
                    failed += 1;
                    first_failure.get_or_insert_with(|| format!("{}: {e}", path.display()));
                }
            }
        }
        if let Err(e) = paths.commit() {
            // Nothing is watched; retry everything after the next walk.
            self.watched.clear();
            tracing::warn!(
                "media-root watch failed to start ({e}); new files appear at the periodic rescan"
            );
            return;
        }
        if limited && !self.warned_limit {
            self.warned_limit = true;
            tracing::warn!(
                watched = self.watched.len(),
                of = dirs.len(),
                "reached the directory-watch limit; new files in the rest appear at the \
                 periodic rescan (on Linux, raise fs.inotify.max_user_watches)"
            );
        }
        if let Some(first) = first_failure {
            if self.warned_failure {
                tracing::debug!(failed, first, "could not watch some media-root directories");
            } else {
                self.warned_failure = true;
                tracing::warn!(
                    failed,
                    first,
                    "could not watch some media-root directories; new files there appear at \
                     the periodic rescan"
                );
            }
        }
        tracing::debug!(
            watched = self.watched.len(),
            unwatched = gone.len(),
            "media-root directory watches refreshed"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use notify::event::{CreateKind, DataChange, Flag, MetadataKind, RemoveKind};

    fn event(kind: EventKind, path: &Path) -> notify::Event {
        notify::Event::new(kind).add_path(path.to_path_buf())
    }

    #[test]
    fn discovery_events_are_arrivals_of_videos_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("ep1.mkv");
        let partial = dir.path().join("ep1.mkv.part");
        let season = dir.path().join("Season 1");
        std::fs::create_dir(&season).unwrap();

        let arrivals = [
            EventKind::Create(CreateKind::File),
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
        ];
        for kind in arrivals {
            assert!(is_discovery(&event(kind, &video)), "{kind:?} on a video");
            assert!(
                is_discovery(&event(kind, &season)),
                "{kind:?} on a directory"
            );
            assert!(!is_discovery(&event(kind, &partial)), "{kind:?} on a .part");
        }
        // Finishing or re-stamping counts for videos only: a directory's
        // metadata changes as entries come and go, which Create covers.
        let finished = [
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
        ];
        for kind in finished {
            assert!(is_discovery(&event(kind, &video)), "{kind:?} on a video");
            assert!(
                !is_discovery(&event(kind, &season)),
                "{kind:?} on a directory"
            );
        }
        // Our own reads, a file growing, and departures are not arrivals.
        let ignored = [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Access(AccessKind::Read),
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            EventKind::Remove(RemoveKind::File),
        ];
        for kind in ignored {
            assert!(!is_discovery(&event(kind, &video)), "{kind:?} on a video");
            assert!(
                !is_discovery(&event(kind, &season)),
                "{kind:?} on a directory"
            );
        }
    }

    #[test]
    fn a_dropped_event_queue_always_rescans() {
        let overflow = notify::Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(is_discovery(&overflow));
    }
}
