use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};

pub struct Traefik;

#[derive(Deserialize)]
struct Overview {
    http: HttpSection,
}

#[derive(Deserialize)]
struct HttpSection {
    routers: Total,
    services: Total,
    middlewares: Total,
}

#[derive(Deserialize)]
struct Total {
    total: i64,
}

#[async_trait]
impl Widget for Traefik {
    fn kind(&self) -> &'static str {
        "traefik"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Routers", "Services", "Middleware"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let mut request = Request::get(format!("{}/api/overview", cfg.url()?));
        if let Some((user, pass)) = cfg.basic() {
            request = request.basic_auth(user, pass);
        }
        let overview: Overview = http.send(request).await?.ok()?.json()?;
        Ok(vec![
            Block::new("Routers", overview.http.routers.total.to_string()),
            Block::new("Services", overview.http.services.total.to_string()),
            Block::new("Middleware", overview.http.middlewares.total.to_string()),
        ]
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const OVERVIEW: &str = include_str!("../../fixtures/traefik/overview.json");

    #[tokio::test]
    async fn shows_http_totals() {
        let http = FakeHttp::new().on(Method::Get, "/api/overview", 200, OVERVIEW);
        let cfg = config(&[("url", "http://traefik-dashboard.traefik.svc:9000")]);
        let data = Traefik.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Routers", "111".into()),
                ("Services", "47".into()),
                ("Middleware", "2".into()),
            ]
        );
    }

    #[tokio::test]
    async fn rejects_a_non_overview_payload() {
        let http = FakeHttp::new().on(Method::Get, "/api/overview", 200, r#"{"tcp":{}}"#);
        let err = Traefik
            .fetch(&http, &config(&[("url", "http://t")]))
            .await
            .unwrap_err();
        assert!(err.0.starts_with("unexpected response"));
    }
}
