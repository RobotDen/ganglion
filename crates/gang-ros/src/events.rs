//! The robot-side event bus: a bounded, in-process fan-out of [`AgentEvent`]s
//! plus a bounded retained window for late/polling subscribers.
//!
//! # Resource model (why this is bounded everywhere)
//!
//! Two structures, both hard-bounded:
//!
//! - A [`tokio::sync::broadcast`] channel of capacity [`BUS_CAPACITY`] for
//!   genuine live consumers (the forthcoming `gang tui` and any push path). A
//!   consumer that falls behind does not grow the robot's memory: the channel
//!   drops the oldest items and the next receive surfaces a
//!   [`AgentEvent::Gap`] with the dropped count, then resumes. A stalled
//!   subscriber can therefore never block the agent or pin unbounded memory.
//!
//! - A [`std::collections::VecDeque`] ring of at most [`RING_CAPACITY`] recent
//!   events, so a request-response subscriber (`gang logs`, `gang connect`,
//!   `gang list`) can fetch "recent context" and resume by sequence number.
//!   When a polling subscriber's cursor is older than the retained window, the
//!   bus reports a [`AgentEvent::Gap`] instead of silently skipping events.
//!
//! Nothing on this path is unbounded. Emission is non-blocking and lock-scoped.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::broadcast;

use gang_core::events::{AgentEvent, EventSeq};

/// Capacity of the live broadcast channel. A lagging live consumer past this
/// many buffered events observes a [`AgentEvent::Gap`] and resumes.
pub const BUS_CAPACITY: usize = 256;

/// Number of recent events retained for polling/late subscribers.
pub const RING_CAPACITY: usize = 256;

/// A bounded, in-process event bus owned by the [`crate::agent::RobotAgent`].
///
/// Cloneable handles are cheap (`Arc`-free: the shared state lives behind the
/// channel sender and a mutex). Emission stamps a monotonic sequence, retains
/// the event in a bounded ring, and fans it out to live subscribers.
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
    next_seq: AtomicU64,
    ring: Mutex<VecDeque<AgentEvent>>,
    ring_capacity: usize,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(BUS_CAPACITY, RING_CAPACITY)
    }
}

impl EventBus {
    /// Create a bus with explicit capacities (for tests). Production code uses
    /// [`EventBus::default`].
    pub fn new(bus_capacity: usize, ring_capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(bus_capacity.max(1));
        Self {
            tx,
            next_seq: AtomicU64::new(0),
            ring: Mutex::new(VecDeque::with_capacity(ring_capacity)),
            ring_capacity,
        }
    }

    /// Emit an event. The closure receives the freshly-allocated sequence
    /// number and returns the fully-built event; the bus retains it in the
    /// bounded ring and fans it out to live subscribers.
    ///
    /// Non-blocking: a send with no receivers, or a full broadcast buffer, is
    /// not an error here — slow live consumers are handled at receive time via
    /// [`AgentEvent::Gap`].
    pub fn publish(&self, make: impl FnOnce(EventSeq) -> AgentEvent) {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let event = make(seq);
        {
            let mut ring = self.ring.lock().expect("event ring poisoned");
            if ring.len() >= self.ring_capacity {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        // Ignore the error: no live subscribers is normal. Lagging subscribers
        // are surfaced as Gap on their receive side, not here.
        let _ = self.tx.send(event);
    }

    /// The sequence that will be assigned to the NEXT event (i.e. one past the
    /// last emitted). Used to stamp a presence snapshot at the current tip.
    pub fn tip(&self) -> EventSeq {
        self.next_seq.load(Ordering::SeqCst)
    }

    /// Subscribe to the live feed. The returned [`EventSubscription`] converts
    /// broadcast lag into a [`AgentEvent::Gap`] so a slow consumer resumes
    /// cleanly instead of erroring or growing memory.
    pub fn subscribe(&self) -> EventSubscription {
        EventSubscription {
            rx: self.tx.subscribe(),
        }
    }

    /// Return retained events with `seq` strictly greater than `since` (or all
    /// retained events when `since` is `None`), capped at `max`.
    ///
    /// The returned `dropped` count is non-zero when `since` predates the
    /// oldest retained event: that many events were evicted from the window
    /// before the caller could see them, and the caller should surface a
    /// [`AgentEvent::Gap`] before the returned events.
    pub fn recent_since(&self, since: Option<EventSeq>, max: usize) -> RecentBatch {
        let ring = self.ring.lock().expect("event ring poisoned");

        let oldest_retained = ring.front().and_then(|e| e.seq());

        let mut dropped = 0u64;
        let mut selected: Vec<AgentEvent> = match since {
            None => ring.iter().cloned().collect(),
            Some(cursor) => {
                // If the cursor is older than what we still retain, some events
                // after the cursor were already evicted.
                if let Some(oldest) = oldest_retained
                    && oldest > cursor + 1
                {
                    dropped = oldest - (cursor + 1);
                }
                ring.iter()
                    .filter(|e| e.seq().map(|s| s > cursor).unwrap_or(false))
                    .cloned()
                    .collect()
            }
        };

        // Cap the batch to `max`, keeping the NEWEST events (a viewer wants the
        // freshest context). Anything trimmed from the front is a gap too.
        if selected.len() > max {
            let trim = selected.len() - max;
            dropped += trim as u64;
            selected.drain(0..trim);
        }

        RecentBatch {
            dropped,
            events: selected,
        }
    }
}

/// Why a subscription request was refused before any event was streamed.
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    /// The subscriber is not in the configured trust store. When a trust store
    /// is configured, only trusted operators may subscribe (the same rule
    /// deploy enforces). An empty trust store is the dev-permissive path.
    #[error("peer {peer} is not authorized to subscribe to the event feed")]
    Unauthorized {
        /// The rejected subscriber's gang id.
        peer: String,
    },
}

/// A batch of retained events plus a count of events dropped before them.
#[derive(Debug, Clone)]
pub struct RecentBatch {
    /// Events evicted before the returned window (surface as a `Gap`).
    pub dropped: u64,
    /// The retained events, oldest first.
    pub events: Vec<AgentEvent>,
}

/// A live subscription to the event bus.
///
/// [`EventSubscription::recv`] translates a broadcast lag into a single
/// [`AgentEvent::Gap`] and then resumes delivery, so a slow consumer never
/// errors out and the robot never buffers without bound on its behalf.
pub struct EventSubscription {
    rx: broadcast::Receiver<AgentEvent>,
}

impl EventSubscription {
    /// Receive the next event, or `None` once the bus is dropped. A lagged
    /// receiver yields a [`AgentEvent::Gap`] with the dropped count, then
    /// resumes on the following calls.
    pub async fn recv(&mut self) -> Option<AgentEvent> {
        match self.rx.recv().await {
            Ok(ev) => Some(ev),
            Err(broadcast::error::RecvError::Lagged(n)) => Some(AgentEvent::Gap { dropped: n }),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn heartbeat(bus: &EventBus) {
        bus.publish(|seq| AgentEvent::Heartbeat {
            seq,
            ts: Utc::now(),
            uptime_secs: 1,
        });
    }

    #[test]
    fn recent_since_returns_all_then_resumes_by_cursor() {
        let bus = EventBus::new(256, 256);
        for _ in 0..5 {
            heartbeat(&bus);
        }
        // Fresh: all 5 retained.
        let all = bus.recent_since(None, 256);
        assert_eq!(all.events.len(), 5);
        assert_eq!(all.dropped, 0);
        assert_eq!(all.events.first().unwrap().seq(), Some(0));

        // Resume after seq 2: only 3, 4 remain (2 events: seq 3, 4).
        let tail = bus.recent_since(Some(2), 256);
        assert_eq!(tail.events.len(), 2);
        assert_eq!(tail.events[0].seq(), Some(3));
        assert_eq!(tail.dropped, 0);
    }

    #[test]
    fn recent_since_reports_gap_when_cursor_predates_window() {
        // Ring holds only 4 events; publish 10 so the first 6 are evicted.
        let bus = EventBus::new(256, 4);
        for _ in 0..10 {
            heartbeat(&bus);
        }
        // Oldest retained is seq 6; a cursor of 1 means seqs 2..=5 were dropped.
        let batch = bus.recent_since(Some(1), 256);
        assert_eq!(batch.dropped, 4, "seqs 2,3,4,5 dropped before the window");
        assert_eq!(batch.events.first().unwrap().seq(), Some(6));
        assert_eq!(batch.events.len(), 4);
    }

    #[test]
    fn recent_since_caps_batch_and_counts_trim_as_dropped() {
        let bus = EventBus::new(256, 256);
        for _ in 0..10 {
            heartbeat(&bus);
        }
        let batch = bus.recent_since(None, 3);
        assert_eq!(batch.events.len(), 3);
        // The 7 older events were trimmed to honor the cap.
        assert_eq!(batch.dropped, 7);
        // The newest 3 (seq 7,8,9) are kept.
        assert_eq!(batch.events[0].seq(), Some(7));
        assert_eq!(batch.events[2].seq(), Some(9));
    }

    #[tokio::test]
    async fn slow_live_consumer_gets_gap_then_resumes() {
        // Small live channel: a consumer that does not receive falls behind.
        let bus = EventBus::new(8, 256);
        let mut sub = bus.subscribe();

        // Publish far more than the channel can hold without any receive.
        for _ in 0..100 {
            heartbeat(&bus);
        }

        // The first receive must be a Gap reporting the lag, not an error.
        match sub.recv().await {
            Some(AgentEvent::Gap { dropped }) => {
                assert!(dropped >= 1, "expected a positive dropped count");
            }
            other => panic!("expected a Gap marker on lag, got {other:?}"),
        }

        // After the gap marker, the subscription resumes delivering the
        // retained events (bounded by the channel capacity) rather than
        // erroring or hanging.
        match sub.recv().await {
            Some(AgentEvent::Heartbeat { .. }) => {}
            other => panic!("expected delivery to resume after gap, got {other:?}"),
        }
    }

    #[test]
    fn tip_advances_with_emission() {
        let bus = EventBus::new(256, 256);
        assert_eq!(bus.tip(), 0);
        heartbeat(&bus);
        heartbeat(&bus);
        assert_eq!(bus.tip(), 2);
    }
}
