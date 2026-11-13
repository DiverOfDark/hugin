//! Background refresh loops. All of them pause while nobody is looking at
//! the dashboard (see [`AppState::is_active`]).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics::{FetchLabels, ResourceLabels, SourceLabels, WidgetLabels};
use crate::model::{Service, WidgetSpec};
use crate::secrets::SecretReader;
use crate::state::{AppState, Resolved, WidgetState};
use crate::widgets::{Http, Widget, WidgetConfig, WidgetData};

const CONCURRENT_WIDGETS: usize = 6;
const SENSITIVE: [&str; 3] = ["key", "password", "token"];
/// Failed refreshes in a row during which the last good values stay up.
const STALE_FAILURES: u32 = 2;

/// Where a widget's credentials came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Secret,
    Annotation,
    None,
}

impl CredentialSource {
    fn label(self) -> &'static str {
        match self {
            CredentialSource::Secret => "secret",
            CredentialSource::Annotation => "annotation",
            CredentialSource::None => "none",
        }
    }
}

/// Merges plain parameters with referenced Secrets. A readable Secret wins;
/// when it cannot be read the plain parameter is the fallback, and without
/// one the widget fails with the reason.
///
/// Credentials end up in `WidgetConfig::secrets`, where scripts can use but
/// not read them: everything that came from a Secret, plus plain parameters
/// named key, password or token.
pub async fn resolve(
    spec: &WidgetSpec,
    secrets: &SecretReader,
) -> Result<(WidgetConfig, CredentialSource), String> {
    let mut cfg = WidgetConfig::default();
    let mut plain: BTreeMap<String, String> = spec.params.clone();
    let mut from_secret = false;
    let mut fell_back = false;
    for (field, reference) in &spec.secrets {
        match secrets.get(reference).await {
            Ok(value) => {
                plain.remove(field);
                cfg.secrets.insert(field.clone(), value);
                from_secret = true;
            }
            Err(reason) if plain.get(field).is_some_and(|v| !v.is_empty()) => {
                tracing::debug!(
                    field,
                    reason,
                    "secret unavailable, using plaintext parameter"
                );
                fell_back = true;
            }
            Err(reason) => return Err(reason),
        }
    }
    let mut has_plain_credential = false;
    for (name, value) in plain {
        if SENSITIVE.contains(&name.as_str()) {
            has_plain_credential |= !value.is_empty();
            cfg.secrets.insert(name, value);
        } else {
            cfg.params.insert(name, value);
        }
    }
    let source = if fell_back || has_plain_credential {
        CredentialSource::Annotation
    } else if from_secret {
        CredentialSource::Secret
    } else {
        CredentialSource::None
    };
    Ok((cfg, source))
}

/// Failures of the network path rather than of the service. After an idle
/// period the first requests of a cycle can hit these (cold DNS and
/// connection paths), and a second try moments later succeeds.
fn is_transport_failure(reason: &str) -> bool {
    matches!(reason, "timeout" | "connection failed" | "request failed")
}

/// Refreshes one widget. Returns the credential source and whether the
/// refresh should be tried again: on a transport failure, unless this is
/// the `last_try`, nothing is recorded yet.
async fn refresh_widget(
    state: &AppState,
    http: &Arc<dyn Http>,
    secrets: &SecretReader,
    service: &Service,
    spec: &WidgetSpec,
    last_try: bool,
) -> (CredentialSource, bool) {
    let started = Instant::now();
    let mut source = CredentialSource::None;
    let widget = match state.resolve_widget(service, &spec.kind) {
        Resolved::Script(widget) => widget,
        Resolved::Pending(reason) => {
            let new_state = WidgetState::Pending(reason);
            let mut widgets = state.widgets.lock().unwrap();
            if widgets.get(&service.id) != Some(&new_state) {
                widgets.insert(service.id.clone(), new_state);
                state.dirty.notify_one();
            }
            return (source, false);
        }
    };
    let result = match resolve(spec, secrets).await {
        Err(reason) => Err(reason),
        Ok((cfg, resolved_source)) => {
            source = resolved_source;
            // A widget makes a handful of sequential calls at most.
            let budget = Duration::from_secs(state.cfg.intervals.request_timeout_seconds * 3);
            match tokio::time::timeout(budget, widget.fetch(http.clone(), &cfg)).await {
                Ok(Ok(data)) => Ok(data),
                Ok(Err(error)) => Err(error.0),
                Err(_) => Err("timeout".to_string()),
            }
        }
    };

    if let (Err(reason), false) = (&result, last_try) {
        if is_transport_failure(reason) {
            return (source, true);
        }
    }

    let outcome = if result.is_ok() { "ok" } else { "error" };
    state
        .metrics
        .widget_fetches
        .get_or_create(&FetchLabels {
            widget: spec.kind.clone(),
            outcome,
        })
        .inc();
    state
        .metrics
        .widget_fetch_seconds
        .get_or_create(&WidgetLabels {
            widget: spec.kind.clone(),
        })
        .observe(started.elapsed().as_secs_f64());

    if record_result(state, &service.id, result) {
        state.dirty.notify_one();
    }
    (source, false)
}

/// Stores a refresh result and reports whether the card changed. A widget
/// that already shows values keeps them through [`STALE_FAILURES`] failed
/// refreshes in a row, so one dropped request does not flash an error.
fn record_result(state: &AppState, id: &str, result: Result<WidgetData, String>) -> bool {
    let mut widgets = state.widgets.lock().unwrap();
    let mut failures = state.widget_failures.lock().unwrap();
    match result {
        Ok(data) => {
            let recovered = failures.remove(id).is_some();
            let new_state = WidgetState::Data(data);
            let changed = widgets.get(id) != Some(&new_state);
            widgets.insert(id.to_string(), new_state);
            changed || recovered
        }
        Err(reason) => {
            tracing::debug!(id, reason, "widget refresh failed");
            let count = failures.get(id).map_or(0, |(n, _)| *n) + 1;
            let keep_values =
                count <= STALE_FAILURES && matches!(widgets.get(id), Some(WidgetState::Data(_)));
            if keep_values {
                failures.insert(id.to_string(), (count, reason));
                true
            } else {
                failures.remove(id);
                let new_state = WidgetState::Error(reason);
                let changed = widgets.get(id) != Some(&new_state);
                widgets.insert(id.to_string(), new_state);
                changed
            }
        }
    }
}

/// Hands freed heap pages back to the kernel. Every refresh builds and
/// drops a Lua VM per widget plus the decoded responses; without this the
/// allocator keeps the high-water mark of the largest response mapped.
fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only releases memory the allocator already holds
    // as free; it takes no pointers and is safe to call at any time.
    unsafe {
        libc::malloc_trim(0);
    }
}

pub async fn widgets_loop(state: Arc<AppState>, http: Arc<dyn Http>, secrets: Arc<SecretReader>) {
    let interval = Duration::from_secs(state.cfg.intervals.widgets_seconds.max(5));
    loop {
        state.wait_active().await;
        state.metrics.polling_active.set(1);

        let services: Vec<(Service, WidgetSpec)> = state
            .services()
            .into_iter()
            .filter_map(|s| s.widget.clone().map(|w| (s, w)))
            .collect();

        let limit = tokio::sync::Semaphore::new(CONCURRENT_WIDGETS);
        let refreshes = services.iter().map(|(service, spec)| {
            let (state, http, secrets, limit) = (&state, &http, &secrets, &limit);
            async move {
                let _permit = limit.acquire().await;
                refresh_widget(state, http, secrets, service, spec, false).await
            }
        });
        let first_pass = futures::future::join_all(refreshes).await;

        // Second and last try for the ones that failed on the way there.
        let retries = services
            .iter()
            .zip(&first_pass)
            .filter(|(_, (_, retry))| *retry)
            .map(|((service, spec), _)| {
                let (state, http, secrets, limit) = (&state, &http, &secrets, &limit);
                async move {
                    let _permit = limit.acquire().await;
                    refresh_widget(state, http, secrets, service, spec, true).await
                }
            });
        futures::future::join_all(retries).await;
        let sources: Vec<CredentialSource> =
            first_pass.into_iter().map(|(source, _)| source).collect();

        for source in [
            CredentialSource::Secret,
            CredentialSource::Annotation,
            CredentialSource::None,
        ] {
            let count = sources.iter().filter(|s| **s == source).count();
            state
                .metrics
                .widget_credentials
                .get_or_create(&SourceLabels {
                    source: source.label(),
                })
                .set(count as i64);
        }

        // Forget widgets whose service is gone.
        {
            let live: std::collections::HashSet<&str> =
                services.iter().map(|(s, _)| s.id.as_str()).collect();
            state
                .widgets
                .lock()
                .unwrap()
                .retain(|id, _| live.contains(id.as_str()));
            state
                .widget_failures
                .lock()
                .unwrap()
                .retain(|id, _| live.contains(id.as_str()));
        }

        release_freed_memory();
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = state.refresh_now.notified() => {}
        }
        if !state.is_active() {
            state.metrics.polling_active.set(0);
        }
    }
}

pub async fn resources_loop(state: Arc<AppState>, client: kube::Client) {
    let interval = Duration::from_secs(state.cfg.intervals.resources_seconds.max(5));
    loop {
        state.wait_active().await;
        if let Err(error) = crate::cluster::refresh_resources(&state, &client).await {
            state
                .metrics
                .poll_errors
                .get_or_create(&ResourceLabels {
                    resource: "metrics",
                })
                .inc();
            tracing::warn!(%error, "cannot refresh node and pod metrics");
        }
        tokio::time::sleep(interval).await;
    }
}

pub async fn weather_loop(state: Arc<AppState>, http: Arc<dyn Http>) {
    let Some(cfg) = state.cfg.header.weather.clone() else {
        return;
    };
    let cache = Duration::from_secs(cfg.cache_minutes.max(1) * 60);
    loop {
        state.wait_active().await;
        let wait = match crate::weather::fetch(http.as_ref(), &cfg).await {
            Ok(now) => {
                let changed = {
                    let mut weather = state.weather.lock().unwrap();
                    let changed = weather.as_ref() != Some(&now);
                    *weather = Some(now);
                    changed
                };
                if changed {
                    state.dirty.notify_one();
                }
                cache
            }
            Err(error) => {
                state
                    .metrics
                    .poll_errors
                    .get_or_create(&ResourceLabels {
                        resource: "weather",
                    })
                    .inc();
                tracing::warn!(%error, "cannot refresh weather");
                Duration::from_secs(60)
            }
        };
        tokio::time::sleep(wait).await;
    }
}

/// Re-renders after changes (debounced) and pushes what differs to the
/// connected clients.
pub async fn render_loop(state: Arc<AppState>) {
    loop {
        state.dirty.notified().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        crate::render::refresh_view(&state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SecretRef;

    fn spec(params: &[(&str, &str)], secrets: &[&str]) -> WidgetSpec {
        WidgetSpec {
            kind: "argocd".into(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            secrets: secrets
                .iter()
                .map(|field| {
                    (
                        field.to_string(),
                        SecretRef {
                            namespace: "ns".into(),
                            name: "s".into(),
                            key: "k".into(),
                        },
                    )
                })
                .collect(),
        }
    }

    // A reader without a cluster client: every Secret is unavailable.
    fn offline() -> SecretReader {
        SecretReader::new(None)
    }

    #[tokio::test]
    async fn unreadable_secret_falls_back_to_the_plaintext_annotation() {
        let (cfg, source) = resolve(
            &spec(&[("url", "http://a"), ("key", "plain")], &["key"]),
            &offline(),
        )
        .await
        .unwrap();
        assert_eq!(cfg.secrets["key"], "plain");
        assert!(
            !cfg.params.contains_key("key"),
            "credentials are not plain parameters"
        );
        assert_eq!(cfg.params["url"], "http://a");
        assert_eq!(source, CredentialSource::Annotation);
    }

    #[tokio::test]
    async fn unreadable_secret_without_fallback_is_an_error() {
        let err = resolve(&spec(&[("url", "http://a")], &["key"]), &offline())
            .await
            .unwrap_err();
        assert_eq!(err, "no cluster access");
    }

    #[test]
    fn last_good_values_survive_two_failures_then_the_error_shows() {
        use crate::widgets::Block;
        let state = AppState::new(crate::config::Config::default());
        let data = || WidgetData::from(vec![Block::new("Queries", "1")]);
        let shown = |state: &AppState| state.widgets.lock().unwrap().get("s").cloned();

        // A failure before any success is shown straight away.
        assert!(record_result(&state, "s", Err("timeout".into())));
        assert_eq!(shown(&state), Some(WidgetState::Error("timeout".into())));

        assert!(record_result(&state, "s", Ok(data())));
        assert!(
            !record_result(&state, "s", Ok(data())),
            "same values, no change"
        );

        for _ in 0..STALE_FAILURES {
            assert!(record_result(&state, "s", Err("timeout".into())));
            assert_eq!(shown(&state), Some(WidgetState::Data(data())));
        }
        assert_eq!(state.widget_failures.lock().unwrap()["s"].0, STALE_FAILURES);

        assert!(record_result(&state, "s", Err("timeout".into())));
        assert_eq!(shown(&state), Some(WidgetState::Error("timeout".into())));
        assert!(state.widget_failures.lock().unwrap().is_empty());

        // Recovery clears the stale marker.
        record_result(&state, "s", Ok(data()));
        record_result(&state, "s", Err("timeout".into()));
        assert!(
            record_result(&state, "s", Ok(data())),
            "stale marker removed"
        );
        assert!(state.widget_failures.lock().unwrap().is_empty());
    }

    /// Fails the first `failures` requests with a timeout, then answers.
    struct Flaky {
        failures: std::sync::atomic::AtomicUsize,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Http for Flaky {
        async fn send(
            &self,
            _request: crate::widgets::Request,
        ) -> Result<crate::widgets::Response, crate::widgets::WidgetError> {
            use std::sync::atomic::Ordering;
            self.calls.fetch_add(1, Ordering::Relaxed);
            let left = self.failures.load(Ordering::Relaxed);
            if left > 0 {
                self.failures.store(left - 1, Ordering::Relaxed);
                return Err(crate::widgets::WidgetError::new("timeout"));
            }
            Ok(crate::widgets::Response {
                status: 200,
                body: br#"{"http":{"routers":{"total":1},"services":{"total":2},"middlewares":{"total":3}}}"#.to_vec(),
                cookies: vec![],
            })
        }
    }

    #[tokio::test]
    async fn a_transport_failure_is_retried_before_anything_is_recorded() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let state = AppState::new(crate::config::Config::default());
        let service = Service {
            id: "s".into(),
            source: crate::model::Source {
                kind: crate::model::SourceKind::Static,
                name: "Traefik".into(),
            },
            group: "g".into(),
            name: "Traefik".into(),
            href: None,
            icon: None,
            description: None,
            weight: 0,
            kube: None,
            widget: None,
            probe: true,
        };
        let mut spec = spec(&[("url", "http://traefik")], &[]);
        spec.kind = "traefik".into();
        state.custom_widgets.lock().unwrap().insert(
            (state.own_namespace.clone(), "traefik".into()),
            crate::state::CustomWidget {
                widget: Some(Arc::new(crate::widgets::LuaWidget::new(
                    "traefik",
                    r#"local o = http.get("/api/overview"):check():json()
                       return { { label = "Routers", value = o.http.routers.total } }"#,
                ))),
                failure: None,
                description: None,
            },
        );
        let flaky = Arc::new(Flaky {
            failures: AtomicUsize::new(1),
            calls: AtomicUsize::new(0),
        });
        let http: Arc<dyn Http> = flaky.clone();

        let (_, retry) = refresh_widget(&state, &http, &offline(), &service, &spec, false).await;
        assert!(retry);
        assert!(
            state.widgets.lock().unwrap().is_empty(),
            "nothing recorded yet"
        );

        let (_, retry) = refresh_widget(&state, &http, &offline(), &service, &spec, true).await;
        assert!(!retry);
        assert!(matches!(
            state.widgets.lock().unwrap().get("s"),
            Some(WidgetState::Data(_))
        ));
        assert_eq!(flaky.calls.load(Ordering::Relaxed), 2);

        // On the last try a transport failure is recorded like any other.
        flaky.failures.store(5, Ordering::Relaxed);
        state.widgets.lock().unwrap().clear();
        let (_, retry) = refresh_widget(&state, &http, &offline(), &service, &spec, true).await;
        assert!(!retry);
        assert_eq!(
            state.widgets.lock().unwrap().get("s"),
            Some(&WidgetState::Error("timeout".into()))
        );
        // Service errors are not retried.
        assert!(!is_transport_failure("HTTP 500"));
    }

    #[tokio::test]
    async fn plain_key_counts_as_annotation_and_no_credentials_as_none() {
        let (_, source) = resolve(
            &spec(&[("url", "http://a"), ("key", "plain")], &[]),
            &offline(),
        )
        .await
        .unwrap();
        assert_eq!(source, CredentialSource::Annotation);
        let (_, source) = resolve(&spec(&[("url", "http://a")], &[]), &offline())
            .await
            .unwrap();
        assert_eq!(source, CredentialSource::None);
    }
}
