//! mpv over JSON IPC (design.md, Player Integration).
//!
//! An mpv process is spawned with `--input-ipc-server` pointing at a
//! fresh Unix socket and `--idle --keep-open`, so one instance persists
//! across the whole session; files are swapped with `loadfile`. A
//! reader task translates mpv's event stream into [`PlayerEvent`]s:
//!
//! - `pause` property changes → [`PlayerEvent::PauseChanged`] — except
//!   while a load is settling (we force `pause=yes` before `loadfile`;
//!   that flip is the load contract, not news) and except the
//!   mechanical pause mpv performs itself when `keep-open` hits end of
//!   file (the EOF transition belongs to the server, not to a phantom
//!   user pause).
//! - a `seek` event followed by `playback-restart` →
//!   [`PlayerEvent::Seeked`], with the landed position fetched via
//!   `get_property time-pos` (the property observation alone may be
//!   stale mid-seek).
//! - `eof-reached` becoming true → [`PlayerEvent::Eof`].
//! - process exit → [`PlayerEvent::Exited`] (clean iff status 0).
//!
//! Echo suppression is *not* done here — the actor above sorts our
//! commands' echoes from user input. This layer only hides mpv's
//! internal mechanics (load pauses, keep-open pauses).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

use super::{Player, PlayerError, PlayerEvent, PlayerFactory};

/// How long to wait for mpv to create its IPC socket.
const SOCKET_WAIT: Duration = Duration::from_secs(10);
/// Grace period between `quit` and a kill on shutdown.
const QUIT_GRACE: Duration = Duration::from_secs(2);

/// Property-observation ids (mpv echoes them back in events).
const OBS_PAUSE: u64 = 1;
const OBS_TIME_POS: u64 = 2;
const OBS_DURATION: u64 = 3;
const OBS_SUB_TEXT: u64 = 4;
const OBS_EOF: u64 = 5;
const OBS_PATH: u64 = 6;

/// One running mpv instance.
pub struct MpvPlayer {
    writer: Arc<Mutex<OwnedWriteHalf>>,
    events: Mutex<mpsc::Receiver<PlayerEvent>>,
    request_id: Arc<AtomicU64>,
    /// Set during `load()`, cleared by the reader on `file-loaded`:
    /// pause flips while loading are mechanics, not news.
    loading: Arc<AtomicBool>,
    /// Tell the supervisor task to stop being patient.
    kill: mpsc::Sender<()>,
    /// True when we attached to a user-launched mpv rather than spawning
    /// our own: shutdown must not `quit` it (it isn't ours to kill).
    attached: bool,
}

impl MpvPlayer {
    /// Spawn mpv and connect to its IPC socket. `extra_args` come after
    /// the defaults, so they can override them (mpv: last flag wins) —
    /// the real-mpv tests pass `--vo=null --ao=null --force-window=no`.
    pub async fn launch(
        binary: &str,
        socket: PathBuf,
        extra_args: &[String],
    ) -> Result<MpvPlayer, PlayerError> {
        // A stale socket from a previous run would make the wait below
        // succeed against nothing.
        let _ = std::fs::remove_file(&socket);
        let child = Command::new(binary)
            .arg("--idle=yes")
            .arg("--keep-open=yes")
            .arg("--force-window=yes")
            .arg("--no-terminal")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .args(extra_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| PlayerError::Setup(format!("spawning {binary}: {e}")))?;
        tracing::info!(binary, socket = %socket.display(), "mpv spawned");

        let stream = wait_for_socket(&socket).await?;
        Self::setup(stream, Some(child)).await
    }

    /// Attach to an mpv the user already launched (a dev/headless aid;
    /// see `--attach-mpv`). The socket is the user's live
    /// `--input-ipc-server`, so we neither remove it nor spawn a process —
    /// and on shutdown we leave that mpv running. The user must launch mpv
    /// with `--idle --keep-open` so our EOF/load mechanics hold.
    pub async fn attach(socket: PathBuf) -> Result<MpvPlayer, PlayerError> {
        tracing::info!(socket = %socket.display(), "attaching to mpv");
        let stream = wait_for_socket(&socket).await?;
        Self::setup(stream, None).await
    }

    /// Wire a connected IPC stream into a running player: split it, start
    /// the reader and the supervisor (process-watching when we spawned
    /// mpv, socket-watching when we attached), and register property
    /// observations. `child` is `Some` only in spawn mode.
    async fn setup(stream: UnixStream, child: Option<Child>) -> Result<MpvPlayer, PlayerError> {
        let attached = child.is_none();
        let (read_half, write_half) = stream.into_split();
        let writer = Arc::new(Mutex::new(write_half));
        let (event_tx, event_rx) = mpsc::channel(256);
        let (kill_tx, kill_rx) = mpsc::channel(1);
        let request_id = Arc::new(AtomicU64::new(1));
        let loading = Arc::new(AtomicBool::new(false));

        let read_task = tokio::spawn(read_loop(
            BufReader::new(read_half),
            Arc::clone(&writer),
            Arc::clone(&request_id),
            Arc::clone(&loading),
            event_tx.clone(),
        ));
        match child {
            // Spawn mode: watch the process; the kill signal gives `quit` a
            // grace period before we kill it.
            Some(child) => {
                tokio::spawn(supervise(child, kill_rx, event_tx));
            }
            // Attach mode: there is no process of ours — the read loop
            // ending means the user's mpv closed the socket.
            None => {
                tokio::spawn(supervise_attached(read_task, kill_rx, event_tx));
            }
        }

        let player = MpvPlayer {
            writer,
            events: Mutex::new(event_rx),
            request_id,
            loading,
            kill: kill_tx,
            attached,
        };
        for (id, name) in [
            (OBS_PAUSE, "pause"),
            (OBS_TIME_POS, "time-pos"),
            (OBS_DURATION, "duration"),
            // ass-full carries the ASS `Name`/actor field (for per-speaker
            // coloring) and the full override-tagged text; we strip the tags
            // ourselves in `parse_ass_full`. Requires mpv >= 0.39.0.
            (OBS_SUB_TEXT, "sub-text/ass-full"),
            (OBS_EOF, "eof-reached"),
            // The loaded file path, so the actor can detect a file the user
            // loaded directly (drag-and-drop) versus one we commanded.
            (OBS_PATH, "path"),
        ] {
            player
                .command(json!(["observe_property", id, name]))
                .await?;
        }
        Ok(player)
    }

    async fn command(&self, command: Value) -> Result<(), PlayerError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        send_command(&self.writer, command, id).await
    }
}

async fn send_command(
    writer: &Mutex<OwnedWriteHalf>,
    command: Value,
    request_id: u64,
) -> Result<(), PlayerError> {
    let mut line = json!({ "command": command, "request_id": request_id }).to_string();
    tracing::trace!(ipc = %line, "mpv command");
    line.push('\n');
    writer
        .lock()
        .await
        .write_all(line.as_bytes())
        .await
        .map_err(|e| PlayerError::Gone(format!("mpv ipc write: {e}")))
}

async fn wait_for_socket(socket: &Path) -> Result<UnixStream, PlayerError> {
    let deadline = tokio::time::Instant::now() + SOCKET_WAIT;
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(PlayerError::Setup(format!(
                        "mpv socket {} never came up: {e}",
                        socket.display()
                    )));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

impl Player for MpvPlayer {
    async fn load(&self, path: &Path, title: Option<&str>) -> Result<(), PlayerError> {
        self.loading.store(true, Ordering::Relaxed);
        // Pause first so the new file opens paused (the trait contract).
        self.command(json!(["set_property", "pause", true])).await?;
        // Override the displayed title (set before loadfile so it applies to
        // the new file). Cached downloads are hash-named on disk, so without
        // this mpv would show the ed2k hash instead of the real filename.
        if let Some(title) = title {
            self.command(json!(["set_property", "force-media-title", title]))
                .await?;
        }
        self.command(json!(["loadfile", path.to_string_lossy(), "replace"]))
            .await
    }

    async fn set_pause(&self, paused: bool) -> Result<(), PlayerError> {
        self.command(json!(["set_property", "pause", paused])).await
    }

    async fn seek(&self, position_millis: u64) -> Result<(), PlayerError> {
        self.command(json!([
            "seek",
            position_millis as f64 / 1000.0,
            "absolute+exact"
        ]))
        .await
    }

    async fn set_speed(&self, speed: f64) -> Result<(), PlayerError> {
        self.command(json!(["set_property", "speed", speed])).await
    }

    async fn show_osd(&self, text: &str) -> Result<(), PlayerError> {
        self.command(json!(["show-text", text, 4000])).await
    }

    async fn recv(&self) -> Result<PlayerEvent, PlayerError> {
        self.events
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| PlayerError::Gone("mpv event stream ended".into()))
    }

    async fn shutdown(&self) {
        // An attached mpv is the user's process, not ours: tell the
        // supervisor to stand down but never `quit` it.
        if !self.attached {
            let _ = self.command(json!(["quit"])).await;
        }
        let _ = self.kill.try_send(());
    }
}

/// Watch the child; report its exit. On a kill signal, give `quit` a
/// grace period and then kill the process.
async fn supervise(
    mut child: Child,
    mut kill: mpsc::Receiver<()>,
    events: mpsc::Sender<PlayerEvent>,
) {
    let status = tokio::select! {
        status = child.wait() => status,
        _ = kill.recv() => {
            match tokio::time::timeout(QUIT_GRACE, child.wait()).await {
                Ok(status) => status,
                Err(_) => {
                    tracing::warn!("mpv ignored quit; killing it");
                    let _ = child.kill().await;
                    child.wait().await
                }
            }
        }
    };
    let clean = status.as_ref().map(|s| s.success()).unwrap_or(false);
    tracing::info!(?status, "mpv exited");
    let _ = events.send(PlayerEvent::Exited { clean }).await;
}

/// Supervisor for an attached mpv (no process of ours to wait on). The
/// read loop ending means the user's mpv closed the socket; a kill signal
/// means we are shutting down. Either way we emit `Exited` so the actor's
/// relaunch path runs — in attach mode that re-attaches, waiting for the
/// user's mpv to come back. We report `clean` because we have no exit
/// status to judge.
async fn supervise_attached(
    read: JoinHandle<()>,
    mut kill: mpsc::Receiver<()>,
    events: mpsc::Sender<PlayerEvent>,
) {
    tokio::select! {
        _ = read => tracing::info!("attached mpv closed its ipc socket"),
        _ = kill.recv() => tracing::info!("detaching from mpv"),
    }
    let _ = events.send(PlayerEvent::Exited { clean: true }).await;
}

/// Translation state for the reader loop.
#[derive(Default)]
struct Translate {
    /// A `seek` event arrived; the next `playback-restart` resolves it.
    seek_pending: bool,
    /// In-flight `get_property time-pos` request for a finished seek.
    seek_pos_request: Option<u64>,
    /// Last observed eof-reached (the keep-open pause filter).
    eof_reached: bool,
    /// Last observed pause state, deduped (mpv re-announces on observe).
    last_pause: Option<bool>,
    /// Last observed loaded path, deduped (mpv re-announces on observe).
    last_path: Option<String>,
}

/// How long a `pause=true` is held back waiting for an `eof-reached`
/// that would mark it as keep-open mechanics rather than a user pause.
/// mpv emits the two in the same wakeup; the window only pads scheduling.
const EOF_PAUSE_WINDOW: Duration = Duration::from_millis(250);

async fn read_loop(
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    request_id: Arc<AtomicU64>,
    loading: Arc<AtomicBool>,
    events: mpsc::Sender<PlayerEvent>,
) {
    let mut lines = reader.lines();
    let mut state = Translate::default();
    // mpv pauses *before* flipping eof-reached when keep-open hits the
    // end, so a pause=true can't be attributed when it arrives: hold it
    // until the next message (or a beat) decides.
    let mut held_pause = false;
    loop {
        let line = if held_pause {
            match tokio::time::timeout(EOF_PAUSE_WINDOW, lines.next_line()).await {
                Ok(line) => line,
                Err(_) => {
                    // Nothing followed: a genuine user pause.
                    held_pause = false;
                    if events.send(PlayerEvent::PauseChanged(true)).await.is_err() {
                        return;
                    }
                    continue;
                }
            }
        } else {
            lines.next_line().await
        };
        let Ok(Some(line)) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!(line, "unparseable mpv ipc line");
            continue;
        };
        tracing::trace!(ipc = %line, "mpv message");
        for event in translate(&msg, &mut state, &loading) {
            let to_send = match event {
                PlayerEvent::PauseChanged(true) => {
                    held_pause = true;
                    continue;
                }
                PlayerEvent::Eof => {
                    // The held pause was keep-open mechanics: drop it.
                    held_pause = false;
                    PlayerEvent::Eof
                }
                // Position/duration/subtitle churn says nothing about
                // *why* the pause happened; it rides along at EOF (a
                // final time-pos lands between the pause and
                // eof-reached). Anything else resolves the hold as a
                // real user pause.
                neutral @ (PlayerEvent::Position { .. }
                | PlayerEvent::DurationKnown { .. }
                | PlayerEvent::SubtitleLine { .. }
                | PlayerEvent::PathChanged { .. }) => neutral,
                other => {
                    if std::mem::take(&mut held_pause)
                        && events.send(PlayerEvent::PauseChanged(true)).await.is_err()
                    {
                        return;
                    }
                    other
                }
            };
            if events.send(to_send).await.is_err() {
                return;
            }
        }
        // A finished seek needs the landed position: ask for it.
        if state.seek_pending
            && msg.get("event").and_then(Value::as_str) == Some("playback-restart")
        {
            state.seek_pending = false;
            let id = request_id.fetch_add(1, Ordering::Relaxed);
            state.seek_pos_request = Some(id);
            let _ = send_command(&writer, json!(["get_property", "time-pos"]), id).await;
        }
    }
    tracing::debug!("mpv ipc reader exiting");
    // Exit reporting is the supervisor's job; nothing to send here.
}

/// Translate one mpv IPC message into player events.
fn translate(msg: &Value, state: &mut Translate, loading: &AtomicBool) -> Vec<PlayerEvent> {
    // The reply to our post-seek position query.
    if let Some(id) = state.seek_pos_request
        && msg.get("request_id").and_then(Value::as_u64) == Some(id)
    {
        state.seek_pos_request = None;
        if let Some(seconds) = msg.get("data").and_then(Value::as_f64) {
            return vec![PlayerEvent::Seeked {
                position_millis: (seconds * 1000.0).max(0.0) as u64,
            }];
        }
        return vec![];
    }

    let Some(event) = msg.get("event").and_then(Value::as_str) else {
        return vec![];
    };
    match event {
        "property-change" => {
            let data = msg.get("data");
            match msg.get("id").and_then(Value::as_u64) {
                Some(OBS_PAUSE) => {
                    let Some(paused) = data.and_then(Value::as_bool) else {
                        return vec![];
                    };
                    if state.last_pause == Some(paused) {
                        return vec![];
                    }
                    state.last_pause = Some(paused);
                    if loading.load(Ordering::Relaxed) {
                        // Our own pre-load pause: the contract, not news.
                        return vec![];
                    }
                    if paused && state.eof_reached {
                        // keep-open pauses at EOF by itself; the server
                        // owns that transition.
                        return vec![];
                    }
                    vec![PlayerEvent::PauseChanged(paused)]
                }
                Some(OBS_TIME_POS) => data
                    .and_then(Value::as_f64)
                    .map(|seconds| PlayerEvent::Position {
                        position_millis: (seconds * 1000.0).max(0.0) as u64,
                    })
                    .into_iter()
                    .collect(),
                Some(OBS_DURATION) => data
                    .and_then(Value::as_f64)
                    .map(|seconds| PlayerEvent::DurationKnown {
                        duration_millis: (seconds * 1000.0).max(0.0) as u64,
                    })
                    .into_iter()
                    .collect(),
                Some(OBS_SUB_TEXT) => {
                    let raw = data.and_then(Value::as_str).unwrap_or_default();
                    let (text, speaker) = parse_ass_full(raw);
                    vec![PlayerEvent::SubtitleLine { text, speaker }]
                }
                Some(OBS_EOF) => {
                    let reached = data.and_then(Value::as_bool).unwrap_or(false);
                    let rising = reached && !state.eof_reached;
                    state.eof_reached = reached;
                    if rising {
                        vec![PlayerEvent::Eof]
                    } else {
                        vec![]
                    }
                }
                Some(OBS_PATH) => {
                    // mpv re-announces on observe and on every load; emit only
                    // on a real change (idle/cleared `path` is null → none).
                    let Some(path) = data.and_then(Value::as_str) else {
                        return vec![];
                    };
                    if state.last_path.as_deref() == Some(path) {
                        return vec![];
                    }
                    state.last_path = Some(path.to_string());
                    vec![PlayerEvent::PathChanged {
                        path: path.to_string(),
                    }]
                }
                _ => vec![],
            }
        }
        "seek" => {
            state.seek_pending = true;
            vec![]
        }
        "file-loaded" => {
            loading.store(false, Ordering::Relaxed);
            state.eof_reached = false;
            vec![PlayerEvent::Loaded]
        }
        _ => vec![],
    }
}

/// Parse mpv's `sub-text/ass-full` value into `(plain_text, speaker)`.
///
/// Each event is a `.ass` file event line:
/// `Dialogue: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text`
/// — so field index 4 is the `Name`/actor (the speaker, never displayed)
/// and field 9 (the remainder, since `Text` may contain commas) is the
/// text, still carrying ASS override tags we strip here. Multiple
/// simultaneous events arrive newline-separated; we join their texts and
/// take the first non-empty speaker. A line that is not a well-formed
/// `Dialogue:` event is treated as plain text with no speaker.
fn parse_ass_full(raw: &str) -> (String, Option<String>) {
    let mut texts: Vec<String> = Vec::new();
    let mut speaker: Option<String> = None;
    for line in raw.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (name, text) = match line.strip_prefix("Dialogue:") {
            Some(rest) => {
                // 9 commas split off the 10 leading fields; the 10th
                // (Text) keeps any internal commas.
                let fields: Vec<&str> = rest.splitn(10, ',').collect();
                if fields.len() == 10 {
                    (fields[4].trim(), fields[9])
                } else {
                    ("", line)
                }
            }
            None => ("", line),
        };
        let stripped = strip_ass_tags(text);
        if !stripped.is_empty() {
            texts.push(stripped);
        }
        if speaker.is_none() && !name.is_empty() {
            speaker = Some(name.to_string());
        }
    }
    (texts.join(" "), speaker)
}

/// Strip ASS override tags from an event's Text field: drop `{...}`
/// override blocks, turn the `\N`/`\n` line breaks and `\h` hard space
/// into spaces, and leave any other escape as-is.
///
/// Also handles **drawing mode**: a `{\p<n>}` block with non-zero `<n>`
/// switches the renderer into vector-drawing, where the following "text"
/// is a path of `m`/`l`/`b` coordinate commands (a sign or shape, not
/// dialogue); `{\p0}` switches back. The path commands are not words and
/// are dropped, so a pure shape collapses to the empty string (and is
/// filtered out by `parse_ass_full`).
fn strip_ass_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut drawing = false;
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                // Consume the override block (an unclosed brace eats the
                // rest, matching libass's lenient behavior), tracking any
                // `\p<n>` drawing-mode toggle inside it.
                let mut block = String::new();
                for inner in chars.by_ref() {
                    if inner == '}' {
                        break;
                    }
                    block.push(inner);
                }
                if let Some(scale) = last_p_tag(&block) {
                    drawing = scale != 0;
                }
            }
            // In drawing mode the literal text is path data; drop it.
            _ if drawing => {}
            '\\' => match chars.peek() {
                Some('N') | Some('n') | Some('h') => {
                    chars.next();
                    out.push(' ');
                }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Return the argument of the last `\p<n>` drawing-scale tag in an ASS
/// override block, or `None` if the block has no such tag. `\p0` disables
/// drawing; any non-zero value enables it. Only `\p` immediately followed
/// by a digit counts — `\pos(...)`, `\pbo`, and other `\p`-prefixed tags
/// are deliberately ignored.
fn last_p_tag(block: &str) -> Option<u32> {
    let bytes = block.as_bytes();
    let mut result = None;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1] == b'p' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                result = block[start..j].parse().ok();
                i = j;
                continue;
            }
        }
        i += 1;
    }
    result
}

/// Produces [`MpvPlayer`]s, either by spawning mpv (the normal case) or by
/// attaching to one the user already launched (the `--attach-mpv` dev aid).
pub struct MpvFactory {
    mode: Mode,
}

enum Mode {
    /// Spawn a fresh mpv per call, each with its own per-instance socket.
    Spawn {
        binary: String,
        socket_dir: PathBuf,
        extra_args: Vec<String>,
        instance: u64,
    },
    /// Connect to a user-launched mpv at a fixed socket path. Every call
    /// re-attaches to the same socket (the relaunch path reuses it).
    Attach { socket: PathBuf },
}

impl MpvFactory {
    /// A factory using `binary` (usually `"mpv"`), with sockets under
    /// the user's runtime directory.
    pub fn new(binary: impl Into<String>) -> MpvFactory {
        Self::with_args(binary, Vec::new())
    }

    /// A factory with extra mpv arguments (appended after the defaults,
    /// so they win).
    pub fn with_args(binary: impl Into<String>, extra_args: Vec<String>) -> MpvFactory {
        let socket_dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        MpvFactory {
            mode: Mode::Spawn {
                binary: binary.into(),
                socket_dir,
                extra_args,
                instance: 0,
            },
        }
    }

    /// A factory that attaches to a user-launched mpv at `socket` instead
    /// of spawning one (see [`MpvPlayer::attach`]).
    pub fn attach(socket: PathBuf) -> MpvFactory {
        MpvFactory {
            mode: Mode::Attach { socket },
        }
    }
}

impl PlayerFactory for MpvFactory {
    type Player = MpvPlayer;

    async fn spawn(&mut self) -> Result<MpvPlayer, PlayerError> {
        match &mut self.mode {
            Mode::Spawn {
                binary,
                socket_dir,
                extra_args,
                instance,
            } => {
                *instance += 1;
                let socket = socket_dir.join(format!(
                    "dessplay-mpv-{}-{}.sock",
                    std::process::id(),
                    instance
                ));
                MpvPlayer::launch(binary, socket, extra_args).await
            }
            Mode::Attach { socket } => MpvPlayer::attach(socket.clone()).await,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn ev(json: &str, state: &mut Translate, loading: bool) -> Vec<PlayerEvent> {
        let loading = AtomicBool::new(loading);
        translate(&serde_json::from_str(json).unwrap(), state, &loading)
    }

    #[test]
    fn pause_property_translates_and_dedups() {
        let mut state = Translate::default();
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":1,"name":"pause","data":true}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::PauseChanged(true)]
        );
        // mpv re-announces on observe; dedup.
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":1,"name":"pause","data":true}"#,
                &mut state,
                false
            ),
            vec![]
        );
    }

    #[test]
    fn path_property_translates_and_dedups() {
        let mut state = Translate::default();
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":6,"name":"path","data":"/media/KillAo - 01.mkv"}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::PathChanged {
                path: "/media/KillAo - 01.mkv".into()
            }]
        );
        // mpv re-announces on observe; dedup.
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":6,"name":"path","data":"/media/KillAo - 01.mkv"}"#,
                &mut state,
                false
            ),
            vec![]
        );
        // A different path emits again (the drag-in case).
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":6,"name":"path","data":"/other/dragged.mkv"}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::PathChanged {
                path: "/other/dragged.mkv".into()
            }]
        );
        // Null (idle / cleared) produces nothing.
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":6,"name":"path","data":null}"#,
                &mut state,
                false
            ),
            vec![]
        );
    }

    #[test]
    fn pause_during_load_is_swallowed() {
        let mut state = Translate::default();
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":1,"name":"pause","data":true}"#,
                &mut state,
                true
            ),
            vec![]
        );
    }

    #[test]
    fn keep_open_pause_at_eof_is_swallowed() {
        let mut state = Translate::default();
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":5,"name":"eof-reached","data":true}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::Eof]
        );
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":1,"name":"pause","data":true}"#,
                &mut state,
                false
            ),
            vec![],
            "keep-open's mechanical pause is not a user pause"
        );
        // EOF is edge-triggered, not re-reported.
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":5,"name":"eof-reached","data":true}"#,
                &mut state,
                false
            ),
            vec![]
        );
    }

    #[test]
    fn file_loaded_resets_eof_and_emits_loaded() {
        let mut state = Translate {
            eof_reached: true,
            ..Default::default()
        };
        assert_eq!(
            ev(r#"{"event":"file-loaded"}"#, &mut state, true),
            vec![PlayerEvent::Loaded]
        );
        assert!(!state.eof_reached);
    }

    #[test]
    fn seek_position_comes_from_the_query_reply() {
        let mut state = Translate::default();
        assert_eq!(ev(r#"{"event":"seek"}"#, &mut state, false), vec![]);
        assert!(state.seek_pending);
        // (read_loop issues the get_property; simulate its bookkeeping.)
        state.seek_pending = false;
        state.seek_pos_request = Some(42);
        assert_eq!(
            ev(
                r#"{"error":"success","request_id":42,"data":63.25}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::Seeked {
                position_millis: 63_250
            }]
        );
    }

    #[test]
    fn positions_and_duration_convert_to_millis() {
        let mut state = Translate::default();
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":2,"name":"time-pos","data":12.5}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::Position {
                position_millis: 12_500
            }]
        );
        // Null (no file) produces nothing.
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":2,"name":"time-pos","data":null}"#,
                &mut state,
                false
            ),
            vec![]
        );
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":3,"name":"duration","data":1440.0}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::DurationKnown {
                duration_millis: 1_440_000
            }]
        );
    }

    #[test]
    fn subtitle_lines_pass_through_and_clear() {
        let mut state = Translate::default();
        // A plain (non-Dialogue) value still passes through as text.
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":4,"name":"sub-text/ass-full","data":"hello"}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::SubtitleLine {
                text: "hello".into(),
                speaker: None
            }]
        );
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":4,"name":"sub-text/ass-full","data":null}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::SubtitleLine {
                text: String::new(),
                speaker: None
            }]
        );
    }

    #[test]
    fn ass_full_dialogue_yields_text_and_speaker() {
        let mut state = Translate::default();
        assert_eq!(
            ev(
                r#"{"event":"property-change","id":4,"name":"sub-text/ass-full","data":"Dialogue: 0,0:00:01.00,0:00:03.00,Default,Frieren,0,0,0,,{\\i1}Hello,{\\i0} there"}"#,
                &mut state,
                false
            ),
            vec![PlayerEvent::SubtitleLine {
                // Comma inside Text is preserved; override tags stripped.
                text: "Hello, there".into(),
                speaker: Some("Frieren".into())
            }]
        );
    }

    #[test]
    fn parse_ass_full_handles_name_tags_breaks_and_fallback() {
        // Normal line with a speaker.
        assert_eq!(
            parse_ass_full("Dialogue: 0,0:00:00.00,0:00:01.00,Default,Stark,0,0,0,,Hi"),
            ("Hi".into(), Some("Stark".into()))
        );
        // Empty Name -> no speaker; override tags and \N break stripped.
        assert_eq!(
            parse_ass_full(
                r"Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{\pos(1,2)}you\Ndemons"
            ),
            ("you demons".into(), None)
        );
        // Multiple events: texts joined, first non-empty speaker wins.
        assert_eq!(
            parse_ass_full(
                "Dialogue: 0,0,0,Default,,0,0,0,,one\nDialogue: 0,0,0,Default,Fern,0,0,0,,two"
            ),
            ("one two".into(), Some("Fern".into()))
        );
        // Not a Dialogue line: plain text, no speaker.
        assert_eq!(parse_ass_full("just text"), ("just text".into(), None));
        // Empty / cleared cue.
        assert_eq!(parse_ass_full(""), (String::new(), None));
    }

    #[test]
    fn strip_ass_tags_drops_vector_drawing_commands() {
        // ASS drawing mode: `\p1` enters vector-drawing, the path
        // commands (m/l/b coordinates) are *not* text, `\p0` leaves it.
        // Only the trailing real text should survive.
        assert_eq!(
            strip_ass_tags(
                r"{\p1}m -6 -56 l -611 -56 l -600 -155 l 338 -156{\p0}Due to heavy snowfall"
            ),
            "Due to heavy snowfall"
        );
        // Combined tags in one block, and text before the shape.
        assert_eq!(
            strip_ass_tags(r"before{\an8\p1}m 0 0 l 100 0 100 100{\p0}after"),
            "beforeafter"
        );
        // `\pos(...)` is a position tag, NOT drawing mode — text kept.
        assert_eq!(strip_ass_tags(r"{\pos(960,540)}real text"), "real text");
        // A pure-shape event collapses to empty (so it is filtered out).
        assert_eq!(
            strip_ass_tags(r"{\p1}m -6 -56 l -611 -56 l -600 -155{\p0}"),
            ""
        );
    }

    #[test]
    fn parse_ass_full_filters_pure_shape_events() {
        // A sign rendered as two shape events (border + fill) followed by
        // the real text event: only the text survives, no path leakage.
        assert_eq!(
            parse_ass_full(concat!(
                r"Dialogue: 0,0,0,Sign,,0,0,0,,{\p1}m -6 -56 l -611 -56 l -600 -155{\p0}",
                "\n",
                r"Dialogue: 0,0,0,Sign,,0,0,0,,{\p1}m -6 -56 l 338 -156 l 349 -55{\p0}",
                "\n",
                r"Dialogue: 0,0,0,Default,,0,0,0,,Due to heavy snowfall, the trains will be delayed."
            )),
            (
                "Due to heavy snowfall, the trains will be delayed.".into(),
                None
            )
        );
    }
}
