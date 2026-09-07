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
            // Backends report absolute/canonical paths (not the spelling from
            // --layout-dir). Keep the same path identity on both sides of the filter.
            let watch_directory = watch_target(&directory).unwrap_or_else(|_| directory.clone());
            let mut watch_root = watch_directory
                .parent()
                .unwrap_or(&watch_directory)
                .to_path_buf();
            while !watch_root.is_dir() {
                if !watch_root.pop() {
                    break;
                }
            }
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
            let watch_error = match &mut watcher {
                Ok(watcher) => watcher
                    .watch(&watch_root, notify::RecursiveMode::Recursive)
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
// Resolve existing ancestors too: the override directory need not exist yet.
fn watch_target(directory: &std::path::Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(directory)?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        missing.push(name.to_owned());
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent;
    }
    let mut target = existing.canonicalize()?;
    for name in missing.into_iter().rev() {
        target.push(name);
    }
    Ok(target)
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
    fn receive(
        watcher: &Watcher,
        wanted: impl Fn(&Result<LayoutBundle, Diagnostic>) -> bool,
    ) -> Result<LayoutBundle, Diagnostic> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let candidate = watcher
                .results
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap_or_else(|error| {
                    panic!(
                        "layout watch did not deliver: {error}; watch error: {:?}",
                        watcher.error()
                    )
                });
            if candidate.generation == watcher.requested.load(Ordering::SeqCst)
                && wanted(&candidate.result)
            {
                return candidate.result;
            }
        }
    }
    fn ready(directory: PathBuf) -> Watcher {
        let watcher = Watcher::start(directory);
        watcher.reload();
        receive(&watcher, Result::is_ok).unwrap();
        watcher
    }
    fn atomic_css(directory: &std::path::Path, css: &str) {
        std::fs::write(directory.join("style.next"), css).unwrap();
        std::fs::rename(directory.join("style.next"), directory.join("style.css")).unwrap();
    }
    #[test]
    fn atomic_saves_deliver_invalid_then_repaired_bundles() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = ready(dir.path().into());
        atomic_css(dir.path(), "#users { padding:");
        let error = receive(&watcher, Result::is_err).unwrap_err();
        assert!(error.line > 0 && error.column > 0);
        atomic_css(dir.path(), "#users { display: none; }");
        let bundle = receive(&watcher, Result::is_ok).unwrap();
        let scene = super::super::Renderer::new(bundle)
            .arrange(
                "app",
                tuirealm::ratatui::layout::Rect::new(0, 0, 100, 40),
                &super::super::Presentation::default(),
            )
            .unwrap();
        assert!(!scene.visible_slots().contains(&"users"));
    }
    #[test]
    fn relative_layout_paths_receive_atomic_save_events() {
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let relative = PathBuf::from(".").join(dir.path().file_name().unwrap());
        let watcher = ready(relative);
        atomic_css(dir.path(), "#users { padding:");
        receive(&watcher, Result::is_err).unwrap_err();
    }
    #[test]
    fn replacing_the_layout_directory_keeps_live_reload_working() {
        let parent = tempfile::tempdir().unwrap();
        let directory = parent.path().join("ui");
        std::fs::create_dir(&directory).unwrap();
        let watcher = ready(directory.clone());
        let replacement = parent.path().join("replacement");
        std::fs::create_dir(&replacement).unwrap();
        atomic_css(&replacement, "#users { display: none; }");
        std::fs::rename(&directory, parent.path().join("previous")).unwrap();
        std::fs::rename(&replacement, &directory).unwrap();
        receive(&watcher, |result| {
            result
                .as_ref()
                .is_ok_and(|bundle| bundle.revision != LayoutBundle::builtin().unwrap().revision)
        })
        .unwrap();
        // A later edit must be observed too, after the old watched inode is gone.
        atomic_css(&directory, "#users { padding:");
        receive(&watcher, Result::is_err).unwrap_err();
    }
    #[test]
    fn creating_a_missing_override_directory_is_observed() {
        let parent = tempfile::tempdir().unwrap();
        let directory = parent.path().join("missing/ui");
        let watcher = ready(directory.clone());
        std::fs::create_dir_all(&directory).unwrap();
        atomic_css(&directory, "#users { padding:");
        receive(&watcher, Result::is_err).unwrap_err();
    }

    #[test]
    fn file_only_demonstrations_install_through_the_live_watcher() {
        use super::super::{Presentation, Renderer};
        use tuirealm::ratatui::{
            layout::{Rect, Size},
            style::Color,
        };
        let dir = tempfile::tempdir().unwrap();
        let watcher = ready(dir.path().into());
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let install_saved = |renderer: &mut Renderer| {
            let expected = LayoutBundle::load(dir.path()).unwrap().revision;
            let bundle = receive(&watcher, |result| {
                result
                    .as_ref()
                    .is_ok_and(|bundle| bundle.revision == expected)
            })
            .unwrap();
            renderer.install(bundle);
        };
        let save_template = |name: &str, xml: &str| {
            let templates = dir.path().join("templates");
            std::fs::create_dir_all(&templates).unwrap();
            std::fs::write(templates.join("next.tmp"), xml).unwrap();
            std::fs::rename(templates.join("next.tmp"), templates.join(name)).unwrap();
        };
        let area = Rect::new(0, 0, 100, 40);
        save_template(
            "app.xml",
            r#"<templates version="1"><template name="app"><row><slot id="users" name="users" style="flex-basis: 50%"/><slot id="chat" name="chat" style="flex-basis: 50%"/></row></template></templates>"#,
        );
        install_saved(&mut renderer);
        let scene = renderer
            .arrange("app", area, &Presentation::default())
            .unwrap();
        assert_eq!(scene.visible_slots(), ["users", "chat"]);
        assert!(scene.slot("users").x < scene.slot("chat").x);

        atomic_css(dir.path(), "#users { display: none; }");
        install_saved(&mut renderer);
        assert_eq!(
            renderer
                .arrange("app", area, &Presentation::default())
                .unwrap()
                .visible_slots(),
            ["chat"]
        );

        save_template(
            "playlist.xml",
            r#"<templates version="1"><template name="playlist-row"><row><text id="demo-watch" bind="watch"/><text id="demo-title" bind="title"/></row></template></templates>"#,
        );
        install_saved(&mut renderer);
        let scene = renderer
            .arrange(
                "playlist-row",
                Rect::new(0, 0, 60, 1),
                &Presentation::default()
                    .text("watch", "seen")
                    .text("title", "Episode"),
            )
            .unwrap();
        assert!(scene.bounds("demo-watch").x < scene.bounds("demo-title").x);

        atomic_css(
            dir.path(),
            "#users { display: none; } #form { background-color: #123456; padding: 0 2ch; }",
        );
        install_saved(&mut renderer);
        let scene = renderer
            .arrange("form", area, &Presentation::default().slot("header", 0, 1))
            .unwrap();
        assert_eq!(scene.style("header").bg, Some(Color::Rgb(0x12, 0x34, 0x56)));
        assert_eq!(scene.slot("header").x, 3);

        let rogue = include_str!("assets/templates/rogue.xml");
        let moved = rogue.replace("        <slot id=\"rogue-map\" name=\"map\" />\n        <slot id=\"rogue-sidebar\" name=\"sidebar\" if=\"wide\" />", "        <slot id=\"rogue-sidebar\" name=\"sidebar\" if=\"wide\" />\n        <slot id=\"rogue-map\" name=\"map\" />");
        assert_ne!(rogue, moved);
        save_template("rogue.xml", &moved);
        install_saved(&mut renderer);
        let scene = renderer
            .arrange(
                "rogue-game",
                area,
                &Presentation::default().boolean("wide", true),
            )
            .unwrap();
        assert!(scene.slot("sidebar").x < scene.slot("map").x);

        atomic_css(
            dir.path(),
            "#timestamp-gutter { display: none; } #chat-attachment-image { border: 0; }",
        );
        install_saved(&mut renderer);
        let picture = image::DynamicImage::new_rgb8(100, 100);
        let scene = renderer
            .attachment("demo", "12:00", Size::new(40, 8), &picture, (8, 16).into())
            .unwrap();
        assert_eq!(scene.slot("image").x, 0);
        assert_eq!(scene.slot_bounds("image"), scene.slot("image"));
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
