//! A minimal Foxglove WebSocket **server** — the projection bridge's local
//! endpoint that Foxglove Studio / Lichtblick connect to.
//!
//! RoboTunnel's smartest move is projecting a robot's data into the tool every
//! ROS engineer already has open, instead of building yet another viz UI.
//! `gang view` copies that shape: it opens this local WebSocket server, and the
//! operator points Foxglove at `ws://localhost:<port>`. Ganglion supplies the
//! *reach* (the live feed arrives through the relay) and the *governance* (the
//! feed is capability-scoped and audited); Foxglove supplies the pixels.
//!
//! This module is deliberately dependency-light: it implements just enough of
//! the [Foxglove WebSocket protocol][spec] to advertise channels and stream
//! JSON messages — the HTTP upgrade handshake, unmasked server frames, masked
//! client-frame decoding, the `serverInfo` / `advertise` control messages, and
//! the binary `MessageData` frame. The protocol encoders/decoders and the
//! SHA-1 used for the handshake are pure functions with unit tests; only the
//! `serve`/`accept` glue touches the network.
//!
//! [spec]: https://github.com/foxglove/ws-protocol

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

/// The WebSocket subprotocol Foxglove clients negotiate.
pub const SUBPROTOCOL: &str = "foxglove.websocket.v1";

/// GUID appended to `Sec-WebSocket-Key` per RFC 6455 §4.2.2.
const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// --- Foxglove control messages (server → client) ------------------------------

/// A channel advertised to the client. One channel per logical stream.
#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    /// Server-assigned channel id.
    pub id: u32,
    /// Topic name shown in the client.
    pub topic: String,
    /// Message encoding (`"json"` here).
    pub encoding: String,
    /// Schema name (free-form; shown in the client).
    #[serde(rename = "schemaName")]
    pub schema_name: String,
    /// Schema text. For JSON encoding this is a JSON Schema document (may be
    /// empty when the client does not require one).
    pub schema: String,
    /// Encoding of the `schema` field (`"jsonschema"`).
    #[serde(rename = "schemaEncoding")]
    pub schema_encoding: String,
}

/// Build the `serverInfo` control message JSON.
pub fn server_info_json(name: &str) -> String {
    serde_json::json!({
        "op": "serverInfo",
        "name": name,
        "capabilities": [],
        "supportedEncodings": ["json"],
        "metadata": {},
    })
    .to_string()
}

/// Build the `advertise` control message JSON for a set of channels.
pub fn advertise_json(channels: &[Channel]) -> String {
    serde_json::json!({ "op": "advertise", "channels": channels }).to_string()
}

/// A client→server control message we care about. Everything else is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// `subscribe`: pairs of (subscription id, channel id).
    Subscribe(Vec<(u32, u32)>),
    /// `unsubscribe`: subscription ids to drop.
    Unsubscribe(Vec<u32>),
    /// Any other/unrecognized control message.
    Other,
}

/// Parse a client control message (text frame payload).
pub fn parse_client_message(text: &str) -> ClientMessage {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return ClientMessage::Other;
    };
    match v.get("op").and_then(|o| o.as_str()) {
        Some("subscribe") => {
            let subs = v
                .get("subscriptions")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| {
                            let id = s.get("id")?.as_u64()? as u32;
                            let ch = s.get("channelId")?.as_u64()? as u32;
                            Some((id, ch))
                        })
                        .collect()
                })
                .unwrap_or_default();
            ClientMessage::Subscribe(subs)
        }
        Some("unsubscribe") => {
            let ids = v
                .get("subscriptionIds")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| Some(x.as_u64()? as u32))
                        .collect()
                })
                .unwrap_or_default();
            ClientMessage::Unsubscribe(ids)
        }
        _ => ClientMessage::Other,
    }
}

/// Encode a binary `MessageData` frame payload (the *WebSocket* payload, before
/// framing): opcode `0x01`, then little-endian `subscription_id` (u32) and
/// `timestamp` (u64 nanoseconds), then the message bytes.
pub fn encode_message_data(subscription_id: u32, timestamp_ns: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + 8 + payload.len());
    out.push(0x01); // MessageData opcode
    out.extend_from_slice(&subscription_id.to_le_bytes());
    out.extend_from_slice(&timestamp_ns.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

// --- WebSocket handshake ------------------------------------------------------

/// Compute the `Sec-WebSocket-Accept` response value for a client key.
pub fn compute_accept_key(sec_websocket_key: &str) -> String {
    let mut input = String::with_capacity(sec_websocket_key.len() + WS_MAGIC.len());
    input.push_str(sec_websocket_key.trim());
    input.push_str(WS_MAGIC);
    let digest = sha1(input.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// Extract the value of a header (case-insensitive name) from a raw HTTP
/// request head.
fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        if k.trim().eq_ignore_ascii_case(name) {
            Some(v.trim())
        } else {
            None
        }
    })
}

/// Build the HTTP 101 upgrade response for a given client key.
fn upgrade_response(sec_websocket_key: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         Sec-WebSocket-Protocol: {}\r\n\r\n",
        compute_accept_key(sec_websocket_key),
        SUBPROTOCOL,
    )
}

// --- WebSocket framing --------------------------------------------------------

/// Opcodes we handle.
mod opcode {
    pub const TEXT: u8 = 0x1;
    pub const BINARY: u8 = 0x2;
    pub const CLOSE: u8 = 0x8;
    pub const PING: u8 = 0x9;
    pub const PONG: u8 = 0xA;
}

/// A decoded client frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Opcode (lower nibble of byte 0).
    pub opcode: u8,
    /// Unmasked payload.
    pub payload: Vec<u8>,
}

/// Encode a server→client frame (single, FIN set, unmasked, ≤ u64 length).
pub fn encode_server_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | (opcode & 0x0f)); // FIN + opcode
    let len = payload.len();
    if len < 126 {
        out.push(len as u8);
    } else if len <= u16::MAX as usize {
        out.push(126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// Read exactly one client frame from an async source, unmasking the payload.
/// Returns `None` on clean EOF.
async fn read_client_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<Frame>> {
    let mut hdr = [0u8; 2];
    if let Err(e) = reader.read_exact(&mut hdr).await {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(e);
    }
    let opcode = hdr[0] & 0x0f;
    let masked = (hdr[1] & 0x80) != 0;
    let len7 = hdr[1] & 0x7f;

    let len = match len7 {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b).await?;
            u16::from_be_bytes(b) as usize
        }
        127 => {
            let mut b = [0u8; 8];
            reader.read_exact(&mut b).await?;
            u64::from_be_bytes(b) as usize
        }
        n => n as usize,
    };

    let mask = if masked {
        let mut m = [0u8; 4];
        reader.read_exact(&mut m).await?;
        Some(m)
    } else {
        None
    };

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    if let Some(m) = mask {
        unmask(&mut payload, &m);
    }
    Ok(Some(Frame { opcode, payload }))
}

/// XOR-unmask a payload in place with a 4-byte key (RFC 6455 §5.3).
fn unmask(payload: &mut [u8], mask: &[u8; 4]) {
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
}

// --- The bridge server --------------------------------------------------------

/// One message to push to connected clients on a given channel.
#[derive(Debug, Clone)]
pub struct BridgeMessage {
    /// Channel id the message belongs to.
    pub channel_id: u32,
    /// Timestamp in nanoseconds since the Unix epoch.
    pub timestamp_ns: u64,
    /// JSON payload bytes.
    pub payload: Vec<u8>,
}

/// Serve the Foxglove bridge on an already-bound listener.
///
/// `channels` are advertised to every client on connect. `feed` delivers
/// [`BridgeMessage`]s (fanned out via a broadcast channel so multiple Foxglove
/// windows can attach); each connected client forwards only the channels it has
/// subscribed to. Runs until the listener errors or the process exits.
pub async fn serve(
    listener: tokio::net::TcpListener,
    server_name: String,
    channels: Vec<Channel>,
    feed: broadcast::Sender<BridgeMessage>,
) -> std::io::Result<()> {
    let channels = Arc::new(channels);
    let server_name = Arc::new(server_name);
    loop {
        let (stream, _peer) = listener.accept().await?;
        let channels = Arc::clone(&channels);
        let server_name = Arc::clone(&server_name);
        let rx = feed.subscribe();
        tokio::spawn(async move {
            let _ = handle_client(stream, &server_name, &channels, rx).await;
        });
    }
}

/// Handle a single Foxglove client: upgrade, advertise, then stream subscribed
/// messages until the client disconnects.
async fn handle_client(
    mut stream: TcpStream,
    server_name: &str,
    channels: &[Channel],
    mut feed: broadcast::Receiver<BridgeMessage>,
) -> std::io::Result<()> {
    // Read the HTTP upgrade request head (up to the blank line).
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Ok(()); // client hung up before completing the handshake
        }
        buf.push(byte[0]);
        if buf.len() > 16 * 1024 {
            return Ok(()); // oversized request head — drop
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let Some(key) = header_value(&request, "Sec-WebSocket-Key") else {
        return Ok(());
    };
    stream.write_all(upgrade_response(key).as_bytes()).await?;

    // Advertise: serverInfo then the channel list.
    stream
        .write_all(&encode_server_frame(
            opcode::TEXT,
            server_info_json(server_name).as_bytes(),
        ))
        .await?;
    stream
        .write_all(&encode_server_frame(
            opcode::TEXT,
            advertise_json(channels).as_bytes(),
        ))
        .await?;

    // channel id -> subscription id, populated as the client subscribes.
    let mut subs: HashMap<u32, u32> = HashMap::new();
    let (mut rd, mut wr) = stream.into_split();

    loop {
        tokio::select! {
            // Client control frames.
            frame = read_client_frame(&mut rd) => {
                match frame? {
                    None => return Ok(()),
                    Some(f) if f.opcode == opcode::CLOSE => return Ok(()),
                    Some(f) if f.opcode == opcode::PING => {
                        wr.write_all(&encode_server_frame(opcode::PONG, &f.payload)).await?;
                    }
                    Some(f) if f.opcode == opcode::TEXT => {
                        match parse_client_message(&String::from_utf8_lossy(&f.payload)) {
                            ClientMessage::Subscribe(pairs) => {
                                for (sub_id, ch_id) in pairs {
                                    subs.insert(ch_id, sub_id);
                                }
                            }
                            ClientMessage::Unsubscribe(ids) => {
                                subs.retain(|_, sub| !ids.contains(sub));
                            }
                            ClientMessage::Other => {}
                        }
                    }
                    Some(_) => {} // ignore other opcodes
                }
            }
            // Feed messages to forward.
            msg = feed.recv() => {
                match msg {
                    Ok(m) => {
                        if let Some(&sub_id) = subs.get(&m.channel_id) {
                            let data = encode_message_data(sub_id, m.timestamp_ns, &m.payload);
                            wr.write_all(&encode_server_frame(opcode::BINARY, &data)).await?;
                        }
                    }
                    // Lagged: skip the gap and keep going.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

// --- Inline SHA-1 (RFC 3174) --------------------------------------------------
//
// A tiny, self-contained SHA-1 so the handshake needs no crypto dependency.
// SHA-1 is used here ONLY for the WebSocket accept-key ritual, which is not a
// security property — RFC 6455 mandates this exact construction.

/// Compute the SHA-1 digest of `data`.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    let ml = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());

    // The padding above guarantees a whole number of 64-byte blocks, so the
    // `as_chunks` remainders are always empty.
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha1_known_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn accept_key_matches_rfc_example() {
        // RFC 6455 §1.3 worked example.
        assert_eq!(
            compute_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn message_data_frame_layout() {
        let f = encode_message_data(7, 0x0102030405060708, b"hi");
        assert_eq!(f[0], 0x01); // opcode
        assert_eq!(&f[1..5], &7u32.to_le_bytes()); // subscription id
        assert_eq!(&f[5..13], &0x0102030405060708u64.to_le_bytes()); // ts
        assert_eq!(&f[13..], b"hi");
    }

    #[test]
    fn server_frame_length_encodings() {
        // < 126 → 1-byte length.
        let small = encode_server_frame(opcode::BINARY, &[0u8; 10]);
        assert_eq!(small[0], 0x82);
        assert_eq!(small[1], 10);
        // 126..=65535 → 2-byte extended length.
        let mid = encode_server_frame(opcode::BINARY, &[0u8; 200]);
        assert_eq!(mid[1], 126);
        assert_eq!(&mid[2..4], &200u16.to_be_bytes());
        // > 65535 → 8-byte extended length.
        let big = encode_server_frame(opcode::BINARY, &vec![0u8; 70_000]);
        assert_eq!(big[1], 127);
        assert_eq!(&big[2..10], &70_000u64.to_be_bytes());
    }

    #[test]
    fn parse_subscribe_and_unsubscribe() {
        let sub = parse_client_message(
            r#"{"op":"subscribe","subscriptions":[{"id":1,"channelId":5},{"id":2,"channelId":9}]}"#,
        );
        assert_eq!(sub, ClientMessage::Subscribe(vec![(1, 5), (2, 9)]));
        let unsub = parse_client_message(r#"{"op":"unsubscribe","subscriptionIds":[1,2]}"#);
        assert_eq!(unsub, ClientMessage::Unsubscribe(vec![1, 2]));
        assert_eq!(parse_client_message("not json"), ClientMessage::Other);
        assert_eq!(
            parse_client_message(r#"{"op":"other"}"#),
            ClientMessage::Other
        );
    }

    #[test]
    fn advertise_and_server_info_shapes() {
        let ch = Channel {
            id: 1,
            topic: "/ganglion/events".into(),
            encoding: "json".into(),
            schema_name: "AgentEvent".into(),
            schema: "{}".into(),
            schema_encoding: "jsonschema".into(),
        };
        let adv = advertise_json(std::slice::from_ref(&ch));
        let v: serde_json::Value = serde_json::from_str(&adv).unwrap();
        assert_eq!(v["op"], "advertise");
        assert_eq!(v["channels"][0]["topic"], "/ganglion/events");
        assert_eq!(v["channels"][0]["schemaName"], "AgentEvent");

        let info: serde_json::Value = serde_json::from_str(&server_info_json("gang view")).unwrap();
        assert_eq!(info["op"], "serverInfo");
        assert_eq!(info["supportedEncodings"][0], "json");
    }

    #[test]
    fn unmask_roundtrip() {
        let mask = [0x12, 0x34, 0x56, 0x78];
        let original = b"hello foxglove".to_vec();
        let mut masked = original.clone();
        unmask(&mut masked, &mask); // mask
        assert_ne!(masked, original);
        unmask(&mut masked, &mask); // unmask
        assert_eq!(masked, original);
    }

    #[tokio::test]
    async fn read_client_frame_decodes_masked_text() {
        // A masked client text frame carrying "hi".
        let mask = [0xAA, 0xBB, 0xCC, 0xDD];
        let mut payload = b"hi".to_vec();
        unmask(&mut payload, &mask);
        let mut wire = vec![0x81, 0x80 | 2]; // FIN+TEXT, MASK + len 2
        wire.extend_from_slice(&mask);
        wire.extend_from_slice(&payload);

        let mut cursor = std::io::Cursor::new(wire);
        let frame = read_client_frame(&mut cursor).await.unwrap().unwrap();
        assert_eq!(frame.opcode, opcode::TEXT);
        assert_eq!(frame.payload, b"hi");
    }
}
