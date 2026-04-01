# ADR-009: Outbound-initiated connectivity

**Status:** Accepted
**Date:** 2026-04-23 (retrospective — decision made at design phase)

## Context

Robots are deployed inside customer networks that the fleet operator does not own and cannot configure. Common constraints:

- NAT: no inbound connections possible
- Firewalls: only specific outbound ports allowed (often only TCP 443)
- CGNAT: carrier-grade NAT with symmetric mapping defeats hole-punching
- Air-gapped: no persistent network connection at all

Design principle #3: "Outbound-initiated by default. Robots dial out. Operators reach robots via a shared broker. Inbound connectivity is never assumed."

## Decision

Robots always initiate outbound connections to known relay nodes. Operators connect to the same relays. Communication is brokered through libp2p circuit relay v2. When possible, DCUtR (Direct Connection Upgrade through Relay) upgrades the relay connection to a direct peer-to-peer channel.

The five network archetypes encode this pattern:
- **Open warehouse:** Direct connection succeeds; relay optional
- **NAT'd office:** Relay + DCUtR upgrade
- **Enterprise DMZ:** Relay on TCP 443 only; DCUtR fails (UDP blocked)
- **Regulated facility:** No network; offline signed bundles
- **Mobile/CGNAT:** Relay only; DCUtR fails (symmetric NAT)

## Consequences

- **Positive:** Works in every network archetype except fully air-gapped (which uses offline bundles instead).
- **Positive:** No firewall configuration required on the customer side — outbound TCP/QUIC to a known relay is sufficient.
- **Positive:** Relay nodes can be operated by the fleet vendor, the customer, or a third party.
- **Negative:** Relay dependency for the common case. If all relays are down, connectivity is lost. Mitigated by supporting multiple relays and automatic failover.
- **Negative:** Relay-mediated traffic has higher latency than direct connections. Acceptable for diagnostic and management operations; not suitable for real-time teleoperation.
