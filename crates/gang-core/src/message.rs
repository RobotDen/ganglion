use serde::{Deserialize, Serialize};

use crate::identity::PeerId;

/// Length-prefixed CBOR framing for Ganglion messages.
/// Wire format: [varint length][CBOR payload]
///
/// Encode a message to length-prefixed CBOR bytes.
pub fn encode_message<T: Serialize>(
    msg: &T,
) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut payload = Vec::new();
    ciborium::into_writer(msg, &mut payload)?;

    let mut frame = Vec::with_capacity(payload.len() + 4);
    write_varint(&mut frame, payload.len() as u64);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a length-prefixed CBOR message from a byte slice.
/// Returns the decoded message and the number of bytes consumed.
pub fn decode_message<T: for<'de> Deserialize<'de>>(
    data: &[u8],
) -> Result<(T, usize), DecodeError> {
    let (len, varint_size) = read_varint(data).ok_or(DecodeError::Incomplete)?;
    let total = varint_size + len as usize;

    if data.len() < total {
        return Err(DecodeError::Incomplete);
    }

    let payload = &data[varint_size..total];
    let msg: T =
        ciborium::from_reader(payload).map_err(|e| DecodeError::CborError(e.to_string()))?;
    Ok((msg, total))
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("incomplete frame, need more data")]
    Incomplete,
    #[error("cbor decode: {0}")]
    CborError(String),
}

fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value > 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

// --- Control protocol messages ---

/// Messages on /ganglion/control/1.0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    /// Presence announcement from a robot agent.
    Presence {
        peer_id: PeerId,
        capabilities: Vec<String>,
        uptime_secs: u64,
        version: String,
    },
    /// Deploy a signed capability to a robot.
    DeployCapability {
        name: String,
        version: String,
        manifest_cbor: Vec<u8>,
        component_bytes: Vec<u8>,
    },
    /// Invoke an installed capability.
    InvokeCapability {
        name: String,
        args: Vec<String>,
        request_id: String,
    },
    /// Response to a capability invocation.
    InvokeResult {
        request_id: String,
        status: InvokeStatus,
        output: Vec<u8>,
    },
    /// List installed capabilities (request).
    ListCapabilities,
    /// List installed capabilities (response).
    CapabilityList { capabilities: Vec<CapabilityInfo> },
    /// Error response.
    Error {
        request_id: Option<String>,
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeStatus {
    Success,
    Failed,
    Timeout,
    PolicyDenied,
    Trapped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityInfo {
    pub name: String,
    pub version: String,
    pub author: PeerId,
    pub declared_capabilities: Vec<String>,
}

// --- Tool protocol messages ---

/// Messages on /ganglion/tool/1.0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolMessage {
    /// Data chunk from capability to operator (or vice versa).
    Data { payload: Vec<u8> },
    /// End of stream.
    Eof,
    /// Error during tool execution.
    Error { message: String },
}

// --- Bulk transfer messages ---

/// Messages on /ganglion/bulk/1.0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkMessage {
    /// Offer an artifact for transfer.
    Offer {
        name: String,
        size: u64,
        hash: String,
    },
    /// Accept the offered artifact.
    Accept,
    /// A data chunk.
    Chunk { offset: u64, data: Vec<u8> },
    /// Transfer complete.
    Complete { hash: String },
    /// Progress report.
    Progress {
        bytes_transferred: u64,
        total_bytes: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for value in [0u64, 1, 127, 128, 255, 16384, u64::MAX / 2] {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            let (decoded, size) = read_varint(&buf).unwrap();
            assert_eq!(decoded, value);
            assert_eq!(size, buf.len());
        }
    }

    #[test]
    fn message_roundtrip() {
        let msg = ControlMessage::Presence {
            peer_id: crate::identity::PeerId::from_public_key(
                &crate::identity::Keypair::generate().public_key(),
            ),
            capabilities: vec!["diagnostics".into()],
            uptime_secs: 3600,
            version: "0.1.0".into(),
        };

        let encoded = encode_message(&msg).unwrap();
        let (decoded, consumed): (ControlMessage, usize) = decode_message(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());

        match decoded {
            ControlMessage::Presence { uptime_secs, .. } => {
                assert_eq!(uptime_secs, 3600);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn incomplete_frame_returns_error() {
        let msg = ControlMessage::ListCapabilities;
        let encoded = encode_message(&msg).unwrap();
        // Truncate
        let result: Result<(ControlMessage, usize), _> =
            decode_message(&encoded[..encoded.len() - 1]);
        assert!(result.is_err());
    }
}
