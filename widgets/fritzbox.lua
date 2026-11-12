-- FRITZ!Box status over the unauthenticated UPnP IGD SOAP API, which must be
-- enabled on the box ("Transmit status information over UPnP").
-- Parameters: fields (optional).
local port = widget.scheme == "https" and 49443 or 49000
local host = widget.host
if host:find(":", 1, true) then host = "[" .. host .. "]" end
local base = widget.scheme .. "://" .. host .. ":" .. port

local fields = {}
for i, field in ipairs(widget.fields or { "connectionStatus", "uptime", "maxDown", "maxUp" }) do
  if i > 4 then break end
  fields[#fields + 1] = field
end
local function wants(...)
  for _, name in ipairs({ ... }) do
    for _, field in ipairs(fields) do
      if field == name then return true end
    end
  end
  return false
end

local function soap(service, action)
  local path = service == "WANIPConnection" and "WANIPConn1" or "WANCommonIFC1"
  local body = "<?xml version='1.0' encoding='utf-8'?>"
    .. "<s:Envelope s:encodingStyle='http://schemas.xmlsoap.org/soap/encoding/' "
    .. "xmlns:s='http://schemas.xmlsoap.org/soap/envelope/'><s:Body>"
    .. "<u:" .. action .. " xmlns:u='urn:schemas-upnp-org:service:" .. service .. ":1' />"
    .. "</s:Body></s:Envelope>"
  local response = http.post(base .. "/igdupnp/control/" .. path, body, {
    headers = {
      ["Content-Type"] = "text/xml; charset='utf-8'",
      SoapAction = "urn:schemas-upnp-org:service:" .. service .. ":1#" .. action,
    },
  })
  if response.status ~= 200 then fail(action .. ": HTTP " .. response.status) end
  return response.body
end

-- The responses are flat, so a pattern is enough.
local function element(xml, tag)
  return xml and xml:match("<" .. tag .. ">%s*(.-)%s*</" .. tag .. ">")
end
local function number(xml, tag)
  return tonumber(element(xml, tag)) or 0
end

local status = wants("connectionStatus", "uptime") and soap("WANIPConnection", "GetStatusInfo")
local link = wants("maxDown", "maxUp") and soap("WANCommonInterfaceConfig", "GetCommonLinkProperties")
local addon = wants("down", "up", "received", "sent") and soap("WANCommonInterfaceConfig", "GetAddonInfos")
local external = wants("externalIPAddress") and soap("WANIPConnection", "GetExternalIPAddress")

local render = {
  connectionStatus = function()
    local raw = element(status, "NewConnectionStatus") or ""
    if raw == "" then raw = "Unconfigured" end
    if raw == "PendingDisconnect" then raw = "Pending Disconnect" end
    return "Status", raw
  end,
  uptime = function() return "Uptime", fmt.duration(number(status, "NewUptime")) end,
  -- Link rates are bits per second; shown as bytes per second.
  maxDown = function() return "Max. Down", fmt.rate(number(link, "NewLayer1DownstreamMaxBitRate") / 8, 1) end,
  maxUp = function() return "Max. Up", fmt.rate(number(link, "NewLayer1UpstreamMaxBitRate") / 8, 1) end,
  down = function() return "Down", fmt.rate(number(addon, "NewByteReceiveRate"), 1) end,
  up = function() return "Up", fmt.rate(number(addon, "NewByteSendRate"), 1) end,
  received = function() return "Received", fmt.bytes(number(addon, "NewX_AVM_DE_TotalBytesReceived64")) end,
  sent = function() return "Sent", fmt.bytes(number(addon, "NewX_AVM_DE_TotalBytesSent64")) end,
  externalIPAddress = function() return "Ext. IP", element(external, "NewExternalIPAddress") or "-" end,
}

local blocks = {}
for _, field in ipairs(fields) do
  if render[field] then
    local label, value = render[field]()
    blocks[#blocks + 1] = { label = label, value = value }
  end
end
return blocks
