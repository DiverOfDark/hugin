//! Every bundled script against a recorded upstream response.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::testing::{config, pairs, FakeHttp};
use super::{Http, Method, Registry, Request, Response, WidgetConfig, WidgetData, WidgetError};

async fn run(
    kind: &str,
    http: &Arc<FakeHttp>,
    cfg: &WidgetConfig,
) -> Result<WidgetData, WidgetError> {
    let widget = Registry::default().get(kind).expect("bundled widget");
    widget.run(http.clone(), cfg).await.0
}

fn s(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!("../../fixtures/", $path))
    };
}

#[tokio::test]
async fn every_bundled_script_compiles() {
    // A syntax error would otherwise only show up at the first refresh.
    for (name, script) in super::BUNDLED {
        let lua = mlua::Lua::new();
        lua.load(script)
            .set_name(format!("={name}"))
            .into_function()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

// --- argocd -----------------------------------------------------------------

#[tokio::test]
async fn argocd_counts_sync_and_health_states() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/v1/applications",
        200,
        fixture!("argocd/applications.json"),
    ));
    let cfg = config(&[("url", "http://argocd-server.argocd.svc/"), ("key", "tok")]);
    let data = run("argocd", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Apps", "8"),
            ("Synced", "8"),
            ("Out Of Sync", "0"),
            ("Healthy", "5")
        ])
    );
    assert_eq!(http.header(0, "Authorization").unwrap(), "Bearer tok");
    assert!(http.requests()[0].url.starts_with(
        "http://argocd-server.argocd.svc/api/v1/applications?fields=items.status.sync.status"
    ));
}

#[tokio::test]
async fn argocd_honours_fields_and_reports_errors() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/v1/applications",
        200,
        fixture!("argocd/applications.json"),
    ));
    let cfg = config(&[
        ("url", "http://argo"),
        ("key", "tok"),
        ("fields", r#"["degraded","apps"]"#),
    ]);
    assert_eq!(
        pairs(&run("argocd", &http, &cfg).await.unwrap()),
        s(&[("Degraded", "3"), ("Apps", "8")])
    );

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/v1/applications", 401, "{}"));
    let cfg = config(&[("url", "http://argo"), ("key", "bad")]);
    assert_eq!(run("argocd", &http, &cfg).await.unwrap_err().0, "HTTP 401");

    let http = Arc::new(FakeHttp::new());
    let err = run("argocd", &http, &config(&[("url", "http://argo")]))
        .await
        .unwrap_err();
    assert_eq!(err.0, "widget.key is not set");
    assert!(http.requests().is_empty());
}

// --- grafana ----------------------------------------------------------------

fn grafana_cfg() -> WidgetConfig {
    config(&[
        ("url", "https://grafana.example.com/"),
        ("username", "admin"),
        ("password", "pw"),
    ])
}

#[tokio::test]
async fn grafana_modern_falls_back_to_builtin_alertmanager() {
    let http = Arc::new(
        FakeHttp::new()
            .on(
                Method::Get,
                "/api/admin/stats",
                200,
                fixture!("grafana/stats.json"),
            )
            .on(
                Method::Get,
                "/api/alertmanager/grafana/api/v2/alerts",
                200,
                fixture!("grafana/alertmanager_alerts.json"),
            )
            .on(
                Method::Get,
                "/api/alerts",
                404,
                fixture!("grafana/alerts_404.json"),
            ),
    );
    let data = run("grafana", &http, &grafana_cfg()).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Dashboards", "25"),
            ("Data Sources", "3"),
            ("Total Alerts", "0"),
            ("Alerts Triggered", "0")
        ])
    );
    // base64("admin:pw")
    assert_eq!(
        http.header(0, "Authorization").unwrap(),
        "Basic YWRtaW46cHc="
    );
}

#[tokio::test]
async fn grafana_legacy_version_two_and_failures() {
    let stats = fixture!("grafana/stats.json");
    let http = Arc::new(
        FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, stats)
            .on(
                Method::Get,
                "/api/alerts",
                200,
                r#"[{"state":"alerting"},{"state":"ok"},{"state":"alerting"}]"#,
            ),
    );
    assert_eq!(
        run("grafana", &http, &grafana_cfg()).await.unwrap().blocks[3].value,
        "2"
    );

    let http = Arc::new(
        FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, stats)
            .on(
                Method::Get,
                "/api/alertmanager/alertmanager/api/v2/alerts",
                200,
                r#"[{"labels":{}},{"labels":{}}]"#,
            ),
    );
    let mut cfg = grafana_cfg();
    cfg.params.insert("version".into(), "2".into());
    cfg.params.insert("alerts".into(), "alertmanager".into());
    assert_eq!(
        run("grafana", &http, &cfg).await.unwrap().blocks[3].value,
        "2"
    );

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/admin/stats", 403, "{}"));
    assert_eq!(
        run("grafana", &http, &grafana_cfg()).await.unwrap_err().0,
        "HTTP 403"
    );

    let http = Arc::new(
        FakeHttp::new()
            .on(Method::Get, "/api/admin/stats", 200, stats)
            .on(Method::Get, "/api/alertmanager/grafana", 500, "")
            .on(
                Method::Get,
                "/api/alerts",
                404,
                fixture!("grafana/alerts_404.json"),
            ),
    );
    assert_eq!(
        run("grafana", &http, &grafana_cfg()).await.unwrap_err().0,
        "HTTP 500"
    );
}

// --- nextcloud --------------------------------------------------------------

#[tokio::test]
async fn nextcloud_shows_server_info() {
    let info = fixture!("nextcloud/serverinfo.json");
    let http = Arc::new(FakeHttp::new().on(Method::Get, "/ocs/v2.php/apps/serverinfo", 200, info));
    let cfg = config(&[("url", "http://nextcloud.nextcloud.svc"), ("key", "tok")]);
    let got = pairs(&run("nextcloud", &http, &cfg).await.unwrap());
    assert_eq!(got[0], ("Free Space".into(), "76.5 GiB".into()));
    assert_eq!(got[1], ("Active Users".into(), "2".into()));
    assert!(got[2].1.contains(','), "files are grouped: {}", got[2].1);
    assert_eq!(got[3], ("Shared Items".into(), "0".into()));
    assert_eq!(http.header(0, "NC-Token").unwrap(), "tok");
    assert!(http.requests()[0].url.ends_with("/info?format=json"));

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/ocs/v2.php", 200, info));
    let cfg = config(&[("url", "http://nc"), ("username", "u"), ("password", "p")]);
    run("nextcloud", &http, &cfg).await.unwrap();
    assert!(http
        .header(0, "Authorization")
        .unwrap()
        .starts_with("Basic "));
    assert!(http.header(0, "NC-Token").is_none());

    let http =
        Arc::new(FakeHttp::new().on(Method::Get, "/ocs/v2.php", 200, "<html>maintenance</html>"));
    let cfg = config(&[("url", "http://nc"), ("key", "tok")]);
    assert_eq!(
        run("nextcloud", &http, &cfg).await.unwrap_err().0,
        "unexpected response: not JSON"
    );
}

// --- prometheus, traefik, suwayomi ------------------------------------------

#[tokio::test]
async fn prometheus_counts_targets_by_health() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/v1/targets?state=active",
        200,
        fixture!("prometheus/targets.json"),
    ));
    let cfg = config(&[("url", "http://prometheus-operated.prometheus.svc:9090")]);
    let data = run("prometheus", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Targets Up", "4"),
            ("Targets Down", "1"),
            ("Total Targets", "5")
        ])
    );
    assert!(http.header(0, "Authorization").is_none());

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/v1/targets", 503, ""));
    assert_eq!(
        run("prometheus", &http, &config(&[("url", "http://p")]))
            .await
            .unwrap_err()
            .0,
        "HTTP 503"
    );
}

#[tokio::test]
async fn traefik_shows_http_totals() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/overview",
        200,
        fixture!("traefik/overview.json"),
    ));
    let cfg = config(&[("url", "http://traefik-dashboard.traefik.svc:9000")]);
    let data = run("traefik", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[("Routers", "111"), ("Services", "47"), ("Middleware", "2")])
    );

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/overview", 200, r#"{"tcp":{}}"#));
    let err = run("traefik", &http, &config(&[("url", "http://t")]))
        .await
        .unwrap_err();
    assert!(err.0.starts_with("unexpected response"), "{}", err.0);
}

#[tokio::test]
async fn suwayomi_shows_library_chapter_counts() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Post,
        "/api/graphql",
        200,
        fixture!("suwayomi/counts.json"),
    ));
    let cfg = config(&[("url", "http://suwayomi-server.nextcloud.svc:4568")]);
    let data = run("suwayomi", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Downloaded", "4,315"),
            ("Non-Downloaded", "3,923"),
            ("Read", "3,170"),
            ("Unread", "5,068")
        ])
    );
    let body: serde_json::Value =
        serde_json::from_str(http.requests()[0].body.as_ref().unwrap()).unwrap();
    let query = body["query"].as_str().unwrap();
    assert!(query.contains("download: chapters(condition: {isDownloaded: true}"));
    assert!(query.contains("filter: {inLibrary: {equalTo: true}}"));

    let http =
        Arc::new(FakeHttp::new().on(Method::Post, "/api/graphql", 200, r#"{"errors":[{}]}"#));
    let err = run("suwayomi", &http, &config(&[("url", "http://s")]))
        .await
        .unwrap_err();
    assert!(err.0.starts_with("unexpected response"), "{}", err.0);
}

// --- rutorrent --------------------------------------------------------------

#[tokio::test]
async fn rutorrent_sums_rates_and_counts_started_torrents() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Post,
        "/plugins/httprpc/action.php",
        200,
        fixture!("rutorrent/list.json"),
    ));
    let cfg = config(&[("url", "http://caddy.seedbox.svc/rutorrent/")]);
    let data = run("rutorrent", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Active", "2"),
            ("Upload", "154 kB/s"),
            ("Download", "2 MB/s")
        ])
    );
    let request = &http.requests()[0];
    assert_eq!(
        request.url,
        "http://caddy.seedbox.svc/rutorrent/plugins/httprpc/action.php"
    );
    assert_eq!(request.body.as_deref(), Some("mode=list"));

    let http =
        Arc::new(FakeHttp::new().on(Method::Post, "/action.php", 200, r#"{"t":[],"cid":1}"#));
    let data = run("rutorrent", &http, &config(&[("url", "http://r")]))
        .await
        .unwrap();
    assert_eq!(data.blocks[0].value, "0");
    assert_eq!(data.blocks[1].value, "0 B/s");

    let http =
        Arc::new(FakeHttp::new().on(Method::Post, "/action.php", 500, "<html>Error 500</html>"));
    assert_eq!(
        run("rutorrent", &http, &config(&[("url", "http://r")]))
            .await
            .unwrap_err()
            .0,
        "HTTP 500"
    );
}

// --- fritzbox ---------------------------------------------------------------

#[tokio::test]
async fn fritzbox_shows_connection_status_and_link_rates() {
    let http = Arc::new(
        FakeHttp::new()
            .on(
                Method::Post,
                "/igdupnp/control/WANIPConn1",
                200,
                fixture!("fritzbox/status_info.xml"),
            )
            .on(
                Method::Post,
                "/igdupnp/control/WANCommonIFC1",
                200,
                fixture!("fritzbox/link_properties.xml"),
            ),
    );
    let data = run("fritzbox", &http, &config(&[("url", "http://192.0.2.1/")]))
        .await
        .unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Status", "Connected"),
            ("Uptime", "6d15h"),
            ("Max. Down", "143.8 MB/s"),
            ("Max. Up", "10.8 MB/s")
        ])
    );
    let requests = http.requests();
    assert_eq!(requests.len(), 2, "only the needed SOAP actions are called");
    assert_eq!(
        requests[0].url,
        "http://192.0.2.1:49000/igdupnp/control/WANIPConn1"
    );
    assert_eq!(
        http.header(0, "SoapAction").unwrap(),
        "urn:schemas-upnp-org:service:WANIPConnection:1#GetStatusInfo"
    );

    let http = Arc::new(FakeHttp::new().on(Method::Post, "/igdupnp/control/WANIPConn1", 500, ""));
    let err = run("fritzbox", &http, &config(&[("url", "http://192.0.2.1")]))
        .await
        .unwrap_err();
    assert_eq!(err.0, "GetStatusInfo: HTTP 500");
}

// --- pihole -----------------------------------------------------------------

#[tokio::test]
async fn pihole_v6_without_password() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/stats/summary",
        200,
        fixture!("pihole/summary.json"),
    ));
    let cfg = config(&[("url", "http://192.0.2.2"), ("version", "6")]);
    let data = run("pihole", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Queries", "186,707"),
            ("Blocked", "36,229 (19%)"),
            ("Gravity", "78,216")
        ])
    );
    assert_eq!(http.requests().len(), 1);
    assert!(http.header(0, "X-FTL-SID").is_none());
}

#[tokio::test]
async fn pihole_v6_logs_in_once_and_reuses_the_session() {
    let http = Arc::new(
        FakeHttp::new()
            .on(
                Method::Post,
                "/api/auth",
                200,
                r#"{"session":{"valid":true,"sid":"abc","validity":1800}}"#,
            )
            .on(
                Method::Get,
                "/api/stats/summary",
                200,
                fixture!("pihole/summary.json"),
            ),
    );
    let cfg = config(&[("url", "http://192.0.2.2"), ("key", "pw")]);
    let widget = Registry::default().get("pihole").unwrap();
    widget.run(http.clone(), &cfg).await.0.unwrap();
    widget.run(http.clone(), &cfg).await.0.unwrap();
    let requests = http.requests();
    assert_eq!(requests.len(), 3, "login, summary, summary");
    assert_eq!(requests[0].body.as_deref(), Some(r#"{"password":"pw"}"#));
    assert_eq!(http.header(1, "X-FTL-SID").unwrap(), "abc");
    assert_eq!(http.header(2, "X-FTL-SID").unwrap(), "abc");
}

#[tokio::test]
async fn pihole_bad_password_and_v5() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Post,
        "/api/auth",
        401,
        r#"{"session":{"valid":false,"sid":null,"validity":-1,"message":"password incorrect"}}"#,
    ));
    let cfg = config(&[("url", "http://192.0.2.2"), ("key", "bad")]);
    assert_eq!(
        run("pihole", &http, &cfg).await.unwrap_err().0,
        "authentication failed"
    );

    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/admin/api.php?summaryRaw",
        200,
        r#"{"domains_being_blocked":100000,"dns_queries_today":2000,"ads_blocked_today":500,"ads_percentage_today":25.0}"#,
    ));
    let cfg = config(&[("url", "http://192.0.2.2"), ("version", "5")]);
    assert_eq!(
        run("pihole", &http, &cfg).await.unwrap().blocks[1].value,
        "500 (25%)"
    );
}

// --- paperless --------------------------------------------------------------

#[tokio::test]
async fn paperless_shows_totals() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/statistics/?format=json",
        200,
        fixture!("paperlessngx/statistics.json"),
    ));
    let cfg = config(&[("url", "http://paperless.paperless.svc"), ("key", "tok")]);
    let data = run("paperlessngx", &http, &cfg).await.unwrap();
    assert_eq!(pairs(&data), s(&[("Inbox", "-"), ("Total", "429")]));
    assert_eq!(http.header(0, "Authorization").unwrap(), "Token tok");

    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/statistics/",
        200,
        r#"{"documents_total":10,"documents_inbox":3}"#,
    ));
    let cfg = config(&[("url", "http://p"), ("key", "tok")]);
    assert_eq!(
        run("paperlessngx", &http, &cfg).await.unwrap().blocks[0].value,
        "3"
    );

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/statistics/", 401, "{}"));
    assert_eq!(
        run("paperlessngx", &http, &cfg).await.unwrap_err().0,
        "HTTP 401"
    );
}

// --- sonarr, radarr ---------------------------------------------------------

fn sonarr_http() -> FakeHttp {
    FakeHttp::new()
        .on(Method::Get, "/api/v3/wanted/missing?", 200, fixture!("sonarr/wanted_missing.json"))
        .on(Method::Get, "/api/v3/queue/details?", 200, r#"[{"seriesId":999,"status":"completed","trackedDownloadState":"importPending","size":0,"sizeleft":0},
            {"seriesId":12,"episode":{"title":"Pilot"},"status":"downloading","size":100,"sizeleft":25,"timeleft":"00:10:00"}]"#)
        .on(Method::Get, "/api/v3/queue?", 200, fixture!("sonarr/queue.json"))
        .on(Method::Get, "/api/v3/series?", 200, fixture!("sonarr/series.json"))
}

#[tokio::test]
async fn sonarr_counts_and_queue() {
    let http = Arc::new(sonarr_http());
    let cfg = config(&[
        ("url", "http://sonarr.media.svc"),
        ("key", "k"),
        ("enableQueue", "false"),
    ]);
    let data = run("sonarr", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[("Wanted", "85"), ("Queued", "0"), ("Series", "29")])
    );
    assert!(data.rows.is_empty());
    assert_eq!(http.requests().len(), 3, "queue details are not fetched");
    assert!(http.requests()[0].url.ends_with("?apikey=k"));

    let http = Arc::new(sonarr_http());
    let cfg = config(&[("url", "http://s"), ("key", "k"), ("enableQueue", "true")]);
    let data = run("sonarr", &http, &cfg).await.unwrap();
    assert_eq!(data.rows.len(), 2);
    assert_eq!(
        data.rows[0].title, "Series 0: Pilot",
        "downloading entries come first"
    );
    assert_eq!(data.rows[0].detail, "downloading · 00:10:00");
    assert_eq!(data.rows[0].progress, 75.0);
    assert_eq!(data.rows[1].title, "Unknown");
    assert_eq!(data.rows[1].detail, "import pending");
}

#[tokio::test]
async fn radarr_counts_and_never_leaks_the_key() {
    let http = Arc::new(
        FakeHttp::new()
            .on(
                Method::Get,
                "/api/v3/movie?",
                200,
                fixture!("radarr/movie.json"),
            )
            .on(
                Method::Get,
                "/api/v3/queue/status?",
                200,
                fixture!("radarr/queue_status.json"),
            ),
    );
    let cfg = config(&[("url", "http://radarr.media.svc/"), ("key", "k")]);
    let data = run("radarr", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[
            ("Wanted", "1"),
            ("Missing", "1"),
            ("Queued", "0"),
            ("Movies", "3")
        ])
    );

    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/v3/movie?", 401, ""));
    let cfg = config(&[("url", "http://radarr"), ("key", "SECRETKEY")]);
    let err = run("radarr", &http, &cfg).await.unwrap_err();
    assert_eq!(err.0, "HTTP 401");
}

// --- jackett ----------------------------------------------------------------

#[tokio::test]
async fn jackett_with_a_valid_key_needs_one_request() {
    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "apikey=good",
        200,
        fixture!("jackett/indexers.json"),
    ));
    let cfg = config(&[("url", "http://jackett.media.svc"), ("key", "good")]);
    let data = run("jackett", &http, &cfg).await.unwrap();
    assert_eq!(pairs(&data), s(&[("Configured", "2"), ("Errored", "0")]));
    assert_eq!(http.requests().len(), 1);
}

/// Stands in for Jackett without an admin password: the first call lands on
/// the dashboard HTML and yields a session cookie, the second works.
struct LoginThenJson(Mutex<Vec<Request>>);

#[async_trait]
impl Http for LoginThenJson {
    async fn send(&self, request: Request) -> Result<Response, WidgetError> {
        let has_cookie = request.headers.iter().any(|(k, _)| k == "Cookie");
        self.0.lock().unwrap().push(request);
        Ok(if has_cookie {
            Response {
                status: 200,
                body: fixture!("jackett/indexers.json").as_bytes().to_vec(),
                cookies: vec![],
            }
        } else {
            Response {
                status: 200,
                body: b"<!DOCTYPE html><html>Dashboard</html>".to_vec(),
                cookies: vec!["TestCookie=1".into(), "Jackett=session".into()],
            }
        })
    }
}

#[tokio::test]
async fn jackett_without_key_uses_and_caches_the_login_cookie() {
    let http = Arc::new(LoginThenJson(Mutex::new(Vec::new())));
    let cfg = config(&[("url", "http://jackett.media.svc")]);
    let widget = Registry::default().get("jackett").unwrap();
    let data = widget.run(http.clone(), &cfg).await.0.unwrap();
    assert_eq!(data.blocks[0].value, "2");
    widget.run(http.clone(), &cfg).await.0.unwrap();
    let requests = http.0.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "html, json, then json from the cached cookie"
    );
    assert!(requests[0].url.contains("apikey=undefined"));
    assert_eq!(
        requests[2].headers[0],
        (
            "Cookie".to_string(),
            "TestCookie=1; Jackett=session".to_string()
        )
    );
}

#[tokio::test]
async fn jackett_html_without_cookie_is_an_auth_failure() {
    let http =
        Arc::new(FakeHttp::new().on(Method::Get, "/api/v2.0/indexers", 200, "<html>login</html>"));
    let err = run("jackett", &http, &config(&[("url", "http://jackett")]))
        .await
        .unwrap_err();
    assert_eq!(err.0, "authentication failed");
}

// --- seerr ------------------------------------------------------------------

#[tokio::test]
async fn seerr_shows_request_counts() {
    let count = fixture!("seerr/request_count.json");
    let http = Arc::new(FakeHttp::new().on(Method::Get, "/api/v1/request/count", 200, count));
    let cfg = config(&[("url", "http://jellyseerr.media.svc"), ("key", "k")]);
    let data = run("jellyseerr", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[("Pending", "0"), ("Approved", "1"), ("Completed", "10")])
    );
    assert_eq!(http.header(0, "X-API-Key").unwrap(), "k");

    let http = Arc::new(FakeHttp::new().on(
        Method::Get,
        "/api/v1/request/count",
        200,
        r#"{"pending":1,"approved":2,"available":7}"#,
    ));
    let cfg = config(&[("url", "http://o"), ("key", "k")]);
    let data = run("seerr", &http, &cfg).await.unwrap();
    assert_eq!(pairs(&data)[2], ("Available".into(), "7".into()));

    let http = Arc::new(
        FakeHttp::new()
            .on(Method::Get, "/api/v1/request/count", 200, count)
            .on(
                Method::Get,
                "/api/v1/issue/count",
                200,
                r#"{"open":2,"total":5}"#,
            ),
    );
    let cfg = config(&[
        ("url", "http://o"),
        ("key", "k"),
        ("fields", "pending,issues"),
    ]);
    let data = run("seerr", &http, &cfg).await.unwrap();
    assert_eq!(
        pairs(&data),
        s(&[("Pending", "0"), ("Open Issues", "2 / 5")])
    );
}
