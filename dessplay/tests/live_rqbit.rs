#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use dessplay::torrent::engine::{TorrentEngine, TorrentImportId};

#[tokio::test]
#[ignore = "live network (nyaa.si); run manually"]
async fn live_add_by_nyaa_url() {
    let dir = tempfile::tempdir().unwrap();
    let engine = dessplay::torrent::rqbit::RqbitEngine::new_for_tests(dir.path().join("torrents"))
        .await
        .unwrap();
    let id = TorrentImportId(1);
    let chosen = dessplay::torrent::nyaa::NyaaMatch {
        title: "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv".into(),
        torrent_url: "https://nyaa.si/download/2129710.torrent".into(),
        info_hash: "123051cef95247353e061c58ee1cb713691f72b4".into(),
    };
    engine.add_import(id, &chosen, dir.path().join("out"));
    // The .torrent fetch is one HTTP GET; metadata is then immediate.
    let mut payload = None;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(s) = engine.import_status(id) {
            assert!(!s.error, "add failed");
            if s.payload.is_some() {
                payload = s.payload;
                break;
            }
        }
    }
    engine.remove_import(id, true);
    let payload = payload.expect("payload path never materialized");
    eprintln!("payload: {}", payload.display());
    assert!(payload.to_string_lossy().ends_with(".mkv"));
}
