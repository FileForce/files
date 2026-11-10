-- libfiles — talk to file FORCE from a Quick Action script.
--
--   local files = require("libfiles")
--   local answers = files.ask("Resize", { files.choice("size", "Width", { "1080", "720" }) })
--   files.made(output); files.notify("Done", "3 pictures resized")
--
-- Messages go to Files as JSON lines on stdout; answers come back on stdin.
local M = {}

local function quote(value)
  return '"' .. tostring(value):gsub('[%c"\\]', function(c) return string.format("\\u%04x", c:byte()) end) .. '"'
end

local function encode(value)
  if type(value) == "table" then
    if #value > 0 or next(value) == nil then
      local items = {}
      for _, item in ipairs(value) do items[#items + 1] = encode(item) end
      return "[" .. table.concat(items, ",") .. "]"
    end
    local items = {}
    for key, item in pairs(value) do items[#items + 1] = quote(key) .. ":" .. encode(item) end
    return "{" .. table.concat(items, ",") .. "}"
  elseif type(value) == "number" or type(value) == "boolean" then
    return tostring(value)
  elseif value == nil then
    return "null"
  end
  return quote(value)
end

local function send(message)
  io.stdout:write("@files " .. encode(message) .. "\n")
  io.stdout:flush()
end

function M.selection() return { table.unpack(arg) } end
function M.answer(key, default) return os.getenv("FILES_ANSWER_" .. key:upper()) or default end
function M.choice(key, label, options) return { kind = "choice", key = key, label = label, options = options } end
function M.text(key, label, default) return { kind = "text", key = key, label = label, default = default or "" } end
function M.number(key, label, default) return { kind = "number", key = key, label = label, default = tostring(default or 0) } end

-- Shows a dialog in Files and waits; returns { key = value }, or nil if cancelled.
function M.ask(title, fields)
  send({ op = "ask", title = title, fields = fields })
  local line = io.read("l") or ""
  if line:find('"cancelled"') then return nil end
  local answers = {}
  for key, value in line:gmatch('"([^"]+)"%s*:%s*"([^"]*)"') do
    if key ~= "answers" then answers[key] = value end
  end
  return answers
end

function M.notify(title, body, level) send({ op = "notify", title = title, body = body, level = level or "success" }) end
function M.made(path) send({ op = "made", path = path }) end
function M.reveal(path) send({ op = "reveal", path = path }) end
function M.progress(text) send({ op = "progress", text = text }) end

return M
