//! The live event bus. Pipeline stages emit Postgres `NOTIFY` messages; a
//! listener task (in the server binary) forwards them here, and SSE clients
//! subscribe. Payloads are small JSON strings (ids and counts, never content).

use tokio::sync::broadcast;

/// A cloneable handle to the in-process event fan-out.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<String>,
}

impl EventBus {
    #[must_use]
    pub fn new(tx: broadcast::Sender<String>) -> Self {
        Self { tx }
    }

    /// Subscribe to the stream of event payloads.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Publish an event payload. Dropped if there are no subscribers, which is
    /// fine: events are ephemeral progress, not durable state.
    pub fn publish(&self, payload: String) {
        let _ = self.tx.send(payload);
    }
}
