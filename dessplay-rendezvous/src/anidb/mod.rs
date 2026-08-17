//! AniDB UDP API integration (server-side only; see docs/design.md,
//! "Parsing files to series/season/episode").
//!
//! Layering, bottom up:
//! - [`protocol`]: pure request/response codec. No I/O, fully offline-
//!   testable.
//! - [`schedule`]: pure re-validation scheduling rules.
//! - [`client`]: the rate-limited UDP client (session management,
//!   retries, backoff) behind the [`client::AniDbApi`] trait so the
//!   worker can be tested against a mock.
//! - [`titles`]: the daily anime-titles dump (name search runs locally
//!   over it; the UDP API has no multi-result search).
//! - [`curator`]: the AI short-title curator — the dump's raw titles
//!   in, the community's display name out, cached forever in SQLite.
//! - [`worker`]: the drainer loop tying it all together — lookup
//!   requests in, metadata/relations LWW writes out.
//!
//! Nothing in here is allowed to touch the real API from a test:
//! AniDB bans aggressively and bans stick. The only real-API contact
//! is the `anidb-probe` binary, run manually.

pub mod client;
pub mod curator;
pub mod protocol;
pub mod record;
pub mod schedule;
pub mod titles;
pub mod worker;
