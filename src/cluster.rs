//! Everything that talks to the Kubernetes API: watches for Ingresses,
//! IngressRoutes and Pods, and periodic reads of nodes and metrics.
//!
//! Pods and nodes are deserialized into trimmed-down types so a large cluster
//! does not cost megabytes of image lists, env vars and volume specs.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{Node, Pod};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{ApiResource, DynamicObject, GroupVersionKind, ListParams};
use kube::core::Object;
use kube::runtime::watcher::{self, Event};
use kube::runtime::WatchStreamExt;
use kube::{Api, Client, ResourceExt};
use serde::{Deserialize, Serialize};

use crate::discovery::service_from_annotations;
use crate::format::quantity;
use crate::metrics::ResourceLabels;
use crate::model::Service;
use crate::state::{AppState, NodeInfo, PodInfo, Usage};

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct PodSpecLite {
    #[serde(default)]
    containers: Vec<ContainerLite>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
struct ContainerLite {
    #[serde(default)]
    resources: Option<ResourcesLite>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
struct ResourcesLite {
    #[serde(default)]
    limits: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct PodStatusLite {
    #[serde(default)]
    phase: Option<String>,
}

type PodLite = Object<PodSpecLite, PodStatusLite>;

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Ignored {}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct NodeStatusLite {
    #[serde(default)]
    capacity: BTreeMap<String, String>,
    #[serde(default)]
    conditions: Vec<ConditionLite>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
struct ConditionLite {
    #[serde(rename = "type")]
    kind: String,
    status: String,
}

type NodeLite = Object<Ignored, NodeStatusLite>;

#[derive(Deserialize)]
struct MetricsList<T> {
    items: Vec<T>,
}

#[derive(Deserialize)]
struct MetricsMeta {
    name: String,
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Deserialize)]
struct UsageRaw {
    #[serde(default)]
    cpu: String,
    #[serde(default)]
    memory: String,
}

impl UsageRaw {
    fn parse(&self) -> Usage {
        Usage {
            cpu: quantity(&self.cpu).unwrap_or(0.0),
            memory: quantity(&self.memory).unwrap_or(0.0),
        }
    }
}

#[derive(Deserialize)]
struct NodeMetrics {
    metadata: MetricsMeta,
    usage: UsageRaw,
}

#[derive(Deserialize)]
struct PodMetrics {
    metadata: MetricsMeta,
    #[serde(default)]
    containers: Vec<ContainerMetrics>,
}

#[derive(Deserialize)]
struct ContainerMetrics {
    usage: UsageRaw,
}

fn ingress_service(ingress: &Ingress) -> Option<(String, Option<Service>)> {
    let namespace = ingress.namespace()?;
    let name = ingress.name_any();
    let spec = ingress.spec.as_ref();
    let host = spec
        .and_then(|s| s.rules.as_ref())
        .and_then(|rules| rules.iter().find_map(|r| r.host.clone()));
    let tls = spec
        .and_then(|s| s.tls.as_ref())
        .is_some_and(|t| !t.is_empty());
    let default_href = host.map(|h| format!("{}://{h}", if tls { "https" } else { "http" }));
    let service = service_from_annotations(
        "ingress",
        &namespace,
        &name,
        ingress.annotations(),
        default_href,
    );
    Some((format!("ingress/{namespace}/{name}"), service))
}

/// First hostname in a Traefik rule such as ``Host(`a.example.com`) && ...``.
fn traefik_host(rule: &str) -> Option<&str> {
    let start = rule.find("Host(`")? + "Host(`".len();
    let end = rule[start..].find('`')? + start;
    Some(&rule[start..end])
}

fn ingressroute_service(route: &DynamicObject) -> Option<(String, Option<Service>)> {
    let namespace = route.namespace()?;
    let name = route.name_any();
    let default_href = route.data["spec"]["routes"]
        .as_array()
        .and_then(|routes| {
            routes
                .iter()
                .find_map(|r| r["match"].as_str().and_then(traefik_host))
        })
        .map(|host| format!("https://{host}"));
    let service = service_from_annotations(
        "ingressroute",
        &namespace,
        &name,
        route.annotations(),
        default_href,
    );
    Some((format!("ingressroute/{namespace}/{name}"), service))
}

/// Applies watch events for one kind of discovered object to
/// `state.discovered`. Keys of that kind which are missing after a re-list
/// are dropped.
struct DiscoverySync {
    prefix: &'static str,
    pending: Option<BTreeMap<String, Service>>,
}

impl DiscoverySync {
    fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            pending: None,
        }
    }

    fn apply(&mut self, state: &AppState, event: Event<(String, Option<Service>)>) {
        match event {
            Event::Init => self.pending = Some(BTreeMap::new()),
            Event::InitApply((key, service)) => {
                if let (Some(pending), Some(service)) = (self.pending.as_mut(), service) {
                    pending.insert(key, service);
                }
            }
            Event::InitDone => {
                let fresh = self.pending.take().unwrap_or_default();
                let mut discovered = state.discovered.lock().unwrap();
                discovered.retain(|key, _| !key.starts_with(self.prefix));
                discovered.extend(fresh);
                drop(discovered);
                state.dirty.notify_one();
            }
            Event::Apply((key, service)) => {
                let mut discovered = state.discovered.lock().unwrap();
                let changed = match service {
                    Some(service) => discovered.insert(key, service.clone()) != Some(service),
                    None => discovered.remove(&key).is_some(),
                };
                drop(discovered);
                if changed {
                    state.dirty.notify_one();
                }
            }
            Event::Delete((key, _)) => {
                if state.discovered.lock().unwrap().remove(&key).is_some() {
                    state.dirty.notify_one();
                }
            }
        }
    }
}

fn map_event<K, T>(event: Event<K>, f: impl Fn(&K) -> Option<T>) -> Option<Event<T>> {
    Some(match event {
        Event::Init => Event::Init,
        Event::InitDone => Event::InitDone,
        Event::InitApply(obj) => Event::InitApply(f(&obj)?),
        Event::Apply(obj) => Event::Apply(f(&obj)?),
        Event::Delete(obj) => Event::Delete(f(&obj)?),
    })
}

fn watch_error(state: &AppState, resource: &'static str, error: &watcher::Error) {
    state
        .metrics
        .watch_errors
        .get_or_create(&ResourceLabels { resource })
        .inc();
    tracing::warn!(resource, %error, "watch failed, retrying");
}

pub async fn watch_ingresses(state: Arc<AppState>, client: Client) {
    let api: Api<Ingress> = Api::all(client);
    let mut sync = DiscoverySync::new("ingress/");
    let mut stream =
        std::pin::pin!(watcher::watcher(api, watcher::Config::default()).default_backoff());
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                let done = matches!(event, Event::InitDone);
                if let Some(event) = map_event(event, ingress_service) {
                    sync.apply(&state, event);
                }
                if done {
                    state.discovery_synced.store(true, Ordering::Relaxed);
                }
            }
            Err(error) => watch_error(&state, "ingresses", &error),
        }
    }
}

pub async fn watch_ingressroutes(state: Arc<AppState>, client: Client) {
    let gvk = GroupVersionKind::gvk("traefik.io", "v1alpha1", "IngressRoute");
    let resource = ApiResource::from_gvk_with_plural(&gvk, "ingressroutes");
    let api: Api<DynamicObject> = Api::all_with(client, &resource);
    let mut sync = DiscoverySync::new("ingressroute/");
    let mut stream =
        std::pin::pin!(watcher::watcher(api, watcher::Config::default()).default_backoff());
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if let Some(event) = map_event(event, ingressroute_service) {
                    sync.apply(&state, event);
                }
            }
            Err(error) => watch_error(&state, "ingressroutes", &error),
        }
    }
}

fn pod_info(pod: &PodLite) -> Option<(String, String, PodInfo)> {
    let namespace = pod.metadata.namespace.clone()?;
    let name = pod.metadata.name.clone()?;
    let cpu_limit = pod
        .spec
        .containers
        .iter()
        .filter_map(|c| c.resources.as_ref()?.limits.as_ref()?.get("cpu"))
        .filter_map(|q| quantity(q))
        .sum();
    let info = PodInfo {
        labels: pod.metadata.labels.clone().unwrap_or_default(),
        phase: pod
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_default(),
        cpu_limit,
    };
    Some((namespace, name, info))
}

pub async fn watch_pods(state: Arc<AppState>, client: Client) {
    let resource = ApiResource::erase::<Pod>(&());
    let api: Api<PodLite> = Api::all_with(client, &resource);
    // Small pages keep the initial list from spiking memory.
    let config = watcher::Config::default().page_size(100);
    let mut pending: Option<HashMap<String, HashMap<String, PodInfo>>> = None;
    let mut stream = std::pin::pin!(watcher::watcher(api, config).default_backoff());
    while let Some(event) = stream.next().await {
        match event {
            Ok(Event::Init) => pending = Some(HashMap::new()),
            Ok(Event::InitApply(pod)) => {
                if let (Some(pending), Some((ns, name, info))) = (pending.as_mut(), pod_info(&pod))
                {
                    pending.entry(ns).or_default().insert(name, info);
                }
            }
            Ok(Event::InitDone) => {
                *state.pods.lock().unwrap() = pending.take().unwrap_or_default();
                state.dirty.notify_one();
            }
            Ok(Event::Apply(pod)) => {
                if let Some((ns, name, info)) = pod_info(&pod) {
                    let mut pods = state.pods.lock().unwrap();
                    let changed =
                        pods.entry(ns).or_default().insert(name, info.clone()) != Some(info);
                    drop(pods);
                    if changed {
                        state.dirty.notify_one();
                    }
                }
            }
            Ok(Event::Delete(pod)) => {
                if let Some((ns, name, _)) = pod_info(&pod) {
                    let mut pods = state.pods.lock().unwrap();
                    let removed = pods.get_mut(&ns).and_then(|m| m.remove(&name)).is_some();
                    if pods.get(&ns).is_some_and(HashMap::is_empty) {
                        pods.remove(&ns);
                    }
                    drop(pods);
                    if removed {
                        state.dirty.notify_one();
                    }
                }
            }
            Err(error) => watch_error(&state, "pods", &error),
        }
    }
}

async fn metrics<T: serde::de::DeserializeOwned>(
    client: &Client,
    path: &str,
) -> anyhow::Result<Vec<T>> {
    let request = http::Request::get(path).body(Vec::new())?;
    let list: MetricsList<T> = client.request(request).await?;
    Ok(list.items)
}

/// Node capacity and readiness plus node and pod usage from metrics-server.
pub async fn refresh_resources(state: &AppState, client: &Client) -> anyhow::Result<()> {
    let resource = ApiResource::erase::<Node>(&());
    let api: Api<NodeLite> = Api::all_with(client.clone(), &resource);
    let nodes = api.list(&ListParams::default()).await?;

    let node_usage: HashMap<String, Usage> =
        metrics::<NodeMetrics>(client, "/apis/metrics.k8s.io/v1beta1/nodes")
            .await?
            .into_iter()
            .map(|m| (m.metadata.name, m.usage.parse()))
            .collect();

    let mut infos: Vec<NodeInfo> = nodes
        .items
        .iter()
        .filter_map(|node| {
            let name = node.metadata.name.clone()?;
            let status = node.status.clone().unwrap_or_default();
            Some(NodeInfo {
                ready: status
                    .conditions
                    .iter()
                    .any(|c| c.kind == "Ready" && c.status == "True"),
                cpu_capacity: status
                    .capacity
                    .get("cpu")
                    .and_then(|q| quantity(q))
                    .unwrap_or(0.0),
                memory_capacity: status
                    .capacity
                    .get("memory")
                    .and_then(|q| quantity(q))
                    .unwrap_or(0.0),
                usage: node_usage.get(&name).copied(),
                name,
            })
        })
        .collect();
    infos.sort_by(|a, b| a.name.cmp(&b.name));

    let mut pod_usage: HashMap<String, HashMap<String, Usage>> = HashMap::new();
    for pod in metrics::<PodMetrics>(client, "/apis/metrics.k8s.io/v1beta1/pods").await? {
        let Some(namespace) = pod.metadata.namespace else {
            continue;
        };
        let total = pod.containers.iter().fold(Usage::default(), |acc, c| {
            let u = c.usage.parse();
            Usage {
                cpu: acc.cpu + u.cpu,
                memory: acc.memory + u.memory,
            }
        });
        pod_usage
            .entry(namespace)
            .or_default()
            .insert(pod.metadata.name, total);
    }

    *state.nodes.lock().unwrap() = infos;
    *state.pod_usage.lock().unwrap() = Some(pod_usage);
    state.dirty.notify_one();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_is_reduced_to_labels_phase_and_cpu_limit() {
        let pod: PodLite = serde_json::from_value(serde_json::json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "web-1", "namespace": "apps", "labels": {"app": "web"},
                         "managedFields": [{"manager": "kubelet"}]},
            "spec": {
                "nodeName": "n1",
                "containers": [
                    {"name": "a", "image": "x", "resources": {"limits": {"cpu": "500m", "memory": "1Gi"}}},
                    {"name": "b", "image": "y", "resources": {"limits": {"cpu": "1"}}},
                    {"name": "c", "image": "z"}
                ]
            },
            "status": {"phase": "Running", "podIP": "10.0.0.1"}
        }))
        .unwrap();
        let (ns, name, info) = pod_info(&pod).unwrap();
        assert_eq!((ns.as_str(), name.as_str()), ("apps", "web-1"));
        assert_eq!(info.phase, "Running");
        assert_eq!(info.cpu_limit, 1.5);
        assert_eq!(info.labels["app"], "web");
    }

    #[test]
    fn ingress_default_href_uses_host_and_tls() {
        let ingress: Ingress = serde_json::from_value(serde_json::json!({
            "metadata": {"name": "web", "namespace": "apps",
                         "annotations": {"gethomepage.dev/enabled": "true"}},
            "spec": {"rules": [{"host": "web.example.com"}], "tls": [{"hosts": ["web.example.com"]}]}
        }))
        .unwrap();
        let (key, service) = ingress_service(&ingress).unwrap();
        assert_eq!(key, "ingress/apps/web");
        assert_eq!(
            service.unwrap().href.as_deref(),
            Some("https://web.example.com")
        );
    }

    #[test]
    fn unannotated_ingress_yields_no_service_but_keeps_its_key() {
        let ingress: Ingress = serde_json::from_value(serde_json::json!({
            "metadata": {"name": "web-external", "namespace": "apps"}
        }))
        .unwrap();
        let (key, service) = ingress_service(&ingress).unwrap();
        assert_eq!(key, "ingress/apps/web-external");
        assert!(service.is_none());
    }

    #[test]
    fn ingressroute_host_is_parsed_from_the_match_rule() {
        assert_eq!(
            traefik_host("Host(`ceph.example.com`) && PathPrefix(`/`)"),
            Some("ceph.example.com")
        );
        assert_eq!(traefik_host("PathPrefix(`/`)"), None);
        let route: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "traefik.io/v1alpha1", "kind": "IngressRoute",
            "metadata": {"name": "ceph", "namespace": "rook",
                         "annotations": {"gethomepage.dev/enabled": "true", "gethomepage.dev/name": "Ceph"}},
            "spec": {"routes": [{"match": "Host(`ceph.example.com`)", "kind": "Rule"}]}
        }))
        .unwrap();
        let (key, service) = ingressroute_service(&route).unwrap();
        assert_eq!(key, "ingressroute/rook/ceph");
        let service = service.unwrap();
        assert_eq!(service.name, "Ceph");
        assert_eq!(service.href.as_deref(), Some("https://ceph.example.com"));
    }

    #[test]
    fn relist_drops_vanished_objects_of_the_same_kind_only() {
        let state = AppState::new(crate::config::Config::default());
        let svc = |kind: &str, name: &str| {
            let ann = [("gethomepage.dev/enabled".to_string(), "true".to_string())].into();
            service_from_annotations(kind, "ns", name, &ann, None).unwrap()
        };
        state
            .discovered
            .lock()
            .unwrap()
            .insert("ingressroute/ns/r".into(), svc("ingressroute", "r"));

        let mut sync = DiscoverySync::new("ingress/");
        sync.apply(&state, Event::Init);
        sync.apply(
            &state,
            Event::InitApply(("ingress/ns/a".into(), Some(svc("ingress", "a")))),
        );
        sync.apply(&state, Event::InitApply(("ingress/ns/skip".into(), None)));
        sync.apply(&state, Event::InitDone);
        sync.apply(
            &state,
            Event::Apply(("ingress/ns/b".into(), Some(svc("ingress", "b")))),
        );
        assert_eq!(state.discovered.lock().unwrap().len(), 3);

        // Annotation removed: the service disappears.
        sync.apply(&state, Event::Apply(("ingress/ns/b".into(), None)));
        // Watch restarts and `a` is gone.
        sync.apply(&state, Event::Init);
        sync.apply(&state, Event::InitDone);
        let keys: Vec<String> = state.discovered.lock().unwrap().keys().cloned().collect();
        assert_eq!(keys, vec!["ingressroute/ns/r"]);
    }
}
