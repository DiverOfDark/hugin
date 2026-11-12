-- Suwayomi: chapter counts of the library.
-- Parameters: username, password (optional); fields (optional).
local counts = {
  download = { "Downloaded", "isDownloaded: true" },
  nondownload = { "Non-Downloaded", "isDownloaded: false" },
  read = { "Read", "isRead: true" },
  unread = { "Unread", "isRead: false" },
  downloadedread = { "Downloaded & Read", "isDownloaded: true, isRead: true" },
  downloadedunread = { "Downloaded & Unread", "isDownloaded: true, isRead: false" },
  nondownloadedread = { "Non-Downloaded & Read", "isDownloaded: false, isRead: true" },
  nondownloadedunread = { "Non-Downloaded & Unread", "isDownloaded: false, isRead: false" },
}

local category = widget.params.category
if category and category ~= "all" then fail("widget.category is not supported") end

local fields = {}
for i, field in ipairs(widget.fields or { "download", "nondownload", "read", "unread" }) do
  if i > 4 then break end
  if counts[field] then fields[#fields + 1] = field end
end

local query = "query Counts {"
for _, field in ipairs(fields) do
  query = query .. string.format(
    " %s: chapters(condition: {%s} filter: {inLibrary: {equalTo: true}}) { totalCount }",
    field, counts[field][2])
end
query = query .. " }"

local opts = { headers = { ["Content-Type"] = "application/json" } }
if secret("password") then
  opts.basic = { secret("username") or widget.params.username or "", secret("password") }
end
local response = http.post("/api/graphql", json.encode({ operationName = "Counts", query = query }), opts)
if response.status == 401 then fail("unauthorized") end
local data = response:check():json().data

local blocks = {}
for _, field in ipairs(fields) do
  local count = data and data[field] and data[field].totalCount
  if not count then fail("unexpected response: missing count") end
  blocks[#blocks + 1] = { label = counts[field][1], value = fmt.number(count) }
end
return blocks
