//! Service widgets. A widget turns one upstream API into a few labelled
//! values ("blocks"). Every widget is a Lua script from a `Widget` custom
//! resource, run by [`LuaWidget`]; none are compiled in. Scripts reach the
//! network only through [`Http`].

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

mod lua;

#[cfg(test)]
pub mod testing;

pub use lua::{LuaWidget, Trace, API_REFERENCE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn post(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: Some(body.into()),
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn basic_auth(self, username: &str, password: &str) -> Self {
        use base64::Engine;
        let token =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        self.header("Authorization", format!("Basic {token}"))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// Cookies set anywhere along the redirect chain, as `name=value`.
    pub cookies: Vec<String>,
}

impl Response {
    /// Errors on HTTP status >= 400.
    pub fn ok(self) -> Result<Self, WidgetError> {
        if self.status >= 400 {
            Err(WidgetError::new(format!("HTTP {}", self.status)))
        } else {
            Ok(self)
        }
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, WidgetError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| WidgetError::new(format!("unexpected response: {e}")))
    }
}

/// A short, user-visible reason. Never contains URLs or credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetError(pub String);

impl WidgetError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for WidgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The only way widgets reach the network.
#[async_trait]
pub trait Http: Send + Sync {
    async fn send(&self, request: Request) -> Result<Response, WidgetError>;
}

/// How a value reads: plainly, or as a problem the application reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Tone {
    #[default]
    Normal,
    Warn,
    Error,
}

impl Tone {
    /// A block's `status` as scripts write it.
    pub fn parse(status: &str) -> Option<Self> {
        match status {
            "ok" => Some(Tone::Normal),
            "warn" => Some(Tone::Warn),
            "error" => Some(Tone::Error),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tone::Normal => "ok",
            Tone::Warn => "warn",
            Tone::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub label: String,
    pub value: String,
    pub tone: Tone,
}

impl Block {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            tone: Tone::Normal,
        }
    }

    pub fn with_tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
}

/// An extra line under the blocks (download queue entries).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub title: String,
    pub detail: String,
    /// 0..=100
    pub progress: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WidgetData {
    pub blocks: Vec<Block>,
    pub rows: Vec<Row>,
}

impl WidgetData {
    /// The worst thing any of the values reports.
    pub fn tone(&self) -> Tone {
        self.blocks.iter().map(|b| b.tone).max().unwrap_or_default()
    }
}

impl From<Vec<Block>> for WidgetData {
    fn from(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            rows: Vec::new(),
        }
    }
}

/// Resolved widget parameters. Credentials are kept apart from the plain
/// parameters: a script can use a secret in a request but cannot read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WidgetConfig {
    pub params: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, String>,
}

impl WidgetConfig {
    /// `widget.url` without trailing slashes.
    pub fn url(&self) -> Result<&str, WidgetError> {
        self.params
            .get("url")
            .map(|u| u.trim_end_matches('/'))
            .filter(|u| !u.is_empty())
            .ok_or_else(|| WidgetError::new("widget.url is not set"))
    }
}

#[async_trait]
pub trait Widget: Send + Sync {
    async fn fetch(
        &self,
        http: Arc<dyn Http>,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_drops_trailing_slashes() {
        let mut cfg = WidgetConfig::default();
        assert!(cfg.url().is_err());
        cfg.params
            .insert("url".into(), "http://caddy.seedbox.svc/rutorrent/".into());
        assert_eq!(cfg.url().unwrap(), "http://caddy.seedbox.svc/rutorrent");
    }
}
