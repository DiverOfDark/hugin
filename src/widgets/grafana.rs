use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

pub struct Grafana;

#[derive(Deserialize)]
struct Stats {
    dashboards: i64,
    #[serde(default)]
    datasources: i64,
    #[serde(default)]
    alerts: i64,
}

#[derive(Deserialize)]
struct LegacyAlert {
    #[serde(default)]
    state: Option<String>,
}

fn request(cfg: &WidgetConfig, endpoint: &str) -> Result<Request, WidgetError> {
    let mut request = Request::get(format!("{}/api/{endpoint}", cfg.url()?));
    if let Some((user, pass)) = cfg.basic() {
        request = request.basic_auth(user, pass);
    }
    Ok(request)
}

#[async_trait]
impl Widget for Grafana {
    fn kind(&self) -> &'static str {
        "grafana"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec![
            "Dashboards",
            "Data Sources",
            "Total Alerts",
            "Alerts Triggered",
        ]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let stats: Stats = http
            .send(request(cfg, "admin/stats")?)
            .await?
            .ok()?
            .json()?;

        // `widget.version: 2` reads one alertmanager directly. Version 1 (the
        // default) tries the legacy alert API first and falls back to
        // Grafana's built-in alertmanager, which is what current Grafana
        // releases need because `/api/alerts` no longer exists.
        let triggered = if cfg.get("version") == Some("2") {
            let endpoint = match cfg.get("alerts") {
                Some("alertmanager") => "alertmanager/alertmanager/api/v2/alerts",
                _ => "alertmanager/grafana/api/v2/alerts",
            };
            let alerts: Vec<serde_json::Value> =
                http.send(request(cfg, endpoint)?).await?.ok()?.json()?;
            alerts.len()
        } else {
            let legacy = match http.send(request(cfg, "alerts")?).await {
                Ok(r) if r.status < 400 => r.json::<Vec<LegacyAlert>>().ok(),
                _ => None,
            };
            match legacy {
                Some(alerts) if !alerts.is_empty() => alerts
                    .iter()
                    .filter(|a| a.state.as_deref() == Some("alerting"))
                    .count(),
                legacy => {
                    let fallback = http
                        .send(request(cfg, "alertmanager/grafana/api/v2/alerts")?)
                        .await
                        .and_then(|r| r.ok())
                        .and_then(|r| r.json::<Vec<serde_json::Value>>());
                    match (legacy, fallback) {
                        (_, Ok(alerts)) => alerts.len(),
                        (Some(_), Err(_)) => 0,
                        (None, Err(e)) => return Err(e),
                    }
                }
            }
        };

        Ok(vec![
            Block::new("Dashboards", format::int(stats.dashboards)),
            Block::new("Data Sources", format::int(stats.datasources)),
            Block::new("Total Alerts", format::int(stats.alerts)),
            Block::new("Alerts Triggered", format::int(triggered as i64)),
        ]
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const STATS: &str = include_str!("../../fixtures/grafana/stats.json");
    const ALERTS_404: &str = include_str!("../../fixtures/grafana/alerts_404.json");
    const AM_ALERTS: &str = include_str!("../../fixtures/grafana/alertmanager_alerts.json");

    fn cfg() -> WidgetConfig {
        config(&[
            ("url", "https://grafana.example.com/"),
            ("username", "admin"),
            ("password", "pw"),
        ])
    }

    #[tokio::test]
    async fn modern_grafana_falls_back_to_builtin_alertmanager() {
        let http = FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, STATS)
            .on(
                Method::Get,
                "/api/alertmanager/grafana/api/v2/alerts",
                200,
                AM_ALERTS,
            )
            .on(Method::Get, "/api/alerts", 404, ALERTS_404);
        let data = Grafana.fetch(&http, &cfg()).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Dashboards", "25".into()),
                ("Data Sources", "3".into()),
                ("Total Alerts", "0".into()),
                ("Alerts Triggered", "0".into()),
            ]
        );
        // base64("admin:pw")
        assert_eq!(
            http.header(0, "Authorization").unwrap(),
            "Basic YWRtaW46cHc="
        );
    }

    #[tokio::test]
    async fn legacy_alert_api_counts_alerting_only() {
        let http = FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, STATS)
            .on(
                Method::Get,
                "/api/alerts",
                200,
                r#"[{"state":"alerting"},{"state":"ok"},{"state":"alerting"}]"#,
            );
        let data = Grafana.fetch(&http, &cfg()).await.unwrap();
        assert_eq!(data.blocks[3].value, "2");
    }

    #[tokio::test]
    async fn version_two_counts_alertmanager_alerts() {
        let http = FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, STATS)
            .on(
                Method::Get,
                "/api/alertmanager/alertmanager/api/v2/alerts",
                200,
                r#"[{"labels":{}},{"labels":{}}]"#,
            );
        let mut cfg = cfg();
        cfg.params.insert("version".into(), "2".into());
        cfg.params.insert("alerts".into(), "alertmanager".into());
        let data = Grafana.fetch(&http, &cfg).await.unwrap();
        assert_eq!(data.blocks[3].value, "2");
    }

    #[tokio::test]
    async fn fails_when_stats_are_forbidden() {
        let http = FakeHttp::new().on(Method::Get, "/api/admin/stats", 403, "{}");
        let err = Grafana.fetch(&http, &cfg()).await.unwrap_err();
        assert_eq!(err.0, "HTTP 403");
    }

    #[tokio::test]
    async fn fails_when_both_alert_sources_fail() {
        let http = FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, STATS)
            .on(Method::Get, "/api/alertmanager/grafana", 500, "")
            .on(Method::Get, "/api/alerts", 404, ALERTS_404);
        let err = Grafana.fetch(&http, &cfg()).await.unwrap_err();
        assert_eq!(err.0, "HTTP 500");
    }
}
