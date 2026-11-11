use async_trait::async_trait;
use serde::Deserialize;

use super::{pick_fields, Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};

const DEFAULT_FIELDS: [&str; 3] = ["pending", "approved", "completed"];

/// Seerr, Jellyseerr and Overseerr (`widget.type` seerr / jellyseerr /
/// overseerr).
pub struct Seerr;

#[derive(Deserialize)]
struct RequestCount {
    pending: i64,
    approved: i64,
    #[serde(default)]
    available: Option<i64>,
    #[serde(default)]
    completed: Option<i64>,
    #[serde(default)]
    processing: Option<i64>,
}

#[derive(Deserialize)]
struct IssueCount {
    open: i64,
    total: i64,
}

fn label(field: &str) -> Option<&'static str> {
    Some(match field {
        "pending" => "Pending",
        "approved" => "Approved",
        "available" => "Available",
        "completed" => "Completed",
        "processing" => "Processing",
        "issues" => "Open Issues",
        _ => return None,
    })
}

fn optional(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| v.to_string())
}

#[async_trait]
impl Widget for Seerr {
    fn kind(&self) -> &'static str {
        "seerr"
    }

    fn labels(&self, cfg: &WidgetConfig) -> Vec<&'static str> {
        pick_fields(cfg, &DEFAULT_FIELDS, 4)
            .iter()
            .filter_map(|f| label(f))
            .collect()
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let base = cfg.url()?;
        let key = cfg.require_key()?;
        let mut fields = pick_fields(cfg, &DEFAULT_FIELDS, 4);

        let request = Request::get(format!("{base}/api/v1/request/count")).header("X-API-Key", key);
        let counts: RequestCount = http.send(request).await?.ok()?.json()?;

        // Older releases report "available" where newer ones say "completed".
        if counts.completed.is_none() {
            for field in fields.iter_mut().filter(|f| *f == "completed") {
                *field = "available".into();
            }
        }

        let issues: Option<IssueCount> = if fields.iter().any(|f| f == "issues") {
            let request =
                Request::get(format!("{base}/api/v1/issue/count")).header("X-API-Key", key);
            Some(http.send(request).await?.ok()?.json()?)
        } else {
            None
        };

        let blocks = fields
            .iter()
            .filter_map(|field| {
                let label = label(field)?;
                let value = match field.as_str() {
                    "pending" => counts.pending.to_string(),
                    "approved" => counts.approved.to_string(),
                    "available" => optional(counts.available),
                    "completed" => optional(counts.completed),
                    "processing" => optional(counts.processing),
                    "issues" => issues
                        .as_ref()
                        .map(|i| format!("{} / {}", i.open, i.total))?,
                    _ => return None,
                };
                Some(Block::new(label, value))
            })
            .collect::<Vec<_>>();
        Ok(blocks.into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::{Method, Registry};
    use super::*;

    const COUNT: &str = include_str!("../../fixtures/seerr/request_count.json");

    #[tokio::test]
    async fn shows_request_counts() {
        let http = FakeHttp::new().on(Method::Get, "/api/v1/request/count", 200, COUNT);
        let cfg = config(&[("url", "http://jellyseerr.media.svc"), ("key", "k")]);
        let registry = Registry::default();
        let widget = registry.get("jellyseerr").unwrap();
        let data = widget.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Pending", "0".into()),
                ("Approved", "1".into()),
                ("Completed", "10".into()),
            ]
        );
        assert_eq!(http.header(0, "X-API-Key").unwrap(), "k");
    }

    #[tokio::test]
    async fn older_api_without_completed_shows_available() {
        let http = FakeHttp::new().on(
            Method::Get,
            "/api/v1/request/count",
            200,
            r#"{"pending":1,"approved":2,"available":7}"#,
        );
        let cfg = config(&[("url", "http://o"), ("key", "k")]);
        let data = Seerr.fetch(&http, &cfg).await.unwrap();
        assert_eq!(data.blocks[2], Block::new("Available", "7"));
    }

    #[tokio::test]
    async fn issues_field_makes_a_second_call() {
        let http = FakeHttp::new()
            .on(Method::Get, "/api/v1/request/count", 200, COUNT)
            .on(
                Method::Get,
                "/api/v1/issue/count",
                200,
                r#"{"open":2,"total":5}"#,
            );
        let cfg = config(&[
            ("url", "http://o"),
            ("key", "k"),
            ("fields", "pending,issues"),
        ]);
        let data = Seerr.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![("Pending", "0".into()), ("Open Issues", "2 / 5".into())]
        );
    }
}
