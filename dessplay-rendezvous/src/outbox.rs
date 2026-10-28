//! Per-connection outbound queues: nothing the server sends to one peer
//! ever waits on that peer.
//!
//! Every reliable frame bound for a connection — its control stream, or
//! the relay stream on its transfer connection — goes through that
//! connection's [`Outbox`]: a queue drained by the connection's own
//! writer task. [`Outbox::push`] never waits, so a broadcast loop, a
//! serve loop, or a relay reader can never be held hostage by a peer
//! that stopped reading (2026-10-28: one did, and the server's awaited
//! per-peer writes froze every broadcast, login, and time-sync answer
//! until a restart; docs/decisions.md has the post-mortem).
//!
//! The queue is bounded in bytes. A peer whose backlog passes the limit
//! is written off as not reading: the outbox closes, and the overflow
//! action closes the connection, after which the peer (if it is a real
//! client) reconnects and resyncs from a snapshot. A frame always goes
//! into an *empty* queue, however large, so a full-state frame bigger
//! than the limit cannot get an honest reader disconnected.
//!
//! [`PeerConn`] wraps an authenticated control connection so the only
//! reliable send it offers is the queued one.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use dessplay_core::net::framing::write_frame;
use dessplay_core::net::{Transport, TransportError, TransportEvent};
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;

/// One encoded frame, shared (not copied) by every recipient of a
/// broadcast — a full-state frame is ~9 MiB.
pub(crate) type Frame = Arc<[u8]>;

/// Where an outbox's writer task puts frames.
pub(crate) trait FrameSink: Send + 'static {
    /// Write one frame, waiting for the destination's flow control as
    /// long as it takes. `false` once the destination is gone.
    fn write(&mut self, frame: &[u8]) -> impl Future<Output = bool> + Send;
}

/// A control stream, written through the transport.
struct ControlSink<T>(Arc<T>);

impl<T: Transport> FrameSink for ControlSink<T> {
    async fn write(&mut self, frame: &[u8]) -> bool {
        self.0.send_control(frame).await.is_ok()
    }
}

/// A raw stream's write half (the relay stream), length-prefixed.
impl FrameSink for Box<dyn AsyncWrite + Send + Unpin> {
    async fn write(&mut self, frame: &[u8]) -> bool {
        write_frame(self, frame).await.is_ok()
    }
}

/// What the writer task and every queue handle share.
struct State {
    /// Bytes pushed and not yet written (the frame being written
    /// included).
    queued: AtomicUsize,
    limit: usize,
    /// Overflowed, or the writer found the destination gone: further
    /// pushes are refused.
    closed: AtomicBool,
    /// Runs once, on overflow (closes the connection).
    on_overflow: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// For logs: whose queue this is.
    label: String,
}

/// A connection's outbound queue; cheap to clone. The writer task ends
/// when the destination fails or every handle is dropped.
#[derive(Clone)]
pub(crate) struct Outbox {
    tx: mpsc::UnboundedSender<Frame>,
    state: Arc<State>,
}

impl Outbox {
    /// Start a queue draining into `sink`. `on_overflow` runs (once) if
    /// the backlog passes `limit` bytes; it should close the connection,
    /// which also fails a write parked on it.
    pub(crate) fn spawn(
        label: String,
        limit: usize,
        sink: impl FrameSink,
        on_overflow: impl FnOnce() + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let state = Arc::new(State {
            queued: AtomicUsize::new(0),
            limit,
            closed: AtomicBool::new(false),
            on_overflow: Mutex::new(Some(Box::new(on_overflow))),
            label,
        });
        tokio::spawn(drain(rx, sink, Arc::clone(&state)));
        Self { tx, state }
    }

    /// Queue `frame`. Never waits. `false` if the outbox is closed —
    /// this push overflowed it, or it already was.
    pub(crate) fn push(&self, frame: Frame) -> bool {
        if self.state.closed.load(Ordering::SeqCst) {
            return false;
        }
        let len = frame.len();
        let before = self.state.queued.fetch_add(len, Ordering::SeqCst);
        if before > 0 && before + len > self.state.limit {
            self.state.queued.fetch_sub(len, Ordering::SeqCst);
            self.overflow(before);
            return false;
        }
        if self.tx.send(frame).is_err() {
            // The writer is gone (destination failed).
            self.state.queued.fetch_sub(len, Ordering::SeqCst);
            return false;
        }
        true
    }

    fn overflow(&self, queued: usize) {
        if self.state.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        tracing::warn!(
            peer = %self.state.label,
            queued,
            limit = self.state.limit,
            "peer is not reading; dropping its connection"
        );
        let action = self
            .state
            .on_overflow
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(action) = action {
            action();
        }
    }
}

/// The writer task: one frame at a time, in order, for as long as the
/// destination takes.
async fn drain(
    mut rx: mpsc::UnboundedReceiver<Frame>,
    mut sink: impl FrameSink,
    state: Arc<State>,
) {
    while let Some(frame) = rx.recv().await {
        if state.closed.load(Ordering::SeqCst) {
            break;
        }
        let ok = sink.write(&frame).await;
        state.queued.fetch_sub(frame.len(), Ordering::SeqCst);
        if !ok {
            state.closed.store(true, Ordering::SeqCst);
            break;
        }
    }
    tracing::trace!(peer = %state.label, "outbox writer exiting");
}

/// An authenticated control connection. Reliable sends are queued
/// ([`Self::send`]); there is deliberately no awaited one.
pub(crate) struct PeerConn<T> {
    conn: Arc<T>,
    outbox: Outbox,
}

impl<T: Transport> PeerConn<T> {
    /// Wrap `conn`, starting its writer. Past `limit` queued bytes the
    /// connection is closed as not reading.
    pub(crate) fn new(conn: Arc<T>, label: String, limit: usize) -> Self {
        let on_overflow = {
            let conn = Arc::clone(&conn);
            move || {
                tokio::spawn(async move {
                    conn.close("not reading its control stream").await;
                });
            }
        };
        let outbox = Outbox::spawn(label, limit, ControlSink(Arc::clone(&conn)), on_overflow);
        Self { conn, outbox }
    }

    /// Queue a control-stream frame. `false` if the connection's queue
    /// is closed (overflowed, or the connection died).
    pub(crate) fn send(&self, frame: Frame) -> bool {
        self.outbox.push(frame)
    }

    /// Send a datagram directly. Datagrams skip the queue: they are
    /// unordered and best-effort, and neither transport waits on the
    /// peer to send one (quinn's `send_datagram` discards its oldest
    /// unsent datagram rather than block).
    pub(crate) async fn send_datagram(&self, frame: &[u8]) -> Result<(), TransportError> {
        self.conn.send_datagram(frame).await
    }

    /// See [`Transport::max_datagram_size`].
    pub(crate) fn max_datagram_size(&self) -> Option<usize> {
        self.conn.max_datagram_size()
    }

    /// See [`Transport::recv`]. Only the connection's serve loop reads.
    pub(crate) async fn recv(&self) -> Result<TransportEvent, TransportError> {
        self.conn.recv().await
    }

    /// Close the connection; anything still queued is discarded.
    pub(crate) async fn close(&self, reason: &str) {
        self.conn.close(reason).await;
    }
}
