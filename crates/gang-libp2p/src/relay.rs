/// Relay operation mode for a Ganglion node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayMode {
    /// This node does not provide relay services. It may use relays as a client.
    Client,
    /// This node acts as a circuit relay v2 server, enabling other nodes to
    /// connect through it. Used for the bootstrap relay at relay.gang.tafy.dev
    /// or self-hosted relays.
    Server,
}
