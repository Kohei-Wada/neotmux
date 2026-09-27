-- A JSON decoder and encoder for the Lua here, sized for one job: the Jev API's answers in,
-- the Jev request out.
--
--   local json = require('neotmux.json')
--   local value, where = json.decode(s)   -- nil and a position when s is not one value
--   local text, why    = json.encode(v)   -- nil and a word when v cannot be JSON
--
-- `null` decodes to `nil`, the way the rest of this runtime says "nothing": a reader that
-- needs to tell the two apart cannot do so from the answer, and the API that answers this
-- does not need to.
--
-- What it will not do, on purpose: NaN, comments, trailing text after the value, and a
-- surrogate half by itself. A decoder that accepts what its reader cannot reason about is a
-- decoder the next reader cannot trust.
local M = {}

local function ws(s, pos)
  local n = #s
  while pos <= n do
    local c = s:byte(pos)
    if c == 0x20 or c == 0x09 or c == 0x0a or c == 0x0d then
      pos = pos + 1
    else
      break
    end
  end
  return pos
end

local function expect(s, pos, word)
  if s:sub(pos, pos + #word - 1) == word then
    return pos + #word
  end
  return nil
end

local ESC = {
  ['"'] = '"',
  ["\\"] = "\\",
  ["/"] = "/",
  ["b"] = "\b",
  ["f"] = "\f",
  ["n"] = "\n",
  ["r"] = "\r",
  ["t"] = "\t",
}

-- Every decoder answers `value, position, ok`, and only the top level folds the third into
-- the pair a caller can check: a `null` is a value, and a decoder that cannot tell it from
-- its own error cannot be used to read a map of them.
local function decode_string(s, pos)
  local out = {}
  local n = #s
  while true do
    if pos > n then
      return nil, pos, false
    end
    local c = s:sub(pos, pos)
    if c == '"' then
      return table.concat(out), pos + 1, true
    end
    if c == "\\" then
      local e = s:sub(pos + 1, pos + 1)
      local simple = ESC[e]
      if simple ~= nil then
        out[#out + 1] = simple
        pos = pos + 2
      elseif e == "u" then
        local hex = s:sub(pos + 2, pos + 5)
        if not hex:match("^%x%x%x%x$") then
          return nil, pos + 2, false
        end
        local cp = tonumber(hex, 16)
        if cp >= 0xD800 and cp <= 0xDBFF then
          local lowhex = s:sub(pos + 8, pos + 11)
          if s:sub(pos + 6, pos + 7) ~= "\\u" or not lowhex:match("^%x%x%x%x$") then
            return nil, pos + 2, false
          end
          local lo = tonumber(lowhex, 16)
          if lo < 0xDC00 or lo > 0xDFFF then
            return nil, pos + 2, false
          end
          cp = 0x10000 + (cp - 0xD800) * 1024 + (lo - 0xDC00)
          pos = pos + 12
        else
          if cp >= 0xDC00 and cp <= 0xDFFF then
            return nil, pos + 2, false
          end
          pos = pos + 6
        end
        out[#out + 1] = utf8.char(cp)
      else
        return nil, pos + 1, false
      end
    else
      out[#out + 1] = c
      pos = pos + 1
    end
  end
end

local function decode_number(s, pos)
  local num = s:match("^([%-%+]?%d+%.?%d*[eE]?[%-%+]?%d*)", pos)
  if num == nil then
    return nil, pos, false
  end
  local value = tonumber(num)
  if value == nil then
    return nil, pos, false
  end
  return value, pos + #num, true
end

local decode_value

local function decode_object(s, pos)
  local obj = {}
  pos = ws(s, pos)
  if pos > #s then
    return nil, pos, false
  end
  if s:sub(pos, pos) == "}" then
    return obj, pos + 1, true
  end
  while true do
    pos = ws(s, pos)
    if s:sub(pos, pos) ~= '"' then
      return nil, pos, false
    end
    local key, kp = decode_string(s, pos + 1)
    if key == nil then
      return nil, kp, false
    end
    pos = ws(s, kp)
    if s:sub(pos, pos) ~= ":" then
      return nil, pos, false
    end
    local value, vp, ok = decode_value(s, pos + 1)
    if not ok then
      return nil, vp, false
    end
    obj[key] = value
    pos = ws(s, vp)
    local c = s:sub(pos, pos)
    if c == "," then
      pos = pos + 1
    elseif c == "}" then
      return obj, pos + 1, true
    else
      return nil, pos, false
    end
  end
end

local function decode_array(s, pos)
  local arr = {}
  pos = ws(s, pos)
  if pos > #s then
    return nil, pos, false
  end
  if s:sub(pos, pos) == "]" then
    return arr, pos + 1, true
  end
  local i = 1
  while true do
    local value, vp, ok = decode_value(s, pos)
    if not ok then
      return nil, vp, false
    end
    arr[i] = value
    i = i + 1
    pos = ws(s, vp)
    local c = s:sub(pos, pos)
    if c == "," then
      pos = pos + 1
    elseif c == "]" then
      return arr, pos + 1, true
    else
      return nil, pos, false
    end
  end
end

decode_value = function(s, pos)
  pos = ws(s, pos)
  if pos > #s then
    return nil, pos, false
  end
  local c = s:sub(pos, pos)
  if c == "{" then
    return decode_object(s, pos + 1)
  end
  if c == "[" then
    return decode_array(s, pos + 1)
  end
  if c == '"' then
    return decode_string(s, pos + 1)
  end
  local p = expect(s, pos, "true")
  if p ~= nil then
    return true, p, true
  end
  p = expect(s, pos, "false")
  if p ~= nil then
    return false, p, true
  end
  p = expect(s, pos, "null")
  if p ~= nil then
    return nil, p, true
  end
  if c:match("[%-%+%d]") ~= nil then
    return decode_number(s, pos)
  end
  return nil, pos, false
end

function M.decode(s)
  if type(s) ~= "string" then
    return nil, 1
  end
  local value, pos, ok = decode_value(s, 1)
  if not ok then
    return nil, pos
  end
  pos = ws(s, pos)
  if pos <= #s then
    return nil, pos
  end
  return value, nil
end

local SHORT = {
  [0x08] = "\\b",
  [0x09] = "\\t",
  [0x0a] = "\\n",
  [0x0c] = "\\f",
  [0x0d] = "\\r",
}

local function encode_string(s)
  local out = { '"' }
  local i = 1
  local n = #s
  while i <= n do
    local byte = s:byte(i)
    if byte < 0x80 then
      if SHORT[byte] ~= nil then
        out[#out + 1] = SHORT[byte]
      elseif byte < 0x20 then
        out[#out + 1] = string.format("\\u%04x", byte)
      elseif byte == 0x22 or byte == 0x5c then
        out[#out + 1] = "\\" .. s:sub(i, i)
      else
        out[#out + 1] = s:sub(i, i)
      end
      i = i + 1
    else
      -- A UTF-8 sequence passes through whole: a decoder that splits one is a decoder the
      -- next reader cannot trust, and splitting here would be the only place it could happen.
      local ok = pcall(utf8.codepoint, s, i)
      local next = ok and utf8.offset(s, 2, i) or (i + 1)
      out[#out + 1] = s:sub(i, next - 1)
      i = next
    end
  end
  out[#out + 1] = '"'
  return table.concat(out)
end

local function is_array(t)
  local count = 0
  for k in pairs(t) do
    count = count + 1
    if type(k) ~= "number" or k ~= math.floor(k) or k < 1 then
      return false
    end
  end
  for i = 1, count do
    if t[i] == nil then
      return false
    end
  end
  return count > 0
end

local function encode_value(v, depth)
  if depth > 64 then
    return nil, "too deep"
  end
  local t = type(v)
  if t == "boolean" then
    return (v and "true") or "false"
  end
  if t == "number" then
    if v ~= v or v == math.huge or v == -math.huge then
      return nil, "not a number"
    end
    return string.format("%.17g", v)
  end
  if t == "string" then
    return encode_string(v)
  end
  if t ~= "table" then
    return nil, t .. " cannot be JSON"
  end
  if is_array(v) then
    local parts = {}
    for i = 1, #v do
      local piece, err = encode_value(v[i], depth + 1)
      if piece == nil then
        return nil, err
      end
      parts[i] = piece
    end
    return "[" .. table.concat(parts, ",") .. "]"
  end
  local values = {}
  for k, value in pairs(v) do
    if type(k) ~= "string" and type(k) ~= "number" then
      return nil, "a key cannot be " .. type(k)
    end
    local name = tostring(k)
    if values[name] == nil then
      values[name] = value
    end
  end
  local names = {}
  for name in pairs(values) do
    names[#names + 1] = name
  end
  table.sort(names)
  local parts = {}
  for _, name in ipairs(names) do
    local piece, err = encode_value(values[name], depth + 1)
    if piece == nil then
      return nil, err
    end
    parts[#parts + 1] = encode_string(name) .. ":" .. piece
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

function M.encode(v)
  local out, err = encode_value(v, 1)
  if out == nil then
    return nil, err
  end
  return out
end

return M
