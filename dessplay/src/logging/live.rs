//! One formatted stream feeds the daily file and a bounded in-memory tail.
//! Reloading its filter changes both destinations together.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::filter::{FilterExt, filter_fn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Filter, SubscriberExt};
use tracing_subscriber::{EnvFilter, Layer, Registry};

use super::{BiblicalDailyWriter, DailyWriterGuard};

const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 16 * 1024;

/// Independently adjustable logging scopes.
#[derive(Clone, Copy, Debug)]
pub enum LogScope {
    /// All workspace crates.
    DessPlay = 0,
    /// The rest of the Rust application (dependencies).
    Rust = 1,
}

/// A session-only override, or the original RUST_LOG/default filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Restore this scope's launch-time filter.
    Startup,
    /// Disable logging.
    Off,
    /// Errors only.
    Error,
    /// Warnings and errors.
    Warn,
    /// Normal operation.
    Info,
    /// Diagnostic detail.
    Debug,
    /// All events.
    Trace,
}

impl LogLevel {
    /// Dropdown order.
    pub const ALL: [Self; 7] = [
        Self::Startup,
        Self::Off,
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    /// Human-readable level.
    pub fn label(self) -> &'static str {
        match self {
            Self::Startup => "Startup",
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

/// A retained physical line, with a stable scroll anchor.
#[derive(Clone)]
pub struct LogLine {
    /// Monotonically increasing line number, unchanged on eviction.
    pub id: u64,
    /// Sanitized text (no terminal control characters).
    pub text: Arc<str>,
}

#[derive(Default)]
struct Tail {
    lines: VecDeque<LogLine>,
    bytes: usize,
    next: u64,
}

impl Tail {
    fn append(&mut self, bytes: &[u8], truncated: bool) {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        if truncated {
            text.push_str(" [event truncated in viewer]");
        }
        for line in text.lines() {
            let clean: String = line.chars().filter(|c| !c.is_control()).collect();
            self.bytes += clean.len();
            self.lines.push_back(LogLine {
                id: self.next,
                text: clean.into(),
            });
            self.next += 1;
            while self.lines.len() > MAX_LINES || self.bytes > MAX_BYTES {
                if let Some(old) = self.lines.pop_front() {
                    self.bytes -= old.text.len();
                }
            }
        }
    }
}

type Reload = dyn Fn([LogLevel; 2]) -> Result<(), String> + Send + Sync;

/// Shared log tail and live filter control; injectable into the UI in tests.
#[derive(Clone)]
pub struct LiveLogging {
    tail: Arc<Mutex<Tail>>,
    levels: Arc<Mutex<[LogLevel; 2]>>,
    startup: Arc<str>,
    reload: Arc<Reload>,
}

impl LiveLogging {
    /// Cheap snapshot: text is shared with the bounded buffer.
    pub fn lines(&self) -> Vec<LogLine> {
        self.tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .lines
            .iter()
            .cloned()
            .collect()
    }

    /// Changes only when new log lines arrive.
    pub fn revision(&self) -> u64 {
        self.tail.lock().unwrap_or_else(|e| e.into_inner()).next
    }

    /// Current choices for DessPlay and Rust respectively.
    pub fn levels(&self) -> [LogLevel; 2] {
        *self.levels.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Original effective launch filter, also used by Startup choices.
    pub fn startup_filter(&self) -> &str {
        &self.startup
    }

    /// Apply a single scope immediately, retaining the other scope.
    pub fn set_level(&self, scope: LogScope, level: LogLevel) -> Result<(), String> {
        let mut levels = self.levels.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = *levels;
        next[scope as usize] = level;
        (self.reload)(next)?;
        *levels = next;
        drop(levels);
        tracing::info!(?scope, ?level, "logging level changed for this session");
        Ok(())
    }
}

fn dessplay(metadata: &tracing::Metadata<'_>) -> bool {
    let target = metadata.target().split("::").next().unwrap_or_default();
    matches!(target, "dessplay" | "dessplay_core" | "dessplay_rendezvous")
}

fn filter(startup: &str, levels: [LogLevel; 2]) -> impl Filter<Registry> + Send + Sync + use<> {
    let scope_filter = |level: LogLevel| {
        EnvFilter::new(if level == LogLevel::Startup {
            startup
        } else {
            level.label()
        })
    };
    filter_fn(dessplay)
        .and(scope_filter(levels[0]))
        .or(filter_fn(|meta| !dessplay(meta)).and(scope_filter(levels[1])))
}

struct LiveWriter {
    daily: Option<BiblicalDailyWriter>,
    tail: Arc<Mutex<Tail>>,
}

struct LiveGuard<'a> {
    daily: Option<DailyWriterGuard<'a>>,
    tail: &'a Mutex<Tail>,
    bytes: Vec<u8>,
    truncated: bool,
}

impl Write for LiveGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let keep = buf
            .len()
            .min(MAX_EVENT_BYTES.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&buf[..keep]);
        self.truncated |= keep != buf.len();
        // A disk failure must not prevent the in-memory viewer from working.
        if let Some(daily) = &mut self.daily {
            let _ = daily.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(daily) = &mut self.daily {
            let _ = daily.flush();
        }
        Ok(())
    }
}

impl Drop for LiveGuard<'_> {
    fn drop(&mut self) {
        self.tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .append(&self.bytes, self.truncated);
    }
}

impl<'a> MakeWriter<'a> for LiveWriter {
    type Writer = LiveGuard<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        LiveGuard {
            daily: self.daily.as_ref().map(MakeWriter::make_writer),
            tail: &self.tail,
            bytes: Vec::new(),
            truncated: false,
        }
    }
}

/// Build the production subscriber without installing a process global;
/// tests can exercise the exact same path with `with_default`.
pub fn interactive_subscriber(
    startup: EnvFilter,
    dir: Option<PathBuf>,
) -> (impl tracing::Subscriber + Send + Sync, LiveLogging) {
    let startup: Arc<str> = startup.to_string().into();
    let levels = [LogLevel::Startup; 2];
    let (filter, handle) = tracing_subscriber::reload::Layer::new(filter(&startup, levels));
    let tail = Arc::new(Mutex::new(Tail::default()));
    let reload_startup = startup.clone();
    let logging = LiveLogging {
        tail: tail.clone(),
        levels: Arc::new(Mutex::new(levels)),
        startup,
        reload: Arc::new(move |levels| {
            handle
                .reload(self::filter(&reload_startup, levels))
                .map_err(|e| e.to_string())
        }),
    };
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(LiveWriter {
                daily: dir.map(BiblicalDailyWriter::new),
                tail,
            })
            .with_filter(filter),
    );
    (subscriber, logging)
}

static RUNTIME: OnceLock<LiveLogging> = OnceLock::new();

/// Get the interactive process's controller, if initialized.
pub fn runtime() -> Option<LiveLogging> {
    RUNTIME.get().cloned()
}

/// Install the interactive subscriber and expose its controller to the UI.
pub fn init(startup: EnvFilter, dir: Option<PathBuf>) {
    use tracing_subscriber::util::SubscriberInitExt;
    let (subscriber, logging) = interactive_subscriber(startup, dir);
    subscriber.init();
    let _ = RUNTIME.set(logging);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    // Reuse the callsites before and after every reload to exercise tracing's
    // interest cache, including re-enabling a previously disabled callsite.
    fn emit() {
        tracing::info!(target: "dessplay", "app info");
        tracing::debug!(target: "dessplay_core::net", "core debug");
        tracing::trace!(target: "dessplay_rendezvous::server", "server trace");
        tracing::info!(target: "quinn", "dependency info");
        tracing::debug!(target: "quinn", "dependency debug");
        tracing::trace!(target: "quinn", "dependency trace");
        tracing::warn!(target: "dessplay_impostor", "other crate warning");
    }

    #[test]
    fn every_level_pair_filters_scopes_independently_and_reloads_callsites() {
        let (subscriber, logs) = interactive_subscriber(EnvFilter::new("info"), None);
        tracing::subscriber::with_default(subscriber, || {
            for app in LogLevel::ALL {
                for rust in LogLevel::ALL {
                    logs.set_level(LogScope::DessPlay, app).unwrap();
                    logs.set_level(LogScope::Rust, rust).unwrap();
                    let start = logs.revision();
                    emit();
                    let text = logs
                        .lines()
                        .into_iter()
                        .filter(|line| line.id >= start)
                        .map(|line| line.text.to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let rank = |level| match level {
                        LogLevel::Off => 0,
                        LogLevel::Error => 1,
                        LogLevel::Warn => 2,
                        LogLevel::Startup | LogLevel::Info => 3,
                        LogLevel::Debug => 4,
                        LogLevel::Trace => 5,
                    };
                    for (message, enabled) in [
                        ("app info", rank(app) >= 3),
                        ("core debug", rank(app) >= 4),
                        ("server trace", rank(app) >= 5),
                        ("dependency info", rank(rust) >= 3),
                        ("dependency debug", rank(rust) >= 4),
                        ("dependency trace", rank(rust) >= 5),
                        ("other crate warning", rank(rust) >= 2),
                    ] {
                        assert_eq!(
                            text.contains(message),
                            enabled,
                            "{app:?}/{rust:?}: {message}: {text}"
                        );
                    }
                }
            }
        });
    }

    #[test]
    fn startup_restores_target_specific_filters_without_changing_other_scope() {
        let (subscriber, logs) =
            interactive_subscriber(EnvFilter::new("warn,dessplay_core=debug,quinn=trace"), None);
        tracing::subscriber::with_default(subscriber, || {
            logs.set_level(LogScope::DessPlay, LogLevel::Off).unwrap();
            logs.set_level(LogScope::Rust, LogLevel::Error).unwrap();
            logs.set_level(LogScope::DessPlay, LogLevel::Startup)
                .unwrap();
            emit();
            let text = logs
                .lines()
                .iter()
                .map(|line| line.text.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("core debug"));
            assert!(!text.contains("app info"));
            assert!(!text.contains("dependency trace"));
            logs.set_level(LogScope::Rust, LogLevel::Startup).unwrap();
            emit();
            assert!(
                logs.lines()
                    .iter()
                    .any(|line| line.text.contains("dependency trace"))
            );
        });
    }

    #[test]
    fn disk_and_viewer_receive_the_same_formatted_events() {
        let dir = tempfile::tempdir().unwrap();
        let (subscriber, logs) =
            interactive_subscriber(EnvFilter::new("info"), Some(dir.path().to_path_buf()));
        tracing::subscriber::with_default(subscriber, || {
            emit();
            logs.set_level(LogScope::DessPlay, LogLevel::Trace).unwrap();
            emit();
        });
        let disk = std::fs::read_to_string(super::super::current_log_path(dir.path())).unwrap();
        let tail = logs
            .lines()
            .iter()
            .map(|line| line.text.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(disk, tail);
    }

    #[test]
    fn tail_bounds_bytes_lines_and_single_events_without_losing_utf8() {
        let writer = LiveWriter {
            daily: None,
            tail: Arc::new(Mutex::new(Tail::default())),
        };
        for _ in 0..MAX_LINES + 10 {
            let mut guard = writer.make_writer();
            // Writer chunks need not align with UTF-8 characters or lines.
            for byte in "é\t\u{1b}hello\n".as_bytes() {
                guard.write_all(&[*byte]).unwrap();
            }
        }
        {
            let tail = writer.tail.lock().unwrap();
            assert_eq!(tail.lines.len(), MAX_LINES);
            assert_eq!(&*tail.lines[0].text, "éhello");
            assert_eq!(tail.lines[0].id, 10);
        }
        for _ in 0..300 {
            writer
                .make_writer()
                .write_all(&vec![b'x'; MAX_EVENT_BYTES * 2])
                .unwrap();
        }
        let tail = writer.tail.lock().unwrap();
        assert!(tail.bytes <= MAX_BYTES);
        assert!(tail.lines.len() < MAX_LINES);
        assert!(
            tail.lines
                .back()
                .unwrap()
                .text
                .ends_with("[event truncated in viewer]")
        );
    }
}
