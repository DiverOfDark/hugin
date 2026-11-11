//! Sonarr and Radarr share the v3 API shape and the download queue view.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::{Block, Http, Request, Row, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

pub struct Sonarr;
pub struct Radarr;

#[derive(Deserialize)]
struct Paged {
    #[serde(rename = "totalRecords")]
    total_records: f64,
}

#[derive(Deserialize)]
struct Titled {
    id: i64,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct Movie {
    id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    monitored: bool,
    #[serde(default, rename = "hasFile")]
    has_file: bool,
    #[serde(default, rename = "isAvailable")]
    is_available: bool,
}

#[derive(Deserialize)]
struct QueueStatus {
    #[serde(rename = "totalCount")]
    total_count: f64,
}

#[derive(Deserialize)]
struct QueueEntry {
    #[serde(default, rename = "trackedDownloadState")]
    tracked_download_state: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    timeleft: Option<String>,
    #[serde(default)]
    size: f64,
    #[serde(default)]
    sizeleft: f64,
    #[serde(default, rename = "movieId")]
    movie_id: Option<i64>,
    #[serde(default, rename = "seriesId")]
    series_id: Option<i64>,
    #[serde(default)]
    episode: Option<Episode>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Deserialize)]
struct Episode {
    #[serde(default)]
    title: Option<String>,
}

async fn get<T: DeserializeOwned>(
    http: &dyn Http,
    cfg: &WidgetConfig,
    endpoint: &str,
) -> Result<T, WidgetError> {
    // The API key travels in the query string; errors must never echo it.
    let url = format!(
        "{}/api/v3/{endpoint}?apikey={}",
        cfg.url()?,
        cfg.require_key()?
    );
    http.send(Request::get(url)).await?.ok()?.json()
}

fn activity(entry: &QueueEntry) -> String {
    const COMPLETED: [&str; 4] = [
        "importBlocked",
        "importPending",
        "importing",
        "failedPending",
    ];
    let tracked = entry.tracked_download_state.as_deref();
    let state = match (entry.status.as_deref(), tracked) {
        (Some("completed"), Some(t)) if COMPLETED.contains(&t) => t,
        (Some(status), _) => status,
        (None, Some(t)) => t,
        (None, None) => "",
    };
    match state {
        "importBlocked" => "import blocked".into(),
        "importPending" => "import pending".into(),
        "failedPending" => "failed pending".into(),
        other => other.to_string(),
    }
}

fn progress(entry: &QueueEntry) -> f64 {
    if !entry.size.is_finite() || entry.size <= 0.0 {
        return 0.0;
    }
    ((1.0 - entry.sizeleft / entry.size) * 100.0).clamp(0.0, 100.0)
}

/// Downloading entries first, then by how close they are to done.
fn sort_queue(queue: &mut [QueueEntry]) {
    let downloading = |e: &QueueEntry| {
        e.status.as_deref().or(e.tracked_download_state.as_deref()) == Some("downloading")
    };
    queue.sort_by(|a, b| {
        downloading(b).cmp(&downloading(a)).then(
            progress(b)
                .partial_cmp(&progress(a))
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
}

fn row(entry: &QueueEntry, title: String) -> Row {
    let mut detail = activity(entry);
    if let Some(left) = entry.timeleft.as_deref().filter(|t| !t.is_empty()) {
        detail.push_str(&format!(" · {left}"));
    }
    Row {
        title,
        detail,
        progress: progress(entry),
    }
}

#[async_trait]
impl Widget for Sonarr {
    fn kind(&self) -> &'static str {
        "sonarr"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Wanted", "Queued", "Series"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let wanted: Paged = get(http, cfg, "wanted/missing").await?;
        let queued: Paged = get(http, cfg, "queue").await?;
        let series: Vec<Titled> = get(http, cfg, "series").await?;

        let mut rows = Vec::new();
        if cfg.flag("enableQueue") {
            let mut queue: Vec<QueueEntry> = get(http, cfg, "queue/details").await?;
            sort_queue(&mut queue);
            for entry in &queue {
                let mut title = String::new();
                if let Some(s) = series.iter().find(|s| Some(s.id) == entry.series_id) {
                    title.push_str(&format!("{}: ", s.title));
                }
                let episode = entry
                    .episode
                    .as_ref()
                    .and_then(|e| e.title.clone())
                    .or_else(|| entry.title.clone());
                if let Some(episode) = episode {
                    title.push_str(&episode);
                }
                if title.is_empty() {
                    title.push_str("Unknown");
                }
                rows.push(row(entry, title));
            }
        }

        Ok(WidgetData {
            blocks: vec![
                Block::new("Wanted", format::number(wanted.total_records)),
                Block::new("Queued", format::number(queued.total_records)),
                Block::new("Series", format::int(series.len() as i64)),
            ],
            rows,
        })
    }
}

#[async_trait]
impl Widget for Radarr {
    fn kind(&self) -> &'static str {
        "radarr"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Wanted", "Missing", "Queued", "Movies"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let movies: Vec<Movie> = get(http, cfg, "movie").await?;
        let queued: QueueStatus = get(http, cfg, "queue/status").await?;

        let wanted = movies
            .iter()
            .filter(|m| m.monitored && !m.has_file && m.is_available)
            .count();
        let missing = movies.iter().filter(|m| m.monitored && !m.has_file).count();
        let have = movies.iter().filter(|m| m.has_file).count();

        let mut rows = Vec::new();
        if cfg.flag("enableQueue") {
            let mut queue: Vec<QueueEntry> = get(http, cfg, "queue/details").await?;
            sort_queue(&mut queue);
            for entry in &queue {
                let title = movies
                    .iter()
                    .find(|m| Some(m.id) == entry.movie_id)
                    .map(|m| m.title.clone())
                    .unwrap_or_else(|| "Unknown".into());
                rows.push(row(entry, title));
            }
        }

        Ok(WidgetData {
            blocks: vec![
                Block::new("Wanted", format::int(wanted as i64)),
                Block::new("Missing", format::int(missing as i64)),
                Block::new("Queued", format::number(queued.total_count)),
                Block::new("Movies", format::int(have as i64)),
            ],
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const SERIES: &str = include_str!("../../fixtures/sonarr/series.json");
    const QUEUE: &str = include_str!("../../fixtures/sonarr/queue.json");
    const WANTED: &str = include_str!("../../fixtures/sonarr/wanted_missing.json");
    const MOVIES: &str = include_str!("../../fixtures/radarr/movie.json");
    const QUEUE_STATUS: &str = include_str!("../../fixtures/radarr/queue_status.json");

    fn sonarr_http() -> FakeHttp {
        FakeHttp::new()
            .on(Method::Get, "/api/v3/wanted/missing?", 200, WANTED)
            .on(Method::Get, "/api/v3/queue?", 200, QUEUE)
            .on(Method::Get, "/api/v3/series?", 200, SERIES)
    }

    #[tokio::test]
    async fn sonarr_counts() {
        let http = sonarr_http();
        let cfg = config(&[
            ("url", "http://sonarr.media.svc"),
            ("key", "k"),
            ("enableQueue", "false"),
        ]);
        let data = Sonarr.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Wanted", "85".into()),
                ("Queued", "0".into()),
                ("Series", "29".into()),
            ]
        );
        assert!(data.rows.is_empty());
        assert_eq!(http.requests().len(), 3, "queue details are not fetched");
        assert!(http.requests()[0].url.ends_with("?apikey=k"));
    }

    #[tokio::test]
    async fn sonarr_queue_rows_when_enabled() {
        let http = sonarr_http().on(
            Method::Get,
            "/api/v3/queue/details?",
            200,
            r#"[{"seriesId":12,"episode":{"title":"Pilot"},"status":"downloading","size":100,"sizeleft":25,"timeleft":"00:10:00"},
                {"seriesId":999,"status":"completed","trackedDownloadState":"importPending","size":0,"sizeleft":0}]"#,
        );
        let cfg = config(&[("url", "http://s"), ("key", "k"), ("enableQueue", "true")]);
        let data = Sonarr.fetch(&http, &cfg).await.unwrap();
        assert_eq!(data.rows.len(), 2);
        assert_eq!(data.rows[0].title, "Series 0: Pilot");
        assert_eq!(data.rows[0].detail, "downloading · 00:10:00");
        assert_eq!(data.rows[0].progress, 75.0);
        assert_eq!(data.rows[1].title, "Unknown");
        assert_eq!(data.rows[1].detail, "import pending");
    }

    #[tokio::test]
    async fn radarr_counts() {
        let http = FakeHttp::new()
            .on(Method::Get, "/api/v3/movie?", 200, MOVIES)
            .on(Method::Get, "/api/v3/queue/status?", 200, QUEUE_STATUS);
        let cfg = config(&[("url", "http://radarr.media.svc/"), ("key", "k")]);
        let data = Radarr.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Wanted", "1".into()),
                ("Missing", "1".into()),
                ("Queued", "0".into()),
                ("Movies", "3".into()),
            ]
        );
    }

    #[tokio::test]
    async fn errors_never_contain_the_api_key() {
        let http = FakeHttp::new().on(Method::Get, "/api/v3/movie?", 401, "");
        let cfg = config(&[("url", "http://radarr"), ("key", "SECRETKEY")]);
        let err = Radarr.fetch(&http, &cfg).await.unwrap_err();
        assert_eq!(err.0, "HTTP 401");
        assert!(!err.0.contains("SECRETKEY"));
    }
}
