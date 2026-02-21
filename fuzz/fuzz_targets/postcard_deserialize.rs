#![no_main]

use libfuzzer_sys::fuzz_target;

// Feed arbitrary bytes to postcard deserialization for all network-facing
// protocol types. Must not panic on any input.
fuzz_target!(|data: &[u8]| {
    let _ = postcard::from_bytes::<dessplay_core::protocol::CrdtOp>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::RvControl>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::PeerControl>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::PeerDatagram>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::RelayEnvelope>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::GapFillRequest>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::GapFillResponse>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::ChunkRequest>(data);
    let _ = postcard::from_bytes::<dessplay_core::protocol::ChunkData>(data);
});
