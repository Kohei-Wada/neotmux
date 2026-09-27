-- The middle layer of `neotmux.state`: what a pane no ruleset in `neotmux.detect.agents`
-- answers is asked of the Jev model, TypeSafe's decision API, and the answer it comes back
-- with is what the next layer down reads.
--
--   local jev = require('neotmux.jev')
--   jev.of(pane)  --> 'working' | 'blocked' | 'idle', or nil when there is no answer
--
-- The layer has no knowledge of any program: it sends the pane's own text and reads back a
-- decision about it, so it answers for a pane running an agent nobody taught neotmux, a
-- pager waiting on a key, or a shell asking a question in words the pattern list below never
-- listed. That is the trade it makes for the knowledge: the pane's text leaves the machine,
-- to `M.endpoint`, while this is on. A session with no key set, with a key the API refuses,
-- or on a host that cannot start a process stops asking and says so once.
--
-- A decision is about the pane at one revision, and is stale the moment the pane is fed
-- again: `of` answers it only while the pane is still showing the screen it was judged on.
-- That is the same cost control as `neotmux.detect` — the screen is read once, for a
-- revision, and judged once, when it has been quiet for `M.settle_ms` — and it is what keeps
-- a spinner's blink from being asked as a question.
--
-- The asking itself is one `neotmux.job`: one `sh`, one `curl`, and the key expanding from
-- the environment in that shell rather than crossing this VM, which cannot read the
-- environment and should not be able to. `TYPESAFE_API_KEY` must be in the server's
-- environment; it never appears in the Lua and never leaves the machine except as a header.
local g = neotmux.group("neotmux.jev")

local json = require("neotmux.json")

local M = {}

-- The three words the pane is asked to answer, in the model's own words: the criteria are
-- the whole of what it is told, and a pane it has never seen is judged against them alone.
M.criteria = {
  working = {
    what = "The program is doing work on its own right now, and more output will come"
      .. " without the user typing anything",
    not_for = "A turn or command that has already finished, even if the screen still shows"
      .. " how long it took",
    examples = {
      "\u{273b} Herding\u{2026} (12s \u{00b7} esc to interrupt)",
      "Compiling neotmux-lua v0.1.0",
      "Building [=======>     ] 187/245",
    },
  },
  blocked = {
    what = "The program has asked the user one specific question and cannot go on until it"
      .. " is answered",
    not_for = "An empty prompt or input box waiting for a new instruction; an editor, pager or"
      .. " file listing whose text merely contains question words or numbered options",
    examples = {
      "Do you want to proceed? \u{276f} 1. Yes  2. No",
      "[sudo] password for user:",
      "Do you want to continue? [Y/n]",
    },
  },
  idle = {
    what = "Nothing is running and nothing specific is being asked: the program is waiting for"
      .. " whatever the user wants next",
    not_for = "A question, a password request, or a menu of options addressed to the user",
    examples = {
      "$",
      "an agent's empty \u{276f} input box after it finished its turn",
      "an editor or pager showing a file",
    },
  },
}

-- One question per pane; `%s` is the state field the pane's text lives under. The field
-- name is all the model is told about which pane is which, so it must stay in the question.
M.question = "Judging only the state field `%s`"
  .. " (a terminal pane: the program it runs and its last lines),"
  .. " what is it doing right now?"

M.endpoint = "https://api.typesafe.ai/v1/systemone"
M.model = "jev-latest"
-- Off until a config turns it on: turning it on is what sends a pane's text off the machine.
M.enabled = false

-- Set when the layer has stopped asking: no key, a refused key, or a host that cannot start
-- a process. Clearing it — with the reason fixed — is the "try again".
M.disabled = false

-- How long a pane must have been quiet before it is worth asking, in milliseconds against
-- `neotmux.now()`: a pane mid-spinner is not a question, it is a coin in the air, and a
-- decision about it is noise.
M.settle_ms = 1500
-- How many of the pane's last visible lines go over the wire. A question is at the bottom
-- of the screen, and the lines above it are what it is about.
M.lines_per_pane = 20
-- Below this the answer is not believed, and the pane falls through to whatever is under
-- this layer. The API's own word on it: confidence is how peaked the distribution is, not
-- how often it is right — a threshold to tune on your own screens, not a guarantee.
M.min_confidence = 0.6
-- A request over this many panes would stop being a batch and become a bulk; the rest wait
-- for the next one.
M.max_panes = 24

-- What each pane is being held at, and against which screen. `quiet` is the settle's own
-- bookkeeping: the last revision this module saw for a pane, and when it saw it.
local verdicts = {}
local quiet = {}
-- The in-flight request, and the revision each of its panes was built from: an answer that
-- comes back for a screen the pane has since left is about a past, and is dropped.
local in_flight = false
local sent = {}
local next_ok_at = 0
local backoff_ms = 500
local said = {}

-- One line per reason per session: the notification is for the user, and the second copy of
-- it is the user's way of saying the first one was not heard.
local function say_once(reason, level, text)
  if not said[reason] then
    said[reason] = true
    neotmux.notify(level, text)
  end
end

-- The pane's text for the wire: what it is running on top, and its last non-blank lines
-- under it. Read only for a pane that is actually sent, which on a settled session is none.
local function snapshot(pane)
  local lines = neotmux.pane.content(pane, { lines = M.lines_per_pane })
  if lines == nil then
    return nil
  end
  local full = {}
  for _, line in ipairs(lines) do
    if line:match("%S") then
      full[#full + 1] = line
    end
  end
  local head = "pane " .. tostring(pane)
  local ok, info = pcall(neotmux.pane.info, pane)
  if ok and info and info.command ~= nil then
    head = head .. " (running: " .. tostring(info.command) .. ")"
  end
  return head .. "\n" .. table.concat(full, "\n")
end

-- The request, or nothing and a word. A pane that goes away between the ask and the build
-- loses its question rather than taking the batch down with it: the others still get theirs
-- on the next settle.
local function build(panes)
  local state = {}
  local questions = {}
  for _, pane in ipairs(panes) do
    local id = "p" .. tostring(pane)
    local text = snapshot(pane)
    if text == nil then
      return nil, "pane " .. tostring(pane) .. " went away before it could be read"
    end
    state[id] = text
    questions[id] = {
      type = "choice",
      instructions = string.format(M.question, id),
      criteria = M.criteria,
    }
  end
  local body, err = json.encode({ state = state, model = M.model, questions = questions })
  if body == nil then
    return nil, err
  end
  return body
end

local function settle_response(panes, fresh, stdout)
  -- The status is the last line of the job's output, and everything above it the body.
  local body, http = stdout:match("^(.*)\n(%d%d%d)$")
  if body == nil then
    body = stdout
  end
  local status = tonumber(http)
  if status == 401 then
    M.disabled = true
    neotmux.notify(
      "error",
      "neotmux.jev: the key was refused (401), so no pane is asked; check TYPESAFE_API_KEY,"
        .. " or set neotmux.jev.enabled = false to keep this quiet"
    )
    return
  end
  if status == 429 or status == 529 then
    backoff_ms = math.min(backoff_ms * 2, 60000)
    next_ok_at = neotmux.now() + backoff_ms
    say_once(
      "throttle",
      "info",
      "neotmux.jev: the API is throttling ("
        .. tostring(http)
        .. "); waiting "
        .. backoff_ms
        .. " ms"
    )
    return
  end
  if status == nil or status < 200 or status >= 300 then
    backoff_ms = math.min(backoff_ms * 2, 60000)
    next_ok_at = neotmux.now() + backoff_ms
    say_once(
      "http",
      "warn",
      "neotmux.jev: the API answered " .. tostring(http) .. " (" .. body:sub(1, 80) .. ")"
    )
    return
  end
  backoff_ms = 500
  local response, where = json.decode(body)
  if response == nil then
    say_once(
      "decode",
      "error",
      "neotmux.jev: the answer is not JSON (near position " .. tostring(where) .. ")"
    )
    return
  end
  local answers = response.answers or {}
  for _, pane in ipairs(panes) do
    local answer = answers["p" .. tostring(pane)]
    local word = answer and answer.choice
    if word ~= nil and M.criteria[word] ~= nil and answer.confidence >= M.min_confidence then
      if neotmux.pane.revision(pane) == fresh[pane] then
        verdicts[pane] = { state = word, revision = fresh[pane] }
      end
      -- The pane has moved since: the decision is about a screen nobody is reading any
      -- more, and a pane that has moved is what the next request is for.
    end
  end
end

local function on_exit(panes, code, stdout, stderr)
  in_flight = false
  local fresh = {}
  for pane, revision in pairs(sent) do
    fresh[pane] = revision
  end
  sent = {}
  local now = neotmux.now()
  if code == nil then
    -- The host ended it: killed, session over, output over the limit. Nothing to read, and
    -- the panes may still want their decision, so the wait is a beat rather than a wall.
    next_ok_at = now + backoff_ms
    return
  end
  if code == 71 then
    M.disabled = true
    neotmux.notify(
      "error",
      "neotmux.jev: TYPESAFE_API_KEY is not set, so no pane is asked;"
        .. " start the server with the key,"
        .. " or set neotmux.jev.enabled = false to keep this quiet"
    )
    return
  end
  if code ~= 0 then
    say_once(
      "curl",
      "warn",
      "neotmux.jev: the request did not go out ("
        .. tostring(stderr):gsub("[%s]+", " "):sub(1, 80)
        .. ") — is curl on this machine?"
    )
    backoff_ms = math.min(backoff_ms * 2, 60000)
    next_ok_at = now + backoff_ms
    return
  end
  settle_response(panes, fresh, stdout)
end

-- One `sh`, one `curl`, the body as `$1` and the endpoint as `$2`: neither is spliced into
-- the shell line, and the key is the one thing that is, because it must not cross the Lua.
local SCRIPT = 'test -n "${TYPESAFE_API_KEY:-}" ||'
  .. ' { echo "neotmux.jev: TYPESAFE_API_KEY is not set" >&2; exit 71; };'
  .. " exec curl -sS --max-time 15 -X POST"
  .. ' -H "Authorization: Bearer ${TYPESAFE_API_KEY}"'
  .. ' -H "Content-Type: application/json"'
  .. ' -w \'\\n%{http_code}\' "$2" --data-raw "$1"'

local function ask(panes)
  local body, err = build(panes)
  if body == nil then
    say_once("build", "error", "neotmux.jev: no request built (" .. tostring(err) .. ")")
    return
  end
  for _, pane in ipairs(panes) do
    sent[pane] = neotmux.pane.revision(pane)
  end
  in_flight = true
  local job, why = g.job.start({ "sh", "-c", SCRIPT, "_", body, M.endpoint }, {
    on_exit = function(code, stdout, stderr)
      on_exit(panes, code, stdout, stderr)
    end,
  })
  if job == nil then
    in_flight = false
    sent = {}
    why = type(why) == "string" and why or "refused"
    if why:find("cannot start processes") ~= nil then
      -- A host with no way to run a program is not a host this layer recovers on.
      M.disabled = true
      neotmux.notify(
        "error",
        "neotmux.jev: this host cannot start processes, so no pane is asked;"
          .. " set neotmux.jev.enabled = false to keep this quiet"
      )
    elseif why:find("^too many jobs") ~= nil or why == "the session is ending" then
      -- Back-pressure, not a fault: the same request goes out again after a beat.
      next_ok_at = neotmux.now() + backoff_ms
    else
      say_once("start", "error", "neotmux.jev: could not start the request (" .. why .. ")")
      next_ok_at = neotmux.now() + backoff_ms
    end
  end
end

-- What this pane is doing right now, if a decision stands for the screen it is showing.
-- `neotmux.state` asks this only for a pane the rules did not answer, and the read costs
-- nothing either way.
function M.of(pane)
  local it = verdicts[pane]
  if it == nil then
    return nil
  end
  local revision = neotmux.pane.revision(pane)
  if revision == nil then
    verdicts[pane] = nil
    return nil
  end
  if it.revision ~= revision then
    return nil
  end
  return it.state
end

-- Start over on this pane: the decision and the settle's memory of it go together, or the
-- next pane given the old one would be answered off a screen it never showed.
function M.forget(pane)
  verdicts[pane] = nil
  quiet[pane] = nil
  sent[pane] = nil
end

g.on_event("pane_closed", function(event)
  M.forget(event.pane)
end)

g.on_tick(function()
  if not M.enabled or M.disabled or in_flight then
    return
  end
  local now = neotmux.now()
  if now < next_ok_at then
    return
  end
  local candidates = {}
  for _, window in ipairs(neotmux.window.list()) do
    for _, pane in ipairs(neotmux.window.panes(window) or {}) do
      local revision = neotmux.pane.revision(pane)
      if revision == nil then
        M.forget(pane)
      else
        local it = quiet[pane]
        if it == nil or it.revision ~= revision then
          quiet[pane] = { revision = revision, at = now }
        elseif now - it.at >= M.settle_ms then
          local held = verdicts[pane]
          if held == nil or held.revision ~= revision then
            candidates[#candidates + 1] = pane
          end
        end
      end
    end
  end
  if #candidates > 0 then
    if #candidates > M.max_panes then
      candidates = { table.unpack(candidates, 1, M.max_panes) }
    end
    ask(candidates)
  end
end)

return M
