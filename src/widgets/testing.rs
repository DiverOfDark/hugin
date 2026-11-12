//! Test double for [`Http`]: serves recorded fixtures and records requests.

use std::sync::Mutex;

use async_trait::async_trait;

use super::{Http, Method, Request, Response, WidgetConfig, WidgetError};

struct Route {
    method: Method,
    url_contains: String,
    response: Response,
}

#[derive(Default)]
pub struct FakeHttp {
    routes: Vec<Route>,
    requests: Mutex<Vec<Request>>,
}

impl FakeHttp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers requests whose URL contains `url_contains`. Routes are
    /// matched in the order they were added.
    pub fn on(mut self, method: Method, url_contains: &str, status: u16, body: &str) -> Self {
        self.routes.push(Route {
            method,
            url_contains: url_contains.to_string(),
            response: Response {
                status,
                body: body.as_bytes().to_vec(),
                cookies: Vec::new(),
            },
        });
        self
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// Value of `header` on the n-th request.
    pub fn header(&self, n: usize, header: &str) -> Option<String> {
        self.requests().get(n).and_then(|r| {
            r.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(header))
                .map(|(_, v)| v.clone())
        })
    }
}

#[async_trait]
impl Http for FakeHttp {
    async fn send(&self, request: Request) -> Result<Response, WidgetError> {
        let found = self
            .routes
            .iter()
            .find(|r| r.method == request.method && request.url.contains(&r.url_contains))
            .map(|r| r.response.clone());
        self.requests.lock().unwrap().push(request);
        found.ok_or_else(|| WidgetError::new("no fixture for this request"))
    }
}

/// Parameters named key, password or token become secrets, as in production.
pub fn config(pairs: &[(&str, &str)]) -> WidgetConfig {
    let mut cfg = WidgetConfig::default();
    for (k, v) in pairs {
        if matches!(*k, "key" | "password" | "token") {
            cfg.secrets.insert(k.to_string(), v.to_string());
        } else {
            cfg.params.insert(k.to_string(), v.to_string());
        }
    }
    cfg
}

/// `[("Label", "value"), ...]` view of the blocks, for compact assertions.
pub fn pairs(data: &super::WidgetData) -> Vec<(String, String)> {
    data.blocks
        .iter()
        .map(|b| (b.label.clone(), b.value.clone()))
        .collect()
}
