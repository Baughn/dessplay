//! BitTorrent support for the explicit Nyaa browse import (design.md,
//! BitTorrent Downloads).
//!
//! This is deliberately *not* a general torrent path: missing playlist
//! files are fetched from peers (see [`crate::download`]); torrents
//! exist only for the Playlist pane's `n` search, where the user picks
//! a release by hand. A selected torrent downloads in the background,
//! is ed2k-hashed on completion, and only then becomes a shared
//! playlist entry. It seeds for the rest of the session (typically past
//! 1:1 on a watch-party evening) and is discarded at shutdown —
//! dessplay is not primarily a torrent client, and resuming last
//! week's seeds on launch would be surprising.

pub mod engine;
pub mod nyaa;
pub mod rqbit;
