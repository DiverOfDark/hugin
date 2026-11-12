//! Favicons of the services, fetched from the services themselves, kept in
//! memory and served from `/icons/<service id>`.
//!
//! The page a service links to is asked for its `<link rel="icon">` tags,
//! then `/favicon.ico`. Only pictures on the service's own host are fetched,
//! and only bytes that look like a picture are kept, so a login page or an
//! error page never ends up as an icon.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Favicons;
use crate::model::Service;
use crate::state::AppState;
use crate::widgets::{Http, Request};

const MAX_ICON_BYTES: usize = 512 * 1024;
const MAX_HTML_BYTES: usize = 256 * 1024;
/// Pictures tried per service, best first, before `/favicon.ico`.
const MAX_LINKED: usize = 3;
const KEEP_FOUND: Duration = Duration::from_secs(24 * 3600);
/// A service without a favicon may only have been starting up.
const RETRY_MISSING: Duration = Duration::from_secs(3600);
const SCAN_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub bytes: Arc<[u8]>,
    pub mime: &'static str,
    /// Changes with the bytes, so browsers can cache the URL for long.
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub icon: Option<Icon>,
    /// The address the icon was looked up at; a changed `href` starts over.
    pub source: String,
    pub refresh_at: Instant,
}

/// The address to look for a favicon at, if this service should get one.
fn wanted(mode: Favicons, service: &Service) -> Option<&str> {
    let href = service.href.as_deref()?;
    let web = href.starts_with("http://") || href.starts_with("https://");
    let needed = match mode {
        Favicons::Off => false,
        Favicons::Fallback => service.icon.is_none(),
        Favicons::Prefer => true,
    };
    (web && needed).then_some(href)
}

/// Where the page finds this service's cached favicon, if there is one.
pub fn url(state: &AppState, service: &Service) -> Option<String> {
    let href = wanted(state.cfg.favicons, service)?;
    let favicons = state.favicons.lock().unwrap();
    let entry = favicons.get(&service.id).filter(|e| e.source == href)?;
    let icon = entry.icon.as_ref()?;
    Some(format!("/icons/{}?v={}", service.id, icon.version))
}

fn version(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash as u32)
}

/// The picture format, judged by the bytes alone. `None` for anything that
/// is not a picture a browser shows in `<img>`.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() > 22 {
        return Some("image/x-icon");
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    let xml = head.starts_with("<svg") || head.starts_with("<?xml") || head.starts_with("<!--");
    (xml && head.contains("<svg")).then_some("image/svg+xml")
}

/// `name="value"` pairs of one tag, names in lower case.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = tag;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() {
            return found;
        }
        let end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let name = rest[..end].to_ascii_lowercase();
        rest = rest[end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'');
            if let Some(quote) = quote {
                let body = &after[1..];
                let close = body.find(quote).unwrap_or(body.len());
                value = body[..close].to_string();
                rest = body.get(close + 1..).unwrap_or("");
            } else {
                let close = after.find(char::is_whitespace).unwrap_or(after.len());
                value = after[..close].to_string();
                rest = &after[close..];
            }
        }
        if !name.is_empty() {
            found.push((name, value));
        }
    }
}

/// The icons a page declares, best first: vector pictures, then the largest.
pub fn linked_icons(html: &str) -> Vec<String> {
    // ASCII lower-casing keeps byte offsets, so tags can be cut from `html`.
    let lower = html.to_ascii_lowercase();
    let mut scored: Vec<(u32, String)> = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<link") {
        let start = at + start + "<link".len();
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        at = end;
        let attrs = attributes(&html[start..end]);
        let get = |name: &str| {
            attrs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        let rel = get("rel").unwrap_or("").to_ascii_lowercase();
        let touch = rel
            .split_whitespace()
            .any(|t| t.starts_with("apple-touch-icon"));
        if !touch && !rel.split_whitespace().any(|t| t == "icon") {
            continue;
        }
        let Some(href) = get("href").map(str::trim).filter(|h| !h.is_empty()) else {
            continue;
        };
        if href.starts_with("data:") {
            continue;
        }
        let path = href
            .split(['?', '#'])
            .next()
            .unwrap_or(href)
            .to_ascii_lowercase();
        let kind = get("type").unwrap_or("").to_ascii_lowercase();
        let size = get("sizes")
            .and_then(|s| s.split(['x', 'X']).next())
            .and_then(|w| w.trim().parse::<u32>().ok());
        let score = if path.ends_with(".svg") || kind == "image/svg+xml" {
            1000
        } else {
            size.unwrap_or(if touch { 180 } else { 32 }).min(999)
        };
        scored.push((score, href.replace("&amp;", "&")));
    }
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, href)| href).collect()
}

/// Looks for the favicon of the page at `href`.
pub async fn fetch(http: &dyn Http, href: &str) -> Option<Icon> {
    let page = reqwest::Url::parse(href).ok()?;
    let mut candidates: Vec<reqwest::Url> = Vec::new();
    if let Ok(response) = http.send(Request::get(page.as_str())).await {
        if response.status == 200 {
            let body = &response.body[..response.body.len().min(MAX_HTML_BYTES)];
            for link in linked_icons(&String::from_utf8_lossy(body)) {
                let Ok(url) = page.join(&link) else { continue };
                let same_host = url.host_str() == page.host_str();
                if same_host
                    && matches!(url.scheme(), "http" | "https")
                    && !candidates.contains(&url)
                {
                    candidates.push(url);
                }
            }
            candidates.truncate(MAX_LINKED);
        }
    }
    if let Ok(default) = page.join("/favicon.ico") {
        if !candidates.contains(&default) {
            candidates.push(default);
        }
    }
    for url in candidates {
        let Ok(response) = http.send(Request::get(url.as_str())).await else {
            continue;
        };
        if response.status != 200 || response.body.len() > MAX_ICON_BYTES {
            continue;
        }
        if let Some(mime) = sniff(&response.body) {
            return Some(Icon {
                version: version(&response.body),
                bytes: Arc::from(response.body),
                mime,
            });
        }
    }
    None
}

/// Fetches what is missing or old. Returns whether any card changed.
pub async fn refresh(state: &AppState, http: &dyn Http) -> bool {
    let mut changed = false;
    let mut seen = HashSet::new();
    for service in state.services() {
        let Some(href) = wanted(state.cfg.favicons, &service) else {
            continue;
        };
        seen.insert(service.id.clone());
        let previous = state.favicons.lock().unwrap().get(&service.id).cloned();
        let previous = previous.filter(|e| e.source == href);
        if previous
            .as_ref()
            .is_some_and(|e| e.refresh_at > Instant::now())
        {
            continue;
        }
        let fetched = fetch(http, href).await;
        let old = previous.and_then(|e| e.icon);
        // A picture that was there stays while the service is unreachable.
        let (icon, keep) = match fetched {
            Some(icon) => (Some(icon), KEEP_FOUND),
            None => (old.clone(), RETRY_MISSING),
        };
        changed |= icon != old;
        tracing::debug!(service = %service.name, found = icon.is_some(), "favicon looked up");
        state.favicons.lock().unwrap().insert(
            service.id.clone(),
            Entry {
                icon,
                source: href.to_string(),
                refresh_at: Instant::now() + keep,
            },
        );
    }
    state
        .favicons
        .lock()
        .unwrap()
        .retain(|id, _| seen.contains(id));
    changed
}

pub async fn run(state: Arc<AppState>, http: Arc<dyn Http>) {
    if state.cfg.favicons == Favicons::Off {
        return;
    }
    loop {
        state.wait_active().await;
        if refresh(&state, http.as_ref()).await {
            state.dirty.notify_one();
        }
        tokio::time::sleep(SCAN_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::testing::FakeHttp;
    use crate::widgets::Method;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg"><circle r="4"/></svg>"#;

    #[test]
    fn pictures_are_recognised_by_their_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(b"GIF89a...."), Some("image/gif"));
        assert_eq!(
            sniff(&[0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 22, 0, 0, 0, 0]),
            Some("image/x-icon")
        );
        assert_eq!(sniff(SVG.as_bytes()), Some("image/svg+xml"));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?>\n<svg></svg>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"<!DOCTYPE html><html><svg></svg></html>"), None);
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><Error/>"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn linked_icons_are_ordered_best_first() {
        let html = r#"<html><HEAD>
            <link rel="stylesheet" href="/a.css">
            <link rel="shortcut icon" href="/small.ico">
            <LINK REL=icon sizes=192x192 href=/big.png>
            <link rel="apple-touch-icon" href='/touch.png'/>
            <link rel="mask-icon" href="/mask.svg">
            <link rel="icon" type="image/svg+xml" href="/logo?v=1&amp;x=2">
            <link rel="icon" href="data:image/png;base64,AAAA">
            <link rel="icon">
        </head>"#;
        assert_eq!(
            linked_icons(html),
            ["/logo?v=1&x=2", "/big.png", "/touch.png", "/small.ico"]
        );
        assert!(linked_icons("<p>no head</p><link").is_empty());
    }

    #[tokio::test]
    async fn the_linked_icon_is_fetched_from_the_same_host() {
        let http = FakeHttp::new()
            .on(Method::Get, "/static/logo.svg", 200, SVG)
            .on(
                Method::Get,
                "app.example/",
                200,
                r#"<link rel="icon" href="https://cdn.example/x.png" sizes="512x512">
                   <link rel="icon" href="static/logo.svg">"#,
            );
        let icon = fetch(&http, "https://app.example/ui/").await.unwrap();
        assert_eq!(icon.mime, "image/svg+xml");
        assert_eq!(&*icon.bytes, SVG.as_bytes());
        let asked: Vec<String> = http.requests().into_iter().map(|r| r.url).collect();
        assert_eq!(
            asked,
            [
                "https://app.example/ui/",
                "https://app.example/ui/static/logo.svg"
            ]
        );
    }

    #[tokio::test]
    async fn favicon_ico_is_the_fallback_and_html_is_not_an_icon() {
        let http = FakeHttp::new()
            .on(Method::Get, "/favicon.ico", 200, "GIF89a-picture")
            .on(Method::Get, "app.example", 200, "<html>no links</html>");
        let icon = fetch(&http, "http://app.example:8080/").await.unwrap();
        assert_eq!(icon.mime, "image/gif");

        // A login page answers every path with HTML.
        let walled = FakeHttp::new().on(Method::Get, "app.example", 200, "<html>Sign in</html>");
        assert!(fetch(&walled, "https://app.example/").await.is_none());
        assert!(fetch(&FakeHttp::new(), "https://down.example/")
            .await
            .is_none());
    }
}
