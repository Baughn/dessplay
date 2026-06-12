//! Replay recorded real-API exchanges through the protocol codec.
//!
//! `anidb-probe scan` (run manually, see testdata/anidb/README.md)
//! records sanitized query→response pairs from the live server; this
//! test parses every recorded response with the real codec, so the
//! parser is pinned to actual AniDB output without any test touching
//! the API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use dessplay_rendezvous::anidb::protocol::{
    self, ANIME, FILE, FILE_AMASK, FILE_FMASK, LOGGED_OUT, LOGIN_ACCEPTED,
    LOGIN_ACCEPTED_NEW_VERSION, MULTIPLE_FILES_FOUND, NO_SUCH_ANIME, NO_SUCH_FILE, NOT_LOGGED_IN,
    PONG,
};
use dessplay_rendezvous::anidb::record::parse_exchanges;

fn param<'a>(request: &'a str, key: &str) -> Option<&'a str> {
    request
        .split_once(' ')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

#[test]
fn recorded_exchanges_replay_through_the_codec() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/anidb");
    let mut recordings: Vec<_> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "txt"))
                .collect()
        })
        .unwrap_or_default();
    recordings.sort();
    if recordings.is_empty() {
        // Nothing recorded yet — run `anidb-probe scan` (manually!) to
        // create fixtures. Not a failure: fresh checkouts and CI must
        // pass before anyone has touched the real API.
        eprintln!("no recordings in {}; skipping", dir.display());
        return;
    }

    let (mut file_hits, mut file_misses, mut anime_hits) = (0u32, 0u32, 0u32);
    let mut exchanges = 0u32;
    for recording in recordings {
        let text = std::fs::read_to_string(&recording).unwrap();
        let pairs = parse_exchanges(&text);
        assert!(!pairs.is_empty(), "{}: no exchanges", recording.display());
        for (request, response) in pairs {
            exchanges += 1;
            let parsed = protocol::parse_response(&response)
                .unwrap_or_else(|e| panic!("unparsable response for {request:?}: {e}"));
            let command = request.split(' ').next().unwrap_or("");
            match command {
                "AUTH" => {
                    if matches!(parsed.code, LOGIN_ACCEPTED | LOGIN_ACCEPTED_NEW_VERSION) {
                        assert!(
                            protocol::session_key(&parsed.text).is_some(),
                            "no session key in {:?}",
                            parsed.text
                        );
                    }
                }
                "FILE" => {
                    // Fixtures must match the masks we actually send.
                    assert_eq!(param(&request, "fmask"), Some(FILE_FMASK), "{request}");
                    assert_eq!(param(&request, "amask"), Some(FILE_AMASK), "{request}");
                    match parsed.code {
                        FILE => {
                            let file = protocol::parse_file_data(&parsed).unwrap_or_else(|e| {
                                panic!("unparsable FILE data for {request:?}: {e}")
                            });
                            assert!(!file.series_name().is_empty(), "{request}");
                            file_hits += 1;
                        }
                        NO_SUCH_FILE | MULTIPLE_FILES_FOUND => file_misses += 1,
                        other => panic!("unexpected FILE reply code {other} for {request:?}"),
                    }
                }
                "ANIME" => {
                    assert_eq!(
                        param(&request, "amask"),
                        Some(protocol::ANIME_AMASK),
                        "{request}"
                    );
                    match parsed.code {
                        ANIME => {
                            let anime = protocol::parse_anime_data(&parsed).unwrap_or_else(|e| {
                                panic!("unparsable ANIME data for {request:?}: {e}")
                            });
                            assert!(!anime.title().is_empty(), "{request}");
                            anime_hits += 1;
                        }
                        NO_SUCH_ANIME => {}
                        other => panic!("unexpected ANIME reply code {other} for {request:?}"),
                    }
                }
                "LOGOUT" => {
                    assert!(matches!(parsed.code, LOGGED_OUT | NOT_LOGGED_IN));
                }
                "PING" => assert_eq!(parsed.code, PONG),
                other => panic!("unknown recorded command {other:?}"),
            }
        }
    }
    println!(
        "replayed {exchanges} exchanges: {file_hits} file hits, {file_misses} misses, {anime_hits} anime"
    );
    // A useful recording exercises both sides of the FILE lookup.
    assert!(file_hits > 0, "recordings contain no FILE hits");
    assert!(file_misses > 0, "recordings contain no FILE misses");
    assert!(anime_hits > 0, "recordings contain no ANIME data");
}
