//! Stream framing: `u32` little-endian length prefix + payload bytes.
//!
//! Datagrams are not framed (QUIC datagrams are self-delimiting). The
//! length cap protects against memory exhaustion from corrupt or
//! malicious prefixes — a full state snapshot is the largest legitimate
//! frame and stays far below it.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum accepted frame payload. Anything larger is a protocol error.
pub const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;

/// Framing errors.
#[derive(Debug)]
pub enum FrameError {
    /// Underlying I/O failed (includes clean EOF mid-frame).
    Io(std::io::Error),
    /// Peer announced a frame larger than [`MAX_FRAME_LEN`].
    TooLarge(usize),
    /// Clean EOF on a frame boundary — the stream ended.
    Closed,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Io(e) => write!(f, "frame i/o error: {e}"),
            FrameError::TooLarge(len) => write!(f, "frame of {len} bytes exceeds cap"),
            FrameError::Closed => write!(f, "stream closed"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// Validate a length prefix. Pure, so the fuzz target can hit it
/// directly.
pub fn validate_len(len: u32) -> Result<usize, FrameError> {
    let len = len as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    Ok(len)
}

/// Write one frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), FrameError> {
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge(payload.len()))?;
    validate_len(len)?;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one frame. Returns [`FrameError::Closed`] on clean EOF at a
/// frame boundary.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut len_bytes = [0u8; 4];
    let mut filled = 0;
    while filled < 4 {
        let n = reader.read(&mut len_bytes[filled..]).await?;
        if n == 0 {
            if filled == 0 {
                return Err(FrameError::Closed);
            }
            return Err(FrameError::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        filled += n;
    }
    let len = validate_len(u32::from_le_bytes(len_bytes))?;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        write_frame(&mut client, b"hello").await.unwrap();
        write_frame(&mut client, b"").await.unwrap();
        write_frame(&mut client, &[7u8; 300]).await.unwrap();

        assert_eq!(read_frame(&mut server).await.unwrap(), b"hello");
        assert_eq!(read_frame(&mut server).await.unwrap(), b"");
        assert_eq!(read_frame(&mut server).await.unwrap(), vec![7u8; 300]);
    }

    #[tokio::test]
    async fn clean_eof_is_closed_mid_frame_is_error() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        write_frame(&mut client, b"full").await.unwrap();
        // Half a length prefix, then drop the writer.
        tokio::io::AsyncWriteExt::write_all(&mut client, &[9u8, 0])
            .await
            .unwrap();
        drop(client);

        assert_eq!(read_frame(&mut server).await.unwrap(), b"full");
        assert!(matches!(
            read_frame(&mut server).await,
            Err(FrameError::Io(_))
        ));
    }

    #[tokio::test]
    async fn oversized_prefix_rejected_without_allocating() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let huge = (MAX_FRAME_LEN as u32 + 1).to_le_bytes();
        tokio::io::AsyncWriteExt::write_all(&mut client, &huge)
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut server).await,
            Err(FrameError::TooLarge(_))
        ));
    }
}
