//! Length-prefixed postcard framing for QUIC streams and datagrams.
//!
//! Stream framing: `[u32 LE length][u8 tag][postcard body]`
//! Datagram framing: `[u8 tag][postcard body]` (no length prefix)

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum frame size (16 MiB). Protects against malicious or corrupt length prefixes.
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

// Message type tags
pub const TAG_RV_CONTROL: u8 = 1;
pub const TAG_PEER_CONTROL: u8 = 2;
pub const TAG_PEER_DATAGRAM: u8 = 3;
pub const TAG_RELAY_ENVELOPE: u8 = 4;
pub const TAG_GAP_FILL_REQUEST: u8 = 5;
pub const TAG_GAP_FILL_RESPONSE: u8 = 6;
pub const TAG_CHUNK_REQUEST: u8 = 7;
pub const TAG_CHUNK_DATA: u8 = 8;

/// Framing errors.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame too large: {size} bytes (max {MAX_FRAME_SIZE})")]
    TooLarge { size: u32 },
    #[error("wrong message tag: expected {expected}, got {actual}")]
    WrongTag { expected: u8, actual: u8 },
    #[error("postcard deserialization failed: {0}")]
    Deserialize(#[from] postcard::Error),
    #[error("empty datagram")]
    EmptyDatagram,
}

/// Write `[u32 LE length][u8 tag][postcard body]` to a stream.
///
/// The length field covers `tag + body` (i.e. total payload after the length prefix).
pub async fn write_framed<W: AsyncWrite + Unpin, M: Serialize>(
    writer: &mut W,
    tag: u8,
    msg: &M,
) -> Result<(), FrameError> {
    let body = postcard::to_allocvec(msg).map_err(FrameError::Deserialize)?;
    let payload_len: u32 = 1 + body.len() as u32; // tag + body
    if payload_len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { size: payload_len });
    }
    writer.write_all(&payload_len.to_le_bytes()).await?;
    writer.write_all(&[tag]).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

/// Read `[u32 LE length][u8 tag][postcard body]` from a stream.
///
/// Returns `Ok(None)` on clean EOF (zero bytes read for the length prefix).
/// Returns an error on truncated frames or tag mismatch.
pub async fn read_framed<R: AsyncRead + Unpin, M: for<'de> Deserialize<'de>>(
    reader: &mut R,
    expected_tag: u8,
) -> Result<Option<M>, FrameError> {
    // Read length prefix
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(FrameError::Io(e)),
    }
    let payload_len = u32::from_le_bytes(len_buf);
    if payload_len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { size: payload_len });
    }
    if payload_len == 0 {
        return Err(FrameError::WrongTag {
            expected: expected_tag,
            actual: 0,
        });
    }

    // Read tag + body
    let mut payload = vec![0u8; payload_len as usize];
    reader.read_exact(&mut payload).await?;

    let actual_tag = payload[0];
    if actual_tag != expected_tag {
        return Err(FrameError::WrongTag {
            expected: expected_tag,
            actual: actual_tag,
        });
    }

    let msg = postcard::from_bytes(&payload[1..])?;
    Ok(Some(msg))
}

/// Encode `[u8 tag][postcard body]` for datagrams (no length prefix).
pub fn encode_datagram<M: Serialize>(tag: u8, msg: &M) -> Result<Vec<u8>, FrameError> {
    let body = postcard::to_allocvec(msg).map_err(FrameError::Deserialize)?;
    let mut buf = Vec::with_capacity(1 + body.len());
    buf.push(tag);
    buf.extend_from_slice(&body);
    Ok(buf)
}

/// Decode `[u8 tag][postcard body]` from a datagram.
pub fn decode_datagram<M: for<'de> Deserialize<'de>>(
    data: &[u8],
    expected_tag: u8,
) -> Result<M, FrameError> {
    if data.is_empty() {
        return Err(FrameError::EmptyDatagram);
    }
    let actual_tag = data[0];
    if actual_tag != expected_tag {
        return Err(FrameError::WrongTag {
            expected: expected_tag,
            actual: actual_tag,
        });
    }
    let msg = postcard::from_bytes(&data[1..])?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PeerControl, PeerDatagram, PeerInfo, RvControl};
    use std::net::{Ipv4Addr, SocketAddr};

    #[tokio::test]
    async fn round_trip_rv_control() -> Result<(), FrameError> {
        let msg = RvControl::AuthOk {
            peer_id: crate::types::PeerId(1),
            observed_addr: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 1234),
        };
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        write_framed(&mut tx, TAG_RV_CONTROL, &msg).await?;
        let decoded: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
        assert_eq!(Some(msg), decoded);
        Ok(())
    }

    #[tokio::test]
    async fn round_trip_peer_control() -> Result<(), FrameError> {
        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(1),
            username: "alice".into(),
        };
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        write_framed(&mut tx, TAG_PEER_CONTROL, &msg).await?;
        let decoded: Option<PeerControl> = read_framed(&mut rx, TAG_PEER_CONTROL).await?;
        assert_eq!(Some(msg), decoded);
        Ok(())
    }

    #[tokio::test]
    async fn round_trip_peer_datagram() -> Result<(), FrameError> {
        let msg = PeerDatagram::Position {
            timestamp: 1000,
            position_secs: 42.5,
        };
        let encoded = encode_datagram(TAG_PEER_DATAGRAM, &msg)?;
        let decoded: PeerDatagram = decode_datagram(&encoded, TAG_PEER_DATAGRAM)?;
        assert_eq!(msg, decoded);
        Ok(())
    }

    #[tokio::test]
    async fn round_trip_auth() -> Result<(), FrameError> {
        let msg = RvControl::Auth {
            password: "secret".into(),
            username: "alice".into(),
        };
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        write_framed(&mut tx, TAG_RV_CONTROL, &msg).await?;
        let decoded: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
        assert_eq!(Some(msg), decoded);
        Ok(())
    }

    #[tokio::test]
    async fn round_trip_time_sync() -> Result<(), FrameError> {
        let msg = RvControl::TimeSyncResponse {
            client_send: 100,
            server_recv: 150,
            server_send: 151,
        };
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        write_framed(&mut tx, TAG_RV_CONTROL, &msg).await?;
        let decoded: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
        assert_eq!(Some(msg), decoded);
        Ok(())
    }

    #[tokio::test]
    async fn round_trip_peer_list() -> Result<(), FrameError> {
        let msg = RvControl::PeerList {
            peers: vec![PeerInfo {
                peer_id: crate::types::PeerId(2),
                username: "bob".into(),
                addresses: vec![SocketAddr::new(Ipv4Addr::new(1, 2, 3, 4).into(), 5000)],
                connected_since: 999,
            }],
        };
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        write_framed(&mut tx, TAG_RV_CONTROL, &msg).await?;
        let decoded: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
        assert_eq!(Some(msg), decoded);
        Ok(())
    }

    #[tokio::test]
    async fn clean_eof_returns_none() -> Result<(), FrameError> {
        let (tx, mut rx) = tokio::io::duplex(4096);
        drop(tx); // close the write side
        let result: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn truncated_frame_returns_error() {
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        // Write a length prefix claiming 100 bytes, then close
        tokio::io::AsyncWriteExt::write_all(&mut tx, &100u32.to_le_bytes())
            .await
            .ok();
        drop(tx);
        let result: Result<Option<RvControl>, _> = read_framed(&mut rx, TAG_RV_CONTROL).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn oversized_frame_rejected() {
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        let huge_len = MAX_FRAME_SIZE + 1;
        tokio::io::AsyncWriteExt::write_all(&mut tx, &huge_len.to_le_bytes())
            .await
            .ok();
        drop(tx);
        let result: Result<Option<RvControl>, _> = read_framed(&mut rx, TAG_RV_CONTROL).await;
        assert!(matches!(result, Err(FrameError::TooLarge { .. })));
    }

    #[tokio::test]
    async fn wrong_tag_rejected_stream() {
        let msg = PeerControl::Hello {
            peer_id: crate::types::PeerId(1),
            username: "alice".into(),
        };
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        let _ = write_framed(&mut tx, TAG_PEER_CONTROL, &msg).await;
        // Try to read with wrong tag
        let result: Result<Option<PeerControl>, _> = read_framed(&mut rx, TAG_RV_CONTROL).await;
        assert!(matches!(result, Err(FrameError::WrongTag { .. })));
    }

    #[test]
    fn wrong_tag_rejected_datagram() {
        let msg = PeerDatagram::Position {
            timestamp: 1000,
            position_secs: 42.5,
        };
        let encoded = encode_datagram(TAG_PEER_DATAGRAM, &msg).ok();
        let encoded = encoded.as_deref();
        assert!(encoded.is_some());
        let result: Result<PeerDatagram, _> =
            decode_datagram(encoded.as_ref().map_or(&[], |v| v), TAG_PEER_CONTROL);
        assert!(matches!(result, Err(FrameError::WrongTag { .. })));
    }

    #[test]
    fn empty_datagram_rejected() {
        let result: Result<PeerDatagram, _> = decode_datagram(&[], TAG_PEER_DATAGRAM);
        assert!(matches!(result, Err(FrameError::EmptyDatagram)));
    }

    #[test]
    fn corrupt_datagram_body() {
        let data = [TAG_PEER_DATAGRAM, 0xFF, 0xFF, 0xFF];
        let result: Result<PeerDatagram, _> = decode_datagram(&data, TAG_PEER_DATAGRAM);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn multiple_frames_on_one_stream() -> Result<(), FrameError> {
        let msgs = vec![
            RvControl::Auth {
                password: "p1".into(),
                username: "test".into(),
            },
            RvControl::AuthFailed,
            RvControl::TimeSyncRequest { client_send: 42 },
        ];
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        for msg in &msgs {
            write_framed(&mut tx, TAG_RV_CONTROL, msg).await?;
        }
        drop(tx);
        for expected in &msgs {
            let decoded: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
            assert_eq!(Some(expected.clone()), decoded);
        }
        // Should get None at EOF
        let final_result: Option<RvControl> = read_framed(&mut rx, TAG_RV_CONTROL).await?;
        assert!(final_result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn zero_length_frame_rejected() {
        let (mut tx, mut rx) = tokio::io::duplex(4096);
        tokio::io::AsyncWriteExt::write_all(&mut tx, &0u32.to_le_bytes())
            .await
            .ok();
        drop(tx);
        let result: Result<Option<RvControl>, _> = read_framed(&mut rx, TAG_RV_CONTROL).await;
        assert!(result.is_err());
    }
}
