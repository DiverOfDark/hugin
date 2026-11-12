-- Grafana: dashboards, data sources and alerts.
-- Parameters: username, password (an admin, for /api/admin/stats);
-- version = 2 with alerts = "alertmanager" reads an external Alertmanager.
local opts = {}
if widget.params.username or secret("username") then
  opts.basic = { secret("username") or widget.params.username, secret("password") or "" }
end

local function get(endpoint)
  return http.get("/api/" .. endpoint, { basic = opts.basic })
end

local stats = get("admin/stats"):check():json()

local triggered
if widget.params.version == "2" then
  local endpoint = "alertmanager/grafana/api/v2/alerts"
  if widget.params.alerts == "alertmanager" then
    endpoint = "alertmanager/alertmanager/api/v2/alerts"
  end
  triggered = #get(endpoint):check():json()
else
  -- Version 1 tries the legacy alert API first and falls back to Grafana's
  -- built-in alertmanager, which current Grafana needs because /api/alerts
  -- no longer exists.
  local legacy = get("alerts")
  local alerts = nil
  if legacy.status < 400 then
    local ok, decoded = pcall(json.decode, legacy.body)
    if ok and type(decoded) == "table" then alerts = decoded end
  end
  if alerts and #alerts > 0 then
    triggered = 0
    for _, alert in ipairs(alerts) do
      if alert.state == "alerting" then triggered = triggered + 1 end
    end
  else
    local fallback = get("alertmanager/grafana/api/v2/alerts")
    if fallback.status < 400 then
      triggered = #fallback:json()
    elseif alerts then
      triggered = 0
    else
      fail("HTTP " .. fallback.status)
    end
  end
end

return {
  { label = "Dashboards", value = fmt.number(stats.dashboards) },
  { label = "Data Sources", value = fmt.number(stats.datasources or 0) },
  { label = "Total Alerts", value = fmt.number(stats.alerts or 0) },
  { label = "Alerts Triggered", value = fmt.number(triggered) },
}
