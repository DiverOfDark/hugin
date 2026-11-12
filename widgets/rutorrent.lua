-- ruTorrent (httprpc plugin): active torrents and transfer rates.
-- Parameters: username, password (optional).
local opts = { headers = { ["Content-Type"] = "application/x-www-form-urlencoded" } }
if widget.params.username or secret("username") then
  opts.basic = { secret("username") or widget.params.username, secret("password") or "" }
end

local response = http.post("/plugins/httprpc/action.php", "mode=list", opts)
if response.status ~= 200 then fail("HTTP " .. response.status) end

-- t maps hash -> positional fields; an empty list arrives as [].
local torrents = response:json().t
if type(torrents) ~= "table" then fail("unexpected response: no torrent list") end

local active, upload, download = 0, 0, 0
for _, torrent in pairs(torrents) do
  if type(torrent) == "table" then
    if tonumber(torrent[4]) == 1 then active = active + 1 end
    upload = upload + (tonumber(torrent[12]) or 0)
    download = download + (tonumber(torrent[13]) or 0)
  end
end

return {
  { label = "Active", value = tostring(active) },
  { label = "Upload", value = fmt.rate(upload, 0) },
  { label = "Download", value = fmt.rate(download, 0) },
}
