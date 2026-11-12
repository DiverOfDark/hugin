-- Nextcloud: server info.
-- Parameters: key (serverinfo token), or username and password.
local opts = {}
if secret("key") then
  opts.headers = { ["NC-Token"] = secret("key") }
elseif secret("password") then
  opts.basic = { secret("username") or widget.params.username or "", secret("password") }
else
  fail("widget.key is not set")
end
opts.query = { format = "json" }

local data = http.get("/ocs/v2.php/apps/serverinfo/api/v1/info", opts):check():json().ocs.data
local nc = data.nextcloud

return {
  { label = "Free Space", value = fmt.bytes(nc.system.freespace, { binary = true, digits = 1 }) },
  { label = "Active Users", value = fmt.number(data.activeUsers.last24hours) },
  { label = "Files", value = fmt.number(nc.storage.num_files) },
  { label = "Shared Items", value = fmt.number(nc.shares.num_shares) },
}
