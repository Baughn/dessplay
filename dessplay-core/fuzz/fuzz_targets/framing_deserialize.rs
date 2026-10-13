//! Raw bytes through the framing layer and wire-message decoder must
//! never panic or over-allocate.

#![no_main]

use std::sync::OnceLock;

use dessplay_core::net::WireMessage;
use dessplay_core::net::framing::{HANDSHAKE_FRAME_LEN, read_frame, read_frame_capped};
use libfuzzer_sys::fuzz_target;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    })
}

fuzz_target!(|data: &[u8]| {
    // Datagram path: bytes straight into the message decoder.
    let _ = dessplay_core::wire::decode::<WireMessage>(data);

    // Stream path: bytes through the framing reader, each recovered
    // frame into the decoder.
    runtime().block_on(async {
        let mut cursor = data;
        while let Ok(frame) = read_frame(&mut cursor).await {
            let _ = dessplay_core::wire::decode::<WireMessage>(&frame);
        }
    });

    // Handshake path: the one pre-admission frame an accepted
    // connection reads, under its smaller cap.
    runtime().block_on(async {
        let mut cursor = data;
        if let Ok(frame) = read_frame_capped(&mut cursor, HANDSHAKE_FRAME_LEN).await {
            assert!(frame.len() <= HANDSHAKE_FRAME_LEN);
            let _ = dessplay_core::wire::decode::<WireMessage>(&frame);
        }
    });
});
