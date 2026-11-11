use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Response, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

/// Jackett answers its API either with a valid `apikey` or with a session
/// cookie. Without an admin password it hands that cookie to anyone who
/// follows its login redirect, which is how a key-less widget works.
#[derive(Default)]
pub struct Jackett {
    cookies: Mutex<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct Indexer {
    #[serde(default)]
    last_error: Option<String>,
}

fn cookie_header(response: &Response) -> Option<String> {
    (!response.cookies.is_empty()).then(|| response.cookies.join("; "))
}

impl Jackett {
    async fn indexers(
        &self,
        http: &dyn Http,
        cfg: &WidgetConfig,
        cookie: Option<&str>,
    ) -> Result<Response, WidgetError> {
        // gethomepage sends the literal "undefined" when no key is set;
        // Jackett treats any wrong key the same way.
        let url = format!(
            "{}/api/v2.0/indexers?apikey={}&configured=true",
            cfg.url()?,
            cfg.key().unwrap_or("undefined")
        );
        let mut request = Request::get(url);
        if let Some(cookie) = cookie {
            request = request.header("Cookie", cookie);
        }
        http.send(request).await
    }
}

#[async_trait]
impl Widget for Jackett {
    fn kind(&self) -> &'static str {
        "jackett"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Configured", "Errored"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let base = cfg.url()?.to_string();
        let mut cookie = self.cookies.lock().unwrap().get(&base).cloned();

        if cookie.is_none() {
            if let Some(password) = cfg.get("password") {
                let body = format!("password={}", urlencode(password));
                let login = Request::post(format!("{base}/UI/Dashboard"), body)
                    .header("Content-Type", "application/x-www-form-urlencoded");
                cookie = cookie_header(&http.send(login).await?);
                if cookie.is_none() {
                    return Err(WidgetError::new("authentication failed"));
                }
            }
        }

        let mut response = self.indexers(http, cfg, cookie.as_deref()).await?;
        let mut parsed = response.json::<Vec<Indexer>>();
        if parsed.is_err() {
            // Not JSON: we were sent through the login redirect. Retry once
            // with whatever session cookie that produced.
            if let Some(fresh) = cookie_header(&response) {
                response = self.indexers(http, cfg, Some(&fresh)).await?;
                parsed = response.json::<Vec<Indexer>>();
                cookie = Some(fresh);
            }
        }
        let indexers = match parsed {
            Ok(indexers) if response.is_success() => indexers,
            _ => {
                self.cookies.lock().unwrap().remove(&base);
                return Err(if response.status >= 400 {
                    WidgetError::new(format!("HTTP {}", response.status))
                } else {
                    WidgetError::new("authentication failed")
                });
            }
        };
        if let Some(cookie) = cookie {
            self.cookies.lock().unwrap().insert(base, cookie);
        }

        let errored = indexers
            .iter()
            .filter(|i| i.last_error.as_deref().is_some_and(|e| !e.is_empty()))
            .count();
        Ok(vec![
            Block::new("Configured", format::int(indexers.len() as i64)),
            Block::new("Errored", format::int(errored as i64)),
        ]
        .into())
    }
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const INDEXERS: &str = include_str!("../../fixtures/jackett/indexers.json");

    #[tokio::test]
    async fn valid_api_key_needs_one_request() {
        let http = FakeHttp::new().on(Method::Get, "apikey=good", 200, INDEXERS);
        let cfg = config(&[("url", "http://jackett.media.svc"), ("key", "good")]);
        let data = Jackett::default().fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![("Configured", "2".into()), ("Errored", "0".into())]
        );
        assert_eq!(http.requests().len(), 1);
    }

    /// Stands in for Jackett without an admin password: the first call lands
    /// on the dashboard HTML and yields a session cookie, the second works.
    struct LoginThenJson(Mutex<Vec<Request>>);

    #[async_trait]
    impl Http for LoginThenJson {
        async fn send(&self, request: Request) -> Result<Response, WidgetError> {
            let has_cookie = request.headers.iter().any(|(k, _)| *k == "Cookie");
            self.0.lock().unwrap().push(request);
            Ok(if has_cookie {
                Response {
                    status: 200,
                    body: INDEXERS.as_bytes().to_vec(),
                    cookies: vec![],
                }
            } else {
                Response {
                    status: 200,
                    body: b"<!DOCTYPE html><html>Dashboard</html>".to_vec(),
                    cookies: vec!["TestCookie=1".into(), "Jackett=session".into()],
                }
            })
        }
    }

    #[tokio::test]
    async fn without_key_the_login_redirect_cookie_is_used_and_cached() {
        let http = LoginThenJson(Mutex::new(Vec::new()));
        let cfg = config(&[("url", "http://jackett.media.svc")]);
        let widget = Jackett::default();
        let data = widget.fetch(&http, &cfg).await.unwrap();
        assert_eq!(data.blocks[0].value, "2");
        widget.fetch(&http, &cfg).await.unwrap();
        let requests = http.0.lock().unwrap();
        assert_eq!(requests.len(), 3, "html, json, then json from the cache");
        assert!(requests[0].url.contains("apikey=undefined"));
        assert_eq!(
            requests[2].headers[0],
            ("Cookie", "TestCookie=1; Jackett=session".to_string())
        );
    }

    #[tokio::test]
    async fn html_without_cookie_is_an_auth_failure() {
        let http = FakeHttp::new().on(Method::Get, "/api/v2.0/indexers", 200, "<html>login</html>");
        let cfg = config(&[("url", "http://jackett")]);
        let err = Jackett::default().fetch(&http, &cfg).await.unwrap_err();
        assert_eq!(err.0, "authentication failed");
    }

    #[test]
    fn password_is_form_encoded() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
    }
}
