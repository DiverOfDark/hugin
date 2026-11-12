-- Pi-hole v6 (default) and the v5 api.php (version = 5).
-- Parameters: key (app password, optional on v6); version; fields (optional).
local summary

if (tonumber(widget.params.version) or 6) < 6 then
  local function loose(value)
    if type(value) == "string" then return tonumber((value:gsub(",", ""))) or 0 end
    return tonumber(value) or 0
  end
  local raw = http.get("/admin/api.php?summaryRaw", { query = { auth = secret("key") or "" } }):check():json()
  summary = {
    queries = loose(raw.dns_queries_today),
    blocked = loose(raw.ads_blocked_today),
    percent = loose(raw.ads_percentage_today),
    gravity = loose(raw.domains_being_blocked),
  }
else
  local function login()
    local response = http.post("/api/auth", json.encode({ password = secret("key") }), {
      headers = { ["Content-Type"] = "application/json" },
    })
    local ok, auth = pcall(json.decode, response.body)
    local session = ok and type(auth) == "table" and auth.session
    if response.status ~= 200 or not session or not session.sid then
      state.sid = nil
      fail("authentication failed")
    end
    state.sid = session.sid
  end

  local function fetch()
    local headers = {}
    if state.sid then headers["X-FTL-SID"] = state.sid end
    return http.get("/api/stats/summary", { headers = headers })
  end

  if secret("key") and not state.sid then login() end
  local response = fetch()
  -- An expired or revoked session: log in again once.
  if response.status == 401 and secret("key") then
    login()
    response = fetch()
  end
  local raw = response:check():json()
  summary = {
    queries = raw.queries.total,
    blocked = raw.queries.blocked,
    percent = raw.queries.percent_blocked,
    gravity = raw.gravity.domains_being_blocked,
  }
end

local fields = widget.fields or { "queries", "blocked", "gravity" }
local shows_percent = false
for _, field in ipairs(fields) do
  if field == "blocked_percent" then shows_percent = true end
end

local percent = fmt.percent(summary.percent)
local render = {
  queries = function() return "Queries", fmt.number(summary.queries) end,
  blocked = function()
    if shows_percent then return "Blocked", fmt.number(summary.blocked) end
    return "Blocked", fmt.number(summary.blocked) .. " (" .. percent .. ")"
  end,
  blocked_percent = function() return "Blocked %", percent end,
  gravity = function() return "Gravity", fmt.number(summary.gravity) end,
}

local blocks = {}
for i, field in ipairs(fields) do
  if i > 4 then break end
  if render[field] then
    local label, value = render[field]()
    blocks[#blocks + 1] = { label = label, value = value }
  end
end
return blocks
