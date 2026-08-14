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

pub mod alert;
pub mod artifacts;
/// Append-only, hash-chained audit log.
pub mod audit;
pub mod bandwidth;
/// Layer 3 broker request/response types and trait.
pub mod broker;
/// Capability groups and installed-capability descriptors.
pub mod capability;
pub mod error;
pub mod events;
/// Peer identity, keypairs, and the local peer registry.
pub mod identity;
/// Signed component manifests and the trust store.
pub mod manifest;
/// Wire message framing and control/tool/bulk protocol messages.
pub mod message;
pub mod pairing;
/// The default-deny capability policy engine.
pub mod policy;
/// Stream protocol identifiers.
pub mod protocol;
pub mod registry;
/// The robot→operator live topic streaming wire model.
pub mod topics;
/// The protocol-agnostic transport adapter trait and supporting types.
pub mod transport;
