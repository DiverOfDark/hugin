//! Turns Ingress / IngressRoute annotations into [`Service`]s.
//!
//! The `gethomepage.dev/*` annotations are read unchanged, so a cluster
//! annotated for gethomepage works without manifest edits. Every annotation
//! may also be written with the `hugin.dev/` prefix, which wins when both are
//! present.
//!
//! Credentials can be referenced instead of inlined:
//!
//! ```yaml
//! hugin.dev/widget.key-from-secret: my-secret/API_KEY
//! ```
//!
//! reads key `API_KEY` of Secret `my-secret` in the object's own namespace
//! and uses it as `widget.key`. A plain `widget.key` annotation, if present,
//! remains the fallback when the Secret cannot be read.

use std::collections::BTreeMap;

use crate::model::{dom_id, KubeRef, SecretRef, Service, Source, SourceKind, WidgetSpec};

pub const LEGACY_PREFIX: &str = "gethomepage.dev/";
pub const NATIVE_PREFIX: &str = "hugin.dev/";
const FROM_SECRET_SUFFIX: &str = "-from-secret";
const DEFAULT_GROUP: &str = "Kubernetes";

/// Builds a service from an annotated object. `kind` distinguishes sources
/// ("ingress", "ingressroute") so ids do not collide. Returns `None` unless
/// the object is opted in with `enabled: "true"`.
pub fn service_from_annotations(
    kind: &str,
    namespace: &str,
    name: &str,
    annotations: &BTreeMap<String, String>,
    default_href: Option<String>,
) -> Option<Service> {
    // Later prefixes override earlier ones.
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    for prefix in [LEGACY_PREFIX, NATIVE_PREFIX] {
        for (key, value) in annotations {
            if let Some(field) = key.strip_prefix(prefix) {
                fields.insert(field, value.trim());
            }
        }
    }

    if fields.get("enabled").copied() != Some("true") {
        return None;
    }

    let mut params = BTreeMap::new();
    let mut secrets = BTreeMap::new();
    for (field, value) in &fields {
        let Some(widget_field) = field.strip_prefix("widget.") else {
            continue;
        };
        if let Some(target) = widget_field.strip_suffix(FROM_SECRET_SUFFIX) {
            // Only the native prefix may reference Secrets.
            let native_key = format!("{NATIVE_PREFIX}{field}");
            if !annotations.contains_key(&native_key) {
                continue;
            }
            match value.split_once('/') {
                Some((secret, key)) if !secret.is_empty() && !key.is_empty() => {
                    secrets.insert(
                        target.to_string(),
                        SecretRef {
                            namespace: namespace.to_string(),
                            name: secret.to_string(),
                            key: key.to_string(),
                        },
                    );
                }
                _ => tracing::warn!(
                    namespace,
                    name,
                    annotation = %native_key,
                    "ignoring secret reference, expected \"<secret-name>/<key>\""
                ),
            }
        } else {
            params.insert(widget_field.to_string(), value.to_string());
        }
    }

    let widget = params.remove("type").map(|kind| WidgetSpec {
        kind,
        params,
        secrets,
    });

    let app = fields.get("app").copied().unwrap_or(name);
    let pod_selector = fields
        .get("pod-selector")
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("app.kubernetes.io/name={app}"));

    let text = |key: &str| {
        fields
            .get(key)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
    };

    Some(Service {
        id: dom_id(&format!("{kind}/{namespace}/{name}")),
        source: Source {
            kind: if kind == "ingressroute" {
                SourceKind::IngressRoute
            } else {
                SourceKind::Ingress
            },
            name: name.to_string(),
        },
        group: text("group").unwrap_or_else(|| DEFAULT_GROUP.to_string()),
        name: text("name").unwrap_or_else(|| name.to_string()),
        href: text("href").or(default_href),
        icon: text("icon"),
        description: text("description"),
        weight: fields
            .get("weight")
            .and_then(|w| w.parse().ok())
            .unwrap_or(0),
        kube: Some(KubeRef {
            namespace: namespace.to_string(),
            pod_selector,
        }),
        widget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn ignores_objects_that_are_not_enabled() {
        let a = ann(&[("gethomepage.dev/name", "X")]);
        assert!(service_from_annotations("ingress", "ns", "x", &a, None).is_none());
        let a = ann(&[("gethomepage.dev/enabled", "false")]);
        assert!(service_from_annotations("ingress", "ns", "x", &a, None).is_none());
    }

    #[test]
    fn reads_gethomepage_annotations_unchanged() {
        let a = ann(&[
            ("cert-manager.io/cluster-issuer", "acme"),
            ("gethomepage.dev/enabled", "true"),
            ("gethomepage.dev/name", "Radarr"),
            ("gethomepage.dev/href", "https://radarr.example.com"),
            ("gethomepage.dev/group", "Media"),
            ("gethomepage.dev/icon", "radarr.png"),
            ("gethomepage.dev/description", "Movies"),
            (
                "gethomepage.dev/pod-selector",
                "app.kubernetes.io/pod=radarr",
            ),
            ("gethomepage.dev/widget.type", "radarr"),
            ("gethomepage.dev/widget.url", "http://radarr.media.svc "),
            ("gethomepage.dev/widget.key", "plain"),
            ("gethomepage.dev/widget.enableQueue", "false"),
        ]);
        let s = service_from_annotations("ingress", "media", "radarr", &a, None).unwrap();
        assert_eq!(s.name, "Radarr");
        assert_eq!(s.group, "Media");
        assert_eq!(s.href.as_deref(), Some("https://radarr.example.com"));
        assert_eq!(s.icon.as_deref(), Some("radarr.png"));
        assert_eq!(s.description.as_deref(), Some("Movies"));
        assert_eq!(s.weight, 0);
        let kube = s.kube.unwrap();
        assert_eq!(kube.namespace, "media");
        assert_eq!(kube.pod_selector, "app.kubernetes.io/pod=radarr");
        let w = s.widget.unwrap();
        assert_eq!(w.kind, "radarr");
        assert_eq!(w.params["url"], "http://radarr.media.svc");
        assert_eq!(w.params["key"], "plain");
        assert_eq!(w.params["enableQueue"], "false");
        assert!(w.secrets.is_empty());
    }

    #[test]
    fn defaults_follow_gethomepage() {
        let a = ann(&[("gethomepage.dev/enabled", "true")]);
        let s = service_from_annotations(
            "ingress",
            "cilium",
            "hubble-ui",
            &a,
            Some("https://hubble.example.com".into()),
        )
        .unwrap();
        assert_eq!(s.name, "hubble-ui");
        assert_eq!(s.group, "Kubernetes");
        assert_eq!(s.href.as_deref(), Some("https://hubble.example.com"));
        assert_eq!(
            s.kube.unwrap().pod_selector,
            "app.kubernetes.io/name=hubble-ui"
        );
        assert!(s.widget.is_none());
    }

    #[test]
    fn secret_references_are_scoped_to_the_object_namespace() {
        let a = ann(&[
            ("gethomepage.dev/enabled", "true"),
            ("gethomepage.dev/widget.type", "grafana"),
            ("gethomepage.dev/widget.url", "https://grafana.example.com/"),
            ("gethomepage.dev/widget.password", "old-plaintext"),
            (
                "hugin.dev/widget.username-from-secret",
                "grafana-admin/GF_USER",
            ),
            (
                "hugin.dev/widget.password-from-secret",
                "grafana-admin/GF_PASSWORD",
            ),
        ]);
        let w = service_from_annotations("ingress", "prometheus", "grafana", &a, None)
            .unwrap()
            .widget
            .unwrap();
        assert_eq!(
            w.secrets["password"],
            SecretRef {
                namespace: "prometheus".into(),
                name: "grafana-admin".into(),
                key: "GF_PASSWORD".into()
            }
        );
        assert_eq!(w.secrets["username"].key, "GF_USER");
        // The plaintext annotation stays available as the fallback.
        assert_eq!(w.params["password"], "old-plaintext");
        assert!(!w.params.contains_key("password-from-secret"));
    }

    #[test]
    fn legacy_prefix_cannot_reference_secrets() {
        let a = ann(&[
            ("gethomepage.dev/enabled", "true"),
            ("gethomepage.dev/widget.type", "argocd"),
            ("gethomepage.dev/widget.key-from-secret", "other/token"),
        ]);
        let w = service_from_annotations("ingress", "ns", "x", &a, None)
            .unwrap()
            .widget
            .unwrap();
        assert!(w.secrets.is_empty());
    }

    #[test]
    fn malformed_secret_reference_is_ignored() {
        let a = ann(&[
            ("gethomepage.dev/enabled", "true"),
            ("gethomepage.dev/widget.type", "argocd"),
            ("hugin.dev/widget.key-from-secret", "no-slash"),
        ]);
        let w = service_from_annotations("ingress", "ns", "x", &a, None)
            .unwrap()
            .widget
            .unwrap();
        assert!(w.secrets.is_empty());
    }

    #[test]
    fn native_prefix_overrides_legacy() {
        let a = ann(&[
            ("gethomepage.dev/enabled", "true"),
            ("gethomepage.dev/name", "Old"),
            ("hugin.dev/name", "New"),
            ("hugin.dev/weight", "7"),
        ]);
        let s = service_from_annotations("ingress", "ns", "x", &a, None).unwrap();
        assert_eq!(s.name, "New");
        assert_eq!(s.weight, 7);
    }
}
