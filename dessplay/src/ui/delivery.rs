//! UI delivery policy, shared by the session, terminal input, and workers.
//!
//! Reliable events never wait for the renderer and never disappear under load.
//! State/progress coalesce within the trailing run of replaceable updates;
//! reliable events are ordering barriers. Shutdown cancels the entire backlog.
//! Only transport bookkeeping is shared: application state stays on its owner.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use super::shell::UiInput;

#[derive(PartialEq, Eq)]
enum UpdateKey<'a> {
    Snapshot,
    Hash(&'a str),
    Search(u64),
    Import(crate::torrent::engine::TorrentImportId),
}

enum Delivery<'a> {
    Reliable,
    Latest(UpdateKey<'a>),
    Shutdown,
}

impl UiInput {
    // Exhaustive on purpose: every new input must choose its delivery policy.
    fn delivery(&self) -> Delivery<'_> {
        match self {
            Self::Snapshot(_) => Delivery::Latest(UpdateKey::Snapshot),
            Self::Hashing {
                filename,
                finished: false,
                ..
            } => Delivery::Latest(UpdateKey::Hash(filename)),
            Self::NyaaSearchProgress { request_id, .. } => {
                Delivery::Latest(UpdateKey::Search(*request_id))
            }
            Self::NyaaImportProgress { id, .. } => Delivery::Latest(UpdateKey::Import(*id)),
            Self::Roguelike(_)
            | Self::Event(_)
            | Self::Subtitle { .. }
            | Self::Hashing { finished: true, .. }
            | Self::System { .. }
            | Self::ChatImage { .. }
            | Self::Irc { .. }
            | Self::Browse { .. }
            | Self::LocalCopyOffer { .. }
            | Self::SearchResults { .. }
            | Self::NyaaResults { .. }
            | Self::NyaaImportFinished { .. }
            | Self::Probe(_) => Delivery::Reliable,
            Self::Shutdown => Delivery::Shutdown,
        }
    }
}

#[derive(Default)]
struct Pending {
    inputs: VecDeque<UiInput>,
    closed: bool,
}

/// Cloneable input endpoint. Sending never waits for queue capacity.
/// The reliable backlog is intentionally uncapped while the UI is alive.
#[derive(Clone)]
pub struct UiSender {
    pending: Arc<Mutex<Pending>>,
    wake: mpsc::SyncSender<()>,
}

/// Sole consumer of UI inputs, used by the production loop and its tests.
pub struct UiReceiver {
    pending: Arc<Mutex<Pending>>,
    wakes: mpsc::Receiver<()>,
}

/// The UI has shut down; further inputs are cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiClosed;

impl std::fmt::Display for UiClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UI mailbox closed")
    }
}

impl std::error::Error for UiClosed {}

/// Create the UI mailbox. The bounded channel carries wakeups only, never
/// application messages; one pending wakeup is sufficient for any backlog.
pub fn channel() -> (UiSender, UiReceiver) {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let (wake, wakes) = mpsc::sync_channel(1);
    (
        UiSender {
            pending: pending.clone(),
            wake,
        },
        UiReceiver { pending, wakes },
    )
}

fn lock(pending: &Mutex<Pending>) -> MutexGuard<'_, Pending> {
    pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl UiSender {
    /// Accept an input, or cancel it if the UI has closed. There is no Full
    /// error: producers cannot accidentally discard a required completion.
    pub fn send(&self, input: UiInput) -> Result<(), UiClosed> {
        let mut pending = lock(&self.pending);
        if pending.closed {
            return Err(UiClosed);
        }
        // Release replaced payloads outside the lock (snapshots/images can
        // own large allocations); the critical section only manages the queue.
        let mut cancelled = VecDeque::new();
        let replaced = match input.delivery() {
            Delivery::Shutdown => {
                pending.closed = true;
                cancelled = std::mem::take(&mut pending.inputs);
                None
            }
            Delivery::Latest(key) => {
                let index = pending
                    .inputs
                    .iter()
                    .enumerate()
                    .rev()
                    .take_while(|(_, input)| matches!(input.delivery(), Delivery::Latest(_)))
                    .find_map(|(index, input)| match input.delivery() {
                        Delivery::Latest(other) if other == key => Some(index),
                        _ => None,
                    });
                index.and_then(|index| pending.inputs.remove(index))
            }
            Delivery::Reliable => None,
        };
        pending.inputs.push_back(input);
        drop(pending);
        // A full wake channel already guarantees the consumer will check the
        // queue. Receiver teardown racing acceptance cancels pending work.
        let _ = self.wake.try_send(());
        drop(replaced);
        drop(cancelled);
        Ok(())
    }

    /// Whether shutdown was requested or the UI receiver has gone away.
    pub fn is_closed(&self) -> bool {
        lock(&self.pending).closed
    }
}

impl UiReceiver {
    /// Take the oldest event/update without waiting.
    pub fn try_recv(&self) -> Result<UiInput, mpsc::TryRecvError> {
        let mut pending = lock(&self.pending);
        if let Some(input) = pending.inputs.pop_front() {
            return Ok(input);
        }
        if pending.closed {
            return Err(mpsc::TryRecvError::Disconnected);
        }
        // Check while holding the queue lock: a concurrent final sender
        // cannot enqueue between the empty check and channel disconnection.
        loop {
            match self.wakes.try_recv() {
                Ok(()) => continue, // stale wake for an already consumed input
                Err(error) => return Err(error),
            }
        }
    }

    /// Wait for input, channel closure, or the UI's next animation deadline.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<UiInput, mpsc::RecvTimeoutError> {
        let started = Instant::now();
        loop {
            match self.try_recv() {
                Ok(input) => return Ok(input),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(mpsc::RecvTimeoutError::Disconnected);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                return Err(mpsc::RecvTimeoutError::Timeout);
            };
            // Always recheck the queue after a wake/closure: a final sender
            // may have enqueued and dropped after the empty check above.
            if self.wakes.recv_timeout(remaining) == Err(mpsc::RecvTimeoutError::Timeout) {
                return Err(mpsc::RecvTimeoutError::Timeout);
            }
        }
    }

    /// Drain available inputs without waiting (primarily harness support).
    pub fn try_iter(&self) -> impl Iterator<Item = UiInput> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }
}

impl Drop for UiReceiver {
    fn drop(&mut self) {
        let mut pending = lock(&self.pending);
        pending.closed = true;
        let cancelled = std::mem::take(&mut pending.inputs);
        drop(pending);
        drop(cancelled);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::actors::file::NyaaImportStage;
    use crate::torrent::{engine::TorrentImportId, nyaa::NyaaSearchProgress};
    use crate::ui::{app::UiSnapshot, msg::BrowseRequest};
    use dessplay_core::types::Ed2kHash;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn notice(timestamp: u64) -> UiInput {
        UiInput::System {
            timestamp,
            text: timestamp.to_string(),
        }
    }

    // Each numeric key is a separate visible state field. Different progress
    // families deliberately reuse the same job number to check isolation.
    fn update(key: u8, value: u64) -> UiInput {
        match key {
            0 => UiInput::Snapshot(Box::new(UiSnapshot {
                now: value,
                ..Default::default()
            })),
            1 | 2 => UiInput::Hashing {
                filename: key.to_string(),
                done_bytes: value,
                total_bytes: 10_000,
                finished: false,
            },
            3 | 4 => UiInput::NyaaSearchProgress {
                request_id: u64::from(key - 2),
                progress: NyaaSearchProgress::Inspecting {
                    done: value as usize,
                    total: 10_000,
                },
            },
            5 | 6 => UiInput::NyaaImportProgress {
                id: TorrentImportId(u64::from(key - 4)),
                filename: key.to_string(),
                stage: NyaaImportStage::Downloading,
                done_bytes: value,
                total_bytes: 10_000,
            },
            _ => unreachable!(),
        }
    }

    fn field(input: UiInput) -> (u8, u64) {
        match input {
            UiInput::Snapshot(snapshot) => (0, snapshot.now),
            UiInput::Hashing {
                filename,
                done_bytes,
                finished: false,
                ..
            } => (filename.parse().unwrap(), done_bytes),
            UiInput::NyaaSearchProgress {
                request_id,
                progress: NyaaSearchProgress::Inspecting { done, .. },
            } => (request_id as u8 + 2, done as u64),
            UiInput::NyaaImportProgress { id, done_bytes, .. } => (id.0 as u8 + 4, done_bytes),
            _ => panic!("expected replaceable state"),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        // Observing a reliable event must see the same state as applying the
        // entire original input trace. A partially drained mailbox is included:
        // consumer timing must not change event order or the state at barriers.
        #[test]
        fn coalescing_preserves_state_at_every_event(
            segments in prop::collection::vec(prop::collection::vec((0u8..7, any::<bool>()), 0..80), 1..20),
        ) {
            let (sender, receiver) = channel();
            let mut expected = BTreeMap::new();
            let mut observed = BTreeMap::new();
            let mut value = 0;
            for (marker, segment) in segments.into_iter().enumerate() {
                let mut touched = std::collections::BTreeSet::new();
                for (key, drain_one) in segment {
                    value += 1;
                    expected.insert(key, value);
                    touched.insert(key);
                    sender.send(update(key, value)).unwrap();
                    if drain_one && let Ok(input) = receiver.try_recv() {
                        let (key, value) = field(input);
                        observed.insert(key, value);
                    }
                }
                sender.send(notice(marker as u64)).unwrap();
                let mut updates = 0;
                loop {
                    match receiver.try_recv().unwrap() {
                        UiInput::System { timestamp, .. } => {
                            prop_assert_eq!(timestamp, marker as u64);
                            prop_assert_eq!(&observed, &expected);
                            break;
                        }
                        input => {
                            let (key, value) = field(input);
                            observed.insert(key, value);
                            updates += 1;
                        }
                    }
                }
                // Arbitrarily many unconsumed ticks occupy at most one slot
                // per state field/job, independently of renderer speed.
                prop_assert!(updates <= touched.len());
            }
            prop_assert!(matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }
    }

    #[test]
    fn state_cannot_move_across_an_event_or_completion() {
        let (sender, receiver) = channel();
        sender.send(update(0, 1)).unwrap();
        sender.send(notice(1)).unwrap();
        sender.send(update(0, 2)).unwrap();
        sender.send(update(0, 3)).unwrap();
        sender.send(update(1, 4)).unwrap();
        sender
            .send(UiInput::Hashing {
                filename: "1".into(),
                done_bytes: 0,
                total_bytes: 0,
                finished: true,
            })
            .unwrap();
        // A new job with the same display key must follow the old completion.
        sender.send(update(1, 5)).unwrap();
        assert_eq!(field(receiver.try_recv().unwrap()), (0, 1));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            UiInput::System { timestamp: 1, .. }
        ));
        assert_eq!(field(receiver.try_recv().unwrap()), (0, 3));
        assert_eq!(field(receiver.try_recv().unwrap()), (1, 4));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            UiInput::Hashing { finished: true, .. }
        ));
        assert_eq!(field(receiver.try_recv().unwrap()), (1, 5));
    }

    #[test]
    fn all_one_shot_inputs_survive_a_stalled_consumer_in_order() {
        let (sender, receiver) = channel();
        for index in 0..128 {
            sender.send(notice(index)).unwrap();
        }
        let inputs = [
            UiInput::Roguelike(Err("saved turn failed".into())),
            UiInput::Event(tuirealm::event::Event::WindowResize(80, 24)),
            UiInput::Subtitle {
                text: "line".into(),
                speaker: None,
                video_millis: 1,
                arrival_millis: 2,
            },
            UiInput::Hashing {
                filename: "episode".into(),
                done_bytes: 0,
                total_bytes: 0,
                finished: true,
            },
            notice(999),
            UiInput::ChatImage {
                url: "image".into(),
                result: Err("failed".into()),
            },
            UiInput::Irc {
                timestamp: 2,
                sender: "kim".into(),
                text: "line".into(),
                action: false,
            },
            UiInput::Browse {
                request: BrowseRequest::Add { after: None },
                files: vec![],
                watched: Default::default(),
                start: None,
            },
            UiInput::LocalCopyOffer {
                file: Ed2kHash([1; 16]),
                filename: "episode".into(),
                candidates: vec![],
            },
            UiInput::SearchResults {
                query: "series".into(),
                results: vec![],
            },
            UiInput::NyaaResults {
                request_id: 1,
                query: "series".into(),
                result: Err("failed".into()),
            },
            UiInput::NyaaImportFinished {
                id: TorrentImportId(1),
            },
            UiInput::Probe(Arc::new(Mutex::new(None))),
        ];
        let expected: Vec<_> = inputs.iter().map(std::mem::discriminant).collect();
        for input in inputs {
            sender.send(input).unwrap();
        }
        for index in 0..128 {
            assert!(
                matches!(receiver.try_recv().unwrap(), UiInput::System { timestamp, .. } if timestamp == index)
            );
        }
        let received: Vec<_> = receiver
            .try_iter()
            .map(|input| std::mem::discriminant(&input))
            .collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn shutdown_bypasses_backlog_and_closes_every_sender() {
        let (sender, receiver) = channel();
        let worker = sender.clone();
        for index in 0..128 {
            sender.send(notice(index)).unwrap();
        }
        worker.send(UiInput::Shutdown).unwrap();
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Ok(UiInput::Shutdown)
        ));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert!(sender.is_closed());
        assert!(worker.send(notice(200)).is_err());
        assert!(sender.send(update(0, 2)).is_err());
    }

    #[test]
    fn receiver_drop_releases_payloads_and_rejects_all_senders() {
        let (sender, receiver) = channel();
        let worker = sender.clone();
        let probe = Arc::new(Mutex::new(None));
        sender.send(UiInput::Probe(probe.clone())).unwrap();
        drop(receiver);
        assert_eq!(Arc::strong_count(&probe), 1);
        assert!(sender.is_closed());
        assert!(worker.send(notice(1)).is_err());
    }

    #[test]
    fn last_sender_close_drains_messages_then_disconnects() {
        let (sender, receiver) = channel();
        let worker = sender.clone();
        drop(sender);
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        worker.send(notice(1)).unwrap();
        drop(worker);
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Ok(UiInput::System { timestamp: 1, .. })
        ));
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn concurrent_workers_wake_receiver_and_preserve_each_senders_order() {
        let (sender, receiver) = channel();
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let sender = sender.clone();
                std::thread::spawn(move || {
                    for index in 0..128 {
                        sender.send(notice(worker * 128 + index)).unwrap();
                    }
                })
            })
            .collect();
        drop(sender);
        let mut counts = [0; 4];
        loop {
            match receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(UiInput::System { timestamp, .. }) => {
                    let worker = timestamp as usize / 128;
                    assert_eq!(timestamp as usize % 128, counts[worker]);
                    counts[worker] += 1;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => panic!("worker delivery stalled"),
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(counts, [128; 4]);
    }
}
