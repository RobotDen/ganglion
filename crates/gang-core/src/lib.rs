//! # gang-core
//!
//! Core types, traits, and protocols for the Ganglion connectivity substrate.
//!
//! Ganglion provides hostile-network reachability and sandboxed field tooling
//! for ROS 2 robot fleets. `gang-core` is the dependency-free foundation shared
//! by every other crate in the workspace; it defines the vocabulary the three
//! architectural layers speak:
//!
//! - **Identity** ([`identity`]): Ed25519 keypairs and the derived [`identity::PeerId`].
//! - **Messaging** ([`message`], [`protocol`]): the length-prefixed CBOR wire
//!   framing and the control/tool/bulk stream protocols.
//! - **Transport** ([`transport`]): the protocol-agnostic [`transport::TransportAdapter`]
//!   trait and happy-eyeballs dialing.
//! - **Capabilities** ([`capability`], [`manifest`], [`broker`], [`policy`]):
//!   signed component manifests, the default-deny policy engine, and the
//!   Layer 3 broker interface.
//! - **Storage & provenance** ([`artifacts`], [`registry`], [`audit`]):
//!   content-addressed artifacts, the capability registry, and the append-only
//!   hash-chained audit log.
//!
//! Error types live in [`error`].
#![warn(missing_docs)]

/// Minimal alerting primitive: metric → threshold → webhook.
pub mod alert;
/// Content-addressed artifact storage.
pub mod artifacts;
/// Append-only, hash-chained audit log.
pub mod audit;
/// Named bandwidth profiles for degraded-link streaming presets.
pub mod bandwidth;
/// Layer 3 broker request/response types and trait.
pub mod broker;
/// Capability groups and installed-capability descriptors.
pub mod capability;
/// Error types shared across the crate.
pub mod error;
/// The robot→operator event subscription wire model.
pub mod events;
/// Peer identity, keypairs, and the local peer registry.
pub mod identity;
/// Signed component manifests and the trust store.
pub mod manifest;
/// Wire message framing and control/tool/bulk protocol messages.
pub mod message;
/// One-line robot enrollment: the pairing-token trust model (`gang pair`).
pub mod pairing;
/// The default-deny capability policy engine.
pub mod policy;
/// Stream protocol identifiers.
pub mod protocol;
/// The capability registry.
pub mod registry;
/// The protocol-agnostic transport adapter trait and supporting types.
pub mod transport;
