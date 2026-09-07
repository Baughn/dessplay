use super::{Diagnostic, LayoutBundle};
use notify::Watcher as _;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Candidate {
    generation: u64,
    result: Result<LayoutBundle, Diagnostic>,
}

/// Independent bounded delivery lane, with retries and stale-result rejection.
pub(crate) struct Watcher {
    requested: Arc<AtomicU64>,
    wake: mpsc::SyncSender<()>,
    results: mpsc::Receiver<Candidate>,
    errors: mpsc::Receiver<Diagnostic>,
    stopped: Arc<AtomicBool>,
}
impl Watcher {
    pub fn start(directory: PathBuf) -> Self {
        let requested = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let (wake, wakes) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(1);
        let (error_sender, errors) = mpsc::sync_channel(1);
        let worker_requested = requested.clone();
        let worker_stopped = stopped.clone();
        let events = wake.clone();
        std::thread::spawn(move || {
            let requested = worker_requested;
            let watch_directory = directory.clone();
            let event_generation = requested.clone();
            let event_errors = error_sender.clone();
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if let Ok(event) = event {
                        if matches!(event.kind, notify::EventKind::Access(_)) {
                            return;
                        }
                        if event.paths.iter().any(|path| {
                            path.starts_with(&watch_directory) || watch_directory.starts_with(path)
                        }) {
                            event_generation.fetch_add(1, Ordering::SeqCst);
                            let _ = events.try_send(());
                        }
                    } else if let Err(error) = event {
                        let _ = event_errors.try_send(Diagnostic::at(
                            &watch_directory,
                            "",
                            0,
                            format!("layout watch error: {error}; use Reload manually"),
                        ));
                    }
                });
            let mut watch_root = directory.as_path();
            while !watch_root.is_dir() {
                match watch_root.parent() {
                    Some(parent) => watch_root = parent,
                    None => break,
                }
            }
            let watch_error = match &mut watcher {
                Ok(watcher) => watcher
                    .watch(watch_root, notify::RecursiveMode::Recursive)
                    .err()
                    .map(|e| e.to_string()),
                Err(error) => Some(error.to_string()),
            };
            if let Some(message) = watch_error {
                let _ = error_sender.try_send(Diagnostic::at(
                    &directory,
                    "",
                    0,
                    format!("cannot watch layouts: {message}; use Reload manually"),
                ));
            }
            let mut pending = None;
            loop {
                if worker_stopped.load(Ordering::Relaxed) {
                    break;
                }
                let wake = if pending.is_some() {
                    wakes.recv_timeout(Duration::from_millis(20))
                } else {
                    wakes
                        .recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                };
                match wake {
                    Ok(()) => {
                        while wakes.recv_timeout(Duration::from_millis(200)).is_ok() {
                            if worker_stopped.load(Ordering::Relaxed) {
                                return;
                            }
                        }
                        let generation = requested.load(Ordering::SeqCst);
                        pending = Some(Candidate {
                            generation,
                            result: LayoutBundle::load(&directory),
                        });
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if !deliver(&sender, &mut pending, requested.load(Ordering::SeqCst)) {
                    break;
                }
            }
        });
        Self {
            requested,
            stopped,
            wake,
            results,
            errors,
        }
    }
    pub fn reload(&self) {
        self.requested.fetch_add(1, Ordering::SeqCst);
        let _ = self.wake.try_send(());
    }
    pub fn error(&self) -> Option<Diagnostic> {
        self.errors.try_recv().ok()
    }
    pub fn poll(&self) -> Option<Result<LayoutBundle, Diagnostic>> {
        let mut latest = None;
        while let Ok(candidate) = self.results.try_recv() {
            if candidate.generation == self.requested.load(Ordering::SeqCst) {
                latest = Some(candidate.result);
            }
        }
        latest
    }
}
fn deliver(
    sender: &mpsc::SyncSender<Candidate>,
    pending: &mut Option<Candidate>,
    generation: u64,
) -> bool {
    if let Some(candidate) = pending.take() {
        if candidate.generation != generation {
            return true;
        }
        match sender.try_send(candidate) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(candidate)) => *pending = Some(candidate),
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
        }
    }
    true
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.wake.try_send(());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    fn candidate(generation: u64) -> Candidate {
        Candidate {
            generation,
            result: LayoutBundle::builtin(),
        }
    }
    #[test]
    fn full_delivery_retries_and_coalesces_newer_generations() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.try_send(candidate(1)).ok().unwrap();
        let mut pending = Some(candidate(2));
        assert!(deliver(&sender, &mut pending, 2));
        assert_eq!(pending.as_ref().unwrap().generation, 2);
        // A new event invalidates pending compilation before delivery.
        assert!(deliver(&sender, &mut pending, 3));
        assert!(pending.is_none());
        pending = Some(candidate(3));
        receiver.recv().unwrap();
        assert!(deliver(&sender, &mut pending, 3));
        assert_eq!(receiver.recv().unwrap().generation, 3);
        assert!(pending.is_none());
    }
    #[test]
    fn queued_old_generation_cannot_be_installed_after_a_new_event() {
        let (sender, results) = mpsc::sync_channel(2);
        let (wake, _wakes) = mpsc::sync_channel(1);
        let watcher = Watcher {
            requested: Arc::new(AtomicU64::new(2)),
            wake,
            results,
            errors: mpsc::sync_channel(1).1,
            stopped: Arc::new(AtomicBool::new(false)),
        };
        sender.try_send(candidate(1)).ok().unwrap();
        assert!(watcher.poll().is_none());
        sender.try_send(candidate(2)).ok().unwrap();
        assert!(watcher.poll().unwrap().is_ok());
    }
}
