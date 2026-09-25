-- The Lua every layer runs: the shipped runtime, the examples, and the plugins the tests load.
std = "lua54"
-- The host's API. Plugins call into it and never assign to it, except the options table, which is
-- there to be written: `neotmux.opt.pane_gutter = 1`.
read_globals = {
  neotmux = {
    other_fields = true,
    fields = { opt = { read_only = false, other_fields = true } },
  },
}
max_line_length = 100
-- `handle:close()` and the rest are methods a caller reaches with `:`, whether or not they use self.
self = false
-- `local ok, err = pcall(...)` again inside a block that already has one is the idiom, not a slip.
ignore = { "421/ok", "421/err" }

-- These loop forever on purpose: they are the budget's test subjects.
files["neotmux/tests/fixtures/lua/plugin/spinner.lua"] = { ignore = { "511" } }
files["neotmux/tests/fixtures/lua/plugin/coroutine_spinner.lua"] = { ignore = { "511" } }
