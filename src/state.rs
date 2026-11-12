//! Shared in-memory state. Everything is behind short-lived std mutexes that
//! are never held across an await.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{broadcast, Notify};

use crate::config::Config;
use crate::metrics::Metrics;
use crate::model::{KubeRef, PodStatus, Selector, Service};
use crate::weather::WeatherNow;
use crate::widgets::{LuaWidget, WidgetData};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PodInfo {
    pub labels: BTreeMap<String, String>,
    pub phase: String,
    /// Sum of container CPU limits, in cores.
    pub cpu_limit: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    /// cores
    pub cpu: f64,
    /// bytes
    pub memory: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeInfo {
    pub name: String,
    pub ready: bool,
    pub cpu_capacity: f64,
    pub memory_capacity: f64,
    pub usage: Option<Usage>,
}

/// What a card shows for a widget.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetState {
    Data(WidgetData),
    Error(String),
    /// No script to run yet (`widget.type: auto` before one is generated).
    Pending(String),
}

/// A `Widget` custom resource as the dashboard needs it.
#[derive(Clone)]
pub struct CustomWidget {
    /// `None` when the resource has no script (a failed generation).
    pub widget: Option<Arc<LuaWidget>>,
    /// Why there is no script, from the generator's status.
    pub failure: Option<String>,
}

/// Outcome of looking up the script for a service.
pub enum Resolved {
    Script(Arc<LuaWidget>),
    Pending(String),
}

/// Pod health and usage for one service.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServiceHealth {
    pub status: PodStatus,
    /// `None` until pod metrics have been fetched.
    pub usage: Option<Usage>,
    pub cpu_limit: f64,
}

#[derive(Debug, Clone)]
pub enum Event {
    Fragment { id: Arc<str>, html: Arc<str> },
    Reload,
}

/// The last rendered dashboard: card HTML by id plus the group layout.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub groups: Vec<(String, Vec<Arc<str>>)>,
    pub fragments: HashMap<Arc<str>, Arc<str>>,
}

pub struct AppState {
    pub cfg: Config,
    /// Scripts from `Widget` resources, by (namespace, name).
    pub custom_widgets: Mutex<BTreeMap<(String, String), CustomWidget>>,
    /// The namespace hugin runs in; its `Widget` resources apply everywhere.
    pub own_namespace: String,
    pub metrics: Metrics,
    pub static_services: Vec<Service>,
    /// Discovered services by source key ("ingress/<ns>/<name>").
    pub discovered: Mutex<BTreeMap<String, Service>>,
    /// namespace -> pod name -> info
    pub pods: Mutex<HashMap<String, HashMap<String, PodInfo>>>,
    /// namespace -> pod name -> usage
    pub pod_usage: Mutex<Option<HashMap<String, HashMap<String, Usage>>>>,
    pub nodes: Mutex<Vec<NodeInfo>>,
    pub widgets: Mutex<HashMap<String, WidgetState>>,
    /// Consecutive failed refreshes per service and the latest reason, for
    /// widgets still showing their last good values.
    pub widget_failures: Mutex<HashMap<String, (u32, String)>>,
    pub weather: Mutex<Option<WeatherNow>>,
    /// Favicons by service id.
    pub favicons: Mutex<HashMap<String, crate::favicon::Entry>>,
    pub view: Mutex<View>,
    pub events: broadcast::Sender<Event>,
    /// Bumped on every broadcast; lets a new SSE client tell whether the
    /// page it loaded is still current.
    pub version: AtomicU64,
    /// Something changed; the render loop should diff and broadcast.
    pub dirty: Notify,
    /// Someone is watching again; pollers should resume.
    pub wake: Notify,
    /// A widget script changed; refresh widgets without waiting out the interval.
    pub refresh_now: Notify,
    pub sse_clients: AtomicUsize,
    last_seen: AtomicU64,
    started: Instant,
}

impl AppState {
    pub fn new(cfg: Config) -> Self {
        let (events, _) = broadcast::channel(256);
        let static_services = cfg.static_services();
        Self {
            cfg,
            custom_widgets: Mutex::default(),
            own_namespace: std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "hugin".into()),
            metrics: Metrics::default(),
            static_services,
            discovered: Mutex::default(),
            pods: Mutex::default(),
            pod_usage: Mutex::default(),
            nodes: Mutex::default(),
            widgets: Mutex::default(),
            widget_failures: Mutex::default(),
            weather: Mutex::default(),
            favicons: Mutex::default(),
            view: Mutex::default(),
            events,
            version: AtomicU64::new(1),
            dirty: Notify::new(),
            wake: Notify::new(),
            refresh_now: Notify::new(),
            sse_clients: AtomicUsize::new(0),
            last_seen: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Records a page view or live client and resumes polling if it had
    /// gone idle.
    pub fn touch(&self) {
        let was_active = self.is_active();
        // +1 so that "never seen" (0) stays distinguishable.
        self.last_seen.store(self.now() + 1, Ordering::Relaxed);
        if !was_active {
            self.wake.notify_waiters();
        }
    }

    /// Upstreams are only polled while somebody is looking.
    pub fn is_active(&self) -> bool {
        if self.sse_clients.load(Ordering::Relaxed) > 0 {
            return true;
        }
        let seen = self.last_seen.load(Ordering::Relaxed);
        seen != 0 && self.now() + 1 - seen <= self.cfg.intervals.idle_after_seconds
    }

    /// Waits until polling should run.
    pub async fn wait_active(&self) {
        loop {
            let notified = self.wake.notified();
            if self.is_active() {
                return;
            }
            notified.await;
        }
    }

    /// All services, grouped and ordered the way gethomepage orders them:
    /// groups alphabetically (after any configured order), services by
    /// weight then name.
    pub fn grouped_services(&self) -> Vec<(String, Vec<Service>)> {
        let mut by_group: BTreeMap<String, Vec<Service>> = BTreeMap::new();
        let discovered = self.discovered.lock().unwrap().clone();
        for service in discovered
            .into_values()
            .chain(self.static_services.iter().cloned())
        {
            by_group
                .entry(service.group.clone())
                .or_default()
                .push(service);
        }
        for services in by_group.values_mut() {
            services.sort_by(|a, b| {
                a.weight
                    .cmp(&b.weight)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        let mut ordered = Vec::with_capacity(by_group.len());
        for name in &self.cfg.group_order {
            if let Some(services) = by_group.remove(name) {
                ordered.push((name.clone(), services));
            }
        }
        ordered.extend(by_group);
        ordered
    }

    pub fn services(&self) -> Vec<Service> {
        self.grouped_services()
            .into_iter()
            .flat_map(|(_, services)| services)
            .collect()
    }

    /// Finds the script for a service's widget.
    ///
    /// 1. A `Widget` named like the service's `widget.type`, in the
    ///    service's namespace, then in hugin's: hand-written ones.
    /// 2. The `Widget` the generator wrote for this service.
    ///
    /// There are no scripts compiled into the binary.
    pub fn resolve_widget(&self, service: &Service, kind: &str) -> Resolved {
        let custom = self.custom_widgets.lock().unwrap();
        let namespace = service
            .kube
            .as_ref()
            .map_or(self.own_namespace.as_str(), |k| k.namespace.as_str());
        let script = |ns: &str, name: &str| {
            custom
                .get(&(ns.to_string(), name.to_string()))
                .and_then(|w| w.widget.clone())
        };
        if let Some(widget) = script(namespace, kind).or_else(|| script(&self.own_namespace, kind))
        {
            return Resolved::Script(widget);
        }
        let generated = service.generated_widget_name();
        if let Some(widget) = script(namespace, &generated) {
            return Resolved::Script(widget);
        }
        let failure = custom
            .get(&(namespace.to_string(), generated))
            .and_then(|w| w.failure.clone());
        Resolved::Pending(match failure {
            Some(reason) => format!("no widget: {reason}"),
            None => "waiting for a widget to be generated".to_string(),
        })
    }

    /// Pod status exactly as gethomepage computes it (pod phase, not
    /// readiness), plus summed usage of the matching pods.
    pub fn health(&self, kube: &KubeRef) -> ServiceHealth {
        let selector = Selector::parse(&kube.pod_selector);
        let pods = self.pods.lock().unwrap();
        let matching: Vec<(&String, &PodInfo)> = pods
            .get(&kube.namespace)
            .map(|ns| {
                ns.iter()
                    .filter(|(_, p)| selector.matches(&p.labels))
                    .collect()
            })
            .unwrap_or_default();

        let up = |p: &PodInfo| p.phase == "Running" || p.phase == "Succeeded";
        let status = if matching.is_empty() {
            PodStatus::NotFound
        } else if matching.iter().all(|(_, p)| up(p)) {
            PodStatus::Running
        } else if matching.iter().any(|(_, p)| up(p)) {
            PodStatus::Partial
        } else {
            PodStatus::Down
        };
        let cpu_limit = matching.iter().map(|(_, p)| p.cpu_limit).sum();

        let usage_map = self.pod_usage.lock().unwrap();
        let usage = usage_map.as_ref().map(|all| {
            let ns = all.get(&kube.namespace);
            matching
                .iter()
                .filter_map(|(name, _)| ns.and_then(|ns| ns.get(*name)))
                .fold(Usage::default(), |acc, u| Usage {
                    cpu: acc.cpu + u.cpu,
                    memory: acc.memory + u.memory,
                })
        });
        ServiceHealth {
            status,
            usage,
            cpu_limit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dom_id;

    fn service(group: &str, name: &str, weight: i64) -> Service {
        Service {
            id: dom_id(&format!("{group}/{name}")),
            source: crate::model::Source {
                kind: crate::model::SourceKind::Ingress,
                name: name.to_lowercase(),
            },
            group: group.into(),
            name: name.into(),
            href: None,
            icon: None,
            description: None,
            weight,
            kube: None,
            widget: None,
        }
    }

    fn pod(phase: &str, app: &str, cpu_limit: f64) -> PodInfo {
        PodInfo {
            labels: [("app".to_string(), app.to_string())].into(),
            phase: phase.into(),
            cpu_limit,
        }
    }

    fn kube_ref(selector: &str) -> KubeRef {
        KubeRef {
            namespace: "ns".into(),
            pod_selector: selector.into(),
        }
    }

    #[test]
    fn groups_alphabetical_services_by_weight_then_name_ignoring_case() {
        let cfg: Config = serde_json::from_str(
            r#"{"services":[{"group":"Infra","name":"FritzBox"},{"group":"Infra","name":"Garage"}]}"#,
        )
        .unwrap();
        let state = AppState::new(cfg);
        {
            let mut d = state.discovered.lock().unwrap();
            for s in [
                service("Media", "Emby", 0),
                service("Infra", "vaultwarden", 0),
                service("Infra", "Victoria Logs", 0),
                service("Infra", "Status Page", 0),
                service("Apps", "TRMNL Cyberpunk", 0),
                service("Apps", "Trackhound", 0),
            ] {
                d.insert(s.id.clone(), s);
            }
        }
        let grouped = state.grouped_services();
        let names =
            |i: usize| -> Vec<&str> { grouped[i].1.iter().map(|s| s.name.as_str()).collect() };
        assert_eq!(
            grouped.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(),
            vec!["Apps", "Infra", "Media"]
        );
        assert_eq!(names(0), vec!["Trackhound", "TRMNL Cyberpunk"]);
        assert_eq!(
            names(1),
            vec![
                "Status Page",
                "vaultwarden",
                "Victoria Logs",
                "FritzBox",
                "Garage"
            ]
        );
    }

    #[test]
    fn configured_group_order_comes_first() {
        let cfg: Config = serde_json::from_str(r#"{"groupOrder":["Media","Nope"]}"#).unwrap();
        let state = AppState::new(cfg);
        {
            let mut d = state.discovered.lock().unwrap();
            for s in [service("Apps", "a", 0), service("Media", "m", 0)] {
                d.insert(s.id.clone(), s);
            }
        }
        let grouped = state.grouped_services();
        assert_eq!(grouped[0].0, "Media");
        assert_eq!(grouped[1].0, "Apps");
    }

    #[test]
    fn health_follows_pod_phase() {
        let state = AppState::new(Config::default());
        assert_eq!(state.health(&kube_ref("app=a")).status, PodStatus::NotFound);

        let mut ns = HashMap::new();
        ns.insert("a-1".to_string(), pod("Running", "a", 0.5));
        ns.insert("a-job".to_string(), pod("Succeeded", "a", 0.0));
        ns.insert("b-1".to_string(), pod("Running", "b", 0.0));
        ns.insert("b-2".to_string(), pod("Pending", "b", 0.0));
        ns.insert("c-1".to_string(), pod("Failed", "c", 0.0));
        state.pods.lock().unwrap().insert("ns".into(), ns);

        let a = state.health(&kube_ref("app=a"));
        assert_eq!(a.status, PodStatus::Running);
        assert_eq!(a.cpu_limit, 0.5);
        assert_eq!(a.usage, None, "no metrics fetched yet");
        assert_eq!(state.health(&kube_ref("app=b")).status, PodStatus::Partial);
        assert_eq!(state.health(&kube_ref("app=c")).status, PodStatus::Down);

        let mut usage = HashMap::new();
        usage.insert(
            "ns".to_string(),
            HashMap::from([
                (
                    "a-1".to_string(),
                    Usage {
                        cpu: 0.2,
                        memory: 100.0,
                    },
                ),
                (
                    "b-1".to_string(),
                    Usage {
                        cpu: 9.0,
                        memory: 9.0,
                    },
                ),
            ]),
        );
        *state.pod_usage.lock().unwrap() = Some(usage);
        assert_eq!(
            state.health(&kube_ref("app=a")).usage,
            Some(Usage {
                cpu: 0.2,
                memory: 100.0
            })
        );
    }

    #[test]
    fn widgets_resolve_hand_written_then_generated() {
        let state = AppState::new(Config::default());
        let mut svc = service("Media", "Radarr", 0);
        svc.kube = Some(KubeRef {
            namespace: "media".into(),
            pod_selector: "app=a".into(),
        });
        let script = |text: &str| CustomWidget {
            widget: Some(Arc::new(LuaWidget::new("x", text))),
            failure: None,
        };
        let which =
            |state: &AppState, svc: &Service, kind: &str| match state.resolve_widget(svc, kind) {
                Resolved::Script(w) => w.script().to_string(),
                Resolved::Pending(m) => format!("pending: {m}"),
            };
        let put = |ns: &str, name: &str, w: CustomWidget| {
            state
                .custom_widgets
                .lock()
                .unwrap()
                .insert((ns.to_string(), name.to_string()), w);
        };

        assert_eq!(
            which(&state, &svc, "radarr"),
            "pending: waiting for a widget to be generated"
        );

        // A failed generation explains itself.
        put(
            "media",
            "auto-radarr",
            CustomWidget {
                widget: None,
                failure: Some("HTTP 500".into()),
            },
        );
        assert_eq!(which(&state, &svc, "auto"), "pending: no widget: HTTP 500");

        put("media", "auto-radarr", script("-- generated"));
        assert_eq!(which(&state, &svc, "radarr"), "-- generated");
        // ...a shared hand-written one beats the generated one...
        put(&state.own_namespace, "radarr", script("-- shared"));
        assert_eq!(which(&state, &svc, "radarr"), "-- shared");
        // ...and one in the service's own namespace beats that.
        put("media", "radarr", script("-- own namespace"));
        assert_eq!(which(&state, &svc, "radarr"), "-- own namespace");

        // A generated widget in another namespace is not this service's.
        let mut other = service("Media", "Radarr", 0);
        other.kube = Some(KubeRef {
            namespace: "elsewhere".into(),
            pod_selector: "app=a".into(),
        });
        assert_eq!(
            which(&state, &other, "auto"),
            "pending: waiting for a widget to be generated"
        );
    }

    #[test]
    fn polling_is_idle_until_someone_looks() {
        let state = AppState::new(Config::default());
        assert!(!state.is_active());
        state.touch();
        assert!(state.is_active());
        state.last_seen.store(0, Ordering::Relaxed);
        state.sse_clients.store(1, Ordering::Relaxed);
        assert!(state.is_active());
    }
}
