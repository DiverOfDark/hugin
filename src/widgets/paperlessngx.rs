use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};

pub struct Paperlessngx;

#[derive(Deserialize)]
struct Statistics {
    documents_total: i64,
    /// Absent on old versions, `null` when no inbox tag is configured.
    #[serde(default, deserialize_with = "present")]
    documents_inbox: Option<Option<i64>>,
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i64>>, D::Error> {
    Option::<i64>::deserialize(d).map(Some)
}

#[async_trait]
impl Widget for Paperlessngx {
    fn kind(&self) -> &'static str {
        "paperlessngx"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Inbox", "Total"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let mut request = Request::get(format!("{}/api/statistics/?format=json", cfg.url()?));
        request = match (cfg.key(), cfg.basic()) {
            (Some(key), _) => request.header("Authorization", format!("Token {key}")),
            (None, Some((user, pass))) => request.basic_auth(user, pass),
            (None, None) => return Err(WidgetError::new("widget.key is not set")),
        };
        let stats: Statistics = http.send(request).await?.ok()?.json()?;
        let mut blocks = Vec::new();
        if let Some(inbox) = stats.documents_inbox {
            let value = inbox.map_or_else(|| "-".to_string(), |n| n.to_string());
            blocks.push(Block::new("Inbox", value));
        }
        blocks.push(Block::new("Total", stats.documents_total.to_string()));
        Ok(blocks.into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const STATS: &str = include_str!("../../fixtures/paperlessngx/statistics.json");

    #[tokio::test]
    async fn shows_totals_and_a_dash_for_a_missing_inbox() {
        let http = FakeHttp::new().on(Method::Get, "/api/statistics/?format=json", 200, STATS);
        let cfg = config(&[("url", "http://paperless.paperless.svc"), ("key", "tok")]);
        let data = Paperlessngx.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![("Inbox", "-".into()), ("Total", "429".into())]
        );
        assert_eq!(http.header(0, "Authorization").unwrap(), "Token tok");
    }

    #[tokio::test]
    async fn inbox_count_is_shown_when_configured() {
        let http = FakeHttp::new().on(
            Method::Get,
            "/api/statistics/",
            200,
            r#"{"documents_total":10,"documents_inbox":3}"#,
        );
        let cfg = config(&[("url", "http://p"), ("key", "tok")]);
        let data = Paperlessngx.fetch(&http, &cfg).await.unwrap();
        assert_eq!(data.blocks[0].value, "3");
    }

    #[tokio::test]
    async fn bad_token_is_reported() {
        let http = FakeHttp::new().on(Method::Get, "/api/statistics/", 401, "{}");
        let cfg = config(&[("url", "http://p"), ("key", "bad")]);
        assert_eq!(
            Paperlessngx.fetch(&http, &cfg).await.unwrap_err().0,
            "HTTP 401"
        );
    }
}
