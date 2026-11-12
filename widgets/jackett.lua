-- Jackett: configured and errored indexers.
-- Parameters: key (API key), or password (admin password), or neither.
--
-- Jackett answers its API with a valid apikey or with a session cookie.
-- Without an admin password it hands that cookie to anyone who follows its
-- login redirect, which is how a key-less widget works.
local function indexers(cookie)
  local opts = {
    -- gethomepage sends the literal "undefined" when no key is set; Jackett
    -- treats any wrong key the same way.
    query = { apikey = secret("key") or "undefined", configured = "true" },
  }
  if cookie then opts.headers = { Cookie = cookie } end
  return http.get("/api/v2.0/indexers", opts)
end

local function decode(response)
  local ok, list = pcall(json.decode, response.body)
  if ok and type(list) == "table" and response.ok then return list end
  return nil
end

local function cookie_of(response)
  if #response.cookies == 0 then return nil end
  return table.concat(response.cookies, "; ")
end

if not state.cookie and secret("password") then
  local login = http.post("/UI/Dashboard", "password=" .. secret("password"), {
    headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
  })
  state.cookie = cookie_of(login) or fail("authentication failed")
end

local response = indexers(state.cookie)
local list = decode(response)
if not list then
  -- Not JSON: we were sent through the login redirect. Retry once with
  -- whatever session cookie that produced.
  local fresh = cookie_of(response)
  if fresh then
    response = indexers(fresh)
    list = decode(response)
    state.cookie = fresh
  end
end
if not list then
  state.cookie = nil
  if response.status >= 400 then fail("HTTP " .. response.status) end
  fail("authentication failed")
end

local errored = 0
for _, indexer in ipairs(list) do
  if indexer.last_error and indexer.last_error ~= "" then errored = errored + 1 end
end

return {
  { label = "Configured", value = fmt.number(#list) },
  { label = "Errored", value = fmt.number(errored) },
}
