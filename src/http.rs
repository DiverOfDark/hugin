//! The real [`Http`] implementation used by widgets.
//!
//! Redirects are followed by hand so cookies set along the chain can be
//! handed back to the widget (Jackett's login flow needs them), and bodies
//! are capped so one misbehaving upstream cannot balloon memory.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderValue, COOKIE, LOCATION, SET_COOKIE};

use crate::widgets::{Http, Method, Request, Response, WidgetError};

const MAX_REDIRECTS: usize = 8;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub struct Client {
    inner: reqwest::Client,
}

impl Client {
    pub fn new(timeout: Duration) -> anyhow::Result<Self> {
        let inner = reqwest::Client::builder()
            .user_agent(concat!("hugin/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            // No shorter connect timeout: upstreams reached over a VPN or a
            // subnet router can take several seconds to accept.
            .timeout(timeout)
            // No idle connections: with a refresh every 30 s most upstreams
            // have closed theirs by the next request, and reusing a dead one
            // fails the refresh. A fresh handshake per request is cheap here.
            .pool_max_idle_per_host(0)
            .build()?;
        Ok(Self { inner })
    }
}

/// Maps a transport error to a short reason. The URL is dropped on purpose:
/// it can contain an API key.
fn describe(error: reqwest::Error) -> WidgetError {
    let error = error.without_url();
    let reason = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_body() || error.is_decode() {
        "invalid response body"
    } else {
        "request failed"
    };
    tracing::debug!(%error, reason, "upstream request failed");
    WidgetError::new(reason)
}

#[async_trait]
impl Http for Client {
    async fn send(&self, request: Request) -> Result<Response, WidgetError> {
        let mut url = reqwest::Url::parse(&request.url)
            .map_err(|_| WidgetError::new("widget.url is not a valid URL"))?;
        let mut method = request.method;
        let mut body = request.body;
        let mut cookies: Vec<String> = Vec::new();

        for _ in 0..=MAX_REDIRECTS {
            let mut builder = match method {
                Method::Get => self.inner.get(url.clone()),
                Method::Post => self.inner.post(url.clone()),
            };
            let mut caller_cookie = None;
            for (name, value) in &request.headers {
                if name.eq_ignore_ascii_case("cookie") {
                    caller_cookie = Some(value.clone());
                } else {
                    builder = builder.header(*name, value);
                }
            }
            let jar = caller_cookie
                .into_iter()
                .chain(cookies.iter().cloned())
                .collect::<Vec<_>>()
                .join("; ");
            if !jar.is_empty() {
                if let Ok(value) = HeaderValue::from_str(&jar) {
                    builder = builder.header(COOKIE, value);
                }
            }
            if let (Method::Post, Some(body)) = (method, &body) {
                builder = builder.body(body.clone());
            }

            let mut response = builder.send().await.map_err(describe)?;
            for value in response.headers().get_all(SET_COOKIE) {
                let pair = value
                    .to_str()
                    .ok()
                    .and_then(|v| v.split(';').next())
                    .map(str::trim)
                    .filter(|p| p.contains('='));
                if let Some(pair) = pair {
                    let name = pair.split('=').next().unwrap_or("");
                    cookies.retain(|c| c.split('=').next() != Some(name));
                    cookies.push(pair.to_string());
                }
            }

            let status = response.status();
            if status.is_redirection() {
                let next = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .and_then(|l| url.join(l).ok());
                if let Some(next) = next {
                    // 307/308 keep the method and body; everything else
                    // becomes a GET, as browsers do.
                    if !matches!(status.as_u16(), 307 | 308) {
                        method = Method::Get;
                        body = None;
                    }
                    url = next;
                    continue;
                }
            }

            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(describe)? {
                if bytes.len() + chunk.len() > MAX_BODY_BYTES {
                    return Err(WidgetError::new("response too large"));
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(Response {
                status: status.as_u16(),
                body: bytes,
                cookies,
            });
        }
        Err(WidgetError::new("too many redirects"))
    }
}
