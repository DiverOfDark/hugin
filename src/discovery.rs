//! Turns Ingress / IngressRoute annotations into [`Service`]s.
//!
//! Every annotation starts with `hugin.dev/`; an object is listed once it
//! carries `hugin.dev/enabled: "true"`.
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

pub const PREFIX: &str = "hugin.dev/";
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
    let fields: BTreeMap<&str, &str> = annotations
        .iter()
        .filter_map(|(key, value)| Some((key.strip_prefix(PREFIX)?, value.trim())))
        .collect();

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
                    annotation = %format!("{PREFIX}{field}"),
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
        let a = ann(&[("hugin.dev/name", "X")]);
        assert!(service_from_annotations("ingress", "ns", "x", &a, None).is_none());
        let a = ann(&[("hugin.dev/enabled", "false")]);
        assert!(service_from_annotations("ingress", "ns", "x", &a, None).is_none());
    }

    #[test]
    fn reads_every_field() {
        let a = ann(&[
            ("cert-manager.io/cluster-issuer", "acme"),
            ("hugin.dev/enabled", "true"),
            ("hugin.dev/name", "Radarr"),
            ("hugin.dev/href", "https://radarr.example.com"),
            ("hugin.dev/group", "Media"),
            ("hugin.dev/icon", "radarr.png"),
            ("hugin.dev/description", "Movies"),
            ("hugin.dev/pod-selector", "app.kubernetes.io/pod=radarr"),
            ("hugin.dev/widget.type", "radarr"),
            ("hugin.dev/widget.url", "http://radarr.media.svc "),
            ("hugin.dev/widget.key", "plain"),
            ("hugin.dev/widget.enableQueue", "false"),
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
    fn defaults_come_from_the_object() {
        let a = ann(&[("hugin.dev/enabled", "true")]);
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
            ("hugin.dev/enabled", "true"),
            ("hugin.dev/widget.type", "grafana"),
            ("hugin.dev/widget.url", "https://grafana.example.com/"),
            ("hugin.dev/widget.password", "old-plaintext"),
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
    fn malformed_secret_reference_is_ignored() {
        let a = ann(&[
            ("hugin.dev/enabled", "true"),
            ("hugin.dev/widget.type", "argocd"),
            ("hugin.dev/widget.key-from-secret", "no-slash"),
        ]);
        let w = service_from_annotations("ingress", "ns", "x", &a, None)
            .unwrap()
            .widget
            .unwrap();
        assert!(w.secrets.is_empty());
    }
}
