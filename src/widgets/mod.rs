//! Service widgets. Each widget turns one upstream API into a few labelled
//! values ("blocks"). They all implement [`Widget`] and talk to the network
//! only through [`Http`], so every widget is unit-tested against a recorded
//! upstream response (see `fixtures/`).

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::de::DeserializeOwned;

mod argocd;
mod arr;
mod fritzbox;
mod grafana;
mod jackett;
mod nextcloud;
mod paperlessngx;
mod pihole;
mod prometheus;
mod rutorrent;
mod seerr;
mod suwayomi;
mod traefik;

#[cfg(test)]
pub mod testing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
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

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
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
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Errors on HTTP status >= 400, like gethomepage's widget API does.
    pub fn ok(self) -> Result<Self, WidgetError> {
        if self.status >= 400 {
            Err(WidgetError::new(format!("HTTP {}", self.status)))
        } else {
            Ok(self)
        }
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, WidgetError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| WidgetError::new(format!("unexpected response: {e}")))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A short, user-visible reason. Never contains URLs, because several
/// upstream APIs carry the API key in the query string.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub label: &'static str,
    pub value: String,
}

impl Block {
    pub fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
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

impl From<Vec<Block>> for WidgetData {
    fn from(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            rows: Vec::new(),
        }
    }
}

/// Resolved widget parameters: annotations / config merged with Secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WidgetConfig {
    pub params: BTreeMap<String, String>,
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

    pub fn get(&self, name: &str) -> Option<&str> {
        self.params
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    pub fn key(&self) -> Option<&str> {
        self.get("key")
    }

    pub fn require_key(&self) -> Result<&str, WidgetError> {
        self.key()
            .ok_or_else(|| WidgetError::new("widget.key is not set"))
    }

    pub fn basic(&self) -> Option<(&str, &str)> {
        Some((self.get("username")?, self.get("password")?))
    }

    pub fn flag(&self, name: &str) -> bool {
        self.get(name) == Some("true")
    }

    /// `widget.fields` as a list: accepts a JSON array or a comma list.
    pub fn fields(&self) -> Option<Vec<String>> {
        let raw = self.get("fields")?;
        if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
            return Some(list);
        }
        Some(
            raw.split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect(),
        )
    }
}

#[async_trait]
pub trait Widget: Send + Sync {
    /// The `widget.type` this implementation answers to.
    fn kind(&self) -> &'static str;

    /// Labels of the blocks, shown as placeholders before the first fetch.
    fn labels(&self, cfg: &WidgetConfig) -> Vec<&'static str>;

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError>;
}

/// Registry of widget implementations, keyed by `widget.type`.
pub struct Registry {
    widgets: Vec<(&'static str, Box<dyn Widget>)>,
}

impl Default for Registry {
    fn default() -> Self {
        let all: Vec<Box<dyn Widget>> = vec![
            Box::new(argocd::Argocd),
            Box::new(grafana::Grafana),
            Box::new(nextcloud::Nextcloud),
            Box::new(prometheus::Prometheus),
            Box::new(rutorrent::Rutorrent),
            Box::new(suwayomi::Suwayomi),
            Box::new(traefik::Traefik),
            Box::new(fritzbox::Fritzbox),
            Box::new(pihole::Pihole::default()),
            Box::new(paperlessngx::Paperlessngx),
            Box::new(arr::Sonarr),
            Box::new(arr::Radarr),
            Box::new(jackett::Jackett::default()),
            Box::new(seerr::Seerr),
        ];
        let mut widgets: Vec<(&'static str, Box<dyn Widget>)> = Vec::new();
        for w in all {
            widgets.push((w.kind(), w));
        }
        Self { widgets }
    }
}

impl Registry {
    pub fn get(&self, kind: &str) -> Option<&dyn Widget> {
        // jellyseerr and overseerr are the same API as seerr.
        let kind = match kind {
            "jellyseerr" | "overseerr" => "seerr",
            other => other,
        };
        self.widgets
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, w)| w.as_ref())
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        self.widgets.iter().map(|(k, _)| *k).collect()
    }
}

/// Keeps the first `max` configured fields, or the defaults.
pub(crate) fn pick_fields(cfg: &WidgetConfig, defaults: &[&str], max: usize) -> Vec<String> {
    let configured = cfg.fields().filter(|f| !f.is_empty());
    let fields = configured.unwrap_or_else(|| defaults.iter().map(|s| s.to_string()).collect());
    fields.into_iter().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_knows_every_supported_type() {
        let registry = Registry::default();
        for kind in [
            "argocd",
            "grafana",
            "jackett",
            "jellyseerr",
            "nextcloud",
            "paperlessngx",
            "prometheus",
            "radarr",
            "rutorrent",
            "sonarr",
            "suwayomi",
            "traefik",
            "fritzbox",
            "pihole",
        ] {
            assert!(registry.get(kind).is_some(), "missing widget {kind}");
        }
        assert!(registry.get("nope").is_none());
    }

    #[test]
    fn url_drops_trailing_slashes() {
        let mut cfg = WidgetConfig::default();
        assert!(cfg.url().is_err());
        cfg.params
            .insert("url".into(), "http://caddy.seedbox.svc/rutorrent/".into());
        assert_eq!(cfg.url().unwrap(), "http://caddy.seedbox.svc/rutorrent");
    }

    #[test]
    fn fields_accept_json_or_comma_list() {
        let mut cfg = WidgetConfig::default();
        cfg.params
            .insert("fields".into(), r#"["apps","synced"]"#.into());
        assert_eq!(cfg.fields().unwrap(), vec!["apps", "synced"]);
        cfg.params.insert("fields".into(), "apps, healthy".into());
        assert_eq!(cfg.fields().unwrap(), vec!["apps", "healthy"]);
        assert_eq!(pick_fields(&cfg, &["x"], 1), vec!["apps"]);
        cfg.params.remove("fields");
        assert_eq!(pick_fields(&cfg, &["x", "y"], 4), vec!["x", "y"]);
    }
}
