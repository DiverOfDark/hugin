# Widget script API

A widget is a Lua 5.4 script. hugin runs it on every refresh, in a fresh
sandbox, and shows what it returns on the service's card.

```lua
local stats = http.get("/api/statistics/", {
  headers = { Authorization = "Token " .. secret("key") },
}):check():json()

return {
  { label = "Inbox", value = fmt.number(stats.documents_inbox) },
  { label = "Total", value = fmt.number(stats.documents_total) },
}
```

## Return value

Return a list of blocks, each `{ label = "...", value = "..." }`. Keep it to
four or fewer; at most six are shown. `value` may be a string or a number.

A block may carry `status = "warn"` or `status = "error"`. The value is then
shown in amber or red and the dashboard counts the service as needing
attention. `status = "ok"` and no status mean the same.

| Status | Means | Examples |
| --- | --- | --- |
| none | Normal operation, nothing to do. | A count, a queue, a standby node, paused items. |
| `warn` | Check it when you have time. The service still does its job and nothing is lost if it waits a day. | Health `WARN`, failed items, a target down, alerts firing, a backup overdue. |
| `error` | Act now. The service cannot do its job, or data can be lost. | Health `ERR`, a sealed vault, no redundancy left, a full disk, the last backup failed, a critical alert. |

Take the application's own severity when it has one, and take the lower
level when unsure. The status is a condition on the value, never a constant.

```lua
local health = http.get("/api/health"):check():json()
return {
  { label = "Health", value = health.status,
    status = health.status == "ERR" and "error"
      or health.status ~= "OK" and "warn" or nil },
  { label = "Used", value = fmt.percent(health.used_percent) },
}
```

A second return value may list rows shown under the blocks, each
`{ title = "...", detail = "...", progress = 0..100 }`.

To report a problem, call `fail("short reason")`. The card then shows
"API Error" with that reason.

## Globals

| Name | Meaning |
| --- | --- |
| `widget.url` | Base URL of the service, without a trailing slash. |
| `widget.scheme`, `widget.host`, `widget.port` | Parts of `widget.url`. |
| `widget.params` | The other plain `widget.*` parameters, as strings. For example `widget.params.enableQueue`. |
| `widget.fields` | `widget.fields` as a list, or `nil` when it is not set. |
| `secret(name)` | A placeholder for the credential `name` (`key`, `password`, ...), or `nil` when it is not set. |
| `state` | A table kept between refreshes of the same service. Use it for session ids and cookies. Only JSON-like values survive. |
| `fail(reason)` | Stops the script with a reason for the card. |
| `log(message)` | Writes a debug log line. |

### Credentials

A script never sees a credential. `secret("key")` returns a placeholder
string; hugin swaps in the real value when the request is sent. Put the
placeholder wherever the service expects the credential: a header, a query
parameter, the request body or `basic`. Credential values that come back in
a response body are replaced with `[redacted]`.

### http

| Call | Meaning |
| --- | --- |
| `http.get(url, opts)` | GET request. |
| `http.post(url, body, opts)` | POST request with a string body. |
| `http.request(opts)` | Same, with `opts.method`, `opts.url` and `opts.body`. |

`url` is a path under `widget.url` such as `"/api/v1/status"`, or a full URL
on the same host (for a different port, build it from `widget.scheme` and
`widget.host`). Requests to any other host are refused.

`opts` may contain:

| Field | Meaning |
| --- | --- |
| `headers` | Table of request headers. |
| `query` | Table of query parameters. They are URL-encoded and appended in alphabetical order. |
| `basic` | `{ username, password }` for HTTP basic auth. |

The response is a table:

| Field | Meaning |
| --- | --- |
| `status` | HTTP status code. |
| `ok` | `true` for 2xx. |
| `body` | Response body as a string. |
| `cookies` | List of `"name=value"` cookies set by the response and by redirects. |
| `:json()` | The body decoded as JSON. Fails the script if it is not JSON. |
| `:check()` | Fails the script with `HTTP <status>` when the status is 400 or higher. Returns the response, so it chains. |

A request that cannot be completed (timeout, connection refused) fails the
script with that reason. Redirects are followed on the same host only.

### json

`json.decode(text)` returns Lua values; JSON `null` becomes `nil`.
`json.encode(value)` returns a string.

### fmt

All of these return `"-"` for `nil`.

| Call | Example |
| --- | --- |
| `fmt.number(n)` | `75831` gives `75,831`. Up to three decimals. |
| `fmt.number(n, digits)` | `fmt.number(0.03471, 4)` gives `0.0347`. |
| `fmt.percent(n)` | `19.3` gives `19%`. |
| `fmt.bytes(n)` | `227000000` gives `227 MB`. |
| `fmt.bytes(n, { binary = true, digits = 1 })` | `82141245440` gives `76.5 GiB`. |
| `fmt.rate(n, digits)` | Bytes per second: `fmt.rate(143800000, 1)` gives `143.8 MB/s`. |
| `fmt.duration(seconds)` | `575290` gives `6d15h`. |

## Examples

A key in a header, and a check that the response is what was expected:

```lua
local key = secret("key") or fail("widget.key is not set")
local counts = http.get("/api/v1/request/count", {
  headers = { ["X-Api-Key"] = key },
}):check():json()
if counts.pending == nil then fail("unexpected response: no pending count") end

return {
  { label = "Pending", value = fmt.number(counts.pending) },
  { label = "Approved", value = fmt.number(counts.approved) },
}
```

A key in the query string, and counting over a list:

```lua
local items = http.get("/api/v3/movie", { query = { apikey = secret("key") } }):check():json()
local missing = 0
for _, item in ipairs(items) do
  if item.monitored and not item.hasFile then missing = missing + 1 end
end
return {
  { label = "Missing", value = fmt.number(missing) },
  { label = "Movies", value = fmt.number(#items) },
}
```

Basic auth with a user name that may be plain or a secret:

```lua
local stats = http.get("/api/admin/stats", {
  basic = { secret("username") or widget.params.username, secret("password") },
}):check():json()
return { { label = "Dashboards", value = fmt.number(stats.dashboards) } }
```

A login that returns a session id, kept in `state` and renewed when it expires:

```lua
local function login()
  local reply = http.post("/api/auth", json.encode({ password = secret("key") }), {
    headers = { ["Content-Type"] = "application/json" },
  }):json()
  state.sid = reply.session and reply.session.sid or fail("authentication failed")
end

local function summary()
  return http.get("/api/stats/summary", { headers = { ["X-Session"] = state.sid } })
end

if secret("key") and not state.sid then login() end
local response = summary()
if response.status == 401 and secret("key") then
  login()
  response = summary()
end
local data = response:check():json()
return { { label = "Queries", value = fmt.number(data.queries.total) } }
```

An API on another port of the same host, with an XML answer read by pattern:

```lua
local base = widget.scheme .. "://" .. widget.host .. ":49000"
local xml = http.post(base .. "/control", "<request/>", {
  headers = { ["Content-Type"] = "text/xml" },
}):check().body
local uptime = tonumber(xml:match("<Uptime>(%d+)</Uptime>"))
return { { label = "Uptime", value = fmt.duration(uptime) } }
```

A session cookie handed out by a redirect, reused on the next request:

```lua
local function list(cookie)
  return http.get("/api/items", { headers = cookie and { Cookie = cookie } or nil })
end

local response = list(state.cookie)
local ok, items = pcall(json.decode, response.body)
if not ok and #response.cookies > 0 then
  state.cookie = table.concat(response.cookies, "; ")
  response = list(state.cookie)
  ok, items = pcall(json.decode, response.body)
end
if not ok then fail("authentication failed") end
return { { label = "Items", value = fmt.number(#items) } }
```

## Sandbox

Available from the standard library: `string`, `table`, `math`, `utf8`,
`coroutine`, and the basic functions (`pairs`, `ipairs`, `tostring`,
`tonumber`, `type`, `select`, `pcall`, `error`, `next`, `setmetatable`, ...).

Not available: files, the operating system, `require`, `load`, `debug`.

Limits per run: 12 requests, 8 MiB per response, 32 MiB of Lua memory, a
fixed instruction budget, and a time limit of three request timeouts.
