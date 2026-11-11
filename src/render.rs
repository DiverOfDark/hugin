//! Server-side HTML. The page is complete without JavaScript; the small
//! script in `assets/app.js` only swaps fragments pushed over SSE.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::format;
use crate::model::{PodStatus, Service};
use crate::state::{AppState, Event, NodeInfo, ServiceHealth, View, WidgetState};
use crate::weather::Icon;

pub const HEADER_RESOURCES_ID: &str = "hdr-resources";
pub const HEADER_WEATHER_ID: &str = "hdr-weather";

const DASHBOARD_ICONS: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons";

fn svg(body: &str, class: &str) -> Markup {
    html! {
        svg.(class) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
            stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            aria-hidden="true" { (PreEscaped(body)) }
    }
}

const SVG_SERVER: &str = r#"<rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/>"#;
const SVG_ALERT: &str = r#"<path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>"#;
const SVG_CLUSTER: &str = r#"<polygon points="12 2 20.5 6.5 22 15.5 16 22 8 22 2 15.5 3.5 6.5"/><circle cx="12" cy="12" r="3"/><line x1="12" y1="2" x2="12" y2="9"/><line x1="22" y1="15.5" x2="14.8" y2="13"/><line x1="2" y1="15.5" x2="9.2" y2="13"/>"#;
const SVG_CPU: &str = r#"<rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><line x1="9" y1="1" x2="9" y2="4"/><line x1="15" y1="1" x2="15" y2="4"/><line x1="9" y1="20" x2="9" y2="23"/><line x1="15" y1="20" x2="15" y2="23"/><line x1="20" y1="9" x2="23" y2="9"/><line x1="20" y1="14" x2="23" y2="14"/><line x1="1" y1="9" x2="4" y2="9"/><line x1="1" y1="14" x2="4" y2="14"/>"#;
const SVG_MEMORY: &str = r#"<rect x="2" y="7" width="20" height="10" rx="1"/><line x1="6" y1="10" x2="6" y2="14"/><line x1="10" y1="10" x2="10" y2="14"/><line x1="14" y1="10" x2="14" y2="14"/><line x1="18" y1="10" x2="18" y2="14"/><line x1="5" y1="17" x2="5" y2="20"/><line x1="12" y1="17" x2="12" y2="20"/><line x1="19" y1="17" x2="19" y2="20"/>"#;
const SVG_CLOUD_PATH: &str = r#"<path d="M20 16.58A5 5 0 0 0 18 7h-1.26A8 8 0 1 0 4 15.25"/>"#;

fn weather_icon(icon: Icon) -> Markup {
    let body = match icon {
        Icon::Sun => r#"<circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>"#.to_string(),
        Icon::Moon => r#"<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>"#.to_string(),
        Icon::Cloud => r#"<path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/>"#.to_string(),
        Icon::Fog => r#"<path d="M18 10h-1.26A8 8 0 1 0 6 17"/><line x1="9" y1="17" x2="21" y2="17"/><line x1="7" y1="21" x2="19" y2="21"/>"#.to_string(),
        Icon::Drizzle => format!(r#"{SVG_CLOUD_PATH}<line x1="8" y1="19" x2="8" y2="21"/><line x1="8" y1="13" x2="8" y2="15"/><line x1="16" y1="19" x2="16" y2="21"/><line x1="16" y1="13" x2="16" y2="15"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="12" y1="15" x2="12" y2="17"/>"#),
        Icon::Rain => format!(r#"{SVG_CLOUD_PATH}<line x1="16" y1="13" x2="16" y2="21"/><line x1="8" y1="13" x2="8" y2="21"/><line x1="12" y1="15" x2="12" y2="23"/>"#),
        Icon::Snow => format!(r#"{SVG_CLOUD_PATH}<line x1="8" y1="16" x2="8.01" y2="16"/><line x1="8" y1="20" x2="8.01" y2="20"/><line x1="12" y1="18" x2="12.01" y2="18"/><line x1="12" y1="22" x2="12.01" y2="22"/><line x1="16" y1="16" x2="16.01" y2="16"/><line x1="16" y1="20" x2="16.01" y2="20"/>"#),
        Icon::Thunder => r#"<path d="M19 16.9A5 5 0 0 0 18 7h-1.26a8 8 0 1 0-11.62 9"/><polyline points="13 11 9 17 15 17 11 23"/>"#.to_string(),
    };
    svg(&body, "wx-icon")
}

/// Resolves an icon reference the way gethomepage does: URLs as they are,
/// `mdi-*` / `si-*` from their icon sets, everything else from
/// dashboard-icons (png unless the name ends in .svg or .webp).
fn icon(reference: &str) -> Markup {
    if reference.starts_with("http://")
        || reference.starts_with("https://")
        || reference.starts_with('/')
    {
        return html! { img src=(reference) width="32" height="32" alt="" loading="lazy" referrerpolicy="no-referrer"; };
    }
    let masked = |base: &str, name: &str| {
        let url = format!("{base}{}.svg", name.trim_end_matches(".svg"));
        html! { span.icon-mask style=(format!("-webkit-mask-image:url({url});mask-image:url({url})")) {} }
    };
    if let Some(name) = reference.strip_prefix("mdi-") {
        return masked("https://cdn.jsdelivr.net/npm/@mdi/svg@latest/svg/", name);
    }
    if let Some(name) = reference.strip_prefix("si-") {
        return masked(
            "https://cdn.jsdelivr.net/npm/simple-icons@latest/icons/",
            name,
        );
    }
    let url = if let Some(name) = reference.strip_suffix(".svg") {
        format!("{DASHBOARD_ICONS}/svg/{name}.svg")
    } else if let Some(name) = reference.strip_suffix(".webp") {
        format!("{DASHBOARD_ICONS}/webp/{name}.webp")
    } else {
        format!(
            "{DASHBOARD_ICONS}/png/{}.png",
            reference.trim_end_matches(".png")
        )
    };
    html! { img src=(url) width="32" height="32" alt="" loading="lazy" referrerpolicy="no-referrer"; }
}

fn block(label: &str, value: Option<&str>) -> Markup {
    html! {
        div.blk.pending[value.is_none()] {
            div.v { (value.unwrap_or("-")) }
            div.l { (label) }
        }
    }
}

fn stats_row(health: &ServiceHealth) -> Markup {
    if !matches!(health.status, PodStatus::Running | PodStatus::Partial) {
        return html! { div.blocks { (block("Status", Some("Offline"))) } };
    }
    let (cpu, memory) = match health.usage {
        None => (None, None),
        Some(usage) => {
            let cpu = if health.cpu_limit > 0.0 {
                format::percent(100.0 * usage.cpu / health.cpu_limit)
            } else {
                format::number_max(usage.cpu, 4)
            };
            (Some(cpu), Some(format::bytes(usage.memory, false, None)))
        }
    };
    html! {
        div.blocks {
            (block("CPU", cpu.as_deref()))
            (block("MEM", memory.as_deref()))
        }
    }
}

fn widget_rows(state: &AppState, service: &Service) -> Markup {
    let Some(spec) = &service.widget else {
        return html! {};
    };
    let current = state.widgets.lock().unwrap().get(&service.id).cloned();
    match current {
        Some(WidgetState::Data(data)) => html! {
            @if !data.blocks.is_empty() {
                div.blocks {
                    @for b in &data.blocks { (block(b.label, Some(&b.value))) }
                }
            }
            @for row in &data.rows {
                div.qrow {
                    div.qbar style=(format!("width:{:.0}%", row.progress)) {}
                    span.qtitle { (row.title) }
                    span.qdetail { (row.detail) }
                }
            }
        },
        Some(WidgetState::Error(reason)) => html! {
            div.werr title=(reason) { "API Error" span { (reason) } }
        },
        None => {
            // Not fetched yet: show the widget's labels as placeholders.
            let labels = state
                .registry
                .get(&spec.kind)
                .map(|w| {
                    w.labels(&crate::widgets::WidgetConfig {
                        params: spec.params.clone(),
                    })
                })
                .unwrap_or_default();
            html! {
                @if !labels.is_empty() {
                    div.blocks { @for label in labels { (block(label, None)) } }
                }
            }
        }
    }
}

fn card(state: &AppState, service: &Service) -> Markup {
    let health = service.kube.as_ref().map(|k| state.health(k));
    let href = service
        .href
        .as_deref()
        .filter(|h| !h.is_empty() && *h != "#");
    let title = html! {
        (service.name)
        @if let Some(description) = &service.description {
            p.desc { (description) }
        }
    };
    html! {
        li.svc #(service.id) data-name=(service.name) {
            div.card {
                div.title {
                    @if let Some(reference) = &service.icon {
                        @if let Some(href) = href {
                            a.icon href=(href) target="_blank" rel="noreferrer" tabindex="-1" aria-hidden="true" { (icon(reference)) }
                        } @else {
                            span.icon { (icon(reference)) }
                        }
                    }
                    @if let Some(href) = href {
                        a.name href=(href) target="_blank" rel="noreferrer" { (title) }
                    } @else {
                        span.name { (title) }
                    }
                    @if let Some(health) = &health {
                        @let class = match health.status {
                            PodStatus::Running => "ok",
                            _ => "warn",
                        };
                        span.dot.(class) title=(health.status.label()) role="img" aria-label=(health.status.label()) {}
                    }
                }
                @if let (Some(health), true) = (&health, state.cfg.show_stats) {
                    (stats_row(health))
                }
                (widget_rows(state, service))
            }
        }
    }
}

fn bar(percent: f64) -> Markup {
    let width = if percent.is_finite() {
        percent.clamp(0.0, 100.0)
    } else {
        0.0
    };
    html! { div.bar { div style=(format!("width:{width:.0}%")) {} } }
}

fn resource(
    icon_svg: &str,
    label: &str,
    cpu_percent: f64,
    memory_free: f64,
    memory_percent: f64,
) -> Markup {
    html! {
        div.res {
            (svg(icon_svg, "res-icon"))
            div.res-body {
                div.res-line { span { (format::percent(cpu_percent)) } (svg(SVG_CPU, "res-mini")) }
                (bar(cpu_percent))
                div.res-line { span { (format::bytes(memory_free, true, Some(0))) } (svg(SVG_MEMORY, "res-mini")) }
                (bar(memory_percent))
                div.res-label { (label) }
            }
        }
    }
}

fn percent_of(part: f64, total: f64) -> f64 {
    if total > 0.0 {
        100.0 * part / total
    } else {
        0.0
    }
}

fn header_resources(state: &AppState) -> Markup {
    let nodes: Vec<NodeInfo> = state.nodes.lock().unwrap().clone();
    let header = &state.cfg.header;
    html! {
        div.resources #(HEADER_RESOURCES_ID) {
            @if header.cluster.show && !nodes.is_empty() {
                @let cpu_total: f64 = nodes.iter().map(|n| n.cpu_capacity).sum();
                @let mem_total: f64 = nodes.iter().map(|n| n.memory_capacity).sum();
                @let cpu_used: f64 = nodes.iter().filter_map(|n| n.usage).map(|u| u.cpu).sum();
                @let mem_used: f64 = nodes.iter().filter_map(|n| n.usage).map(|u| u.memory).sum();
                (resource(SVG_CLUSTER, &header.cluster.label, percent_of(cpu_used, cpu_total),
                          mem_total - mem_used, percent_of(mem_used, mem_total)))
            }
            @if header.nodes.show {
                @for node in &nodes {
                    @let usage = node.usage.unwrap_or_default();
                    (resource(if node.ready { SVG_SERVER } else { SVG_ALERT }, &node.name,
                              percent_of(usage.cpu, node.cpu_capacity),
                              node.memory_capacity - usage.memory,
                              percent_of(usage.memory, node.memory_capacity)))
                }
            }
        }
    }
}

fn header_weather(state: &AppState) -> Markup {
    let weather = state.weather.lock().unwrap().clone();
    html! {
        div.weather #(HEADER_WEATHER_ID) {
            @if let Some(now) = weather {
                (weather_icon(now.icon))
                div {
                    div.wx-primary { (now.primary) }
                    div.wx-secondary { (now.condition) }
                }
            }
        }
    }
}

fn search(state: &AppState) -> Markup {
    let Some(cfg) = &state.cfg.header.search else {
        return html! {};
    };
    let (action, param, mark) = match cfg.provider.as_str() {
        "duckduckgo" => ("https://duckduckgo.com/", "q", "D"),
        "bing" => ("https://www.bing.com/search", "q", "B"),
        "brave" => ("https://search.brave.com/search", "q", "B"),
        "custom" => (
            cfg.url
                .as_deref()
                .unwrap_or("https://www.google.com/search"),
            "q",
            "↵",
        ),
        _ => ("https://www.google.com/search", "q", "G"),
    };
    html! {
        form.search action=(action) method="get" target=(cfg.target) role="search" {
            input type="search" name=(param) placeholder="Search…" aria-label="Search" autocomplete="off" autofocus;
            button type="submit" title=(format!("Search engine: {}", cfg.provider)) { (mark) }
        }
    }
}

fn hostname(href: &str) -> &str {
    href.split("://")
        .nth(1)
        .unwrap_or(href)
        .split('/')
        .next()
        .unwrap_or("")
}

fn bookmarks(state: &AppState) -> Markup {
    html! {
        @if !state.cfg.bookmarks.is_empty() {
            section.bookmarks {
                @for group in &state.cfg.bookmarks {
                    div.group {
                        h2 { (group.group) }
                        ul {
                            @for item in &group.items {
                                li {
                                    a.bookmark href=(item.href) target="_blank" rel="noreferrer" {
                                        span.abbr {
                                            @if let Some(reference) = &item.icon { (icon(reference)) }
                                            @else { (item.abbr.as_deref().unwrap_or("")) }
                                        }
                                        span.bm-name { (item.name) }
                                        span.bm-host { (hostname(&item.href)) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Renders every fragment from current state.
pub fn build_view(state: &AppState) -> View {
    let mut view = View::default();
    view.fragments.insert(
        Arc::from(HEADER_RESOURCES_ID),
        Arc::from(header_resources(state).into_string()),
    );
    view.fragments.insert(
        Arc::from(HEADER_WEATHER_ID),
        Arc::from(header_weather(state).into_string()),
    );
    let mut total = 0;
    for (group, services) in state.grouped_services() {
        let mut ids = Vec::with_capacity(services.len());
        for service in &services {
            let id: Arc<str> = Arc::from(service.id.as_str());
            view.fragments
                .insert(id.clone(), Arc::from(card(state, service).into_string()));
            ids.push(id);
            total += 1;
        }
        view.groups.push((group, ids));
    }
    state.metrics.services.set(total);
    view
}

/// Rebuilds the view, stores it, and tells connected clients what changed:
/// a reload when the layout changed, otherwise only the changed fragments.
pub fn refresh_view(state: &AppState) {
    let new = build_view(state);
    let mut current = state.view.lock().unwrap();
    let mut changed = false;
    if current.groups != new.groups {
        changed = true;
        let _ = state.events.send(Event::Reload);
    } else {
        for (id, html) in &new.fragments {
            if current.fragments.get(id) != Some(html) {
                changed = true;
                let _ = state.events.send(Event::Fragment {
                    id: id.clone(),
                    html: html.clone(),
                });
            }
        }
    }
    if changed {
        state.version.fetch_add(1, Ordering::Relaxed);
    }
    *current = new;
}

/// The whole page, from the stored view.
pub fn page(state: &AppState) -> String {
    let view = state.view.lock().unwrap().clone();
    let version = state.version.load(Ordering::Relaxed);
    let fragment = |id: &str| -> Markup {
        PreEscaped(
            view.fragments
                .get(id)
                .map(|html| html.to_string())
                .unwrap_or_default(),
        )
    };
    let columns = state.cfg.columns.clamp(1, 8);
    let asset = |name: &str| format!("/assets/{name}?v={}", env!("CARGO_PKG_VERSION"));
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="dark";
                title { (state.cfg.title) }
                link rel="icon" href="/favicon.svg" type="image/svg+xml";
                link rel="stylesheet" href=(asset("style.css"));
                script src=(asset("app.js")) defer {}
            }
            body data-version=(version) style=(format!("--cols:{columns}")) {
                header.top {
                    (fragment(HEADER_RESOURCES_ID))
                    (fragment(HEADER_WEATHER_ID))
                    (search(state))
                }
                main.groups {
                    @for (group, ids) in &view.groups {
                        section.group {
                            h2 { (group) }
                            ul { @for id in ids { (fragment(id)) } }
                        }
                    }
                }
                (bookmarks(state))
            }
        }
    }
    .into_string()
}

/// Plain-data snapshot of the dashboard, for comparing against another
/// dashboard and for debugging. Never contains credentials.
pub fn snapshot(state: &AppState) -> serde_json::Value {
    let widgets: HashMap<String, WidgetState> = state.widgets.lock().unwrap().clone();
    let groups: Vec<serde_json::Value> = state
        .grouped_services()
        .into_iter()
        .map(|(group, services)| {
            let services: Vec<serde_json::Value> = services
                .iter()
                .map(|s| {
                    let health = s.kube.as_ref().map(|k| state.health(k));
                    let widget = s.widget.as_ref().map(|spec| {
                        let (blocks, error) = match widgets.get(&s.id) {
                            Some(WidgetState::Data(d)) => (
                                d.blocks
                                    .iter()
                                    .map(
                                        |b| serde_json::json!({"label": b.label, "value": b.value}),
                                    )
                                    .collect::<Vec<_>>(),
                                None,
                            ),
                            Some(WidgetState::Error(e)) => (Vec::new(), Some(e.clone())),
                            None => (Vec::new(), None),
                        };
                        serde_json::json!({
                            "type": spec.kind,
                            "blocks": blocks,
                            "error": error,
                            "secretFields": spec.secrets.keys().collect::<Vec<_>>(),
                        })
                    });
                    serde_json::json!({
                        "name": s.name,
                        "href": s.href,
                        "namespace": s.kube.as_ref().map(|k| k.namespace.clone()),
                        "podSelector": s.kube.as_ref().map(|k| k.pod_selector.clone()),
                        "status": health.map(|h| h.status.label()),
                        "cpu": health.and_then(|h| h.usage).map(|u| u.cpu),
                        "memory": health.and_then(|h| h.usage).map(|u| u.memory),
                        "widget": widget,
                    })
                })
                .collect();
            serde_json::json!({"group": group, "services": services})
        })
        .collect();
    serde_json::json!({ "groups": groups })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::discovery::service_from_annotations;
    use crate::state::{PodInfo, Usage};
    use crate::widgets::{Block, WidgetData};

    fn state_with_service() -> (AppState, Service) {
        let cfg: Config = serde_json::from_str(
            r#"{"title":"Test <Lab>","header":{"search":{"provider":"google"}},
                "bookmarks":[{"group":"Dev","items":[{"name":"Github","abbr":"GH","href":"https://github.com/"}]}],
                "services":[{"group":"Infra","name":"Garage","href":"http://garage:3902/","description":"Backup storage"}]}"#,
        )
        .unwrap();
        let state = AppState::new(cfg);
        let ann = [
            ("gethomepage.dev/enabled", "true"),
            ("gethomepage.dev/name", "Argo <CD>"),
            ("gethomepage.dev/group", "Cluster"),
            ("gethomepage.dev/href", "https://argo.example.com"),
            ("gethomepage.dev/icon", "argo-cd.png"),
            ("gethomepage.dev/pod-selector", "app=argocd"),
            ("gethomepage.dev/widget.type", "argocd"),
            ("gethomepage.dev/widget.url", "http://argocd"),
            ("gethomepage.dev/widget.key", "TOPSECRET"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let service =
            service_from_annotations("ingress", "argocd", "argocd-server", &ann, None).unwrap();
        state
            .discovered
            .lock()
            .unwrap()
            .insert("ingress/argocd/argocd-server".into(), service.clone());
        (state, service)
    }

    #[test]
    fn page_lists_groups_cards_and_escapes_text() {
        let (state, service) = state_with_service();
        refresh_view(&state);
        let html = page(&state);
        assert!(html.contains("<title>Test &lt;Lab&gt;</title>"));
        assert!(html.contains("Argo &lt;CD&gt;"));
        assert!(html.contains(&format!(r#"id="{}""#, service.id)));
        assert!(html.contains("<h2>Cluster</h2>"));
        assert!(html.contains("<h2>Infra</h2>"));
        assert!(html.contains("Backup storage"));
        assert!(html.contains("dashboard-icons/png/argo-cd.png"));
        assert!(html.contains(r#"action="https://www.google.com/search""#));
        assert!(html.contains("github.com"));
        // Placeholders for the widget before its first fetch.
        assert!(html.contains("Out Of Sync"));
        // No pods known: orange dot, and stats say Offline.
        assert!(html.contains(r#"class="dot warn" title="not found""#));
        assert!(html.contains("Offline"));
    }

    #[test]
    fn credentials_never_reach_html_or_the_snapshot() {
        let (state, service) = state_with_service();
        state.widgets.lock().unwrap().insert(
            service.id.clone(),
            WidgetState::Data(WidgetData::from(vec![Block::new("Apps", "75")])),
        );
        refresh_view(&state);
        assert!(!page(&state).contains("TOPSECRET"));
        assert!(!snapshot(&state).to_string().contains("TOPSECRET"));
    }

    #[test]
    fn running_pods_show_a_green_dot_and_usage() {
        let (state, service) = state_with_service();
        let pod = PodInfo {
            labels: [("app".to_string(), "argocd".to_string())].into(),
            phase: "Running".into(),
            cpu_limit: 0.0,
        };
        state
            .pods
            .lock()
            .unwrap()
            .insert("argocd".into(), [("argocd-0".to_string(), pod)].into());
        *state.pod_usage.lock().unwrap() = Some(
            [(
                "argocd".to_string(),
                [(
                    "argocd-0".to_string(),
                    Usage {
                        cpu: 0.0314,
                        memory: 1_540_000_000.0,
                    },
                )]
                .into(),
            )]
            .into(),
        );
        let html = card(&state, &service).into_string();
        assert!(html.contains(r#"class="dot ok" title="running""#));
        assert!(html.contains("0.0314"));
        assert!(html.contains("1.54 GB"));
    }

    #[test]
    fn widget_errors_are_shown_on_the_card() {
        let (state, service) = state_with_service();
        state
            .widgets
            .lock()
            .unwrap()
            .insert(service.id.clone(), WidgetState::Error("HTTP 401".into()));
        let html = card(&state, &service).into_string();
        assert!(html.contains("API Error"));
        assert!(html.contains("HTTP 401"));
    }

    #[test]
    fn refresh_broadcasts_only_changed_fragments_and_reloads_on_layout_change() {
        let (state, service) = state_with_service();
        refresh_view(&state);
        let mut rx = state.events.subscribe();
        let before = state.version.load(Ordering::Relaxed);

        refresh_view(&state);
        assert!(rx.try_recv().is_err(), "nothing changed, nothing sent");
        assert_eq!(state.version.load(Ordering::Relaxed), before);

        state
            .widgets
            .lock()
            .unwrap()
            .insert(service.id.clone(), WidgetState::Error("boom".into()));
        refresh_view(&state);
        match rx.try_recv().unwrap() {
            Event::Fragment { id, html } => {
                assert_eq!(&*id, service.id.as_str());
                assert!(html.contains("boom"));
            }
            Event::Reload => panic!("expected a fragment"),
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(state.version.load(Ordering::Relaxed), before + 1);

        state.discovered.lock().unwrap().clear();
        refresh_view(&state);
        assert!(matches!(rx.try_recv().unwrap(), Event::Reload));
    }

    #[test]
    fn icon_references_resolve_like_gethomepage() {
        assert!(icon("zitadel").into_string().contains("/png/zitadel.png"));
        assert!(icon("openbao.svg")
            .into_string()
            .contains("/svg/openbao.svg"));
        assert!(icon("mdi-package-variant-closed")
            .into_string()
            .contains("@mdi/svg@latest/svg/package-variant-closed.svg"));
        assert!(icon("https://example.com/a.png?s=1&v=4")
            .into_string()
            .contains(r#"src="https://example.com/a.png?s=1&amp;v=4""#));
    }

    #[test]
    fn hostname_of_bookmark() {
        assert_eq!(hostname("https://github.com/"), "github.com");
        assert_eq!(hostname("http://a.b:3902/x"), "a.b:3902");
    }
}
