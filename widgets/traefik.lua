-- Traefik: HTTP routers, services and middlewares.
-- Parameters: username, password (optional).
local opts = {}
if secret("password") then
  opts.basic = { secret("username") or widget.params.username or "", secret("password") }
end

local overview = http.get("/api/overview", opts):check():json()
if type(overview.http) ~= "table" then fail("unexpected response: no http section") end

return {
  { label = "Routers", value = tostring(overview.http.routers.total) },
  { label = "Services", value = tostring(overview.http.services.total) },
  { label = "Middleware", value = tostring(overview.http.middlewares.total) },
}
