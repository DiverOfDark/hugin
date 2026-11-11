use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

pub struct Rutorrent;

// Positions in the httprpc `mode=list` torrent tuple.
const STATE: usize = 3;
const UP_RATE: usize = 11;
const DOWN_RATE: usize = 12;

#[derive(Deserialize)]
struct List {
    /// hash -> positional fields. An empty list arrives as `[]`.
    t: Torrents,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Torrents {
    Map(BTreeMap<String, Vec<serde_json::Value>>),
    Empty(#[allow(dead_code)] Vec<serde_json::Value>),
}

fn as_number(value: Option<&serde_json::Value>) -> f64 {
    match value {
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0.0),
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[async_trait]
impl Widget for Rutorrent {
    fn kind(&self) -> &'static str {
        "rutorrent"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Active", "Upload", "Download"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let url = format!("{}/plugins/httprpc/action.php", cfg.url()?);
        let mut request = Request::post(url, "mode=list")
            .header("Content-Type", "application/x-www-form-urlencoded");
        if let Some(user) = cfg.get("username") {
            request = request.basic_auth(user, cfg.get("password").unwrap_or(""));
        }
        let response = http.send(request).await?;
        if response.status != 200 {
            return Err(WidgetError::new(format!("HTTP {}", response.status)));
        }
        let list: List = response.json()?;
        let torrents: Vec<Vec<serde_json::Value>> = match list.t {
            Torrents::Map(map) => map.into_values().collect(),
            Torrents::Empty(_) => Vec::new(),
        };
        let active = torrents
            .iter()
            .filter(|t| as_number(t.get(STATE)) == 1.0)
            .count();
        let upload: f64 = torrents.iter().map(|t| as_number(t.get(UP_RATE))).sum();
        let download: f64 = torrents.iter().map(|t| as_number(t.get(DOWN_RATE))).sum();
        Ok(vec![
            Block::new("Active", active.to_string()),
            Block::new("Upload", format::byterate(upload, 0)),
            Block::new("Download", format::byterate(download, 0)),
        ]
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const LIST: &str = include_str!("../../fixtures/rutorrent/list.json");

    #[tokio::test]
    async fn sums_rates_and_counts_started_torrents() {
        let http = FakeHttp::new().on(Method::Post, "/plugins/httprpc/action.php", 200, LIST);
        let cfg = config(&[("url", "http://caddy.seedbox.svc/rutorrent/")]);
        let data = Rutorrent.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Active", "2".into()),
                ("Upload", "154 kB/s".into()),
                ("Download", "2 MB/s".into()),
            ]
        );
        let request = &http.requests()[0];
        assert_eq!(
            request.url,
            "http://caddy.seedbox.svc/rutorrent/plugins/httprpc/action.php"
        );
        assert_eq!(request.body.as_deref(), Some("mode=list"));
    }

    #[tokio::test]
    async fn empty_torrent_list_is_zero_not_an_error() {
        let http = FakeHttp::new().on(Method::Post, "/action.php", 200, r#"{"t":[],"cid":1}"#);
        let data = Rutorrent
            .fetch(&http, &config(&[("url", "http://r")]))
            .await
            .unwrap();
        assert_eq!(data.blocks[0].value, "0");
        assert_eq!(data.blocks[1].value, "0 B/s");
    }

    #[tokio::test]
    async fn upstream_500_is_reported() {
        let http = FakeHttp::new().on(Method::Post, "/action.php", 500, "<html>Error 500</html>");
        let err = Rutorrent
            .fetch(&http, &config(&[("url", "http://r")]))
            .await
            .unwrap_err();
        assert_eq!(err.0, "HTTP 500");
    }
}
