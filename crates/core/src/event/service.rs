use std::sync::Arc;

use tokio::sync::broadcast;

use crate::event::model::AppEvent;

/// In-process broadcast bus for coarse [`AppEvent`]s. A single `mailkeep`
/// process fans events out to all connected SSE clients.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
pub trait EventService: Send + Sync {
    /// Subscribe to the event stream. Each subscriber receives every event
    /// published after it subscribed.
    fn subscribe(&self) -> broadcast::Receiver<AppEvent>;
    /// Publish that the account set changed. Best-effort: with no live
    /// subscribers the send is a no-op (not an error).
    fn notify_accounts_changed(&self);
}

pub(crate) struct EventServiceImpl {
    tx: broadcast::Sender<AppEvent>,
}

impl EventServiceImpl {
    #[must_use]
    pub fn new() -> Self {
        // Small buffer: subscribers only need the latest "changed" nudge, and a
        // lagged receiver coalesces to one re-fetch anyway.
        let (tx, _rx) = broadcast::channel(64);
        Self { tx }
    }
}

impl Default for EventServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl EventService for EventServiceImpl {
    fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }

    fn notify_accounts_changed(&self) {
        // Err means "no subscribers" — that's fine.
        let _ = self.tx.send(AppEvent::AccountsChanged);
    }
}

#[must_use]
pub fn create_event_service() -> Arc<dyn EventService> {
    Arc::new(EventServiceImpl::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_receives_published_event() {
        let svc = EventServiceImpl::new();
        let mut rx = svc.subscribe();
        svc.notify_accounts_changed();
        assert_eq!(rx.recv().await.unwrap(), AppEvent::AccountsChanged);
    }

    #[tokio::test]
    async fn multiple_subscribers_each_receive() {
        let svc = EventServiceImpl::new();
        let mut rx1 = svc.subscribe();
        let mut rx2 = svc.subscribe();
        svc.notify_accounts_changed();
        assert_eq!(rx1.recv().await.unwrap(), AppEvent::AccountsChanged);
        assert_eq!(rx2.recv().await.unwrap(), AppEvent::AccountsChanged);
    }

    #[test]
    fn notify_with_no_subscribers_is_noop() {
        let svc = EventServiceImpl::new();
        svc.notify_accounts_changed(); // must not panic
    }
}
