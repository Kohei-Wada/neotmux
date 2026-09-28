//! `neotmux.manifest`, herdr's manifest engine, and the herdr manifests it runs. The region and
//! gate tests are herdr's own (src/detect/manifest/tests.rs), their inputs and expectations
//! carried over unchanged.

mod support;

use support::{built, feed, keys, logged, running, running_job};

/// Runs `body` on `J` and answers what it logged. The modules are loaded by init, as a plugin
/// loads them: loading every ruleset is more than a key's budget has room for.
fn run(body: &str) -> Vec<String> {
    let (_dir, sub) = built(&format!(
        "require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             {body}\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    ));
    keys(&sub, &["J"]);
    logged(&sub)
}

#[test]
fn a_regex_is_rusts() {
    let said = run(
        "neotmux.log.info(tostring(neotmux.regex.is_match('(?m)^b$', 'a\\nb\\nc')))\n\
         neotmux.log.info(tostring(neotmux.regex.is_match('\\\\Aa\\\\z', 'a\\nb')))\n\
         local ok, err = pcall(neotmux.regex.is_match, '(', 'x')\n\
         neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))",
    );
    assert_eq!(said[0], "true");
    assert_eq!(said[1], "false");
    assert!(
        said[2].starts_with("false ") && said[2].contains("unclosed group"),
        "{said:?}"
    );
}

/// herdr's `screen_regions_extract_structure_without_classifying_agent_state`, and its two
/// tests of which occurrence the counted regions take.
#[test]
fn the_regions_are_herdrs() {
    let said = run(r#"
local manifest = require('neotmux.manifest')
local cases = {
  { "old\n\nnew\n", "bottom_lines(2)", "\nnew\n" },
  { "before\n› input\nafter\n", "after_last_prompt_marker", "after\n" },
  { "before\n› input\nafter\n", "before_current_prompt_marker", "before\n" },
  { "before\n› input\nafter\n", "whole_recent_without_current_prompt_marker", "" },
  { "no marker\n", "whole_recent_without_current_prompt_marker", "no marker\n" },
  { "• old\n■ latest\n› input\n", "current_prompt_block_marker", "■ latest" },
  { "• old\n■ latest\n› input\n", "after_current_prompt_block_marker", "■ latest\n› input\n" },
  { "› old\n• new\n", "current_prompt_block_marker", "" },
  { "above\n\n───\nbody\n───\nfooter\n", "above_prompt_box", "above\n\n" },
  { "above\n\n───\nbody\n───\nfooter\n", "last_non_empty_above_prompt_box", "above" },
  { "above\n───\nbody\n───\nfooter\n", "prompt_box_body", "body\n" },
  { "above\n───\nbody\n───\nfooter\n", "after_last_horizontal_rule", "footer\n" },
  { "marker\nold\n\nmiddle\nmarker\nnew\n", "bottom_non_empty_lines(2)", "marker\nnew\n" },
  { "\nmarker\nold\n\nmiddle\nmarker\nnew\n", "top_non_empty_lines(2)", "\nmarker\nold\n" },
  { "a\nb\n", "top_non_empty_lines(01)", "" },
  { "a\nb\n", "no_such_region", "" },
}
for _, case in ipairs(cases) do
  local screen, spec, want = case[1], case[2], case[3]
  local view = { title = "", text = function() return screen end }
  local got = manifest.region(view, spec)
  if got ~= want then
    neotmux.log.info(string.format("%s: %q, not %q", spec, got, want))
  end
end
neotmux.log.info("done")
"#);
    assert_eq!(said, vec!["done".to_string()]);
}

/// herdr's `rule_semantics_apply_gates_priority_and_line_regex`: the highest priority that
/// matches wins, nested `all`/`any` gate it, `not` takes it away, `line_regex` is per line.
#[test]
fn the_gates_are_herdrs() {
    let said = run(r#"
local manifest = require('neotmux.manifest')
local detect = require('neotmux.detect')
local rules = manifest.ruleset({ id = "test", names = {}, rules = {
  { id = "low_contains", state = "idle", priority = 1, contains = { "match" } },
  { id = "high_nested_gates", state = "working", priority = 10, contains = { "match" },
    all = { { any = { { regex = { "w[io]n" } }, { contains = { "fallback" } } } } },
    ["not"] = { { contains = { "blocked" } } } },
  { id = "line_regex", state = "blocked", priority = 20, line_regex = { "^exact line$" } },
} })
for _, screen in ipairs({ "match win", "match win blocked", "before\nexact line\nafter", "nothing" }) do
  local view = { title = "", text = function() return screen end }
  local state, id = detect.judge(rules, view)
  neotmux.log.info(tostring(state) .. " " .. tostring(id))
end
"#);
    assert_eq!(
        said,
        vec![
            "working high_nested_gates",
            "idle low_contains",
            "blocked line_regex",
            "idle default_known_agent_idle_fallback",
        ]
    );
}

/// Every manifest taken from herdr loads, and every pattern in it compiles. A pattern is only
/// compiled when a screen gets as far as it, so this walks the gates rather than trusting the
/// screens the other tests happen to show.
#[test]
fn every_herdr_manifest_compiles() {
    let said = run(r#"
local detect = require('neotmux.detect')
local count, patterns = 0, 0
local function walk(g, where)
  for _, field in ipairs({ "regex", "line_regex" }) do
    for _, p in ipairs(g[field] or {}) do
      patterns = patterns + 1
      local ok, err = pcall(neotmux.regex.is_match, p, "")
      if not ok then neotmux.log.info(where .. ": " .. tostring(err)) end
    end
  end
  for _, field in ipairs({ "all", "any", "not" }) do
    for _, nested in ipairs(g[field] or {}) do walk(nested, where) end
  end
end
for name, rules in pairs(detect.agents) do
  if name ~= "claude" then
    count = count + 1
    for _, rule in ipairs(rules) do
      if rule.gate then walk(rule.gate, name .. "/" .. rule.id) end
    end
  end
end
neotmux.log.info(count .. " manifests " .. patterns .. " patterns")
"#);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("21 manifests"), "{said:?}");
}

/// End to end: a pane running `codex` is Codex's, not Claude Code's — the Claude ruleset claims
/// every program that is not a shell, and would otherwise run first — and a live turn on it is
/// `working` by herdr's `screen_working_fallback`.
#[test]
fn a_codex_pane_is_read_by_herdrs_codex_manifest() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info(tostring(state.of(1)) .. ' ' .. tostring(detect.agent(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    running(&sub, "codex");
    feed(
        &sub,
        "• Working (12s • esc to interrupt)\r\n\r\n› ".as_bytes(),
    );
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub).last().map(String::as_str),
        Some("working codex")
    );
}

/// A program herdr has a manifest for and whose screen none of its rules recognise is idle —
/// the agent is up and not visibly doing anything — except Codex, which herdr calls ambiguous.
#[test]
fn an_unrecognised_screen_is_idle_for_an_agent_herdr_knows_and_not_for_codex() {
    let (_dir, sub) = built(
        "local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info(tostring(detect.of(1)) .. ' ' .. tostring(detect.agent(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    running(&sub, "gemini");
    feed(&sub, b"hello");
    keys(&sub, &["J"]);
    assert_eq!(logged(&sub).last().map(String::as_str), Some("idle gemini"));

    running(&sub, "codex");
    keys(&sub, &["J"]);
    assert_eq!(logged(&sub).last().map(String::as_str), Some("nil nil"));
}

/// Codex as npm installs it: the pane's foreground process is `node`, under the name Node gives
/// its main thread, and it is herdr's reading of the job that says Codex. The Claude ruleset, which claims `node` by
/// elimination, steps back from a pane the host has named.
#[test]
fn a_codex_run_by_node_is_codexs_by_what_the_host_identified() {
    let (_dir, sub) = built(
        "local state = require('neotmux.state')\n\
         local detect = require('neotmux.detect')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info(tostring(state.of(1)) .. ' ' .. tostring(detect.agent(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
    );
    running_job(
        &sub,
        "node-MainThread",
        &[
            (
                100,
                "node-MainThread",
                &["node", "/home/user/.npm-global/bin/codex"],
            ),
            (
                101,
                "codex",
                &["/home/user/.npm-global/lib/node_modules/@openai/codex/vendor/codex"],
            ),
        ],
    );
    feed(
        &sub,
        "• Working (12s • esc to interrupt)\r\n\r\n› ".as_bytes(),
    );
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub).last().map(String::as_str),
        Some("working codex")
    );
}
