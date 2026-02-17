use serde::{Deserialize, Serialize};

/// Current wire protocol version.
pub const WIRE_VERSION: u8 = 2;

/// Top-level message envelope for all datagram traffic.
///
/// Clock sync messages are handled by the `ClockSyncService`;
/// application messages are passed through to upper layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    ClockSync(ClockSyncMessage),
    /// Opaque payload for upper layers (sync engine, etc.)
    Application(Vec<u8>),
}

/// Clock synchronization protocol messages (NTP-like).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClockSyncMessage {
    Ping { t1_us: i64 },
    Pong { t1_us: i64, t2_us: i64, t3_us: i64 },
}

/// Errors from wire format decoding.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("empty message")]
    Empty,
    #[error("unknown wire version: {0}")]
    UnknownVersion(u8),
    #[error("deserialization failed: {0}")]
    Deserialize(postcard::Error),
}

/// Encode a `WireMessage` into bytes: `[version_byte][postcard_payload]`.
pub fn encode(msg: &WireMessage) -> Vec<u8> {
    let payload = postcard::to_allocvec(msg).expect("WireMessage serialization should not fail");
    let mut buf = Vec::with_capacity(1 + payload.len());
    buf.push(WIRE_VERSION);
    buf.extend_from_slice(&payload);
    buf
}

/// Decode bytes into a `WireMessage`. Returns `Err` for empty input,
/// unknown versions, or deserialization failures.
pub fn decode(data: &[u8]) -> Result<WireMessage, WireError> {
    if data.is_empty() {
        return Err(WireError::Empty);
    }
    let version = data[0];
    if version != WIRE_VERSION {
        return Err(WireError::UnknownVersion(version));
    }
    postcard::from_bytes(&data[1..]).map_err(WireError::Deserialize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_clock_sync() {
        let msg = WireMessage::ClockSync(ClockSyncMessage::Ping { t1_us: 12345 });
        let encoded = encode(&msg);
        assert_eq!(encoded[0], WIRE_VERSION);
        let decoded = decode(&encoded).unwrap();
        match decoded {
            WireMessage::ClockSync(ClockSyncMessage::Ping { t1_us }) => {
                assert_eq!(t1_us, 12345);
            }
            _ => panic!("expected ClockSync Ping"),
        }
    }

    #[test]
    fn round_trip_application() {
        let payload = vec![1, 2, 3, 4, 5];
        let msg = WireMessage::Application(payload.clone());
        let encoded = encode(&msg);
        let decoded = decode(&encoded).unwrap();
        match decoded {
            WireMessage::Application(data) => assert_eq!(data, payload),
            _ => panic!("expected Application"),
        }
    }

    #[test]
    fn decode_empty() {
        assert!(matches!(decode(&[]), Err(WireError::Empty)));
    }

    #[test]
    fn decode_unknown_version() {
        assert!(matches!(decode(&[255, 0]), Err(WireError::UnknownVersion(255))));
    }
}
