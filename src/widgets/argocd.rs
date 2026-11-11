use async_trait::async_trait;
use serde::Deserialize;

use super::{pick_fields, Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};

const DEFAULT_FIELDS: [&str; 4] = ["apps", "synced", "outOfSync", "healthy"];

pub struct Argocd;

#[derive(Deserialize)]
struct Applications {
    #[serde(default)]
    items: Option<Vec<Application>>,
}

#[derive(Deserialize)]
struct Application {
    #[serde(default)]
    status: Status,
}

#[derive(Deserialize, Default)]
struct Status {
    #[serde(default)]
    sync: StatusValue,
    #[serde(default)]
    health: StatusValue,
}

#[derive(Deserialize, Default)]
struct StatusValue {
    #[serde(default)]
    status: Option<String>,
}

fn label(field: &str) -> Option<&'static str> {
    Some(match field {
        "apps" => "Apps",
        "synced" => "Synced",
        "outOfSync" => "Out Of Sync",
        "healthy" => "Healthy",
        "degraded" => "Degraded",
        "progressing" => "Progressing",
        "missing" => "Missing",
        "suspended" => "Suspended",
        _ => return None,
    })
}

#[async_trait]
impl Widget for Argocd {
    fn kind(&self) -> &'static str {
        "argocd"
    }

    fn labels(&self, cfg: &WidgetConfig) -> Vec<&'static str> {
        pick_fields(cfg, &DEFAULT_FIELDS, 4)
            .iter()
            .filter_map(|f| label(f))
            .collect()
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        // The unfiltered list is megabytes on a mid-sized cluster; only the
        // sync and health status are needed. (`items.status.health.status`
        // is dropped by Argo CD's field filter, `items.status.health` is not.)
        let url = format!(
            "{}/api/v1/applications?fields=items.status.sync.status,items.status.health",
            cfg.url()?
        );
        let request =
            Request::get(url).header("Authorization", format!("Bearer {}", cfg.require_key()?));
        let apps: Applications = http.send(request).await?.ok()?.json()?;
        let items = apps.items.unwrap_or_default();

        let blocks = pick_fields(cfg, &DEFAULT_FIELDS, 4)
            .iter()
            .filter_map(|field| {
                let label = label(field)?;
                let count = if field == "apps" {
                    items.len()
                } else {
                    let matches = |s: &StatusValue| {
                        s.status
                            .as_deref()
                            .is_some_and(|v| v.eq_ignore_ascii_case(field))
                    };
                    items
                        .iter()
                        .filter(|a| matches(&a.status.sync) || matches(&a.status.health))
                        .count()
                };
                Some(Block::new(label, count.to_string()))
            })
            .collect::<Vec<_>>();
        Ok(blocks.into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const APPS: &str = include_str!("../../fixtures/argocd/applications.json");

    #[tokio::test]
    async fn counts_sync_and_health_states() {
        let http = FakeHttp::new().on(Method::Get, "/api/v1/applications", 200, APPS);
        let cfg = config(&[("url", "http://argocd-server.argocd.svc/"), ("key", "tok")]);
        let data = Argocd.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Apps", "8".into()),
                ("Synced", "8".into()),
                ("Out Of Sync", "0".into()),
                ("Healthy", "5".into()),
            ]
        );
        assert_eq!(http.header(0, "Authorization").unwrap(), "Bearer tok");
        let url = &http.requests()[0].url;
        assert!(url.starts_with("http://argocd-server.argocd.svc/api/v1/applications?fields="));
    }

    #[tokio::test]
    async fn honours_configured_fields() {
        let http = FakeHttp::new().on(Method::Get, "/api/v1/applications", 200, APPS);
        let cfg = config(&[
            ("url", "http://argo"),
            ("key", "tok"),
            ("fields", r#"["degraded","apps"]"#),
        ]);
        let data = Argocd.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![("Degraded", "3".into()), ("Apps", "8".into())]
        );
        assert_eq!(Argocd.labels(&cfg), vec!["Degraded", "Apps"]);
    }

    #[tokio::test]
    async fn reports_http_errors_without_the_url() {
        let http = FakeHttp::new().on(Method::Get, "/api/v1/applications", 401, "{}");
        let cfg = config(&[("url", "http://argo"), ("key", "bad")]);
        let err = Argocd.fetch(&http, &cfg).await.unwrap_err();
        assert_eq!(err.0, "HTTP 401");
    }

    #[tokio::test]
    async fn requires_a_key() {
        let http = FakeHttp::new();
        let err = Argocd
            .fetch(&http, &config(&[("url", "http://argo")]))
            .await
            .unwrap_err();
        assert_eq!(err.0, "widget.key is not set");
        assert!(http.requests().is_empty());
    }
}
