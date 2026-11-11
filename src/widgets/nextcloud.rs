use async_trait::async_trait;
use serde::Deserialize;

use super::{Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

pub struct Nextcloud;

#[derive(Deserialize)]
struct Envelope {
    ocs: Ocs,
}

#[derive(Deserialize)]
struct Ocs {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    nextcloud: Info,
    #[serde(rename = "activeUsers")]
    active_users: ActiveUsers,
}

#[derive(Deserialize)]
struct Info {
    system: System,
    storage: Storage,
    shares: Shares,
}

#[derive(Deserialize)]
struct System {
    freespace: f64,
}

#[derive(Deserialize)]
struct Storage {
    num_files: f64,
}

#[derive(Deserialize)]
struct Shares {
    num_shares: f64,
}

#[derive(Deserialize)]
struct ActiveUsers {
    last24hours: f64,
}

#[async_trait]
impl Widget for Nextcloud {
    fn kind(&self) -> &'static str {
        "nextcloud"
    }

    fn labels(&self, _cfg: &WidgetConfig) -> Vec<&'static str> {
        vec!["Free Space", "Active Users", "Files", "Shared Items"]
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let url = format!(
            "{}/ocs/v2.php/apps/serverinfo/api/v1/info?format=json",
            cfg.url()?
        );
        let mut request = Request::get(url);
        request = match (cfg.key(), cfg.basic()) {
            (Some(key), _) => request.header("NC-Token", key),
            (None, Some((user, pass))) => request.basic_auth(user, pass),
            (None, None) => return Err(WidgetError::new("widget.key is not set")),
        };
        let envelope: Envelope = http.send(request).await?.ok()?.json()?;
        let data = envelope.ocs.data;
        Ok(vec![
            Block::new(
                "Free Space",
                format::bytes(data.nextcloud.system.freespace, true, Some(1)),
            ),
            Block::new(
                "Active Users",
                format::number(data.active_users.last24hours),
            ),
            Block::new("Files", format::number(data.nextcloud.storage.num_files)),
            Block::new(
                "Shared Items",
                format::number(data.nextcloud.shares.num_shares),
            ),
        ]
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const INFO: &str = include_str!("../../fixtures/nextcloud/serverinfo.json");

    #[tokio::test]
    async fn shows_server_info_with_token_auth() {
        let http = FakeHttp::new().on(Method::Get, "/ocs/v2.php/apps/serverinfo", 200, INFO);
        let cfg = config(&[("url", "http://nextcloud.nextcloud.svc"), ("key", "tok")]);
        let data = Nextcloud.fetch(&http, &cfg).await.unwrap();
        let got = pairs(&data);
        assert_eq!(got[0], ("Free Space", "76.5 GiB".into()));
        assert_eq!(got[1], ("Active Users", "2".into()));
        assert_eq!(got[2].0, "Files");
        assert!(got[2].1.contains(','), "files are grouped: {}", got[2].1);
        assert_eq!(got[3], ("Shared Items", "0".into()));
        assert_eq!(http.header(0, "NC-Token").unwrap(), "tok");
    }

    #[tokio::test]
    async fn basic_auth_is_the_alternative_to_a_token() {
        let http = FakeHttp::new().on(Method::Get, "/ocs/v2.php", 200, INFO);
        let cfg = config(&[("url", "http://nc"), ("username", "u"), ("password", "p")]);
        Nextcloud.fetch(&http, &cfg).await.unwrap();
        assert!(http
            .header(0, "Authorization")
            .unwrap()
            .starts_with("Basic "));
        assert!(http.header(0, "NC-Token").is_none());
    }

    #[tokio::test]
    async fn html_error_page_is_reported() {
        let http = FakeHttp::new().on(Method::Get, "/ocs/v2.php", 200, "<html>maintenance</html>");
        let cfg = config(&[("url", "http://nc"), ("key", "tok")]);
        let err = Nextcloud.fetch(&http, &cfg).await.unwrap_err();
        assert!(err.0.starts_with("unexpected response"));
    }
}
