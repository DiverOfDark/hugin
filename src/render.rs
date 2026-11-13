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
use crate::widgets::Tone;

pub const HEADER_RESOURCES_ID: &str = "hdr-resources";
pub const HEADER_WEATHER_ID: &str = "hdr-weather";
pub const HEADER_SUMMARY_ID: &str = "hdr-summary";

const DASHBOARD_ICONS: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons";

fn svg(body: &str, class: &str) -> Markup {
    html! {
        svg.(class) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
            stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            aria-hidden="true" { (PreEscaped(body)) }
    }
}

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

/// Resolves an icon reference: URLs as they are,
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

fn block(label: &str, value: Option<&str>, tone: Tone) -> Markup {
    html! {
        div.blk.pending[value.is_none()].warn[tone == Tone::Warn].down[tone == Tone::Error] {
            div.v { (value.unwrap_or("-")) }
            div.l { (label) }
        }
    }
}

/// CPU cores and memory in use, kept small beside the name: on most cards
/// they say "idle", so they must not compete with the widget's values.
fn usage(health: &ServiceHealth) -> Markup {
    let Some(usage) = health.usage else {
        return html! {};
    };
    let cpu = if health.cpu_limit > 0.0 {
        format::percent(100.0 * usage.cpu / health.cpu_limit)
    } else {
        format!("{:.2}", usage.cpu)
    };
    html! {
        span.use {
            span { (cpu) " cpu" }
            span { (format::bytes(usage.memory, false, None)) }
        }
    }
}

fn widget_rows(state: &AppState, service: &Service) -> Markup {
    if service.widget.is_none() {
        return html! {};
    }
    let current = state.widgets.lock().unwrap().get(&service.id).cloned();
    // Set while the values on screen are older than the latest (failed) refresh.
    let stale = state
        .widget_failures
        .lock()
        .unwrap()
        .get(&service.id)
        .map(|(_, reason)| format!("Last refresh failed: {reason}"));
    match current {
        Some(WidgetState::Data(data)) => html! {
            @if !data.blocks.is_empty() {
                div.blocks.stale[stale.is_some()] title=[stale.as_deref()] {
                    @for b in &data.blocks { (block(&b.label, Some(&b.value), b.tone)) }
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
            div.werr title=(reason) { b { "Widget failed" } (reason) }
        },
        Some(WidgetState::Pending(reason)) => html! {
            div.wpending title=(reason) { (reason) }
        },
        // Not fetched yet.
        None => html! {},
    }
}

/// The light on a card.
#[derive(Debug, Clone, PartialEq)]
enum Light {
    /// Nothing is known: a static entry that is not probed.
    Off,
    Ok(&'static str),
    Bad(Tone, String),
}

/// Pods first, then whether the page answers: a pod can run behind a 502.
fn light(state: &AppState, service: &Service) -> Light {
    let pods = service.kube.as_ref().map(|k| state.health(k).status);
    match pods {
        Some(PodStatus::Down) => return Light::Bad(Tone::Error, PodStatus::Down.label().into()),
        Some(status) if status != PodStatus::Running => {
            return Light::Bad(Tone::Warn, status.label().into())
        }
        _ => {}
    }
    match (crate::probe::result(state, service), pods) {
        (Some(Err(reason)), _) => Light::Bad(Tone::Error, reason),
        (_, Some(status)) => Light::Ok(status.label()),
        (Some(Ok(())), None) => Light::Ok("answers"),
        (None, None) => Light::Off,
    }
}

/// The worst thing a service's widget reports. A widget that cannot be
/// read is a warning: the service itself may be fine.
fn widget_tone(widget: Option<&WidgetState>) -> Tone {
    match widget {
        Some(WidgetState::Data(data)) => data.tone(),
        Some(WidgetState::Error(_)) => Tone::Warn,
        _ => Tone::Normal,
    }
}

fn card(state: &AppState, service: &Service) -> Markup {
    let health = service.kube.as_ref().map(|k| state.health(k));
    let generated_description = match service.description {
        Some(_) => None,
        None => state.widget_description(service),
    };
    let description = service
        .description
        .as_ref()
        .or(generated_description.as_ref());
    let href = service
        .href
        .as_deref()
        .filter(|h| !h.is_empty() && *h != "#");
    let title = html! {
        (service.name)
        @if let Some(description) = description {
            p.desc { (description) }
        }
    };
    // The same picture slot on every card keeps the names in one column.
    let favicon = crate::favicon::url(state, service);
    let favicon_first = state.cfg.favicons == crate::config::Favicons::Prefer;
    let picture = match (&favicon, &service.icon) {
        (Some(url), reference) if favicon_first || reference.is_none() => Some(html! {
            img src=(url) width="32" height="32" alt="" loading="lazy";
        }),
        (_, Some(reference)) => Some(icon(reference)),
        _ => None,
    };
    let initial = picture.is_none();
    let picture = picture.unwrap_or_else(|| {
        html! { (service.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()) }
    });
    let light = light(state, service);
    let light_tone = match &light {
        Light::Bad(tone, _) => *tone,
        _ => Tone::Normal,
    };
    // The edge marks every card the summary names.
    let edge = match light_tone.max(widget_tone(state.widgets.lock().unwrap().get(&service.id))) {
        Tone::Normal => "ok",
        Tone::Warn => "warn",
        Tone::Error => "down",
    };
    html! {
        li.svc #(service.id) data-name=(service.name) data-href=[href] {
            div.card.(edge) {
                div.title {
                    @if let Some(href) = href {
                        a.icon.mono[initial] href=(href) target="_blank" rel="noreferrer" tabindex="-1" aria-hidden="true" { (picture) }
                        a.name href=(href) target="_blank" rel="noreferrer" { (title) }
                    } @else {
                        span.icon.mono[initial] aria-hidden="true" { (picture) }
                        span.name { (title) }
                    }
                    @if let Some(health) = &health {
                        @if state.cfg.show_stats { (usage(health)) }
                    }
                    @match &light {
                        Light::Off => {},
                        // A healthy service gets a quiet dot; anything else says what is wrong.
                        Light::Ok(label) => span.dot title=(label) role="img" aria-label=(label) {},
                        Light::Bad(tone, label) => span.state.warn[*tone == Tone::Warn].down[*tone == Tone::Error] { (label) },
                    }
                }
                (widget_rows(state, service))
            }
        }
    }
}

/// A node meter turns amber here and red at [`METER_HOT`].
const METER_WARN: f64 = 80.0;
const METER_HOT: f64 = 90.0;

fn meter(label: &str, percent: f64) -> Markup {
    let percent = if percent.is_finite() {
        percent.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let (warm, hot) = (
        (METER_WARN..METER_HOT).contains(&percent),
        percent >= METER_HOT,
    );
    html! {
        div.meter.warm[warm].hot[hot] {
            span { (label) }
            div.bar { div style=(format!("width:{percent:.0}%")) {} }
            span { (format::percent(percent)) }
        }
    }
}

fn resource(
    label: &str,
    ready: bool,
    cpu_percent: f64,
    memory_free: f64,
    memory_percent: f64,
) -> Markup {
    let free = format!("{} memory free", format::bytes(memory_free, true, Some(0)));
    html! {
        div.res.down[!ready] title=(free) {
            div.res-name { (label) @if !ready { " is not ready" } }
            (meter("cpu", cpu_percent))
            (meter("mem", memory_percent))
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
                (resource(&header.cluster.label, true, percent_of(cpu_used, cpu_total),
                          mem_total - mem_used, percent_of(mem_used, mem_total)))
            }
            @if header.nodes.show {
                @for node in &nodes {
                    @let usage = node.usage.unwrap_or_default();
                    (resource(&node.name, node.ready,
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
                span.wx-primary { (now.primary) }
                span { (now.condition) }
            }
        }
    }
}

fn search(state: &AppState) -> Markup {
    let Some(cfg) = &state.cfg.header.search else {
        return html! {};
    };
    let (action, engine) = match cfg.provider.as_str() {
        "duckduckgo" => ("https://duckduckgo.com/", "DuckDuckGo"),
        "bing" => ("https://www.bing.com/search", "Bing"),
        "brave" => ("https://search.brave.com/search", "Brave"),
        "custom" => (
            cfg.url
                .as_deref()
                .unwrap_or("https://www.google.com/search"),
            "the web",
        ),
        _ => ("https://www.google.com/search", "Google"),
    };
    let hint = format!("Find a service or search {engine}");
    // Typing narrows the cards (see app.js); Enter opens the only match,
    // or sends the text to the search engine.
    html! {
        form.search action=(action) method="get" target=(cfg.target) role="search" {
            input type="search" name="q" placeholder=(hint) aria-label=(hint) autocomplete="off" autofocus;
            kbd aria-hidden="true" { "/" }
        }
    }
}

/// Names listed in the summary before it says "and N more".
const SUMMARY_NAMES: usize = 4;

/// The first thing on the page: whether anything needs looking at, and what.
fn summary(state: &AppState) -> Markup {
    let widgets = state.widgets.lock().unwrap().clone();
    let mut total = 0;
    let mut troubled: Vec<(String, String)> = Vec::new();
    let mut any_down = false;
    for (_, services) in state.grouped_services() {
        for service in services {
            total += 1;
            let light = match light(state, &service) {
                Light::Bad(tone, _) => tone,
                _ => Tone::Normal,
            };
            let worst = light.max(widget_tone(widgets.get(&service.id)));
            any_down |= worst == Tone::Error;
            if worst != Tone::Normal {
                troubled.push((service.id.clone(), service.name.clone()));
            }
        }
    }
    html! {
        // The stylesheet tints the page from these classes.
        p.summary.warn[!troubled.is_empty() && !any_down].down[any_down] #(HEADER_SUMMARY_ID) {
            @if troubled.is_empty() {
                @if total == 1 { "The one service is running" } @else { "All " (total) " services are running" }
            } @else {
                (troubled.len()) " of " (total) " services need attention: "
                @for (i, (id, name)) in troubled.iter().take(SUMMARY_NAMES).enumerate() {
                    @if i > 0 { ", " }
                    a href=(format!("#{id}")) { (name) }
                }
                @if troubled.len() > SUMMARY_NAMES {
                    " and " (troubled.len() - SUMMARY_NAMES) " more"
                }
            }
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
        Arc::from(HEADER_SUMMARY_ID),
        Arc::from(summary(state).into_string()),
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
                    (fragment(HEADER_SUMMARY_ID))
                    (fragment(HEADER_WEATHER_ID))
                    (fragment(HEADER_RESOURCES_ID))
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
                footer.foot {
                    a href="https://github.com/DiverOfDark/hugin" target="_blank" rel="noreferrer" {
                        "Hugin " (env!("CARGO_PKG_VERSION")) " on GitHub"
                    }
                }
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
                                    .map(|b| {
                                        serde_json::json!({
                                            "label": b.label,
                                            "value": b.value,
                                            "status": b.tone.label(),
                                        })
                                    })
                                    .collect::<Vec<_>>(),
                                None,
                            ),
                            Some(WidgetState::Error(e)) => (Vec::new(), Some(e.clone())),
                            Some(WidgetState::Pending(_)) | None => (Vec::new(), None),
                        };
                        let stale = state
                            .widget_failures
                            .lock()
                            .unwrap()
                            .get(&s.id)
                            .map(|(_, reason)| reason.clone());
                        serde_json::json!({
                            "type": spec.kind,
                            "blocks": blocks,
                            "error": error,
                            "staleReason": stale,
                            "pending": match widgets.get(&s.id) {
                                Some(WidgetState::Pending(reason)) => Some(reason.clone()),
                                _ => None,
                            },
                            "secretFields": spec.secrets.keys().collect::<Vec<_>>(),
                        })
                    });
                    serde_json::json!({
                        "name": s.name,
                        "href": s.href,
                        "namespace": s.kube.as_ref().map(|k| k.namespace.clone()),
                        "podSelector": s.kube.as_ref().map(|k| k.pod_selector.clone()),
                        "status": health.map(|h| h.status.label()),
                        "favicon": crate::favicon::url(state, s).is_some(),
                        "description": s.description.clone().or_else(|| state.widget_description(s)),
                        "page": crate::probe::result(state, s).map(|r| match r {
                            Ok(()) => "answers".to_string(),
                            Err(reason) => reason,
                        }),
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
            ("hugin.dev/enabled", "true"),
            ("hugin.dev/name", "Argo <CD>"),
            ("hugin.dev/group", "Cluster"),
            ("hugin.dev/href", "https://argo.example.com"),
            ("hugin.dev/icon", "argo-cd.png"),
            ("hugin.dev/pod-selector", "app=argocd"),
            ("hugin.dev/widget.type", "argocd"),
            ("hugin.dev/widget.url", "http://argocd"),
            ("hugin.dev/widget.key", "TOPSECRET"),
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
        assert!(html.contains(r#"href="https://github.com/DiverOfDark/hugin""#));
        // No pods known: the card and the summary both say so.
        assert!(html.contains(r#"<span class="state warn">not found</span>"#));
        assert!(html.contains("1 of 2 services need attention: "));
        assert!(html.contains(&format!(
            r##"<a href="#{}">Argo &lt;CD&gt;</a>"##,
            service.id
        )));
        // A card without an icon gets its initial instead.
        assert!(html.contains(r#"class="icon mono""#));
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
        assert!(html.contains(r#"class="dot" title="running""#));
        assert!(html.contains("0.03 cpu"));
        assert!(summary(&state)
            .into_string()
            .contains("All 2 services are running"));
        assert!(html.contains("1.54 GB"));
    }

    #[test]
    fn stale_values_are_marked_with_the_reason() {
        let (state, service) = state_with_service();
        state.widgets.lock().unwrap().insert(
            service.id.clone(),
            WidgetState::Data(WidgetData::from(vec![Block::new("Apps", "75")])),
        );
        assert!(!card(&state, &service).into_string().contains("stale"));
        state
            .widget_failures
            .lock()
            .unwrap()
            .insert(service.id.clone(), (1, "timeout".into()));
        let html = card(&state, &service).into_string();
        assert!(html.contains(r#"class="blocks stale" title="Last refresh failed: timeout""#));
        assert!(html.contains("75"));
        assert_eq!(
            snapshot(&state)["groups"][0]["services"][0]["widget"]["staleReason"],
            "timeout"
        );
    }

    #[test]
    fn pending_widgets_say_why() {
        let (state, service) = state_with_service();
        state.widgets.lock().unwrap().insert(
            service.id.clone(),
            WidgetState::Pending("no widget for ghcr.io/x/y yet".into()),
        );
        let html = card(&state, &service).into_string();
        assert!(html.contains(r#"class="wpending""#));
        assert!(html.contains("no widget for ghcr.io/x/y yet"));
        assert!(!html.contains("Widget failed"));
        assert_eq!(
            snapshot(&state)["groups"][0]["services"][0]["widget"]["pending"],
            "no widget for ghcr.io/x/y yet"
        );
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
        assert!(html.contains("Widget failed"));
        assert!(html.contains("HTTP 401"));
        assert!(html.contains(r#"class="card warn""#));
        assert!(summary(&state).into_string().contains("need attention"));
    }

    #[test]
    fn a_cached_favicon_fills_in_for_a_missing_icon() {
        let (state, with_icon) = state_with_service();
        let garage = state.static_services[0].clone();
        assert!(card(&state, &garage)
            .into_string()
            .contains(r#"class="icon mono""#));
        for service in [&garage, &with_icon] {
            state.favicons.lock().unwrap().insert(
                service.id.clone(),
                crate::favicon::Entry {
                    icon: Some(crate::favicon::Icon {
                        bytes: Arc::from(&b"GIF89a"[..]),
                        mime: "image/gif",
                        version: "abc".into(),
                    }),
                    source: service.href.clone().unwrap(),
                    refresh_at: std::time::Instant::now(),
                },
            );
        }
        let html = card(&state, &garage).into_string();
        assert!(
            html.contains(&format!(r#"src="/icons/{}?v=abc""#, garage.id)),
            "{html}"
        );
        assert!(!html.contains("mono"));
        // An icon that was chosen by hand stays.
        let html = card(&state, &with_icon).into_string();
        assert!(html.contains("dashboard-icons/png/argo-cd.png"));
        assert!(!html.contains("/icons/"));
    }

    #[test]
    fn a_long_list_of_troubled_services_is_cut_short() {
        let (state, _) = state_with_service();
        for n in 0..6 {
            let ann = [
                ("hugin.dev/enabled", "true".to_string()),
                ("hugin.dev/name", format!("App {n}")),
                ("hugin.dev/pod-selector", "app=x".to_string()),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
            let service =
                service_from_annotations("ingress", "ns", &format!("app{n}"), &ann, None).unwrap();
            state
                .discovered
                .lock()
                .unwrap()
                .insert(format!("ingress/ns/app{n}"), service);
        }
        let html = summary(&state).into_string();
        assert!(html.contains("7 of 8 services need attention: "), "{html}");
        assert_eq!(html.matches("<a href").count(), SUMMARY_NAMES);
        assert!(html.contains(" and 3 more"));
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
    fn icon_references_resolve_to_their_icon_sets() {
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

    #[test]
    fn a_value_that_reports_a_problem_colours_the_card_and_the_summary() {
        use crate::widgets::Tone;
        let (state, service) = state_with_service();
        let show = |tone| {
            state.widgets.lock().unwrap().insert(
                service.id.clone(),
                WidgetState::Data(WidgetData::from(vec![
                    Block::new("Health", "WARN").with_tone(tone),
                    Block::new("Used", "59%"),
                ])),
            );
        };
        // The pod is missing in this fixture, which is a warning of its own.
        state.pods.lock().unwrap().insert(
            "argocd".into(),
            [(
                "argocd-0".to_string(),
                PodInfo {
                    labels: [("app".to_string(), "argocd".to_string())].into(),
                    phase: "Running".into(),
                    cpu_limit: 0.0,
                },
            )]
            .into(),
        );
        show(Tone::Normal);
        assert!(card(&state, &service)
            .into_string()
            .contains(r#"class="card ok""#));
        assert!(!summary(&state).into_string().contains("need attention"));

        show(Tone::Warn);
        let html = card(&state, &service).into_string();
        assert!(html.contains(r#"class="card warn""#));
        assert!(html.contains(r#"<div class="blk warn"><div class="v">WARN</div>"#));
        assert!(html.contains(r#"<div class="blk"><div class="v">59%</div>"#));
        let top = summary(&state).into_string();
        assert!(top.contains(r#"class="summary warn""#));
        assert!(top.contains("1 of 2 services need attention"));

        show(Tone::Error);
        assert!(card(&state, &service)
            .into_string()
            .contains(r#"class="card down""#));
        assert!(summary(&state)
            .into_string()
            .contains(r#"class="summary down""#));
        assert_eq!(
            snapshot(&state)["groups"][0]["services"][0]["widget"]["blocks"][0]["status"],
            "error"
        );
    }

    #[tokio::test]
    async fn a_page_that_does_not_answer_turns_the_light_red() {
        use crate::widgets::testing::FakeHttp;
        use crate::widgets::Method;
        let (state, _) = state_with_service();
        let garage = state
            .services()
            .into_iter()
            .find(|s| s.name == "Garage")
            .unwrap();
        // Not probed yet: a static entry has no light.
        let html = card(&state, &garage).into_string();
        assert!(!html.contains("class=\"dot\"") && !html.contains("class=\"state"));

        let up = FakeHttp::new().on(Method::Get, "garage", 403, "");
        crate::probe::refresh(&state, &up).await;
        assert!(card(&state, &garage)
            .into_string()
            .contains(r#"class="dot" title="answers""#));

        let down = FakeHttp::new().on(Method::Get, "garage", 502, "");
        crate::probe::refresh(&state, &down).await;
        crate::probe::refresh(&state, &down).await;
        let html = card(&state, &garage).into_string();
        assert!(html.contains(r#"class="card down""#));
        assert!(html.contains(r#"<span class="state down">HTTP 502</span>"#));
        let top = summary(&state).into_string();
        assert!(top.contains(r#"class="summary down""#));
        assert!(top.contains(">Garage</a>"));
        assert_eq!(
            snapshot(&state)["groups"][1]["services"][0]["page"],
            "HTTP 502"
        );
    }

    #[test]
    fn node_meters_turn_amber_then_red() {
        assert!(meter("mem", 79.0)
            .into_string()
            .contains(r#"class="meter""#));
        assert!(meter("mem", 83.0)
            .into_string()
            .contains(r#"class="meter warm""#));
        assert!(meter("mem", 90.0)
            .into_string()
            .contains(r#"class="meter hot""#));
    }

    #[test]
    fn a_card_without_a_description_takes_the_widget_s() {
        use crate::state::CustomWidget;
        use crate::widgets::LuaWidget;
        let (state, mut service) = state_with_service();
        state.custom_widgets.lock().unwrap().insert(
            ("argocd".into(), "auto-argocd-server".into()),
            CustomWidget {
                widget: Some(Arc::new(LuaWidget::new("x", "return {}"))),
                failure: None,
                description: Some("GitOps <deployments>".into()),
            },
        );
        assert!(card(&state, &service)
            .into_string()
            .contains(r#"<p class="desc">GitOps &lt;deployments&gt;</p>"#));
        // What the owner wrote wins.
        service.description = Some("My deploys".into());
        let html = card(&state, &service).into_string();
        assert!(html.contains(r#"<p class="desc">My deploys</p>"#));
        assert!(!html.contains("GitOps"));
    }
}
