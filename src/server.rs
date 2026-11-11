//! HTTP endpoints. The public listener serves the dashboard; the ops
//! listener serves health and metrics so they are not exposed by the Ingress.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::Stream;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

use crate::render;
use crate::state::{AppState, Event};

const STYLE: &str = include_str!("../assets/style.css");
const SCRIPT: &str = include_str!("../assets/app.js");
const FAVICON: &str = include_str!("../assets/favicon.svg");
const CSP: &str = "default-src 'none'; img-src * data:; style-src 'self' 'unsafe-inline'; \
                   script-src 'self'; connect-src 'self'; form-action *; base-uri 'none'; \
                   frame-ancestors 'none'";

pub fn public_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/events", get(events))
        .route("/api/state", get(api_state))
        .route(
            "/assets/style.css",
            get(|| asset("text/css; charset=utf-8", STYLE)),
        )
        .route(
            "/assets/app.js",
            get(|| asset("text/javascript; charset=utf-8", SCRIPT)),
        )
        .route("/favicon.svg", get(|| asset("image/svg+xml", FAVICON)))
        .route("/healthz", get(healthz))
        .with_state(state)
}

pub fn ops_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .with_state(state)
}

async fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            ),
        ],
        body,
    )
        .into_response()
}

async fn healthz() -> &'static str {
    "ok"
}

async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("application/openmetrics-text; version=1.0.0; charset=utf-8"),
        )],
        state.metrics.encode(),
    )
        .into_response()
}

async fn index(State(state): State<Arc<AppState>>) -> Response {
    state.touch();
    state.metrics.page_views.inc();
    // Make sure the stored view reflects the latest state before serving it.
    render::refresh_view(&state);
    (
        [
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                HeaderName::from_static("content-security-policy"),
                HeaderValue::from_static(CSP),
            ),
            (
                HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            ),
            (
                HeaderName::from_static("referrer-policy"),
                HeaderValue::from_static("no-referrer"),
            ),
        ],
        Html(render::page(&state)),
    )
        .into_response()
}

async fn api_state(State(state): State<Arc<AppState>>) -> Response {
    state.touch();
    (
        [
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
        ],
        render::snapshot(&state).to_string(),
    )
        .into_response()
}

#[derive(Deserialize)]
struct EventsQuery {
    /// The view version the page was rendered from.
    #[serde(default)]
    v: u64,
}

/// Counts a connected client for as long as its stream lives.
struct ClientGuard(Arc<AppState>);

impl ClientGuard {
    fn new(state: Arc<AppState>) -> Self {
        state.touch();
        let clients = state.sse_clients.fetch_add(1, Ordering::Relaxed) + 1;
        state.metrics.sse_clients.set(clients as i64);
        state.wake.notify_waiters();
        Self(state)
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let clients = self.0.sse_clients.fetch_sub(1, Ordering::Relaxed) - 1;
        self.0.metrics.sse_clients.set(clients as i64);
        // Keep polling for the idle window in case of a quick reconnect.
        self.0.touch();
    }
}

fn to_sse(event: &Event) -> SseEvent {
    match event {
        Event::Fragment { id, html } => SseEvent::default()
            .event("fragment")
            .data(format!("{id}\n{html}")),
        Event::Reload => SseEvent::default().event("reload").data("layout"),
    }
}

struct EventStream {
    receiver: Receiver<Event>,
    backlog: VecDeque<Event>,
    _guard: ClientGuard,
}

async fn events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    // Subscribe before comparing versions so no update can fall in between.
    let receiver = state.events.subscribe();
    let mut backlog = VecDeque::new();
    if query.v != state.version.load(Ordering::Relaxed) {
        // The page is older than the current view (or this is a reconnect):
        // resend everything.
        let view = state.view.lock().unwrap();
        for (id, html) in &view.fragments {
            backlog.push_back(Event::Fragment {
                id: id.clone(),
                html: html.clone(),
            });
        }
    }
    let stream = EventStream {
        receiver,
        backlog,
        _guard: ClientGuard::new(state),
    };
    let stream = futures::stream::unfold(stream, |mut s| async move {
        if let Some(event) = s.backlog.pop_front() {
            return Some((Ok(to_sse(&event)), s));
        }
        match s.receiver.recv().await {
            Ok(event) => Some((Ok(to_sse(&event)), s)),
            // Too far behind to catch up fragment by fragment.
            Err(RecvError::Lagged(_)) => Some((Ok(to_sse(&Event::Reload)), s)),
            Err(RecvError::Closed) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}

pub async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
