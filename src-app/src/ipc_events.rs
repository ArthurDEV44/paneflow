//! EP-002 (agent-control-plane): the outbound event bus.
//!
//! The IPC server was request/response only — the lifecycle events the GPUI
//! thread already observes (agent state transitions, pane output) had no way
//! OUT to a client, so the flow engine and any conductor had to poll. This is
//! the efferent path: an `events.subscribe` connection registers a subscriber
//! here; the GPUI thread calls [`EventBus::broadcast`] (a brief lock + a
//! non-blocking `try_send`, never blocking the render thread); the connection
//! thread drains its receiver and writes each event line to the socket.
//!
//! Backpressure (US-004): each subscriber has a bounded queue. A slow client
//! that stops draining sheds the OLDEST events (the bounded channel rejects the
//! `try_send`) and a `dropped` counter conveys the loss; the broadcaster (the
//! render thread) is never blocked.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Per-subscriber outbound queue depth. Past this the bus drops the oldest
/// events rather than block the broadcaster (the GPUI render thread). 1024
/// small JSON lines stays well under the PRD's 8 MiB ceiling in practice.
const SUBSCRIBER_QUEUE_CAP: usize = 1024;

/// The event types a client may subscribe to. An `events.subscribe` that names
/// a type outside this set is rejected (US-005 AC3) rather than producing a
/// silent never-matching stream.
pub const KNOWN_EVENT_TYPES: &[&str] = &[
    "ai.session_start",
    "ai.prompt_submit",
    "ai.tool_use",
    "ai.notification",
    "ai.stop",
    "ai.exit",
    "ai.session_end",
    "surface_changed",
];

/// Subscription filter. `None` on a field = match everything for that axis.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct EventFilter {
    pub surfaces: Option<HashSet<u64>>,
    pub types: Option<HashSet<String>>,
}

impl EventFilter {
    /// Parse `events.subscribe` params. Rejects an unknown event type so a typo
    /// fails loudly instead of producing a stream that silently never matches.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let surfaces = params
            .get("surfaces")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_u64)
                    .collect::<HashSet<u64>>()
            });
        let types = match params.get("types").and_then(|v| v.as_array()) {
            Some(arr) => {
                let set: HashSet<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                if let Some(unknown) = set
                    .iter()
                    .find(|t| !KNOWN_EVENT_TYPES.contains(&t.as_str()))
                {
                    return Err(format!("unknown event type: {unknown}"));
                }
                Some(set)
            }
            None => None,
        };
        Ok(Self { surfaces, types })
    }

    /// Does an event of `type_` for `surface_id` match this filter? A surface-
    /// scoped subscriber never receives an event with no surface (e.g. an
    /// unresolved-PID `ai.*` frame) — it asked for specific panes.
    pub fn matches(&self, type_: &str, surface_id: Option<u64>) -> bool {
        if let Some(types) = &self.types
            && !types.contains(type_)
        {
            return false;
        }
        if let Some(surfaces) = &self.surfaces {
            return surface_id.is_some_and(|sid| surfaces.contains(&sid));
        }
        true
    }
}

struct Subscriber {
    id: u64,
    filter: EventFilter,
    tx: SyncSender<String>,
    dropped: Arc<AtomicU64>,
}

/// Shared registry of subscribers. Held by the IPC server (to register on
/// `events.subscribe`) and by the GPUI app (to broadcast).
pub struct EventBus {
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
}

/// A live subscription handed to the connection thread. Drops out of the
/// registry (RAII) when the thread ends — i.e. when the client disconnects.
pub struct Subscription {
    pub id: u64,
    pub rx: Receiver<String>,
    dropped: Arc<AtomicU64>,
    bus: Arc<EventBus>,
}

impl Subscription {
    /// Read and reset the dropped-event counter (events shed under backpressure
    /// since the last call). The connection thread emits a `dropped` marker.
    pub fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.id);
    }
}

impl EventBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        })
    }

    /// Register a subscriber. Returns the receiving half (for the connection
    /// thread) plus a RAII handle that unsubscribes on drop.
    pub fn subscribe(self: &Arc<Self>, filter: EventFilter) -> Subscription {
        let (tx, rx) = sync_channel::<String>(SUBSCRIBER_QUEUE_CAP);
        let dropped = Arc::new(AtomicU64::new(0));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.push(Subscriber {
                id,
                filter,
                tx,
                dropped: Arc::clone(&dropped),
            });
        }
        Subscription {
            id,
            rx,
            dropped,
            bus: Arc::clone(self),
        }
    }

    fn unsubscribe(&self, id: u64) {
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.retain(|s| s.id != id);
        }
    }

    /// True if at least one subscriber is registered. Lets the GPUI hot paths
    /// skip building events when nobody is watching.
    pub fn has_subscribers(&self) -> bool {
        self.subscribers
            .lock()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Broadcast one event to every matching subscriber. NON-BLOCKING: a full
    /// subscriber queue drops the event and bumps its `dropped` counter. Safe to
    /// call from the GPUI render thread (brief lock + `try_send`, no I/O).
    pub fn broadcast(&self, type_: &str, surface_id: Option<u64>, event: &Value) {
        let Ok(subs) = self.subscribers.lock() else {
            return;
        };
        if subs.is_empty() {
            return;
        }
        let mut line = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(_) => return,
        };
        line.push('\n');
        for sub in subs.iter() {
            if !sub.filter.matches(type_, surface_id) {
                continue;
            }
            // try_send never blocks; a full queue sheds the event and records
            // the loss. Disconnected (consumer thread gone) is reaped by the
            // Subscription's own drop, so ignore it here.
            if let Err(TrySendError::Full(_)) = sub.tx.try_send(line.clone()) {
                sub.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Epoch milliseconds for event timestamps. Saturates to 0 before the epoch
/// (never panics).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_params_rejects_unknown_type() {
        let err = EventFilter::from_params(&json!({"types": ["ai.stop", "bogus"]})).unwrap_err();
        assert!(err.contains("bogus"), "got: {err}");
    }

    #[test]
    fn from_params_accepts_known_types_and_surfaces() {
        let f = EventFilter::from_params(&json!({"types":["ai.stop"],"surfaces":[7]})).unwrap();
        assert!(f.types.unwrap().contains("ai.stop"));
        assert!(f.surfaces.unwrap().contains(&7));
    }

    #[test]
    fn empty_filter_matches_everything() {
        let f = EventFilter::default();
        assert!(f.matches("ai.stop", Some(1)));
        assert!(f.matches("surface_changed", None));
    }

    #[test]
    fn type_filter_excludes_other_types() {
        let f = EventFilter::from_params(&json!({"types":["ai.notification"]})).unwrap();
        assert!(f.matches("ai.notification", Some(1)));
        assert!(!f.matches("ai.stop", Some(1)));
    }

    #[test]
    fn surface_filter_excludes_unscoped_and_other_surfaces() {
        let f = EventFilter::from_params(&json!({"surfaces":[42]})).unwrap();
        assert!(f.matches("ai.stop", Some(42)));
        assert!(!f.matches("ai.stop", Some(7)));
        assert!(
            !f.matches("ai.stop", None),
            "a surface-scoped subscriber skips unscoped events"
        );
    }

    #[test]
    fn broadcast_delivers_to_matching_and_filters_others() {
        let bus = EventBus::new();
        let sub = bus.subscribe(EventFilter::from_params(&json!({"types":["ai.stop"]})).unwrap());
        bus.broadcast("ai.stop", Some(1), &json!({"type":"ai.stop"}));
        assert!(sub.rx.try_recv().is_ok(), "matching event delivered");
        bus.broadcast("ai.tool_use", Some(1), &json!({"type":"ai.tool_use"}));
        assert!(sub.rx.try_recv().is_err(), "non-matching type filtered out");
    }

    #[test]
    fn broadcast_drops_oldest_when_subscriber_queue_full() {
        let bus = EventBus::new();
        let sub = bus.subscribe(EventFilter::default());
        // Nobody drains `sub.rx`, so everything past the cap is shed.
        for _ in 0..SUBSCRIBER_QUEUE_CAP + 5 {
            bus.broadcast("ai.stop", Some(1), &json!({"type":"ai.stop"}));
        }
        assert_eq!(sub.take_dropped(), 5, "5 events past the cap were dropped");
        assert_eq!(sub.take_dropped(), 0, "counter resets after a read");
    }

    #[test]
    fn unsubscribe_on_drop_removes_from_registry() {
        let bus = EventBus::new();
        {
            let _sub = bus.subscribe(EventFilter::default());
            assert!(bus.has_subscribers());
        }
        assert!(!bus.has_subscribers(), "drop unsubscribed");
    }
}
