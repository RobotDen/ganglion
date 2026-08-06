//! Length-prefixed framing over a raw libp2p push substream (ADR-024).
//!
//! The event feed reuses the exact wire framing of the control codec:
//! `[varint length][CBOR payload]` (see [`gang_core::message::encode_message`] /
//! [`gang_core::message::decode_message`]). This module is the async
//! read/write half of that framing over a [`libp2p::Stream`] (which is
//! `futures::AsyncRead + AsyncWrite`), shared by the robot push loop and the
//! operator decode loop so both sides agree on framing byte-for-byte.

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum accepted frame length (16 MiB), matching the control codec ceiling.
/// A peer that announces a larger frame is treated as hostile/broken.
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Write a pre-framed message (the output of
/// [`gang_core::message::encode_message`], i.e. varint length + CBOR) to the
/// stream and flush it, so the peer sees the frame promptly (true push, no
/// batching delay).
pub async fn write_frame<W>(w: &mut W, frame: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    w.write_all(frame).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed frame from the stream, returning the FULL frame
/// bytes (varint prefix + CBOR payload) so the caller can decode it with
/// [`gang_core::message::decode_message`] — the same decoder the buffered path
/// used.
///
/// Returns `Ok(None)` on a clean EOF at a frame boundary (the peer closed the
/// stream), which the caller treats as end-of-feed rather than an error.
pub async fn read_frame<R>(r: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    // Decode the varint length prefix one byte at a time, accumulating the raw
    // prefix bytes so the returned frame is byte-identical to what the writer
    // produced. A clean EOF before the first byte means "no more frames".
    let mut frame = Vec::with_capacity(16);
    let mut len: u64 = 0;
    let mut shift = 0u32;
    loop {
        let mut byte = [0u8; 1];
        if r.read(&mut byte).await? == 0 {
            if frame.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "EOF in the middle of a frame length prefix",
            ));
        }
        let b = byte[0];
        frame.push(b);
        len |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame length varint too long",
            ));
        }
    }

    let len = len as usize;
    if len > MAX_FRAME_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes (max {MAX_FRAME_SIZE})"),
        ));
    }

    let prefix_len = frame.len();
    frame.resize(prefix_len + len, 0);
    r.read_exact(&mut frame[prefix_len..]).await?;
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gang_core::events::AgentEvent;
    use gang_core::message::{decode_message, encode_message};

    #[tokio::test]
    async fn frames_roundtrip_and_stop_at_clean_eof() {
        // Write two events into an in-memory buffer, then read them back frame
        // by frame; the trailing read must report a clean EOF as `None`.
        let events = [
            AgentEvent::Gap { dropped: 5 },
            AgentEvent::PresenceSnapshot {
                seq: 7,
                ganglion_version: "2.1.0".into(),
                uptime_secs: 3,
                archetype: None,
                installed_capabilities: vec!["diagnostics".into()],
            },
        ];
        let mut buf: Vec<u8> = Vec::new();
        for ev in &events {
            write_frame(&mut buf, &encode_message(ev).unwrap())
                .await
                .unwrap();
        }

        let mut reader = futures::io::Cursor::new(buf);
        let first = read_frame(&mut reader).await.unwrap().expect("first frame");
        let (a, _) = decode_message::<AgentEvent>(&first).unwrap();
        assert!(matches!(a, AgentEvent::Gap { dropped: 5 }));

        let second = read_frame(&mut reader)
            .await
            .unwrap()
            .expect("second frame");
        let (b, _) = decode_message::<AgentEvent>(&second).unwrap();
        assert!(matches!(b, AgentEvent::PresenceSnapshot { seq: 7, .. }));

        assert!(
            read_frame(&mut reader).await.unwrap().is_none(),
            "clean EOF at a frame boundary must yield None"
        );
    }
}
