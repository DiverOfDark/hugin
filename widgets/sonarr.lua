-- Sonarr: wanted episodes, queue and series.
-- Parameters: key; enableQueue = "true" lists the download queue.
local key = secret("key") or fail("widget.key is not set")

-- The API key travels in the query string.
local function get(endpoint)
  return http.get("/api/v3/" .. endpoint, { query = { apikey = key } }):check():json()
end

local function progress(entry)
  local size = tonumber(entry.size) or 0
  if size <= 0 then return 0 end
  return math.max(0, math.min(100, (1 - (tonumber(entry.sizeleft) or 0) / size) * 100))
end

local function activity(entry)
  local completed = { importBlocked = true, importPending = true, importing = true, failedPending = true }
  local state = entry.status or entry.trackedDownloadState or ""
  if entry.status == "completed" and completed[entry.trackedDownloadState or ""] then
    state = entry.trackedDownloadState
  end
  local words = { importBlocked = "import blocked", importPending = "import pending", failedPending = "failed pending" }
  return words[state] or state
end

-- Downloading entries first, then by how close they are to done.
local function sort_queue(queue)
  local function downloading(entry)
    return (entry.status or entry.trackedDownloadState) == "downloading"
  end
  table.sort(queue, function(a, b)
    if downloading(a) ~= downloading(b) then return downloading(a) end
    return progress(a) > progress(b)
  end)
end

local function row(entry, title)
  local detail = activity(entry)
  if entry.timeleft and entry.timeleft ~= "" then detail = detail .. " · " .. entry.timeleft end
  return { title = title, detail = detail, progress = progress(entry) }
end

local wanted = get("wanted/missing")
local queued = get("queue")
local series = get("series")

local rows = {}
if widget.params.enableQueue == "true" then
  local queue = get("queue/details")
  sort_queue(queue)
  for _, entry in ipairs(queue) do
    local title = ""
    for _, s in ipairs(series) do
      if s.id == entry.seriesId then title = (s.title or "") .. ": " end
    end
    local episode = (entry.episode and entry.episode.title) or entry.title
    if episode then title = title .. episode end
    if title == "" then title = "Unknown" end
    rows[#rows + 1] = row(entry, title)
  end
end

return {
  { label = "Wanted", value = fmt.number(wanted.totalRecords) },
  { label = "Queued", value = fmt.number(queued.totalRecords) },
  { label = "Series", value = fmt.number(#series) },
}, rows
