//! Echo suppression filter for player events.
//!
//! When we send a command to mpv (e.g. pause), mpv fires back an event
//! confirming the state change. Without filtering, this echo would be
//! treated as a user-initiated action and re-broadcast to peers.
//!
//! The filter works by registering commands as they are sent and consuming
//! matching events within a time window.

use std::collections::VecDeque;

use tokio::time::{Duration, Instant};

use super::PlayerEvent;

/// Window within which an event is considered an echo of a sent command.
const ECHO_WINDOW: Duration = Duration::from_millis(500);

/// Tolerance for seek echo matching (mpv may seek to nearest keyframe).
const SEEK_TOLERANCE_SECS: f64 = 1.0;

/// A pending command registration that may produce an echo.
#[derive(Debug)]
enum PendingEcho {
    Pause { registered_at: Instant },
    Unpause { registered_at: Instant },
    Seek { target: f64, registered_at: Instant },
}

/// Filters out player events that are echoes of commands we sent.
#[derive(Debug, Default)]
pub struct EchoFilter {
    pending: VecDeque<PendingEcho>,
}

impl EchoFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register that we sent a pause command. Call before sending to the player.
    pub fn register_pause(&mut self) {
        self.pending.push_back(PendingEcho::Pause {
            registered_at: Instant::now(),
        });
    }

    /// Register that we sent an unpause command.
    pub fn register_unpause(&mut self) {
        self.pending.push_back(PendingEcho::Unpause {
            registered_at: Instant::now(),
        });
    }

    /// Register that we sent a seek command.
    pub fn register_seek(&mut self, target: f64) {
        self.pending.push_back(PendingEcho::Seek {
            target,
            registered_at: Instant::now(),
        });
    }

    /// Filter an incoming player event. Returns `None` if the event is an echo
    /// of a command we sent, `Some(event)` otherwise.
    pub fn filter(&mut self, event: PlayerEvent) -> Option<PlayerEvent> {
        // Prune expired entries first
        let now = Instant::now();
        self.pending
            .retain(|p| now.duration_since(registered_at(p)) < ECHO_WINDOW);

        // Try to find and consume a matching pending echo
        let match_idx = self.pending.iter().position(|p| matches_event(p, &event));

        if let Some(idx) = match_idx {
            self.pending.remove(idx);
            None
        } else {
            Some(event)
        }
    }
}

fn registered_at(p: &PendingEcho) -> Instant {
    match p {
        PendingEcho::Pause { registered_at } => *registered_at,
        PendingEcho::Unpause { registered_at } => *registered_at,
        PendingEcho::Seek { registered_at, .. } => *registered_at,
    }
}

fn matches_event(pending: &PendingEcho, event: &PlayerEvent) -> bool {
    match (pending, event) {
        (PendingEcho::Pause { .. }, PlayerEvent::Paused) => true,
        (PendingEcho::Unpause { .. }, PlayerEvent::Unpaused) => true,
        (PendingEcho::Seek { target, .. }, PlayerEvent::Seeked { position_secs }) => {
            (target - position_secs).abs() < SEEK_TOLERANCE_SECS
        }
        _ => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn suppresses_pause_echo() {
        let mut filter = EchoFilter::new();
        filter.register_pause();
        assert!(filter.filter(PlayerEvent::Paused).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn suppresses_unpause_echo() {
        let mut filter = EchoFilter::new();
        filter.register_unpause();
        assert!(filter.filter(PlayerEvent::Unpaused).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn suppresses_seek_echo_within_tolerance() {
        let mut filter = EchoFilter::new();
        filter.register_seek(120.0);
        // mpv seeks to nearest keyframe at 120.5
        assert!(filter
            .filter(PlayerEvent::Seeked {
                position_secs: 120.5
            })
            .is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn passes_seek_outside_tolerance() {
        let mut filter = EchoFilter::new();
        filter.register_seek(120.0);
        // User seeked to a very different position
        let event = PlayerEvent::Seeked {
            position_secs: 200.0,
        };
        assert!(filter.filter(event).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn passes_unregistered_events() {
        let mut filter = EchoFilter::new();
        // No commands registered — everything passes through
        assert!(filter.filter(PlayerEvent::Paused).is_some());
        assert!(filter.filter(PlayerEvent::Unpaused).is_some());
        assert!(filter
            .filter(PlayerEvent::Seeked {
                position_secs: 10.0
            })
            .is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn echo_expires_after_window() {
        let mut filter = EchoFilter::new();
        filter.register_pause();
        // Advance time past the echo window
        tokio::time::advance(Duration::from_millis(600)).await;
        // Now the pause echo has expired — event should pass through
        assert!(filter.filter(PlayerEvent::Paused).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn consumes_only_one_echo_per_registration() {
        let mut filter = EchoFilter::new();
        filter.register_pause();
        // First pause is consumed
        assert!(filter.filter(PlayerEvent::Paused).is_none());
        // Second pause is not — no more registrations
        assert!(filter.filter(PlayerEvent::Paused).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn multiple_seeks_consumed_independently() {
        let mut filter = EchoFilter::new();
        filter.register_seek(10.0);
        filter.register_seek(20.0);
        // First seek echo consumed
        assert!(filter
            .filter(PlayerEvent::Seeked {
                position_secs: 10.0
            })
            .is_none());
        // Second seek echo consumed
        assert!(filter
            .filter(PlayerEvent::Seeked {
                position_secs: 20.0
            })
            .is_none());
        // Third seek is not an echo
        assert!(filter
            .filter(PlayerEvent::Seeked {
                position_secs: 30.0
            })
            .is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn non_player_events_pass_through() {
        let mut filter = EchoFilter::new();
        filter.register_pause();
        // Position and Eof events are never echoes
        assert!(filter
            .filter(PlayerEvent::Position {
                position_secs: 5.0
            })
            .is_some());
        assert!(filter.filter(PlayerEvent::Eof).is_some());
        assert!(filter.filter(PlayerEvent::Crashed).is_some());
        // The pause registration is still pending
        assert!(filter.filter(PlayerEvent::Paused).is_none());
    }
}
