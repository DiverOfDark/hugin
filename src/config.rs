//! Static configuration, loaded from a JSON file (the Helm chart renders
//! `.Values.config` into it). Everything has a default, so an empty or
//! missing file yields a working dashboard that only shows discovered
//! services.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::model::{SecretRef, Service, WidgetSpec};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub title: String,
    /// Maximum number of group columns on wide screens.
    pub columns: u8,
    /// Show per-service CPU / memory under each discovered service.
    pub show_stats: bool,
    /// Explicit group order; groups not listed follow alphabetically.
    pub group_order: Vec<String>,
    pub discovery: Discovery,
    pub header: Header,
    pub bookmarks: Vec<BookmarkGroup>,
    /// Entries that are not discovered from the cluster.
    pub services: Vec<StaticService>,
    pub intervals: Intervals,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            title: "Hugin".into(),
            columns: 5,
            show_stats: true,
            group_order: Vec::new(),
            discovery: Discovery::default(),
            header: Header::default(),
            bookmarks: Vec::new(),
            services: Vec::new(),
            intervals: Intervals::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Discovery {
    pub ingress: bool,
    /// Also read annotations from Traefik `IngressRoute` objects.
    pub traefik_ingress_routes: bool,
}

impl Default for Discovery {
    fn default() -> Self {
        Self {
            ingress: true,
            traefik_ingress_routes: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Intervals {
    pub widgets_seconds: u64,
    pub resources_seconds: u64,
    /// Polling stops this long after the last page view or SSE client.
    pub idle_after_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl Default for Intervals {
    fn default() -> Self {
        Self {
            widgets_seconds: 30,
            resources_seconds: 15,
            idle_after_seconds: 90,
            request_timeout_seconds: 10,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Header {
    pub cluster: ClusterHeader,
    pub nodes: NodesHeader,
    pub weather: Option<Weather>,
    pub search: Option<Search>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ClusterHeader {
    pub show: bool,
    pub label: String,
}

impl Default for ClusterHeader {
    fn default() -> Self {
        Self {
            show: true,
            label: "cluster".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NodesHeader {
    pub show: bool,
}

impl Default for NodesHeader {
    fn default() -> Self {
        Self { show: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Weather {
    #[serde(default)]
    pub label: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// "metric" or "imperial".
    #[serde(default = "default_units")]
    pub units: String,
    #[serde(default = "default_cache_minutes")]
    pub cache_minutes: u64,
    #[serde(default = "default_weather_digits")]
    pub maximum_fraction_digits: usize,
}

fn default_timezone() -> String {
    "auto".into()
}
fn default_units() -> String {
    "metric".into()
}
fn default_cache_minutes() -> u64 {
    10
}
fn default_weather_digits() -> usize {
    0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Search {
    /// google, duckduckgo, bing, brave, or "custom" with `url`.
    pub provider: String,
    /// For `provider: custom`: URL the query is appended to.
    pub url: Option<String>,
    pub target: String,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            provider: "google".into(),
            url: None,
            target: "_blank".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookmarkGroup {
    pub group: String,
    #[serde(default)]
    pub items: Vec<Bookmark>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bookmark {
    pub name: String,
    pub href: String,
    #[serde(default)]
    pub abbr: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StaticService {
    pub group: String,
    pub name: String,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub weight: Option<i64>,
    #[serde(default)]
    pub widget: Option<StaticWidget>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StaticWidget {
    #[serde(rename = "type")]
    pub kind: String,
    /// Plain parameters (`url`, `version`, ...). Values may be strings,
    /// numbers or booleans.
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    /// Parameters read from Kubernetes Secrets, e.g.
    /// `key: { namespace: pihole, name: pihole-api, key: password }`.
    #[serde(default)]
    pub secrets: BTreeMap<String, StaticSecretRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StaticSecretRef {
    pub namespace: String,
    pub name: String,
    pub key: String,
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) if text.trim().is_empty() => Ok(Self::default()),
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %path.display(), "config file not found, using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Static entries as services. Like gethomepage, entry N of a group gets
    /// weight (N+1)*100 unless set, so static entries sort after discovered
    /// ones (weight 0) and keep their configured order.
    pub fn static_services(&self) -> Vec<Service> {
        let mut per_group: BTreeMap<&str, i64> = BTreeMap::new();
        self.services
            .iter()
            .map(|s| {
                let n = per_group.entry(s.group.as_str()).or_insert(0);
                *n += 1;
                let widget = s.widget.as_ref().map(|w| WidgetSpec {
                    kind: w.kind.clone(),
                    params: w
                        .params
                        .iter()
                        .map(|(k, v)| {
                            let text = match v {
                                serde_json::Value::String(s) => s.trim().to_string(),
                                other => other.to_string(),
                            };
                            (k.clone(), text)
                        })
                        .collect(),
                    secrets: w
                        .secrets
                        .iter()
                        .map(|(k, r)| {
                            (
                                k.clone(),
                                SecretRef {
                                    namespace: r.namespace.clone(),
                                    name: r.name.clone(),
                                    key: r.key.clone(),
                                },
                            )
                        })
                        .collect(),
                });
                Service {
                    id: crate::model::dom_id(&format!("static/{}/{}", s.group, s.name)),
                    group: s.group.clone(),
                    name: s.name.clone(),
                    href: s.href.clone(),
                    icon: s.icon.clone(),
                    description: s.description.clone(),
                    weight: s.weight.unwrap_or(*n * 100),
                    kube: None,
                    widget,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_valid() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.columns, 5);
        assert!(cfg.discovery.ingress);
        assert!(cfg.services.is_empty());
    }

    #[test]
    fn static_services_get_increasing_weights() {
        let cfg: Config = serde_json::from_str(
            r#"{"services":[
                {"group":"Infra","name":"FritzBox","widget":{"type":"fritzbox","params":{"url":"http://192.0.2.1/"}}},
                {"group":"Infra","name":"Pihole","widget":{"type":"pihole","params":{"url":"http://192.0.2.2","version":6},
                  "secrets":{"key":{"namespace":"dns","name":"pihole","key":"password"}}}},
                {"group":"Other","name":"Link","href":"http://example.com"}
            ]}"#,
        )
        .unwrap();
        let services = cfg.static_services();
        assert_eq!(services[0].weight, 100);
        assert_eq!(services[1].weight, 200);
        assert_eq!(services[2].weight, 100);
        let pihole = services[1].widget.as_ref().unwrap();
        assert_eq!(pihole.params["version"], "6");
        assert_eq!(pihole.secrets["key"].name, "pihole");
        assert!(services[2].widget.is_none());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(serde_json::from_str::<Config>(r#"{"titel":"x"}"#).is_err());
    }
}
