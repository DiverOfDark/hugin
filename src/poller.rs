//! Background refresh loops. All of them pause while nobody is looking at
//! the dashboard (see [`AppState::is_active`]).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics::{FetchLabels, ResourceLabels, SourceLabels, WidgetLabels};
use crate::model::{Service, WidgetSpec};
use crate::secrets::SecretReader;
use crate::state::{AppState, WidgetState};
use crate::widgets::{Http, WidgetConfig};

const CONCURRENT_WIDGETS: usize = 6;
const SENSITIVE: [&str; 3] = ["key", "password", "token"];

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
pub async fn resolve(
    spec: &WidgetSpec,
    secrets: &SecretReader,
) -> Result<(WidgetConfig, CredentialSource), String> {
    let mut params: BTreeMap<String, String> = spec.params.clone();
    let mut from_secret = false;
    let mut fell_back = false;
    for (field, reference) in &spec.secrets {
        match secrets.get(reference).await {
            Ok(value) => {
                params.insert(field.clone(), value);
                from_secret = true;
            }
            Err(reason) if params.contains_key(field) => {
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
    let has_plain_credential = params
        .keys()
        .any(|k| SENSITIVE.contains(&k.as_str()) && !spec.secrets.contains_key(k));
    let source = if fell_back || has_plain_credential {
        CredentialSource::Annotation
    } else if from_secret {
        CredentialSource::Secret
    } else {
        CredentialSource::None
    };
    Ok((WidgetConfig { params }, source))
}

async fn refresh_widget(
    state: &AppState,
    http: &dyn Http,
    secrets: &SecretReader,
    service: &Service,
    spec: &WidgetSpec,
) -> CredentialSource {
    let started = Instant::now();
    let mut source = CredentialSource::None;
    let result = match state.registry.get(&spec.kind) {
        None => Err(format!("unsupported widget type \"{}\"", spec.kind)),
        Some(widget) => match resolve(spec, secrets).await {
            Err(reason) => Err(reason),
            Ok((cfg, resolved_source)) => {
                source = resolved_source;
                // A widget makes a handful of sequential calls at most.
                let budget = Duration::from_secs(state.cfg.intervals.request_timeout_seconds * 3);
                match tokio::time::timeout(budget, widget.fetch(http, &cfg)).await {
                    Ok(Ok(data)) => Ok(data),
                    Ok(Err(error)) => Err(error.0),
                    Err(_) => Err("timeout".to_string()),
                }
            }
        },
    };

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

    let new_state = match result {
        Ok(data) => WidgetState::Data(data),
        Err(reason) => {
            tracing::debug!(service = %service.name, widget = %spec.kind, reason, "widget refresh failed");
            WidgetState::Error(reason)
        }
    };
    let changed = {
        let mut widgets = state.widgets.lock().unwrap();
        widgets.insert(service.id.clone(), new_state.clone()) != Some(new_state)
    };
    if changed {
        state.dirty.notify_one();
    }
    source
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
                refresh_widget(state, http.as_ref(), secrets, service, spec).await
            }
        });
        let sources: Vec<CredentialSource> = futures::future::join_all(refreshes).await;

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
        }

        tokio::time::sleep(interval).await;
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
        assert_eq!(cfg.key(), Some("plain"));
        assert_eq!(source, CredentialSource::Annotation);
    }

    #[tokio::test]
    async fn unreadable_secret_without_fallback_is_an_error() {
        let err = resolve(&spec(&[("url", "http://a")], &["key"]), &offline())
            .await
            .unwrap_err();
        assert_eq!(err, "no cluster access");
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
