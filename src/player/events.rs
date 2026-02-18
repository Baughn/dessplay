#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// Continuous position update during normal playback (for sync tracking).
    /// Suppressed during pending programmatic seeks (stale data).
    PositionChanged(f64),

    /// User initiated a seek via the player UI (keyboard/mouse).
    /// NOT emitted for programmatic seeks via bridge.seek().
    UserSeeked { position: f64 },

    /// User toggled pause via the player UI (e.g. pressed space).
    /// NOT emitted for programmatic bridge.pause()/play().
    UserPauseToggled { paused: bool },

    /// File duration became known (reported once after file load).
    DurationChanged(f64),

    /// Current file reached end-of-file.
    EndOfFile,

    /// Player process exited.
    Exited { clean: bool },
}
