//! Server-Sent Events endpoint. Relays coarse `AppEvent`s from the core
//! `EventService` bus to connected browser `EventSource` clients, which
//! re-fetch on each nudge (MK-19). The event is contentless; per-user scoping
//! is enforced by the client's `list_accounts` re-fetch.

use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use axum_session_auth::Authentication;
use futures::stream::Stream;
use mk_core::{CoreServices, event::AppEvent};
use tokio::sync::broadcast::{Receiver, error::RecvError};

use crate::server::AuthSession;

/// Collapse a burst of events into a single client nudge. Also the emission
/// latency floor: even a lone change waits one window before being sent.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Keep-alive ping interval to defeat idle-proxy timeouts.
const KEEP_ALIVE_SECS: u64 = 30;

pub(crate) fn events_router() -> Router {
    Router::new().route("/api/v1/events", get(events_handler))
}

async fn events_handler(
    auth_session: AuthSession,
    Extension(core): Extension<Arc<CoreServices>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Auth gate: only authenticated users may open the stream.
    let authed = auth_session.current_user.as_ref().is_some_and(Authentication::is_authenticated);
    if !authed {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let rx = core.event_service.subscribe();
    Ok(Sse::new(account_event_stream(rx)).keep_alive(KeepAlive::new().interval(Duration::from_secs(KEEP_ALIVE_SECS))))
}

/// One SSE `accounts_changed` event per debounced burst. A lagged receiver is
/// treated as "changed"; a closed channel ends the stream.
fn account_event_stream(rx: Receiver<AppEvent>) -> impl Stream<Item = Result<Event, Infallible>> {
    futures::stream::unfold(rx, |mut rx| async move {
        // Wait for the first event of a burst (or end the stream).
        match rx.recv().await {
            Ok(AppEvent::AccountsChanged) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return None,
        }
        // Debounce: swallow further events for the fixed window, then emit once.
        drain_window(&mut rx, DEBOUNCE).await;
        let event = Event::default().event("accounts_changed").data("updated");
        Some((Ok(event), rx))
    })
}

/// Discard events until `window` elapses (fixed-window debounce). Stops early
/// if the channel closes.
async fn drain_window(rx: &mut Receiver<AppEvent>, window: Duration) {
    let deadline = tokio::time::sleep(window);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => return,
            r = rx.recv() => match r {
                Ok(_) | Err(RecvError::Lagged(_)) => {}

                Err(RecvError::Closed) => return,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use tokio::sync::broadcast;

    use super::*;

    #[tokio::test]
    async fn collapses_burst_into_single_event() {
        let (tx, rx) = broadcast::channel(16);
        let mut stream = Box::pin(account_event_stream(rx));
        for _ in 0..5 {
            tx.send(AppEvent::AccountsChanged).unwrap();
        }
        // The whole burst collapses into exactly one emitted SSE event.
        assert!(stream.next().await.is_some());
        // No second event: the next poll blocks (channel open, nothing new), so
        // a short wait times out rather than yielding another event.
        let second = tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await;
        assert!(second.is_err(), "expected the burst to collapse into a single event");
    }

    #[tokio::test]
    async fn ends_stream_when_channel_closed() {
        let (tx, rx) = broadcast::channel(16);
        let mut stream = Box::pin(account_event_stream(rx));
        drop(tx);
        assert!(stream.next().await.is_none());
    }
}
