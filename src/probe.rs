//! Asks the page each card links to whether it answers.
//!
//! Pod phase says a container runs, not that the service works: a pod can
//! be `Running` behind a 502. And a static entry has no pods at all. So
//! every card with a link gets one GET per interval, and the light on the
//! card goes red when that fails twice in a row.
//!
//! Anything below 500 counts as an answer, a login page or a redirect to a
//! single sign-on included: the question is whether something is serving,
//! not whether hugin may come in.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::model::Service;
use crate::state::AppState;
use crate::widgets::{Http, Request};

const CONCURRENT: usize = 8;
/// Failed probes in a row before a card says so. One lost request is not
/// an outage.
const FAILURES: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    /// Failed probes in a row.
    failures: u32,
    /// Why the latest one failed.
    reason: String,
}

/// The link to ask, when this service is probed at all.
fn wanted<'a>(state: &AppState, service: &'a Service) -> Option<&'a str> {
    if !state.cfg.probes || !service.probe {
        return None;
    }
    service
        .href
        .as_deref()
        .filter(|h| h.starts_with("http://") || h.starts_with("https://"))
}

/// What the probe found for a service: `None` when it is not probed or not
/// yet, `Err` with a short reason when its page does not answer.
pub fn result(state: &AppState, service: &Service) -> Option<Result<(), String>> {
    wanted(state, service)?;
    let probes = state.probes.lock().unwrap();
    let entry = probes.get(&service.id)?;
    Some(if entry.failures >= FAILURES {
        Err(entry.reason.clone())
    } else {
        Ok(())
    })
}

async fn ask(http: &dyn Http, href: &str) -> Result<(), String> {
    match http.send(Request::get(href)).await {
        Ok(response) if response.status < 500 => Ok(()),
        Ok(response) => Err(format!("HTTP {}", response.status)),
        Err(error) => Err(error.0),
    }
}

/// Probes every service once. Returns whether any card changed.
pub async fn refresh(state: &AppState, http: &dyn Http) -> bool {
    let services = state.services();
    let targets: Vec<(&Service, &str)> = services
        .iter()
        .filter_map(|s| wanted(state, s).map(|href| (s, href)))
        .collect();
    let limit = tokio::sync::Semaphore::new(CONCURRENT);
    let answers = futures::future::join_all(targets.iter().map(|(_, href)| {
        let limit = &limit;
        async move {
            let _permit = limit.acquire().await;
            ask(http, href).await
        }
    }))
    .await;

    let mut changed = false;
    let mut probes = state.probes.lock().unwrap();
    let shown =
        |entry: Option<&Entry>| entry.map(|e| (e.failures >= FAILURES).then(|| e.reason.clone()));
    for ((service, _), answer) in targets.iter().zip(answers) {
        let before = shown(probes.get(&service.id));
        let entry = probes.entry(service.id.clone()).or_default();
        match answer {
            Ok(()) => *entry = Entry::default(),
            Err(reason) => {
                tracing::debug!(service = %service.name, reason, "page does not answer");
                entry.failures += 1;
                entry.reason = reason;
            }
        }
        changed |= shown(Some(entry)) != before;
    }
    let seen: HashSet<&str> = targets.iter().map(|(s, _)| s.id.as_str()).collect();
    probes.retain(|id, _| seen.contains(id.as_str()));
    changed
}

pub async fn run(state: Arc<AppState>, http: Arc<dyn Http>) {
    if !state.cfg.probes {
        return;
    }
    let interval = Duration::from_secs(state.cfg.intervals.probe_seconds.max(10));
    loop {
        state.wait_active().await;
        if refresh(&state, http.as_ref()).await {
            state.dirty.notify_one();
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::widgets::testing::FakeHttp;
    use crate::widgets::Method;

    fn state(config: &str) -> AppState {
        AppState::new(serde_json::from_str::<Config>(config).unwrap())
    }

    const TWO: &str = r#"{"services":[
        {"group":"Infra","name":"Router","href":"http://router.test/"},
        {"group":"Infra","name":"Broken","href":"http://broken.test/"},
        {"group":"Infra","name":"Quiet","href":"http://quiet.test/","probe":false},
        {"group":"Infra","name":"Note"}
    ]}"#;

    fn by_name(state: &AppState, name: &str) -> Service {
        state
            .services()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    #[tokio::test]
    async fn a_page_that_fails_twice_is_reported_and_recovers_at_once() {
        let state = state(TWO);
        let down = FakeHttp::new().on(Method::Get, "router.test", 302, "").on(
            Method::Get,
            "broken.test",
            502,
            "Bad Gateway",
        );
        let found = |name: &str| result(&state, &by_name(&state, name));

        assert_eq!(found("Router"), None);
        // First failure: nothing is said yet, but both now have a light.
        assert!(refresh(&state, &down).await);
        assert_eq!(found("Router"), Some(Ok(())));
        assert_eq!(found("Broken"), Some(Ok(())));
        assert!(refresh(&state, &down).await);
        assert_eq!(found("Broken"), Some(Err("HTTP 502".into())));
        assert!(!refresh(&state, &down).await);

        let up = FakeHttp::new().on(Method::Get, ".test", 401, "");
        assert!(refresh(&state, &up).await);
        assert_eq!(found("Broken"), Some(Ok(())));
    }

    #[tokio::test]
    async fn services_without_a_link_or_opted_out_are_left_alone() {
        let state = state(TWO);
        let http = FakeHttp::new().on(Method::Get, ".test", 200, "");
        refresh(&state, &http).await;
        assert_eq!(http.requests().len(), 2);
        assert_eq!(result(&state, &by_name(&state, "Quiet")), None);
        assert_eq!(result(&state, &by_name(&state, "Note")), None);
    }

    #[tokio::test]
    async fn probes_can_be_turned_off() {
        let state = state(
            r#"{"probes":false,"services":[{"group":"a","name":"b","href":"http://b.test/"}]}"#,
        );
        let http = FakeHttp::new().on(Method::Get, ".test", 200, "");
        assert!(!refresh(&state, &http).await);
        assert!(http.requests().is_empty());
    }
}
