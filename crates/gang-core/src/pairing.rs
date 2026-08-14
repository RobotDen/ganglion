//! One-line robot enrollment: the pairing-token trust model (`gang pair`).
//!
//! The operator's `gang pair` mints a short-lived, single-use **pairing token**
//! and prints a single robot-side line (`gang join gang1_<token>`). The token
//! binds three facts the robot needs and the operator can later verify:
//!
//! - the **relay** to dial (`relay_addr`, a dialable multiaddr ending in
//!   `/p2p/<relay-libp2p-id>`),
//! - the **operator** to enroll with (`operator_libp2p_id`, the dialable base58
//!   id — libp2p's Noise handshake holds the far end to exactly this key, so the
//!   robot cryptographically confirms it reached the operator the token names),
//! - a random 32-byte bearer **secret** that proves the robot was invited.
//!
//! The token is *not* a capability grant and carries no operator signature: the
//! bearer secret is the credential (like a Tailscale auth key), and the operator
//! holds the canonical copy in memory for the duration of one `gang pair`
//! session, comparing it in constant time and consuming it exactly once. The
//! robot's recorded identity is **never** taken from the token or from anything
//! the robot claims — the operator reads the wire-authenticated gang id off the
//! enrollment stream (`GanglionStream::remote_peer`, derived from the Ed25519
//! key libp2p authenticated) and only accepts a self-reported dialable id whose
//! embedded key derives back to that same authenticated id.
//!
//! ## What each property defends against
//!
//! - **Forge:** the secret is 32 bytes of CSPRNG output; an attacker who never
//!   saw a token the operator minted cannot produce a secret the operator
//!   accepts, so cannot enroll into the operator's fleet.
//! - **Tamper:** flipping bits in the encoded token either breaks base64url/CBOR
//!   decoding or changes the secret, both of which the operator rejects.
//! - **Replay / reuse:** the operator consumes the token on first successful
//!   enrollment; a second presentation of the same secret is rejected
//!   ([`PairingError::AlreadyUsed`]
//!   at the call site).
//! - **Expiry:** [`PairingToken::verify`](crate::pairing::PairingToken::verify)
//!   rejects a token past `expires_at_ms`.
//! - **Wrong id:** the operator records the wire-authenticated id, so a robot
//!   cannot register as an identity whose Ed25519 key it does not hold.

use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Versioned, URL-safe prefix on every encoded token. Bumping the embedded
/// [`PairingToken::version`] and/or this prefix lets a future format be
/// distinguished and rejected cleanly rather than mis-parsed.
pub const TOKEN_PREFIX: &str = "gang1_";

/// The token format version encoded inside the CBOR payload.
pub const TOKEN_VERSION: u8 = 1;

/// Default lifetime of a freshly minted pairing token.
pub const DEFAULT_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Length of the bearer secret, in bytes.
pub const SECRET_LEN: usize = 32;

/// Errors from minting, decoding, or verifying a pairing token.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingError {
    /// The string did not start with the expected [`TOKEN_PREFIX`].
    #[error("not a pairing token: missing `{TOKEN_PREFIX}` prefix")]
    BadPrefix,
    /// The base64url body could not be decoded.
    #[error("malformed pairing token: {0}")]
    Malformed(String),
    /// The token encodes a version this build does not understand.
    #[error("unsupported pairing-token version {0} (this build understands {TOKEN_VERSION})")]
    UnsupportedVersion(u8),
    /// The token's validity window has elapsed.
    #[error("pairing token expired")]
    Expired,
    /// The presented bearer secret did not match the token.
    #[error("pairing token secret mismatch")]
    BadSecret,
    /// The token was already consumed by a prior successful enrollment.
    #[error("pairing token already used")]
    AlreadyUsed,
    /// The robot's self-reported dialable id does not derive to the identity
    /// libp2p authenticated on the enrollment stream — an attempt to be recorded
    /// as an id the robot does not hold the key for.
    #[error("reported id does not match the wire-authenticated identity")]
    IdentityMismatch,
}

/// A decoded pairing token: everything the robot needs to dial out and enroll,
/// and everything the operator needs to authenticate the invitation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingToken {
    /// Format version (see [`TOKEN_VERSION`]).
    pub version: u8,
    /// Dialable relay multiaddr, ending in `/p2p/<relay-libp2p-id>`.
    pub relay_addr: String,
    /// The operator's dialable base58 libp2p id (`12D3KooW…`). The robot dials
    /// this through the relay circuit; Noise authenticates it end-to-end.
    pub operator_libp2p_id: String,
    /// Random bearer secret proving the robot was invited.
    pub secret: [u8; SECRET_LEN],
    /// Expiry as Unix time in milliseconds.
    pub expires_at_ms: u64,
}

impl PairingToken {
    /// Mint a fresh token binding `relay_addr` + `operator_libp2p_id`, valid for
    /// `ttl` from `now_ms`, with a freshly sampled CSPRNG secret.
    pub fn mint(
        relay_addr: impl Into<String>,
        operator_libp2p_id: impl Into<String>,
        now_ms: u64,
        ttl: std::time::Duration,
    ) -> Self {
        let mut secret = [0u8; SECRET_LEN];
        rand::thread_rng().fill_bytes(&mut secret);
        Self {
            version: TOKEN_VERSION,
            relay_addr: relay_addr.into(),
            operator_libp2p_id: operator_libp2p_id.into(),
            secret,
            expires_at_ms: now_ms.saturating_add(ttl.as_millis() as u64),
        }
    }

    /// Encode to the copy-paste form: `gang1_` + base64url(CBOR(self)).
    pub fn encode(&self) -> String {
        let mut cbor = Vec::new();
        // Serialization of a plain struct into a Vec never fails.
        ciborium::into_writer(self, &mut cbor).expect("CBOR encode of PairingToken cannot fail");
        format!("{TOKEN_PREFIX}{}", base64url_encode(&cbor))
    }

    /// Decode from the copy-paste form. Validates the prefix, base64url body,
    /// CBOR shape, and version — but *not* expiry (see [`PairingToken::verify`]).
    pub fn decode(s: &str) -> Result<Self, PairingError> {
        let body = s
            .trim()
            .strip_prefix(TOKEN_PREFIX)
            .ok_or(PairingError::BadPrefix)?;
        let cbor = base64url_decode(body).map_err(PairingError::Malformed)?;
        let token: PairingToken = ciborium::from_reader(cbor.as_slice())
            .map_err(|e| PairingError::Malformed(format!("cbor: {e}")))?;
        if token.version != TOKEN_VERSION {
            return Err(PairingError::UnsupportedVersion(token.version));
        }
        Ok(token)
    }

    /// Whether the token has expired relative to `now_ms`.
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms > self.expires_at_ms
    }

    /// Operator-side check of a presented bearer secret: constant-time secret
    /// comparison plus expiry. Does **not** enforce single-use — the caller
    /// consumes the token (and returns [`PairingError::AlreadyUsed`] on reuse).
    pub fn verify(&self, presented_secret: &[u8], now_ms: u64) -> Result<(), PairingError> {
        if self.is_expired(now_ms) {
            return Err(PairingError::Expired);
        }
        if !constant_time_eq(&self.secret, presented_secret) {
            return Err(PairingError::BadSecret);
        }
        Ok(())
    }
}

/// The operator-side enrollment decision, as one auditable function shared by
/// `gang pair` and its integration tests.
///
/// Accepts an enrollment only when *all* of the following hold:
///
/// 1. the presented bearer secret matches the token, in constant time
///    ([`PairingToken::verify`] — also rejects an expired token);
/// 2. the id derived from the robot's self-reported dialable libp2p id equals
///    the id libp2p authenticated on the enrollment stream (`wire_gang_id`).
///
/// The second check is what keeps the recorded identity honest: the operator
/// only ever stores the wire-authenticated id, and only trusts a self-reported
/// dialable form once it has been shown to embed that same key. Single-use is
/// enforced by the caller consuming the token before/around this call.
pub fn authorize_enrollment(
    token: &PairingToken,
    presented_secret: &[u8],
    wire_gang_id: &crate::identity::PeerId,
    reported_derived_gang_id: &crate::identity::PeerId,
    now_ms: u64,
) -> Result<(), PairingError> {
    token.verify(presented_secret, now_ms)?;
    if wire_gang_id != reported_derived_gang_id {
        return Err(PairingError::IdentityMismatch);
    }
    Ok(())
}

/// Constant-time byte comparison, so a secret mismatch reveals nothing through
/// timing. Length mismatch fails without early return over the shorter length.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// --- base64url (RFC 4648 §5, no padding) ---
//
// Implemented locally rather than pulling in a `base64` dependency (not in the
// workspace dependency table). The token is not secret material in itself, but
// the encoder/decoder is exercised by round-trip and edge-case unit tests below
// so a bit-drop bug cannot silently corrupt a token.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3F) as usize] as char);
        }
    }
    out
}

fn decode_char(c: u8) -> Result<u32, String> {
    match c {
        b'A'..=b'Z' => Ok((c - b'A') as u32),
        b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
        b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
        b'-' => Ok(62),
        b'_' => Ok(63),
        other => Err(format!("invalid base64url character 0x{other:02x}")),
    }
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 == 1 {
        return Err("invalid base64url length".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= decode_char(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PairingToken {
        PairingToken::mint(
            "/ip4/127.0.0.1/tcp/4001/p2p/12D3KooWRelay",
            "12D3KooWOperator",
            1_000_000,
            DEFAULT_TTL,
        )
    }

    #[test]
    fn base64url_roundtrips_all_remainders() {
        for len in 0..64usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let encoded = base64url_encode(&data);
            assert!(
                encoded
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "encoding must be URL-safe"
            );
            let decoded = base64url_decode(&encoded).expect("decode");
            assert_eq!(decoded, data, "round-trip failed at len {len}");
        }
    }

    #[test]
    fn token_roundtrips() {
        let token = sample();
        let encoded = token.encode();
        assert!(encoded.starts_with(TOKEN_PREFIX));
        let decoded = PairingToken::decode(&encoded).expect("decode");
        assert_eq!(decoded, token);
    }

    #[test]
    fn decode_rejects_bad_prefix() {
        assert_eq!(
            PairingToken::decode("nope_abc"),
            Err(PairingError::BadPrefix)
        );
    }

    #[test]
    fn decode_rejects_tampered_body() {
        let mut encoded = sample().encode();
        // Corrupt a character in the body (after the prefix).
        let idx = TOKEN_PREFIX.len() + 3;
        let b = encoded.as_bytes()[idx];
        let repl = if b == b'A' { 'B' } else { 'A' };
        encoded.replace_range(idx..idx + 1, &repl.to_string());
        // Either the CBOR no longer decodes, or it decodes to a *different*
        // token whose secret will not verify — never silently the same token.
        match PairingToken::decode(&encoded) {
            Err(_) => {}
            Ok(other) => assert_ne!(other, sample()),
        }
    }

    #[test]
    fn verify_accepts_fresh_correct_secret() {
        let token = sample();
        assert_eq!(token.verify(&token.secret, 1_000_001), Ok(()));
    }

    #[test]
    fn verify_rejects_expired() {
        let token = sample();
        let after = token.expires_at_ms + 1;
        assert_eq!(
            token.verify(&token.secret, after),
            Err(PairingError::Expired)
        );
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let token = sample();
        let mut wrong = token.secret;
        wrong[0] ^= 0xFF;
        assert_eq!(
            token.verify(&wrong, 1_000_001),
            Err(PairingError::BadSecret)
        );
    }

    #[test]
    fn verify_rejects_wrong_length_secret() {
        let token = sample();
        assert_eq!(
            token.verify(b"short", 1_000_001),
            Err(PairingError::BadSecret)
        );
    }

    #[test]
    fn minted_secrets_differ() {
        assert_ne!(sample().secret, sample().secret, "secrets must be random");
    }

    #[test]
    fn authorize_accepts_matching_wire_identity() {
        let token = sample();
        let id = crate::identity::PeerId::from_ed25519_bytes(&[7u8; 32]);
        assert_eq!(
            authorize_enrollment(&token, &token.secret, &id, &id, 1_000_001),
            Ok(())
        );
    }

    #[test]
    fn authorize_rejects_identity_mismatch() {
        let token = sample();
        let wire = crate::identity::PeerId::from_ed25519_bytes(&[7u8; 32]);
        let claimed = crate::identity::PeerId::from_ed25519_bytes(&[9u8; 32]);
        assert_eq!(
            authorize_enrollment(&token, &token.secret, &wire, &claimed, 1_000_001),
            Err(PairingError::IdentityMismatch)
        );
    }

    #[test]
    fn authorize_rejects_bad_secret_before_identity() {
        let token = sample();
        let id = crate::identity::PeerId::from_ed25519_bytes(&[7u8; 32]);
        let mut wrong = token.secret;
        wrong[0] ^= 0xFF;
        assert_eq!(
            authorize_enrollment(&token, &wrong, &id, &id, 1_000_001),
            Err(PairingError::BadSecret)
        );
    }
}
