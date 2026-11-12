//! The `Widget` custom resource (`widgets.hugin.dev`): a Lua widget script
//! stored in the cluster.
//!
//! A service selects one with `widget.type: <name>`; hugin looks in the
//! service's own namespace first, then in hugin's namespace, then at the
//! bundled scripts. `widget.type: auto` selects by the image the service's
//! pods run (`spec.image`), which is how generated widgets are found.

use kube::api::{ApiResource, GroupVersionKind};
use kube::core::Object;
use serde::{Deserialize, Serialize};

pub const GROUP: &str = "hugin.dev";
pub const VERSION: &str = "v1alpha1";
pub const KIND: &str = "Widget";
pub const PLURAL: &str = "widgets";
/// Set on widgets written by the generator. Widgets without it are never
/// overwritten.
pub const GENERATED_LABEL: &str = "hugin.dev/generated";

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSpec {
    /// The Lua script. See docs/widget-api.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// Image repository (no tag) this widget is for, used by
    /// `widget.type: auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetStatus {
    /// Ready or Failed (generated widgets only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    /// Image tag the script was generated and validated against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
}

pub type WidgetResource = Object<WidgetSpec, WidgetStatus>;

pub fn api_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(GROUP, VERSION, KIND), PLURAL)
}

/// `ghcr.io/org/app:1.2@sha256:...` -> (`ghcr.io/org/app`, `1.2`).
pub fn split_image(image: &str) -> (String, Option<String>) {
    let without_digest = image.split('@').next().unwrap_or(image);
    match without_digest.rfind(':') {
        // A colon before the last slash is a registry port, not a tag.
        Some(colon) if !without_digest[colon..].contains('/') => (
            without_digest[..colon].to_string(),
            Some(without_digest[colon + 1..].to_string()),
        ),
        _ => (without_digest.to_string(), None),
    }
}

/// A DNS-label-safe object name for an image repository.
pub fn widget_name_for_image(repo: &str) -> String {
    let mut name = String::new();
    for c in repo.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-');
    // Keep the end, which is the distinctive part of an image path.
    let tail = if name.len() > 58 {
        &name[name.len() - 58..]
    } else {
        name
    };
    format!("auto-{}", tail.trim_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_split_into_repo_and_tag() {
        assert_eq!(
            split_image("ghcr.io/linuxserver/radarr:5.2.1"),
            ("ghcr.io/linuxserver/radarr".into(), Some("5.2.1".into()))
        );
        assert_eq!(
            split_image("localhost:25050/hugin/hugin@sha256:abc"),
            ("localhost:25050/hugin/hugin".into(), None)
        );
        assert_eq!(
            split_image("registry:5000/app:v2@sha256:abc"),
            ("registry:5000/app".into(), Some("v2".into()))
        );
        assert_eq!(split_image("nginx"), ("nginx".into(), None));
    }

    #[test]
    fn widget_names_are_valid_object_names() {
        assert_eq!(
            widget_name_for_image("ghcr.io/linuxserver/radarr"),
            "auto-ghcr-io-linuxserver-radarr"
        );
        let long = widget_name_for_image(&format!("registry.example.com/{}/app", "x".repeat(80)));
        assert!(long.len() <= 63, "{long}");
        assert!(long.ends_with("-app"));
    }

    #[test]
    fn spec_uses_camel_case_and_omits_empty_fields() {
        let status = WidgetStatus {
            phase: Some("Ready".into()),
            image_tag: Some("1.2".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"phase":"Ready","imageTag":"1.2"}"#
        );
    }
}
