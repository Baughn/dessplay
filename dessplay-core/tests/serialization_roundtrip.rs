#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::net::{Ipv6Addr, SocketAddr};

use dessplay_core::protocol::*;
use dessplay_core::types::*;

fn fid(n: u8) -> FileId {
    let mut id = [0u8; 16];
    id[0] = n;
    FileId(id)
}

fn uid(name: &str) -> UserId {
    UserId(name.to_string())
}

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    val: &T,
) {
    let bytes = postcard::to_allocvec(val).unwrap();
    let decoded: T = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(*val, decoded);
}

// --- CrdtOp variants ---

#[test]
fn roundtrip_lww_write_user_state() {
    roundtrip(&CrdtOp::LwwWrite {
        timestamp: 12345,
        value: LwwValue::UserState(uid("alice"), UserState::Ready),
    });
}

#[test]
fn roundtrip_lww_write_file_state() {
    roundtrip(&CrdtOp::LwwWrite {
        timestamp: 100,
        value: LwwValue::FileState(
            uid("alice"),
            fid(1),
            FileState::Downloading { progress: 0.75 },
        ),
    });
}

#[test]
fn roundtrip_lww_write_anidb() {
    roundtrip(&CrdtOp::LwwWrite {
        timestamp: 200,
        value: LwwValue::AniDb(
            fid(2),
            Some(AniDbMetadata {
                anime_id: 12345,
                anime_name: "Frieren".into(),
                episode_number: 1,
                episode_name: "The Journey's End".into(),
                group_name: "SubsPlease".into(),
            }),
        ),
    });
}

#[test]
fn roundtrip_lww_write_anidb_none() {
    roundtrip(&CrdtOp::LwwWrite {
        timestamp: 300,
        value: LwwValue::AniDb(fid(3), None),
    });
}

#[test]
fn roundtrip_playlist_op() {
    roundtrip(&CrdtOp::PlaylistOp {
        timestamp: 42,
        action: PlaylistAction::Add {
            file_id: fid(1),
            after: Some(fid(2)),
        },
    });
    roundtrip(&CrdtOp::PlaylistOp {
        timestamp: 43,
        action: PlaylistAction::Remove { file_id: fid(3) },
    });
    roundtrip(&CrdtOp::PlaylistOp {
        timestamp: 44,
        action: PlaylistAction::Move {
            file_id: fid(1),
            after: None,
        },
    });
}

#[test]
fn roundtrip_chat_append() {
    roundtrip(&CrdtOp::ChatAppend {
        user_id: uid("bob"),
        seq: 7,
        timestamp: 999,
        text: "hello world".into(),
    });
}

// --- Version vectors ---

#[test]
fn roundtrip_version_vectors() {
    let mut vv = VersionVectors::new(3);
    vv.lww_versions
        .insert(RegisterId::UserState(uid("alice")), 100);
    vv.chat_versions.insert(uid("bob"), 5);
    vv.playlist_version = 42;
    roundtrip(&vv);
}

// --- Gap fill ---

#[test]
fn roundtrip_gap_fill() {
    roundtrip(&GapFillRequest {
        lww_needed: vec![(RegisterId::AniDb(fid(1)), 50)],
        chat_needed: vec![(uid("alice"), 3)],
        playlist_after: Some(10),
    });
    roundtrip(&GapFillResponse {
        ops: vec![CrdtOp::ChatAppend {
            user_id: uid("bob"),
            seq: 0,
            timestamp: 1,
            text: "hi".into(),
        }],
    });
}

// --- Snapshot ---

#[test]
fn roundtrip_crdt_snapshot() {
    let mut user_states = BTreeMap::new();
    user_states.insert(uid("alice"), (100u64, UserState::Paused));

    let snap = CrdtSnapshot {
        user_states,
        file_states: BTreeMap::new(),
        anidb: BTreeMap::new(),
        playlist_ops: vec![(1, PlaylistAction::Add { file_id: fid(1), after: None })],
        chat: BTreeMap::new(),
    };
    roundtrip(&snap);
}

// --- PeerInfo ---

#[test]
fn roundtrip_peer_info() {
    roundtrip(&PeerInfo {
        peer_id: PeerId(1),
        username: "alice".into(),
        addresses: vec![SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 8080)],
        connected_since: 12345,
    });
}

// --- RvControl variants ---

#[test]
fn roundtrip_rv_control() {
    roundtrip(&RvControl::Auth {
        password: "secret".into(),
        username: "alice".into(),
    });
    roundtrip(&RvControl::AuthOk {
        peer_id: PeerId(1),
        observed_addr: SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 1234),
    });
    roundtrip(&RvControl::AuthFailed);
    roundtrip(&RvControl::TimeSyncRequest { client_send: 100 });
    roundtrip(&RvControl::TimeSyncResponse {
        client_send: 100,
        server_recv: 101,
        server_send: 102,
    });
    roundtrip(&RvControl::PeerList {
        peers: vec![PeerInfo {
            peer_id: PeerId(2),
            username: "bob".into(),
            addresses: vec![],
            connected_since: 0,
        }],
    });
    roundtrip(&RvControl::StateOp {
        op: CrdtOp::ChatAppend {
            user_id: uid("x"),
            seq: 0,
            timestamp: 0,
            text: String::new(),
        },
    });
    roundtrip(&RvControl::StateSummary {
        versions: VersionVectors::new(0),
    });
}

// --- PeerControl variants ---

#[test]
fn roundtrip_peer_control() {
    roundtrip(&PeerControl::Hello {
        peer_id: PeerId(1),
        username: "alice".into(),
    });
    roundtrip(&PeerControl::StateOp {
        op: CrdtOp::PlaylistOp {
            timestamp: 1,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        },
    });
    roundtrip(&PeerControl::StateSummary {
        epoch: 5,
        versions: VersionVectors::new(5),
    });
    roundtrip(&PeerControl::FileAvailability {
        file_id: fid(1),
        bitfield: vec![0b10110000],
    });
}

// --- PeerDatagram variants ---

#[test]
fn roundtrip_peer_datagram() {
    roundtrip(&PeerDatagram::Position {
        timestamp: 100,
        position_secs: 42.5,
    });
    roundtrip(&PeerDatagram::Seek {
        timestamp: 200,
        target_secs: 90.0,
    });
    roundtrip(&PeerDatagram::StateOp {
        op: CrdtOp::ChatAppend {
            user_id: uid("alice"),
            seq: 0,
            timestamp: 0,
            text: "hi".into(),
        },
    });
}

// --- RelayEnvelope ---

#[test]
fn roundtrip_relay_envelope() {
    roundtrip(&RelayEnvelope::Forward {
        to: PeerId(42),
        message: vec![1, 2, 3],
    });
    roundtrip(&RelayEnvelope::Forwarded {
        from: PeerId(7),
        message: vec![4, 5, 6],
    });
}

// --- File transfer ---

#[test]
fn roundtrip_chunk_request() {
    roundtrip(&ChunkRequest {
        file_id: fid(1),
        chunks: vec![0, 1, 5, 10],
    });
}

#[test]
fn roundtrip_chunk_data() {
    roundtrip(&ChunkData {
        index: 42,
        data: vec![0xAB; 256],
    });
}

/// Regression test: bitvec serde panic on corrupted input.
/// Crash artifact from fuzz/artifacts/postcard_deserialize/crash-7ca0899...
#[test]
fn deserialize_corrupted_peer_control_no_panic() {
    let bad_bytes: &[u8] = &[
        0x03, 0x00, 0x05, 0x13, 0x13, 0x13, 0x3f, 0x13, 0x33, 0x00, 0x13, 0x1b,
        0xf1, 0x4c, 0x13, 0x13, 0x13, 0x13, 0x62, 0x69, 0x74, 0x76, 0x65, 0x63,
        0x3a, 0x3a, 0x6f, 0x72, 0x64, 0x65, 0x72, 0x3a, 0x3a, 0x4c, 0x73, 0x62,
        0x30, 0x40, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x63,
        0x3a, 0x3a, 0x6f, 0x72, 0x33, 0x00, 0x64, 0x65, 0x72, 0x3a, 0x3a,
    ];
    // Must not panic — should return Err.
    let _ = postcard::from_bytes::<PeerControl>(bad_bytes);
}

// --- Core types ---

#[test]
fn roundtrip_user_state() {
    roundtrip(&UserState::Ready);
    roundtrip(&UserState::Paused);
    roundtrip(&UserState::NotWatching);
}

#[test]
fn roundtrip_file_state() {
    roundtrip(&FileState::Ready);
    roundtrip(&FileState::Missing);
    roundtrip(&FileState::Downloading { progress: 0.5 });
}

#[test]
fn roundtrip_anidb_metadata() {
    roundtrip(&AniDbMetadata {
        anime_id: 12345,
        anime_name: "Frieren".into(),
        episode_number: 1,
        episode_name: "The Journey's End".into(),
        group_name: "SubsPlease".into(),
    });
    let none: Option<AniDbMetadata> = None;
    roundtrip(&none);
}
