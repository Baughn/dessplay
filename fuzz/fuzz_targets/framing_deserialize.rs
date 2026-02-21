#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// Feed random bytes to the framing layer's deserialization functions.
/// Tests length prefix parsing, tag validation, and postcard deserialization
/// on top. Must not panic on any input.
#[derive(Arbitrary, Debug)]
struct FramingFuzzInput {
    stream_bytes: Vec<u8>,
    datagram_bytes: Vec<u8>,
    tag: u8,
}

fuzz_target!(|input: FramingFuzzInput| {
    // Test datagram decoding with various tags
    for tag in [1u8, 2, 3, 4, 5, 6, 7, 8, input.tag] {
        let _ = dessplay_core::framing::decode_datagram::<dessplay_core::protocol::PeerDatagram>(
            &input.datagram_bytes,
            tag,
        );
        let _ = dessplay_core::framing::decode_datagram::<dessplay_core::protocol::PeerControl>(
            &input.datagram_bytes,
            tag,
        );
        let _ = dessplay_core::framing::decode_datagram::<dessplay_core::protocol::RvControl>(
            &input.datagram_bytes,
            tag,
        );
        let _ = dessplay_core::framing::decode_datagram::<dessplay_core::protocol::RelayEnvelope>(
            &input.datagram_bytes,
            tag,
        );
    }

    // Test stream framing by feeding bytes to read_framed via a cursor.
    // We use a runtime since read_framed is async.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    if let Ok(rt) = rt {
        let bytes = input.stream_bytes.clone();
        let tag = input.tag;
        rt.block_on(async move {
            let mut cursor = &bytes[..];
            let _ = dessplay_core::framing::read_framed::<_, dessplay_core::protocol::RvControl>(
                &mut cursor,
                tag,
            )
            .await;
        });
    }
});
