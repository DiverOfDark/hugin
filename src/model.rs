//! Core data types shared by discovery, polling and rendering.

use std::collections::BTreeMap;

/// A key inside a Kubernetes Secret.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretRef {
    pub namespace: String,
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetSpec {
    pub kind: String,
    /// Plain parameters from annotations or config (`url`, `key`, ...).
    pub params: BTreeMap<String, String>,
    /// Parameters to read from Secrets. They win over `params`; `params` is
    /// the fallback when the Secret cannot be read.
    pub secrets: BTreeMap<String, SecretRef>,
}

/// Where a discovered service's pods live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubeRef {
    pub namespace: String,
    /// Label selector for the pods behind the service.
    pub pod_selector: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Stable, DOM-safe identifier.
    pub id: String,
    pub group: String,
    pub name: String,
    pub href: Option<String>,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub weight: i64,
    pub kube: Option<KubeRef>,
    pub widget: Option<WidgetSpec>,
}

/// Pod health as gethomepage reports it: based on pod phase only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodStatus {
    Running,
    Partial,
    Down,
    NotFound,
}

impl PodStatus {
    pub fn label(self) -> &'static str {
        match self {
            PodStatus::Running => "running",
            PodStatus::Partial => "partial",
            PodStatus::Down => "down",
            PodStatus::NotFound => "not found",
        }
    }
}

/// FNV-1a based id usable as an HTML id and an SSE fragment key.
pub fn dom_id(source: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in source.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("s-{hash:016x}")
}

/// A parsed `key=value,key!=value,key` label selector (equality based only,
/// which is all gethomepage's pod-selector annotation is used for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(Vec<Requirement>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Requirement {
    Eq(String, String),
    Ne(String, String),
    Exists(String),
    NotExists(String),
}

impl Selector {
    pub fn parse(text: &str) -> Self {
        let reqs = text
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|part| {
                if let Some((k, v)) = part.split_once("!=") {
                    Requirement::Ne(k.trim().into(), v.trim().into())
                } else if let Some((k, v)) = part.split_once("==") {
                    Requirement::Eq(k.trim().into(), v.trim().into())
                } else if let Some((k, v)) = part.split_once('=') {
                    Requirement::Eq(k.trim().into(), v.trim().into())
                } else if let Some(k) = part.strip_prefix('!') {
                    Requirement::NotExists(k.trim().into())
                } else {
                    Requirement::Exists(part.into())
                }
            })
            .collect();
        Selector(reqs)
    }

    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.0.iter().all(|r| match r {
            Requirement::Eq(k, v) => labels.get(k) == Some(v),
            Requirement::Ne(k, v) => labels.get(k) != Some(v),
            Requirement::Exists(k) => labels.contains_key(k),
            Requirement::NotExists(k) => !labels.contains_key(k),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn selector_matches_all_requirements() {
        let s = Selector::parse("app.kubernetes.io/name=harbor,component=core");
        assert!(s.matches(&labels(&[
            ("app.kubernetes.io/name", "harbor"),
            ("component", "core"),
            ("extra", "x")
        ])));
        assert!(!s.matches(&labels(&[("app.kubernetes.io/name", "harbor")])));
    }

    #[test]
    fn selector_supports_negation_and_existence() {
        let s = Selector::parse("tier, env!=dev, !legacy");
        assert!(s.matches(&labels(&[("tier", "web"), ("env", "prod")])));
        assert!(!s.matches(&labels(&[("tier", "web"), ("env", "dev")])));
        assert!(!s.matches(&labels(&[("tier", "web"), ("legacy", "1")])));
        assert!(!s.matches(&labels(&[("env", "prod")])));
    }

    #[test]
    fn dom_ids_are_stable_and_distinct() {
        assert_eq!(dom_id("ingress/a/b"), dom_id("ingress/a/b"));
        assert_ne!(dom_id("ingress/a/b"), dom_id("ingress/a/c"));
        assert!(dom_id("x").starts_with("s-"));
    }
}
