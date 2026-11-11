use async_trait::async_trait;

use super::{pick_fields, Block, Http, Request, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

const DEFAULT_FIELDS: [&str; 4] = ["connectionStatus", "uptime", "maxDown", "maxUp"];

/// FRITZ!Box status over the unauthenticated UPnP IGD SOAP API (TR-064
/// "igdupnp"), which must be enabled on the box ("Transmit status
/// information over UPnP").
pub struct Fritzbox;

fn label(field: &str) -> Option<&'static str> {
    Some(match field {
        "connectionStatus" => "Status",
        "uptime" => "Uptime",
        "maxDown" => "Max. Down",
        "maxUp" => "Max. Up",
        "down" => "Down",
        "up" => "Up",
        "received" => "Received",
        "sent" => "Sent",
        "externalIPAddress" => "Ext. IP",
        _ => return None,
    })
}

fn connection_status(raw: &str) -> String {
    match raw {
        "PendingDisconnect" => "Pending Disconnect".into(),
        "" => "Unconfigured".into(),
        other => other.to_string(),
    }
}

/// Text of the first `<tag>...</tag>` element. The SOAP responses are flat,
/// so this avoids pulling in an XML parser.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(xml[start..end].trim())
}

fn number(xml: &str, tag: &str) -> f64 {
    element(xml, tag)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

async fn soap(
    http: &dyn Http,
    base: &str,
    service: &str,
    action: &str,
) -> Result<String, WidgetError> {
    let path = if service == "WANIPConnection" {
        "WANIPConn1"
    } else {
        "WANCommonIFC1"
    };
    let body = format!(
        "<?xml version='1.0' encoding='utf-8'?>\
         <s:Envelope s:encodingStyle='http://schemas.xmlsoap.org/soap/encoding/' \
         xmlns:s='http://schemas.xmlsoap.org/soap/envelope/'><s:Body>\
         <u:{action} xmlns:u='urn:schemas-upnp-org:service:{service}:1' />\
         </s:Body></s:Envelope>"
    );
    let request = Request::post(format!("{base}/igdupnp/control/{path}"), body)
        .header("Content-Type", "text/xml; charset='utf-8'")
        .header(
            "SoapAction",
            format!("urn:schemas-upnp-org:service:{service}:1#{action}"),
        );
    let response = http.send(request).await?;
    if response.status != 200 {
        return Err(WidgetError::new(format!(
            "{action}: HTTP {}",
            response.status
        )));
    }
    Ok(response.text())
}

/// `http://host[:port]/path` -> `http://host:49000` (49443 for https).
fn api_base(url: &str) -> Result<String, WidgetError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| WidgetError::new("widget.url must start with http:// or https://"))?;
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(v6_end) = authority.find(']') {
        &authority[..=v6_end]
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err(WidgetError::new("widget.url has no host"));
    }
    let port = if scheme == "https" { 49443 } else { 49000 };
    Ok(format!("{scheme}://{host}:{port}"))
}

#[async_trait]
impl Widget for Fritzbox {
    fn kind(&self) -> &'static str {
        "fritzbox"
    }

    fn labels(&self, cfg: &WidgetConfig) -> Vec<&'static str> {
        pick_fields(cfg, &DEFAULT_FIELDS, 4)
            .iter()
            .filter_map(|f| label(f))
            .collect()
    }

    async fn fetch(&self, http: &dyn Http, cfg: &WidgetConfig) -> Result<WidgetData, WidgetError> {
        let base = api_base(cfg.url()?)?;
        let fields = pick_fields(cfg, &DEFAULT_FIELDS, 4);
        let wants = |names: &[&str]| fields.iter().any(|f| names.contains(&f.as_str()));

        let status = if wants(&["connectionStatus", "uptime"]) {
            soap(http, &base, "WANIPConnection", "GetStatusInfo").await?
        } else {
            String::new()
        };
        let link = if wants(&["maxDown", "maxUp"]) {
            soap(
                http,
                &base,
                "WANCommonInterfaceConfig",
                "GetCommonLinkProperties",
            )
            .await?
        } else {
            String::new()
        };
        let addon = if wants(&["down", "up", "received", "sent"]) {
            soap(http, &base, "WANCommonInterfaceConfig", "GetAddonInfos").await?
        } else {
            String::new()
        };
        let external = if wants(&["externalIPAddress"]) {
            soap(http, &base, "WANIPConnection", "GetExternalIPAddress").await?
        } else {
            String::new()
        };

        let blocks = fields
            .iter()
            .filter_map(|field| {
                let label = label(field)?;
                let value = match field.as_str() {
                    "connectionStatus" => {
                        connection_status(element(&status, "NewConnectionStatus").unwrap_or(""))
                    }
                    "uptime" => format::duration(number(&status, "NewUptime") as u64),
                    // Link rates are bits per second; shown as bytes per second.
                    "maxDown" => {
                        format::byterate(number(&link, "NewLayer1DownstreamMaxBitRate") / 8.0, 1)
                    }
                    "maxUp" => {
                        format::byterate(number(&link, "NewLayer1UpstreamMaxBitRate") / 8.0, 1)
                    }
                    "down" => format::byterate(number(&addon, "NewByteReceiveRate"), 1),
                    "up" => format::byterate(number(&addon, "NewByteSendRate"), 1),
                    "received" => format::bytes(
                        number(&addon, "NewX_AVM_DE_TotalBytesReceived64"),
                        false,
                        None,
                    ),
                    "sent" => {
                        format::bytes(number(&addon, "NewX_AVM_DE_TotalBytesSent64"), false, None)
                    }
                    "externalIPAddress" => element(&external, "NewExternalIPAddress")
                        .unwrap_or("-")
                        .to_string(),
                    _ => return None,
                };
                Some(Block::new(label, value))
            })
            .collect::<Vec<_>>();
        Ok(blocks.into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::super::Method;
    use super::*;

    const STATUS: &str = include_str!("../../fixtures/fritzbox/status_info.xml");
    const LINK: &str = include_str!("../../fixtures/fritzbox/link_properties.xml");

    #[tokio::test]
    async fn shows_connection_status_and_link_rates() {
        let http = FakeHttp::new()
            .on(Method::Post, "/igdupnp/control/WANIPConn1", 200, STATUS)
            .on(Method::Post, "/igdupnp/control/WANCommonIFC1", 200, LINK);
        let cfg = config(&[("url", "http://192.0.2.1/")]);
        let data = Fritzbox.fetch(&http, &cfg).await.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Status", "Connected".into()),
                ("Uptime", "6d15h".into()),
                ("Max. Down", "143.8 MB/s".into()),
                ("Max. Up", "10.8 MB/s".into()),
            ]
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
    }

    #[tokio::test]
    async fn soap_failure_names_the_action_only() {
        let http = FakeHttp::new().on(Method::Post, "/igdupnp/control/WANIPConn1", 500, "");
        let err = Fritzbox
            .fetch(&http, &config(&[("url", "http://192.0.2.1")]))
            .await
            .unwrap_err();
        assert_eq!(err.0, "GetStatusInfo: HTTP 500");
    }

    #[test]
    fn api_base_swaps_the_port() {
        assert_eq!(
            api_base("http://fritz.box").unwrap(),
            "http://fritz.box:49000"
        );
        assert_eq!(
            api_base("https://fritz.box:443/x").unwrap(),
            "https://fritz.box:49443"
        );
        assert_eq!(
            api_base("http://[fd00::1]:80/").unwrap(),
            "http://[fd00::1]:49000"
        );
        assert!(api_base("fritz.box").is_err());
    }
}
