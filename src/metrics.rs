//! Prometheus metrics, served on the ops port.

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct FetchLabels {
    pub widget: String,
    pub outcome: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct WidgetLabels {
    pub widget: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ResourceLabels {
    pub resource: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SourceLabels {
    pub source: &'static str,
}

pub struct Metrics {
    registry: Registry,
    pub widget_fetches: Family<FetchLabels, Counter>,
    pub widget_fetch_seconds: Family<WidgetLabels, Histogram>,
    pub watch_errors: Family<ResourceLabels, Counter>,
    pub poll_errors: Family<ResourceLabels, Counter>,
    /// Widgets by where their credentials came from: "secret", "annotation"
    /// (plaintext fallback) or "none". "annotation" at zero means the
    /// plaintext annotations can be removed.
    pub widget_credentials: Family<SourceLabels, Gauge>,
    pub services: Gauge,
    pub sse_clients: Gauge,
    pub polling_active: Gauge,
    pub page_views: Counter,
    resident_memory: Gauge,
}

impl Default for Metrics {
    fn default() -> Self {
        let mut registry = Registry::default();
        let widget_fetches = Family::<FetchLabels, Counter>::default();
        let widget_fetch_seconds = Family::<WidgetLabels, Histogram>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.01, 2.0, 11))
        });
        let watch_errors = Family::<ResourceLabels, Counter>::default();
        let poll_errors = Family::<ResourceLabels, Counter>::default();
        let widget_credentials = Family::<SourceLabels, Gauge>::default();
        let services = Gauge::default();
        let sse_clients = Gauge::default();
        let polling_active = Gauge::default();
        let page_views = Counter::default();
        let resident_memory = Gauge::default();

        registry.register(
            "hugin_widget_fetches",
            "Widget upstream fetches by outcome",
            widget_fetches.clone(),
        );
        registry.register(
            "hugin_widget_fetch_seconds",
            "Duration of one widget refresh, all upstream calls included",
            widget_fetch_seconds.clone(),
        );
        registry.register(
            "hugin_watch_errors",
            "Kubernetes watch stream errors by resource",
            watch_errors.clone(),
        );
        registry.register(
            "hugin_poll_errors",
            "Failed periodic refreshes (node and pod metrics, weather)",
            poll_errors.clone(),
        );
        registry.register(
            "hugin_widget_credentials",
            "Widgets by credential source: secret, annotation (plaintext fallback) or none",
            widget_credentials.clone(),
        );
        registry.register(
            "hugin_services",
            "Services on the dashboard",
            services.clone(),
        );
        registry.register(
            "hugin_sse_clients",
            "Connected live-update clients",
            sse_clients.clone(),
        );
        registry.register(
            "hugin_polling_active",
            "1 while upstreams are being polled, 0 when nobody is watching",
            polling_active.clone(),
        );
        registry.register(
            "hugin_page_views",
            "Dashboard page renders",
            page_views.clone(),
        );
        registry.register(
            "process_resident_memory_bytes",
            "Resident memory size in bytes",
            resident_memory.clone(),
        );

        Self {
            registry,
            widget_fetches,
            widget_fetch_seconds,
            watch_errors,
            poll_errors,
            widget_credentials,
            services,
            sse_clients,
            polling_active,
            page_views,
            resident_memory,
        }
    }
}

/// Resident set size from `/proc/self/statm` (second field, in pages).
fn resident_bytes() -> Option<i64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: i64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

impl Metrics {
    pub fn encode(&self) -> String {
        if let Some(rss) = resident_bytes() {
            self.resident_memory.set(rss);
        }
        let mut out = String::new();
        if prometheus_client::encoding::text::encode(&mut out, &self.registry).is_err() {
            out.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposition_contains_registered_series() {
        let m = Metrics::default();
        m.widget_fetches
            .get_or_create(&FetchLabels {
                widget: "argocd".into(),
                outcome: "ok",
            })
            .inc();
        m.widget_fetch_seconds
            .get_or_create(&WidgetLabels {
                widget: "argocd".into(),
            })
            .observe(0.2);
        m.services.set(38);
        let text = m.encode();
        assert!(text.contains(r#"hugin_widget_fetches_total{widget="argocd",outcome="ok"} 1"#));
        assert!(text.contains("hugin_services 38"));
        assert!(text.contains("# TYPE hugin_widget_fetch_seconds histogram"));
        assert!(text.trim_end().ends_with("# EOF"));
    }
}
