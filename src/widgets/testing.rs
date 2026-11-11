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
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request);
        found.ok_or_else(|| WidgetError::new(format!("no fixture for {url}")))
    }
}

pub fn config(pairs: &[(&str, &str)]) -> WidgetConfig {
    WidgetConfig {
        params: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

/// `[("Label", "value"), ...]` view of the blocks, for compact assertions.
pub fn pairs(data: &super::WidgetData) -> Vec<(&'static str, String)> {
    data.blocks
        .iter()
        .map(|b| (b.label, b.value.clone()))
        .collect()
}
