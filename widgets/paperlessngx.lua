-- Paperless-ngx: inbox and total document counts.
-- Parameters: key (API token), or username and password.
local opts = { query = { format = "json" } }
if secret("key") then
  opts.headers = { Authorization = "Token " .. secret("key") }
elseif secret("password") then
  opts.basic = { secret("username") or widget.params.username or "", secret("password") }
else
  fail("widget.key is not set")
end

local stats = http.get("/api/statistics/", opts):check():json()
if stats.documents_total == nil then fail("unexpected response: no documents_total") end

return {
  -- nil when no inbox tag is configured
  { label = "Inbox", value = stats.documents_inbox and tostring(stats.documents_inbox) or "-" },
  { label = "Total", value = tostring(stats.documents_total) },
}
