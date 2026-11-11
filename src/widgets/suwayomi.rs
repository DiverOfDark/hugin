use async_trait::async_trait;

use super::{pick_fields, Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

const DEFAULT_FIELDS: [&str; 4] = ["download", "nondownload", "read", "unread"];

pub struct Suwayomi;

/// (field, label, GraphQL condition)
const COUNTS: [(&str, &str, &str); 8] = [
    ("download", "Downloaded", "isDownloaded: true"),
    ("nondownload", "Non-Downloaded", "isDownloaded: false"),
    ("read", "Read", "isRead: true"),
    ("unread", "Unread", "isRead: false"),
    (
        "downloadedread",
        "Downloaded & Read",
        "isDownloaded: true, isRead: true",
    ),
    (
        "downloadedunread",
        "Downloaded & Unread",
        "isDownloaded: true, isRead: false",
    ),
    (
        "nondownloadedread",
        "Non-Downloaded & Read",
        "isDownloaded: false, isRead: true",
    ),
    (
        "nondownloadedunread",
        "Non-Downloaded & Unread",
        "isDownloaded: false, isRead: false",
    ),
];

fn known_fields(cfg: &WidgetConfig) -> Vec<(&'static str, &'static str, &'static str)> {
    pick_fields(cfg, &DEFAULT_FIELDS, 4)
        .iter()
        .filter_map(|f| COUNTS.iter().find(|(name, _, _)| name == f).copied())
        .collect()
}

#[async_trait]
impl Widget for Suwayomi {
    fn kind(&self) -> &'static str {
        "suwayomi"
    }

    fn labels(&self, cfg: &WidgetConfig) -> Vec<&'static str> {
        known_fields(cfg)
            .iter()
            .map(|(_, label, _)| *label)
            .collect()
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        if cfg.get("category").is_some_and(|c| c != "all") {
            return Err(WidgetError::new("widget.category is not supported"));
        }
        let fields = known_fields(cfg);
        let mut query = String::from("query Counts {");
        for (name, _, condition) in &fields {
            query.push_str(&format!(
                " {name}: chapters(condition: {{{condition}}} filter: {{inLibrary: {{equalTo: true}}}}) {{ totalCount }}"
            ));
        }
        query.push_str(" }");
        let body = serde_json::json!({ "operationName": "Counts", "query": query }).to_string();

        let mut request = Request::post(format!("{}/api/graphql", cfg.url()?), body)
            .header("Content-Type", "application/json");
        if let Some((user, pass)) = cfg.basic() {
            request = request.basic_auth(user, pass);
        }
        let response = http.send(request).await?;
        if response.status == 401 {
            return Err(WidgetError::new("unauthorized"));
        }
        let json: serde_json::Value = response.ok()?.json()?;
        let blocks = fields
            .iter()
            .map(|(name, label, _)| {
                json["data"][name]["totalCount"]
                    .as_f64()
                    .map(|count| Block::new(label, format::number(count)))
                    .ok_or_else(|| WidgetError::new("unexpected response: missing count"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(blocks.into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const COUNTS_JSON: &str = include_str!("../../fixtures/suwayomi/counts.json");

    #[tokio::test]
    async fn shows_library_chapter_counts() {
        let http = FakeHttp::new().on(Method::Post, "/api/graphql", 200, COUNTS_JSON);
        let cfg = config(&[("url", "http://suwayomi-server.nextcloud.svc:4568")]);
        let data = Suwayomi.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Downloaded", "4,315".into()),
                ("Non-Downloaded", "3,923".into()),
                ("Read", "3,170".into()),
                ("Unread", "5,068".into()),
            ]
        );
        let request = &http.requests()[0];
        let body: serde_json::Value = serde_json::from_str(request.body.as_ref().unwrap()).unwrap();
        let query = body["query"].as_str().unwrap();
        assert!(query.contains("download: chapters(condition: {isDownloaded: true}"));
        assert!(query.contains("filter: {inLibrary: {equalTo: true}}"));
    }

    #[tokio::test]
    async fn graphql_error_without_data_is_reported() {
        let http = FakeHttp::new().on(Method::Post, "/api/graphql", 200, r#"{"errors":[{}]}"#);
        let err = Suwayomi
            .fetch(&http, &config(&[("url", "http://s")]))
            .await
            .unwrap_err();
        assert!(err.0.starts_with("unexpected response"));
    }
}
