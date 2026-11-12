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

## Sandbox

Available from the standard library: `string`, `table`, `math`, `utf8`,
`coroutine`, and the basic functions (`pairs`, `ipairs`, `tostring`,
`tonumber`, `type`, `select`, `pcall`, `error`, `next`, `setmetatable`, ...).

Not available: files, the operating system, `require`, `load`, `debug`.

Limits per run: 12 requests, 8 MiB per response, 32 MiB of Lua memory, a
fixed instruction budget, and a time limit of three request timeouts.
