-- Which agent a pane is running, the way herdr decides it: by the names of the processes in the
-- pane's foreground job and, for a runtime or a shell that is only carrying one, by the script
-- it was handed. `node .../bin/codex` is Codex, where the pane's `command` says `node`.
--
--   require('neotmux.agent').of(neotmux.pane.info(pane))  --> 'codex', or nil
--
-- A port of herdr's `identify_agent_in_job` and what it calls, src/detect/mod.rs
-- (herdrdev/herdr, Apache-2.0; see NOTICE). The agents are herdr's labels. Two differences, both
-- about what a Lua VM can see: a path is not resolved through its symlinks, since that takes the
-- filesystem, and lower-casing is ASCII. And one of neotmux's own: a `comm` of `node-MainThread`,
-- Node's name for its main thread, is read as the `node` it is — without that, an agent that is
-- Node and nothing else, Gemini as npm installs it, is never found.
local M = {}

-- herdr's `lookup_agent`: the names each agent is known by, to its label.
local LOOKUP = {
  pi = "pi",
  claude = "claude",
  ["claude-code"] = "claude",
  codex = "codex",
  gemini = "gemini",
  cursor = "cursor",
  ["cursor-agent"] = "cursor",
  devin = "devin",
  ["devin-cli"] = "devin",
  ["devin cli"] = "devin",
  agy = "agy",
  antigravity = "agy",
  ["antigravity-cli"] = "agy",
  cline = "cline",
  [".cline"] = "cline",
  omp = "omp",
  mastracode = "mastracode",
  ["mastra-code"] = "mastracode",
  ["mastra code"] = "mastracode",
  opencode = "opencode",
  opencode2 = "opencode",
  ["open-code"] = "opencode",
  copilot = "copilot",
  ["github-copilot"] = "copilot",
  ghcs = "copilot",
  kimi = "kimi",
  ["kimi-code"] = "kimi",
  ["kimi code"] = "kimi",
  kiro = "kiro",
  ["kiro-cli"] = "kiro",
  droid = "droid",
  amp = "amp",
  ["amp-local"] = "amp",
  grok = "grok",
  ["grok-build"] = "grok",
  hermes = "hermes",
  ["hermes-agent"] = "hermes",
  kilo = "kilo",
  ["kilo-code"] = "kilo",
  ["kilo code"] = "kilo",
  qodercli = "qodercli",
  qoderclicn = "qodercli",
  qoder = "qodercli",
  qodercn = "qodercli",
  qwen = "qwen",
  ["qwen-code"] = "qwen",
  ["qwen code"] = "qwen",
  letta = "letta",
  ["letta-code"] = "letta",
  ["letta code"] = "letta",
  maki = "maki",
  muse = "muse",
  ["muse-code"] = "muse",
  ["muse-cli"] = "muse",
}

local function trim(text)
  return (text:match("^%s*(.-)%s*$"))
end

local function starts(text, prefix)
  return text:sub(1, #prefix) == prefix
end

local function ends(text, suffix)
  return suffix == "" or text:sub(-#suffix) == suffix
end

local function components(path)
  local out = {}
  for part in path:gmatch("[^/\\]+") do
    out[#out + 1] = part
  end
  return out
end

local function path_basename(path)
  local parts = components(path)
  return parts[#parts] or path
end

local function normalized_agent_lookup_name(name)
  name = trim(name):lower()
  for _, suffix in ipairs({ ".exe", ".cmd", ".bat", ".ps1", ".js" }) do
    if ends(name, suffix) then
      return name:sub(1, #name - #suffix)
    end
  end
  return name
end

local function is_muse_versioned_binary(name)
  local rest = path_basename(name):match("^muse%-bin%-(.*)$")
  return rest ~= nil and rest:match("^%d") ~= nil
end

local function lookup_agent(name)
  name = path_basename(name)
  local label = LOOKUP[name]
  if label ~= nil then
    return label
  end
  if is_muse_versioned_binary(name) then
    return "muse"
  end
  return nil
end

-- herdr's `parse_agent_label` and `identify_agent`, which come to the same lookup.
function M.identify_name(name)
  return lookup_agent(normalized_agent_lookup_name(name))
end

local identify_name = M.identify_name

local function is_python_runtime(name)
  if name == "python" then
    return true
  end
  local version = name:match("^python(.+)$")
  if version == nil then
    return false
  end
  for part in (version .. "."):gmatch("(.-)%.") do
    if part == "" or part:match("^%d+$") == nil then
      return false
    end
  end
  return true
end

local GENERIC = {
  sh = true,
  bash = true,
  zsh = true,
  fish = true,
  tmux = true,
  node = true,
  bun = true,
  cmd = true,
  powershell = true,
  pwsh = true,
}

local function is_generic_runtime_or_shell(name)
  name = normalized_agent_lookup_name(path_basename(name))
  return is_python_runtime(name) or GENERIC[name] == true
end

local TAKES_VALUE = {
  ["-r"] = true,
  ["--require"] = true,
  ["--loader"] = true,
  ["--import"] = true,
  ["--experimental-loader"] = true,
  ["--inspect-port"] = true,
  ["-W"] = true,
  ["-X"] = true,
  ["-S"] = true,
  ["-L"] = true,
  ["-o"] = true,
}

local function short_flag_payload(arg, flag)
  return starts(flag, "-") and not starts(flag, "--") and starts(arg, flag) and #arg > #flag
end

local function long_flag_value(arg, flag)
  return starts(flag, "--") and starts(arg, flag .. "=")
end

local function flag_matches(arg, flags)
  for _, flag in ipairs(flags) do
    if arg == flag or short_flag_payload(arg, flag) or long_flag_value(arg, flag) then
      return true
    end
  end
  return false
end

local function ends_with_components(parts, suffix)
  if #parts < #suffix then
    return false
  end
  local offset = #parts - #suffix
  for i, expected in ipairs(suffix) do
    if parts[offset + i]:lower() ~= expected:lower() then
      return false
    end
  end
  return true
end

local function has_window(parts, window)
  for start = 1, #parts - #window + 1 do
    local all = true
    for i, expected in ipairs(window) do
      if parts[start + i - 1] ~= expected then
        all = false
        break
      end
    end
    if all then
      return true
    end
  end
  return false
end

local function agent_name_from_known_package_path(path)
  local raw = components(path)
  if
    ends_with_components(
      raw,
      { "node_modules", "@earendil-works", "pi-coding-agent", "dist", "cli.js" }
    )
    or ends_with_components(
      raw,
      { "node_modules", "@earendil-works", "pi-coding-agent", "dist", "bundle", "cli.js" }
    )
  then
    return "pi"
  end
  if
    ends_with_components(raw, { "node_modules", "@oh-my-pi", "pi-coding-agent", "dist", "cli.js" })
  then
    return "omp"
  end
  if
    ends_with_components(raw, { "node_modules", "@moonshot-ai", "kimi-code", "dist", "main.mjs" })
  then
    return "kimi"
  end
  local parts = {}
  for i, part in ipairs(raw) do
    parts[i] = normalized_agent_lookup_name(part)
  end
  if has_window(parts, { "node_modules", "@qwen-code", "qwen-code", "dist", "index" }) then
    return "qwen"
  end
  if has_window(parts, { "node_modules", "mastracode", "dist", "cli" }) then
    return "mastracode"
  end
  if has_window(parts, { "node_modules", "@letta-ai", "letta-code", "letta" }) then
    return "letta"
  end
  return nil
end

local function agent_name_from_path_token(token)
  local trimmed = token:match("^[\"']*(.-)[\"']*$")
  if trimmed == "" or starts(trimmed, "-") then
    return nil
  end
  return identify_name(path_basename(trimmed)) or agent_name_from_known_package_path(trimmed)
end

local function script_arg_agent_name(argv, eval_flags, module_flags)
  local i = 2
  while argv[i] ~= nil do
    local arg = argv[i]
    if arg == "--" then
      return argv[i + 1] and agent_name_from_path_token(argv[i + 1]) or nil
    end
    if flag_matches(arg, eval_flags) or flag_matches(arg, module_flags) then
      return nil
    end
    if starts(arg, "-") then
      i = i + (TAKES_VALUE[arg] and 2 or 1)
    else
      return agent_name_from_path_token(arg)
    end
  end
  return nil
end

local function command_text_token(input)
  input = input:match("^%s*(.*)$")
  if input == "" then
    return nil
  end
  local first = input:sub(1, 1)
  if first == '"' or first == "'" then
    local close = input:find(first, 2, true)
    if close ~= nil then
      return input:sub(2, close - 1), input:sub(close + 1)
    end
    return input:sub(2), ""
  end
  local stop = input:find("%s") or (#input + 1)
  return input:sub(1, stop - 1), input:sub(stop)
end

local function command_text_agent_name(command)
  local rest = command
  while true do
    local token, nxt = command_text_token(rest)
    if token == nil then
      return nil
    end
    token = trim(token)
    local lower = token:lower()
    if lower == "&" or lower == "." or lower == "call" then
      rest = nxt
    else
      return agent_name_from_path_token(token)
    end
  end
end

-- `cmd /d /s /c "codex --model x"`: whatever follows `/c` or `/k` is the command. herdr names
-- the other switches only to step over them, which is what any other word gets too.
local function windows_cmd_arg_agent_name(argv)
  local i = 2
  while argv[i] ~= nil do
    local flag = argv[i]:gsub('^"+', ""):gsub('"+$', ""):lower()
    if flag == "/c" or flag == "/k" then
      return argv[i + 1] and command_text_agent_name(argv[i + 1]) or nil
    end
    i = i + 1
  end
  return nil
end

local PS_VALUE_OPTIONS = {
  ["-configurationname"] = true,
  ["-executionpolicy"] = true,
  ["-outputformat"] = true,
  ["-psconsolefile"] = true,
  ["-version"] = true,
  ["-windowstyle"] = true,
  ["-workingdirectory"] = true,
}

local function powershell_arg_agent_name(argv)
  local i = 2
  while argv[i] ~= nil do
    local arg = argv[i]
    local flag = arg:gsub('^"+', ""):gsub('"+$', ""):lower()
    if flag == "-file" or flag == "-f" or flag == "/file" then
      return argv[i + 1] and agent_name_from_path_token(argv[i + 1]) or nil
    elseif flag == "-command" or flag == "-c" or flag == "/command" or flag == "/c" then
      return argv[i + 1] and command_text_agent_name(argv[i + 1]) or nil
    elseif
      flag == "-encodedcommand"
      or flag == "-enc"
      or flag == "/encodedcommand"
      or flag == "/enc"
    then
      return nil
    elseif PS_VALUE_OPTIONS[flag] then
      i = i + 2
    elseif starts(flag, "-") or starts(flag, "/") then
      i = i + 1
    else
      return agent_name_from_path_token(arg)
    end
  end
  return nil
end

local function path_parent_and_basename(path)
  local split = nil
  for i = #path, 1, -1 do
    local c = path:sub(i, i)
    if c == "/" or c == "\\" then
      split = i
      break
    end
  end
  if split == nil then
    return nil
  end
  local parent = path:sub(1, split - 1):gsub("[/\\]+$", "")
  local basename = path:sub(split + 1)
  if parent == "" or basename == "" then
    return nil
  end
  return parent, basename
end

local function cursor_agent_name_from_bundled_node_argv(argv)
  if argv[1] == nil or argv[2] == nil then
    return nil
  end
  local runtime_parent, runtime_name = path_parent_and_basename(argv[1])
  local script_parent, script_name = path_parent_and_basename(argv[2])
  if runtime_parent == nil or script_parent == nil then
    return nil
  end
  if
    runtime_name:lower() ~= "node.exe"
    or script_name:lower() ~= "index.js"
    or runtime_parent:lower() ~= script_parent:lower()
  then
    return nil
  end
  local parts = components(runtime_parent)
  local version, versions, package = parts[#parts], parts[#parts - 1], parts[#parts - 2]
  if version == nil or versions == nil or package == nil then
    return nil
  end
  if
    package:lower() == "cursor-agent"
    and versions:lower() == "versions"
    and trim(version) ~= ""
  then
    return "cursor"
  end
  return nil
end

local function wrapped_agent_name_from_runtime_argv(runtime, argv)
  if argv == nil then
    return nil
  end
  local name = normalized_agent_lookup_name(path_basename(runtime))
  if name == "node" then
    return cursor_agent_name_from_bundled_node_argv(argv)
      or script_arg_agent_name(argv, { "-e", "--eval", "-p", "--print" }, {})
  elseif name == "bun" then
    return script_arg_agent_name(argv, { "-e", "--eval", "-p", "--print" }, {})
  elseif is_python_runtime(name) then
    return script_arg_agent_name(argv, { "-c" }, { "-m" })
  elseif name == "sh" or name == "bash" or name == "zsh" or name == "fish" then
    return script_arg_agent_name(argv, { "-c" }, {})
  elseif name == "cmd" then
    return windows_cmd_arg_agent_name(argv)
  elseif name == "powershell" or name == "pwsh" then
    return powershell_arg_agent_name(argv)
  end
  return nil
end

local function argv0_agent_name(argv)
  return argv and argv[1] and agent_name_from_path_token(argv[1]) or nil
end

local function effective_name(name)
  if name:lower() == "node-mainthread" then
    return "node"
  end
  return name
end

local function normalized_process_name(process)
  local effective = effective_name(process.name)
  local lower_effective = effective:lower()

  if is_generic_runtime_or_shell(lower_effective) then
    local wrapped = wrapped_agent_name_from_runtime_argv(lower_effective, process.argv)
    if wrapped ~= nil then
      return wrapped
    end
  end

  if identify_name(effective) ~= nil then
    return effective
  end

  local runtime = process.argv and process.argv[1]
  if runtime ~= nil then
    local runtime_name = normalized_agent_lookup_name(path_basename(runtime))
    if runtime_name == "node" or runtime_name == "bun" then
      local wrapped = wrapped_agent_name_from_runtime_argv(runtime, process.argv)
      local agent = wrapped and identify_name(wrapped)
      if agent == "qwen" or agent == "cline" or agent == "letta" then
        return wrapped
      end
    end
  end

  -- herdr tries the argv's first word, then the cmdline's; with the cmdline the argv joined by
  -- spaces, both are the same word.
  return argv0_agent_name(process.argv) or effective
end

local LETTA_NONINTERACTIVE = {
  ["-p"] = true,
  ["--print"] = true,
  ["--prompt"] = true,
  ["--json"] = true,
  ["--stream-json"] = true,
  ["--run"] = true,
  ["--disable-memory-guard"] = true,
  ["--output-format"] = true,
  ["--input-format"] = true,
  ["--include-partial-messages"] = true,
  ["--from-agent"] = true,
  ["--environment"] = true,
  ["--env"] = true,
  ["--pre-load-skills"] = true,
  ["--tags"] = true,
  ["--ephemeral"] = true,
  ["--stateless"] = true,
  ["--max-turns"] = true,
  ["--memfs-startup"] = true,
  ["-h"] = true,
  ["--help"] = true,
  ["-v"] = true,
  ["--version"] = true,
  ["--info"] = true,
  ["--update"] = true,
  ["--upgrade"] = true,
}

local function letta_entrypoint_index(argv)
  local function is_letta(arg)
    return agent_name_from_path_token(arg) == "letta"
  end
  if argv[1] ~= nil and is_letta(argv[1]) then
    return 1
  end
  if argv[1] == nil then
    return nil
  end
  local runtime = normalized_agent_lookup_name(path_basename(argv[1]))
  if runtime ~= "node" and runtime ~= "bun" then
    return nil
  end
  local i = 2
  while argv[i] ~= nil do
    local arg = argv[i]
    if arg == "--" then
      if argv[i + 1] ~= nil and is_letta(argv[i + 1]) then
        return i + 1
      end
      return nil
    end
    if flag_matches(arg, { "-e", "--eval", "-p", "--print" }) then
      return nil
    end
    if starts(arg, "-") then
      i = i + (TAKES_VALUE[arg] and 2 or 1)
    else
      return is_letta(arg) and i or nil
    end
  end
  return nil
end

local function is_interactive_letta_process(process)
  local argv = process.argv
  if argv == nil then
    return true
  end
  local from = (letta_entrypoint_index(argv) or 0) + 1
  local args = {}
  for i = from, #argv do
    args[#args + 1] = argv[i]
  end
  for _, arg in ipairs(args) do
    local option = arg:match("^([^=]*)=") or arg
    if LETTA_NONINTERACTIVE[option] then
      return false
    end
  end
  local i = 1
  while args[i] ~= nil do
    local arg = args[i]
    if arg == "--backend" then
      i = i + 2
    elseif starts(arg, "--backend=") then
      i = i + 1
    else
      return starts(arg, "-")
    end
  end
  return true
end

local function process_priority(process, normalized_name)
  local lower = normalized_name:lower()
  if lower ~= process.name:lower() then
    return 3
  end
  if not is_generic_runtime_or_shell(lower) then
    return 2
  end
  return 1
end

-- herdr's `identify_agent_in_job`: the group's leader if it is an agent, otherwise the member
-- most plainly one — found under a name it was not called, then one that is not a runtime or a
-- shell, then anything.
function M.identify_job(job)
  for _, process in ipairs(job.processes) do
    if process.pid == job.group then
      local agent = identify_name(normalized_process_name(process))
      if agent ~= nil and (agent ~= "letta" or is_interactive_letta_process(process)) then
        return agent
      end
      break
    end
  end
  local best_score, best = nil, nil
  for _, process in ipairs(job.processes) do
    local candidate = normalized_process_name(process)
    local agent = identify_name(candidate)
    if agent ~= nil and (agent ~= "letta" or is_interactive_letta_process(process)) then
      local score = process_priority(process, candidate)
      if best_score == nil or score > best_score then
        best_score, best = score, agent
      end
    end
  end
  return best
end

-- The agent a `pane.info` describes, or nil: none there, or no job the host could read.
function M.of(info)
  if info == nil or info.job == nil then
    return nil
  end
  return M.identify_job(info.job)
end

return M
