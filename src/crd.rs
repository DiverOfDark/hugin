//! The `Widget` custom resource (`widgets.hugin.dev`): a Lua widget script
//! stored in the cluster.
//!
//! The generator writes one per service, named after the Ingress and owned
//! by it. A hand-written `Widget` named like the service's `widget.type`,
//! in the service's namespace or in hugin's, takes precedence.

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Written by the generator on widgets it created.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetStatus {
    /// Ready or Failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    /// Image the service ran when the script was validated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tag: Option<String>,
    /// Hash of the widget annotations the script was written for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hash: Option<String>,
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
