use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;

use super::{pick_fields, Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

const DEFAULT_FIELDS: [&str; 3] = ["queries", "blocked", "gravity"];

/// Pi-hole v6 (`widget.version: 6`, the default here) and the v5 `api.php`.
#[derive(Default)]
pub struct Pihole {
    /// v6 session ids by base URL, so a login is not needed on every poll.
    sessions: Mutex<HashMap<String, (String, Instant)>>,
}

#[derive(Deserialize)]
struct SummaryV6 {
    queries: Queries,
    gravity: Gravity,
}

#[derive(Deserialize)]
struct Queries {
    total: f64,
    blocked: f64,
    percent_blocked: f64,
}

#[derive(Deserialize)]
struct Gravity {
    domains_being_blocked: f64,
}

#[derive(Deserialize)]
struct SummaryV5 {
    domains_being_blocked: serde_json::Value,
    dns_queries_today: serde_json::Value,
    ads_blocked_today: serde_json::Value,
    ads_percentage_today: serde_json::Value,
}

#[derive(Deserialize)]
struct Auth {
    session: Option<Session>,
}

#[derive(Deserialize)]
struct Session {
    sid: Option<String>,
    #[serde(default)]
    validity: u64,
}

struct Summary {
    queries: f64,
    blocked: f64,
    percent: f64,
    gravity: f64,
}

fn loose_number(value: &serde_json::Value) -> f64 {
    match value {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.replace(',', "").parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

impl Pihole {
    fn cached_sid(&self, base: &str) -> Option<String> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(base)
            .filter(|(_, expires)| *expires > Instant::now())
            .map(|(sid, _)| sid.clone())
    }

    async fn login(&self, http: &dyn Http, base: &str, key: &str) -> Result<String, WidgetError> {
        let body = serde_json::json!({ "password": key }).to_string();
        let request = Request::post(format!("{base}/api/auth"), body)
            .header("Content-Type", "application/json");
        let response = http.send(request).await?;
        let auth: Auth = response
            .json()
            .map_err(|_| WidgetError::new("authentication failed"))?;
        match auth
            .session
            .and_then(|s| s.sid.map(|sid| (sid, s.validity)))
        {
            Some((sid, validity)) if response.status == 200 => {
                // Renew a little early so a poll never races the expiry.
                let ttl = Duration::from_secs(validity.saturating_sub(30).max(30));
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(base.to_string(), (sid.clone(), Instant::now() + ttl));
                Ok(sid)
            }
            _ => Err(WidgetError::new("authentication failed")),
        }
    }

    async fn summary_v6(
        &self,
        http: &dyn Http,
        cfg: &WidgetConfig,
    ) -> Result<Summary, WidgetError> {
        let base = cfg.url()?;
        let mut sid = match (cfg.key(), self.cached_sid(base)) {
            (Some(_), Some(sid)) => Some(sid),
            (Some(key), None) => Some(self.login(http, base, key).await?),
            (None, _) => None,
        };
        let mut retried = false;
        loop {
            let mut request = Request::get(format!("{base}/api/stats/summary"));
            if let Some(sid) = &sid {
                request = request.header("X-FTL-SID", sid.clone());
            }
            let response = http.send(request).await?;
            // An expired or revoked session: log in again once.
            if response.status == 401 && !retried {
                if let Some(key) = cfg.key() {
                    self.sessions.lock().unwrap().remove(base);
                    sid = Some(self.login(http, base, key).await?);
                    retried = true;
                    continue;
                }
            }
            let summary: SummaryV6 = response.ok()?.json()?;
            return Ok(Summary {
                queries: summary.queries.total,
                blocked: summary.queries.blocked,
                percent: summary.queries.percent_blocked,
                gravity: summary.gravity.domains_being_blocked,
            });
        }
    }

    async fn summary_v5(
        &self,
        http: &dyn Http,
        cfg: &WidgetConfig,
    ) -> Result<Summary, WidgetError> {
        let url = format!(
            "{}/admin/api.php?summaryRaw&auth={}",
            cfg.url()?,
            cfg.key().unwrap_or("")
        );
        let summary: SummaryV5 = http.send(Request::get(url)).await?.ok()?.json()?;
        Ok(Summary {
            queries: loose_number(&summary.dns_queries_today),
            blocked: loose_number(&summary.ads_blocked_today),
            percent: loose_number(&summary.ads_percentage_today),
            gravity: loose_number(&summary.domains_being_blocked),
        })
    }
}

fn label(field: &str) -> Option<&'static str> {
    Some(match field {
        "queries" => "Queries",
        "blocked" => "Blocked",
        "blocked_percent" => "Blocked %",
        "gravity" => "Gravity",
        _ => return None,
    })
}

#[async_trait]
impl Widget for Pihole {
    fn kind(&self) -> &'static str {
        "pihole"
    }

    fn labels(&self, cfg: &WidgetConfig) -> Vec<&'static str> {
        pick_fields(cfg, &DEFAULT_FIELDS, 4)
            .iter()
            .filter_map(|f| label(f))
            .collect()
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let v5 = cfg
            .get("version")
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|v| v < 6);
        let summary = if v5 {
            self.summary_v5(http, cfg).await?
        } else {
            self.summary_v6(http, cfg).await?
        };

        let fields = pick_fields(cfg, &DEFAULT_FIELDS, 4);
        let percent = format::percent(summary.percent);
        let shows_percent = fields.iter().any(|f| f == "blocked_percent");
        let blocks = fields
            .iter()
            .filter_map(|field| {
                let label = label(field)?;
                let value = match field.as_str() {
                    "queries" => format::number(summary.queries),
                    "blocked" if shows_percent => format::number(summary.blocked),
                    "blocked" => format!("{} ({percent})", format::number(summary.blocked)),
                    "blocked_percent" => percent.clone(),
                    "gravity" => format::number(summary.gravity),
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
    use super::super::Method;
    use super::*;

    const SUMMARY: &str = include_str!("../../fixtures/pihole/summary.json");

    #[tokio::test]
    async fn v6_without_password_reads_the_summary() {
        let http = FakeHttp::new().on(Method::Get, "/api/stats/summary", 200, SUMMARY);
        let cfg = config(&[("url", "http://192.0.2.2"), ("version", "6")]);
        let data = Pihole::default().fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Queries", "186,707".into()),
                ("Blocked", "36,229 (19%)".into()),
                ("Gravity", "78,216".into()),
            ]
        );
        assert_eq!(http.requests().len(), 1);
        assert!(http.header(0, "X-FTL-SID").is_none());
    }

    #[tokio::test]
    async fn v6_with_password_logs_in_once_and_reuses_the_session() {
        let http = FakeHttp::new()
            .on(
                Method::Post,
                "/api/auth",
                200,
                r#"{"session":{"valid":true,"sid":"abc","validity":1800}}"#,
            )
            .on(Method::Get, "/api/stats/summary", 200, SUMMARY);
        let cfg = config(&[("url", "http://192.0.2.2"), ("key", "pw")]);
        let widget = Pihole::default();
        widget.fetch(&http, &cfg).await.unwrap();
        widget.fetch(&http, &cfg).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 3, "login, summary, summary");
        assert_eq!(requests[0].body.as_deref(), Some(r#"{"password":"pw"}"#));
        assert_eq!(http.header(1, "X-FTL-SID").unwrap(), "abc");
        assert_eq!(http.header(2, "X-FTL-SID").unwrap(), "abc");
    }

    #[tokio::test]
    async fn rejected_password_is_an_auth_error() {
        let http = FakeHttp::new().on(
            Method::Post,
            "/api/auth",
            401,
            r#"{"session":{"valid":false,"sid":null,"validity":-1,"message":"password incorrect"}}"#,
        );
        let cfg = config(&[("url", "http://192.0.2.2"), ("key", "bad")]);
        let err = Pihole::default().fetch(&http, &cfg).await.unwrap_err();
        assert_eq!(err.0, "authentication failed");
    }

    #[tokio::test]
    async fn v5_summary_is_supported() {
        let http = FakeHttp::new().on(
            Method::Get,
            "/admin/api.php?summaryRaw",
            200,
            r#"{"domains_being_blocked":100000,"dns_queries_today":2000,"ads_blocked_today":500,"ads_percentage_today":25.0}"#,
        );
        let cfg = config(&[("url", "http://192.0.2.2"), ("version", "5")]);
        let data = Pihole::default().fetch(&http, &cfg).await.unwrap();
        assert_eq!(data.blocks[1].value, "500 (25%)");
    }
}
