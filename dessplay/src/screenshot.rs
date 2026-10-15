//! Frames from the player, each user of them in its own private slot.
//!
//! Two features ask mpv for screenshots: AI commentary, which sends its
//! frame to the Anthropic API, and the houseguest's TV (phase 5c D7),
//! whose frame must never leave the machine. Each owns a [`Slot`]: a
//! stable path inside its own private directory, so neither can read,
//! delete or race the other's frame. Sharing one path would let
//! commentary delete the TV's frame mid-poll or, worse, attach a frame
//! the TV asked for.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// How many times [`poll`] looks for the frame, and how far apart: two
/// seconds in all. A real mpv answers a 4K `video` grab in about 0.1 s
/// (measured 2026-10-07: 73–128 ms at 3840×2160, 41–63 ms at 1080p,
/// `--vo=null`).
const POLLS: u32 = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A screenshot drop point: one stable path (`frame.jpg`) inside a
/// temporary directory its owner holds, mode 0700 on Unix. A
/// predictable name in the shared, world-writable `$TMPDIR` was a
/// symlink-following exfiltration hazard (2026-08-12 review): [`poll`]
/// reads whatever the path resolves to, so the path must live where only
/// this user can plant anything. The path stays the same for the slot's
/// life (mpv overwrites it), and the directory (with any leftover frame)
/// is removed on drop.
pub struct Slot {
    path: PathBuf,
    /// Whether a [`Claim`] on the slot lives (one frame at a time).
    busy: Arc<AtomicBool>,
    /// Owns the directory; kept alive for the slot's lifetime.
    _dir: tempfile::TempDir,
}

impl Slot {
    /// A new private slot, its directory named from `prefix` (each user
    /// its own: `dessplay-commentary-`, `dessplay-tv-`).
    pub fn create(prefix: &str) -> std::io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(prefix);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let dir = builder.tempdir()?;
        // .jpg drives mpv's format inference: a PNG of a 10-bit source
        // is 16-bit and ~8MB, where a JPEG frame is a few hundred KB.
        let path = dir.path().join("frame.jpg");
        Ok(Self {
            path,
            busy: Arc::default(),
            _dir: dir,
        })
    }

    /// Where the player writes this slot's frame.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ready the slot for a new frame: any leftover is deleted, and the
    /// moment returned is the request's (pass it to [`poll`], which
    /// rejects a frame written before it).
    pub fn clear(&self) -> SystemTime {
        let requested_at = SystemTime::now();
        let _ = std::fs::remove_file(&self.path);
        requested_at
    }

    /// The slot for one frame, [`Slot::clear`]ed: `None` while an earlier
    /// claim lives (its poll still running), so two requests never race
    /// on the one path, one deleting or reading the other's frame. The
    /// claim frees the slot when dropped, polled or not.
    pub fn claim(&self) -> Option<Claim> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(Claim {
            requested_at: self.clear(),
            path: self.path.clone(),
            busy: Arc::clone(&self.busy),
        })
    }
}

/// One frame's hold on a [`Slot`] ([`Slot::claim`]).
pub struct Claim {
    path: PathBuf,
    requested_at: SystemTime,
    busy: Arc<AtomicBool>,
}

impl Claim {
    /// Where the player is to write the frame.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// [`poll`] for the frame, then free the slot. Blocking.
    pub fn poll(self) -> Option<Vec<u8>> {
        poll(&self.path, self.requested_at)
    }

    /// [`Claim::poll`], `polls` times `interval` apart (tests wait less).
    pub fn poll_for(self, polls: u32, interval: Duration) -> Option<Vec<u8>> {
        poll_for(&self.path, self.requested_at, polls, interval)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

/// Wait for the player to finish writing the frame at `path`: the file
/// must exist, be non-empty, and hold the same size across two polls
/// ([`POLLS`] of them, [`POLL_INTERVAL`] apart). Blocking: call it from
/// the blocking pool. The frame is deleted once read. A miss is `None`.
/// A file whose mtime predates `requested_at` is a leftover an earlier
/// request's slow player finished late (the caller clears the path
/// before each request, but a late write can still race in behind that)
/// — it is deleted and never returned.
pub fn poll(path: &Path, requested_at: SystemTime) -> Option<Vec<u8>> {
    poll_for(path, requested_at, POLLS, POLL_INTERVAL)
}

/// [`poll`], `polls` times `interval` apart.
pub fn poll_for(
    path: &Path,
    requested_at: SystemTime,
    polls: u32,
    interval: Duration,
) -> Option<Vec<u8>> {
    let mut last_len = None;
    for _ in 0..polls {
        std::thread::sleep(interval);
        if let Ok(meta) = std::fs::metadata(path) {
            let len = meta.len();
            if len > 0 && last_len == Some(len) {
                if meta
                    .modified()
                    .ok()
                    .is_some_and(|written| written < requested_at)
                {
                    let _ = std::fs::remove_file(path);
                    tracing::debug!("screenshot predates the request; dropped");
                    return None;
                }
                let frame = std::fs::read(path).ok();
                let _ = std::fs::remove_file(path);
                return frame;
            }
            last_len = Some(len);
        }
    }
    let _ = std::fs::remove_file(path);
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Tests poll briefly (the real poll waits two seconds).
    const SHORT: Duration = Duration::from_millis(20);

    /// Two slots never share a directory or a path, each directory is
    /// private (0700), and a frame written to one is never what a poll
    /// of the other returns: commentary's slot can't read, delete or
    /// send the TV's frame (phase 5c D7: her TV's frames never leave the
    /// machine, and commentary's are sent to the API).
    #[test]
    fn slots_are_private_and_apart() {
        let commentary = Slot::create("dessplay-commentary-").unwrap();
        let tv = Slot::create("dessplay-tv-").unwrap();
        assert_ne!(commentary.path(), tv.path());
        let (cdir, tdir) = (
            commentary.path().parent().unwrap(),
            tv.path().parent().unwrap(),
        );
        assert_ne!(cdir, tdir);
        assert!(!tv.path().starts_with(cdir) && !commentary.path().starts_with(tdir));
        #[cfg(unix)]
        for dir in [cdir, tdir] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", dir.display());
        }
        // The TV's frame, written as mpv would after its request (at
        // least some 40 ms later: a file's mtime is the kernel's coarse
        // clock, which may read a few ms behind the request's).
        let asked = tv.clear();
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(tv.path(), b"the TV's frame").unwrap();
        // Commentary's request and poll see nothing of it, and leave it.
        let asked_commentary = commentary.clear();
        assert_eq!(
            poll_for(commentary.path(), asked_commentary, 3, SHORT),
            None
        );
        assert!(tv.path().exists(), "commentary never touches the TV's slot");
        assert_eq!(
            poll_for(tv.path(), asked, 3, SHORT).as_deref(),
            Some(&b"the TV's frame"[..])
        );
        let dir = tdir.to_path_buf();
        drop(tv);
        assert!(!dir.exists(), "a slot cleans up its directory");
    }

    /// A leftover frame from before the request is never returned (and
    /// is cleaned up), as commentary's own regression pins.
    #[test]
    fn a_frame_from_before_the_request_is_dropped() {
        let slot = Slot::create("dessplay-test-").unwrap();
        std::fs::write(slot.path(), b"stale").unwrap();
        let later = SystemTime::now() + Duration::from_secs(3600);
        assert_eq!(poll_for(slot.path(), later, 3, SHORT), None);
        assert!(!slot.path().exists());
    }

    /// One frame at a time: while a claim lives (its poll running), the
    /// slot can't be claimed again, so a second request never clears,
    /// reads or deletes the first's frame (step 12b review); dropped or
    /// polled, it frees the slot.
    #[test]
    fn a_slot_holds_one_claim_at_a_time() {
        let slot = Slot::create("dessplay-test-").unwrap();
        let first = slot.claim().expect("free");
        assert!(slot.claim().is_none(), "the first still polls");
        std::fs::write(first.path(), b"the first's frame").unwrap();
        assert!(slot.claim().is_none());
        assert!(first.path().exists(), "a refused claim clears nothing");
        drop(first);
        let second = slot.claim().expect("freed on drop");
        assert!(!second.path().exists(), "a claim clears the slot");
        assert_eq!(second.poll_for(2, SHORT), None);
        assert!(slot.claim().is_some(), "freed once polled");
    }
}
