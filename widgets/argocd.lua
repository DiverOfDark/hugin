-- Argo CD: applications by sync and health status.
-- Parameters: key (API token); fields (optional).
local key = secret("key") or fail("widget.key is not set")

-- The unfiltered list is megabytes on a mid-sized cluster; ask only for the
-- sync and health status. ("items.status.health.status" is dropped by
-- Argo CD's field filter, "items.status.health" is not.)
local apps = http.get("/api/v1/applications", {
  query = { fields = "items.status.sync.status,items.status.health" },
  headers = { Authorization = "Bearer " .. key },
}):check():json()
local items = apps.items or {}

local labels = {
  apps = "Apps", synced = "Synced", outOfSync = "Out Of Sync", healthy = "Healthy",
  degraded = "Degraded", progressing = "Progressing", missing = "Missing", suspended = "Suspended",
}

local blocks = {}
for i, field in ipairs(widget.fields or { "apps", "synced", "outOfSync", "healthy" }) do
  if i > 4 then break end
  if labels[field] then
    local count = 0
    if field == "apps" then
      count = #items
    else
      local want = field:lower()
      for _, app in ipairs(items) do
        local status = app.status or {}
        local sync = ((status.sync or {}).status or ""):lower()
        local health = ((status.health or {}).status or ""):lower()
        if sync == want or health == want then count = count + 1 end
      end
    end
    blocks[#blocks + 1] = { label = labels[field], value = tostring(count) }
  end
end
return blocks
