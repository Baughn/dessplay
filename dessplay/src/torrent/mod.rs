//! Torrent-first downloads (design.md, BitTorrent Downloads).
//!
//! A missing playlist file is fetched via BitTorrent whenever it can be
//! found on nyaa.si; the Phase-9B peer transfer remains the fallback for
//! rare files. This module holds the nyaa search ([`nyaa`]) and, as
//! later phases land, the fetch policy core and the engine seam.

pub mod nyaa;
