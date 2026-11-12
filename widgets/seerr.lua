-- Seerr, Jellyseerr and Overseerr: request counts.
-- Parameters: key; fields (optional, may add available, processing, issues).
local key = secret("key") or fail("widget.key is not set")
local opts = { headers = { ["X-API-Key"] = key } }

local counts = http.get("/api/v1/request/count", opts):check():json()

local fields = {}
local wants_issues = false
for i, field in ipairs(widget.fields or { "pending", "approved", "completed" }) do
  if i > 4 then break end
  -- Older releases report "available" where newer ones say "completed".
  if field == "completed" and counts.completed == nil then field = "available" end
  if field == "issues" then wants_issues = true end
  fields[#fields + 1] = field
end

local issues = wants_issues and http.get("/api/v1/issue/count", opts):check():json()

local function optional(value)
  return value and tostring(value) or "-"
end
local render = {
  pending = function() return "Pending", tostring(counts.pending) end,
  approved = function() return "Approved", tostring(counts.approved) end,
  available = function() return "Available", optional(counts.available) end,
  completed = function() return "Completed", optional(counts.completed) end,
  processing = function() return "Processing", optional(counts.processing) end,
  issues = function() return "Open Issues", issues.open .. " / " .. issues.total end,
}

local blocks = {}
for _, field in ipairs(fields) do
  if render[field] then
    local label, value = render[field]()
    blocks[#blocks + 1] = { label = label, value = value }
  end
end
return blocks
