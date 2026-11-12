-- Prometheus: scrape targets by health.
-- Parameters: username, password (optional).
local opts = { query = { state = "active" } }
if secret("password") then
  opts.basic = { secret("username") or widget.params.username or "", secret("password") }
end

local targets = http.get("/api/v1/targets", opts):check():json().data.activeTargets
local up, down = 0, 0
for _, target in ipairs(targets) do
  if target.health == "up" then up = up + 1 end
  if target.health == "down" then down = down + 1 end
end

return {
  { label = "Targets Up", value = fmt.number(up) },
  { label = "Targets Down", value = fmt.number(down) },
  { label = "Total Targets", value = fmt.number(#targets) },
}
