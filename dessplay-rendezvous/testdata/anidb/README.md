# Recorded AniDB exchanges

Real query→response pairs captured from the live UDP API by

```
cargo run -p dessplay-rendezvous --bin anidb-probe -- scan ~/Videos
```

(credentials via `DESSPLAY_ANIDB_USER` / `DESSPLAY_ANIDB_PASSWORD`;
the recorder redacts usernames, passwords, and session keys before
anything touches disk).

`tests/anidb_replay.rs` runs the real protocol codec over every `.txt`
file here, so the parser stays pinned to actual server output without
any test ever contacting the API. If the FILE/ANIME masks change, the
replay test will fail until a fresh scan is recorded — that's the
point: fixtures must match what we actually send.
