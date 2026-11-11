use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

pub struct Prometheus;

#[derive(Deserialize)]
struct Targets {
    data: TargetData,
}

#[derive(Deserialize)]
struct TargetData {
    #[serde(rename = "activeTargets")]
    active_targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    #[serde(default)]
    health: String,
}

#[async_trait]
impl Widget for Prometheus {
    fn kind(&self) -> &'static str {
        "prometheus"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Targets Up", "Targets Down", "Total Targets"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let mut request = Request::get(format!("{}/api/v1/targets?state=active", cfg.url()?));
        if let Some((user, pass)) = cfg.basic() {
            request = request.basic_auth(user, pass);
        }
        let targets: Targets = http.send(request).await?.ok()?.json()?;
        let all = &targets.data.active_targets;
        let up = all.iter().filter(|t| t.health == "up").count();
        let down = all.iter().filter(|t| t.health == "down").count();
        Ok(vec![
            Block::new("Targets Up", format::int(up as i64)),
            Block::new("Targets Down", format::int(down as i64)),
            Block::new("Total Targets", format::int(all.len() as i64)),
        ]
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const TARGETS: &str = include_str!("../../fixtures/prometheus/targets.json");

    #[tokio::test]
    async fn counts_targets_by_health() {
        let http = FakeHttp::new().on(Method::Get, "/api/v1/targets?state=active", 200, TARGETS);
        let cfg = config(&[("url", "http://prometheus-operated.prometheus.svc:9090")]);
        let data = Prometheus.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Targets Up", "4".into()),
                ("Targets Down", "1".into()),
                ("Total Targets", "5".into()),
            ]
        );
        assert!(http.header(0, "Authorization").is_none());
    }

    #[tokio::test]
    async fn upstream_failure_is_an_error() {
        let http = FakeHttp::new().on(Method::Get, "/api/v1/targets", 503, "");
        let err = Prometheus
            .fetch(&http, &config(&[("url", "http://p")]))
            .await
            .unwrap_err();
        assert_eq!(err.0, "HTTP 503");
    }
}
