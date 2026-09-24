//! `neotmux.list`: a keyboard-driven list in a float, driven without a terminal — keys in,
//! draw commands and logged calls out.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::{DrawCmd, Epoch, Geometry, PaneId, Rect, Size, WindowId, geometry::Window};

mod support;

use support::Built;

const SIZE: Size = Size { rows: 10, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

/// A layer of the test's own in front of the shipped one, so `neotmux.list` is the real file.
fn built(init: &str) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            windows: vec![Window {
                id: WindowId::from(1),
                panes: vec![pane()],
                name: None,
            }],
            window: Some(WindowId::from(1)),
        },
    );
    (dir, Built::new(sub, terminals))
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

fn frame(sub: &LuaSubsystem) -> Vec<DrawCmd> {
    sub.dispatch_tick(support::one());
    sub.frame_output()
}

fn texts(cmds: &[DrawCmd]) -> Vec<String> {
    cmds.iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(Some(support::one()))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A list of three, logging what `done` and `cancel` are handed so a test can read the
/// choice back the way a person would: through the door the module hands the caller, not
/// through a peek at its state.
const THREE: &str = "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      title = 'fruit',
      items = function() return { 'apple', 'banana', 'cherry' } end,
      row = function(item) return { { text = item } } end,
      done = function(item, handle) neotmux.log.info('chosen=' .. item) handle:close() end,
      cancel = function() neotmux.log.info('cancelled') end,
    })
    return true
  end
  return false
end)
";

fn open_three(sub: &Built) {
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
}

/// The list draws what the caller's `row` says, with the cursor on the first item.
#[test]
fn a_list_draws_its_items_and_its_title() {
    let (_dir, sub) = built(THREE);
    open_three(&sub);
    let cmds = frame(&sub);
    let said = texts(&cmds);
    assert!(said.iter().any(|t| t.contains("apple")), "{said:?}");
    assert!(said.iter().any(|t| t.contains("banana")), "{said:?}");
    assert!(said.iter().any(|t| t.contains("cherry")), "{said:?}");
    assert!(said.iter().any(|t| t.contains("fruit")), "{said:?}");
}

/// `j` and `k` move and wrap, and the caller's `done` is handed the item under the cursor.
#[test]
fn j_and_k_move_the_cursor_and_enter_chooses_what_is_under_it() {
    let (_dir, sub) = built(THREE);
    open_three(&sub);
    keys(&sub, &["j", "j", "Enter"]);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "chosen=cherry"), "{said:?}");

    open_three(&sub);
    keys(&sub, &["k", "Enter"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "chosen=cherry"),
        "k from the top did not wrap to the last item: {said:?}"
    );
}

/// The items are asked for again on every key, so a list whose items went away does not
/// point past the end.
#[test]
fn the_cursor_is_clamped_to_the_items_that_are_there_now() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
local items = { 'a', 'b', 'c' }
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return items end,
      row = function(item) return { { text = item } } end,
      done = function(item) neotmux.log.info('chosen=' .. item) end,
      keys = { ['x'] = function() table.remove(items) end },
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    // To the last item, `c`, then shrink the list out from under the cursor.
    keys(&sub, &["j", "j", "x", "Enter"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "chosen=b"),
        "the cursor was left pointing past the shrunk list: {said:?}"
    );
}

/// More items than rows: the window follows the cursor rather than the cursor leaving.
#[test]
fn a_list_longer_than_its_rows_scrolls_to_keep_the_cursor_on_screen() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    local items = {}
    for i = 1, 20 do items[i] = 'item' .. i end
    list.open({
      rows = 5,
      items = function() return items end,
      row = function(item) return { { text = item } } end,
      done = function() end,
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    keys(&sub, &["j", "j", "j", "j", "j", "j", "j", "j"]);
    let said = texts(&frame(&sub));
    assert!(
        said.iter().any(|t| t == "item9"),
        "the ninth item was not drawn: {said:?}"
    );
    assert!(
        !said.iter().any(|t| t == "item1"),
        "the first item was still on screen: {said:?}"
    );
}

/// Esc closes, and a closed list draws nothing and claims no keys.
#[test]
fn esc_closes_the_list_and_gives_the_keyboard_back() {
    let (_dir, sub) = built(THREE);
    open_three(&sub);
    assert_eq!(
        sub.dispatch_key(support::one(), "Esc"),
        KeyOutcome::Consumed
    );
    assert!(frame(&sub).is_empty());
    assert_eq!(
        sub.dispatch_key(support::one(), "j"),
        KeyOutcome::Fallthrough
    );
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "cancelled"), "{said:?}");
}

/// A key the caller claimed reaches it with the item under the cursor.
#[test]
fn a_key_the_caller_claimed_is_handed_the_item() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return { 'apple', 'banana', 'cherry' } end,
      row = function(item) return { { text = item } } end,
      done = function() end,
      keys = {
        ['c'] = function(item) neotmux.log.info('claimed=' .. item) end,
      },
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    keys(&sub, &["j", "c"]);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "claimed=banana"), "{said:?}");
}

/// Unclaimed keys are swallowed: a list that let them through would type into the pane. A
/// stub that swallows every key forever would pass a weaker version of this test, so it also
/// checks that the list is still up and drawing afterwards, and that the very same key stops
/// being consumed once the list is actually gone.
#[test]
fn a_key_nobody_claimed_is_swallowed_while_a_list_is_up() {
    let (_dir, sub) = built(THREE);
    open_three(&sub);
    assert_eq!(sub.dispatch_key(support::one(), "z"), KeyOutcome::Consumed);
    let said = texts(&frame(&sub));
    assert!(
        said.iter().any(|t| t.contains("apple")),
        "the list stopped drawing after an unclaimed key: {said:?}"
    );

    assert_eq!(
        sub.dispatch_key(support::one(), "C-x"),
        KeyOutcome::Consumed
    );
    assert!(
        !frame(&sub).is_empty(),
        "the list closed on an unclaimed key"
    );

    sub.dispatch_key(support::one(), "Esc");
    assert_eq!(
        sub.dispatch_key(support::one(), "z"),
        KeyOutcome::Fallthrough,
        "the same key was still consumed once the list was gone"
    );
}

/// A handle from a list that has since been replaced is inert: `:selected()` answers nothing
/// and `:close()` does nothing, rather than reading or tearing down whatever list is open now.
/// The palette opens a second list from inside a callback of the first, so this is the
/// ordinary case and not a corner of it.
#[test]
fn a_handle_from_a_replaced_list_is_inert() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return { 'a' } end,
      row = function(item) return { { text = item } } end,
      done = function() end,
      keys = {
        ['n'] = function(item, handle)
          -- Opens a second list on top of this one, then reaches for the first list's own
          -- handle — the one this very callback was handed — after it no longer names
          -- anything that is open.
          list.open({
            title = 'second',
            items = function() return { 'b' } end,
            row = function(item) return { { text = item } } end,
            done = function() end,
          })
          neotmux.log.info('selected=' .. tostring(handle:selected()))
          handle:close()
        end,
      },
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    assert_eq!(sub.dispatch_key(support::one(), "n"), KeyOutcome::Consumed);
    let said = logged(&sub);
    assert!(
        said.iter().any(|l| l == "selected=nil"),
        "a stale handle still read the list that replaced it: {said:?}"
    );
    let drawn = texts(&frame(&sub));
    assert!(
        drawn.iter().any(|t| t.contains("second")),
        "closing through the stale handle tore down the list that replaced it: {drawn:?}"
    );
    assert!(drawn.iter().any(|t| t == "b"), "{drawn:?}");
}

/// A filtered list is typed into rather than moved through by letter, so cursor movement while
/// filtering is `C-j`/`C-k`: a plain `j` is a character the query may hold.
#[test]
fn a_filtered_list_moves_with_c_j_and_c_k_and_types_j_as_a_letter() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return { 'jam', 'jar', 'cherry' } end,
      row = function(item) return { { text = item } } end,
      done = function(item) neotmux.log.info('chosen=' .. item) end,
      filter = function(item, query) return item:find(query, 1, true) ~= nil end,
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    assert_eq!(sub.dispatch_key(support::one(), "j"), KeyOutcome::Consumed);
    let said = texts(&frame(&sub));
    assert!(
        said.iter().any(|t| t == "/ j"),
        "`j` did not reach the query: {said:?}"
    );
    assert!(said.iter().any(|t| t == "jam"), "{said:?}");
    assert!(said.iter().any(|t| t == "jar"), "{said:?}");
    assert!(
        !said.iter().any(|t| t == "cherry"),
        "cherry survived a query it does not match: {said:?}"
    );

    assert_eq!(
        sub.dispatch_key(support::one(), "C-j"),
        KeyOutcome::Consumed
    );
    assert_eq!(
        sub.dispatch_key(support::one(), "Enter"),
        KeyOutcome::Consumed
    );
    let said = logged(&sub);
    assert!(
        said.iter().any(|l| l == "chosen=jar"),
        "C-j did not move the cursor onto the second item: {said:?}"
    );
}

/// `ask`'s `accept` is asked before a character joins the buffer: a leading `-` is refused and
/// never appears, while the same character later in the string is accepted.
#[test]
fn asks_accept_hook_can_refuse_a_leading_character() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return { 'apple' } end,
      row = function(item) return { { text = item } } end,
      done = function() end,
      keys = {
        ['a'] = function(item, handle)
          handle:ask({
            label = 'name',
            accept = function(k, text)
              if k == '-' and text == '' then
                return false
              end
              return true
            end,
            done = function(text) neotmux.log.info('typed=' .. text) end,
          })
        end,
      },
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    keys(&sub, &["a", "-", "a", "-", "Enter"]);
    let said = logged(&sub);
    assert!(
        said.iter().any(|l| l == "typed=a-"),
        "a leading '-' was let through, or one later in the string was refused: {said:?}"
    );
}

/// `ask` turns the footer into an input line and hands back what was typed.
#[test]
fn ask_collects_a_line_and_hands_it_to_done() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return { 'apple', 'banana', 'cherry' } end,
      row = function(item) return { { text = item } } end,
      done = function() end,
      keys = {
        ['a'] = function(item, handle)
          handle:ask({
            label = 'name',
            done = function(text) neotmux.log.info('typed=' .. text) end,
          })
        end,
      },
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    keys(&sub, &["a", "a", "b", "c", "Enter"]);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "typed=abc"), "{said:?}");
}

/// `filter` turns the input line on for the life of the list and narrows what is drawn.
#[test]
fn a_filtered_list_narrows_as_it_is_typed_into() {
    let (_dir, sub) = built(
        "
local list = require('neotmux.list')
neotmux.on_key(function(key)
  if key == 'o' then
    list.open({
      items = function() return { 'apple', 'banana', 'cherry' } end,
      row = function(item) return { { text = item } } end,
      done = function() end,
      filter = function(item, query) return item:find(query, 1, true) ~= nil end,
    })
    return true
  end
  return false
end)
",
    );
    assert_eq!(sub.dispatch_key(support::one(), "o"), KeyOutcome::Consumed);
    let said = texts(&frame(&sub));
    assert!(said.iter().any(|t| t.contains("apple")), "{said:?}");
    assert!(said.iter().any(|t| t.contains("banana")), "{said:?}");
    assert!(said.iter().any(|t| t.contains("cherry")), "{said:?}");

    keys(&sub, &["b", "a"]);
    let said = texts(&frame(&sub));
    assert!(
        said.iter().any(|t| t.contains("banana")),
        "banana was filtered out: {said:?}"
    );
    assert!(
        !said.iter().any(|t| t.contains("apple")),
        "apple survived a query it does not match: {said:?}"
    );
    assert!(
        !said.iter().any(|t| t.contains("cherry")),
        "cherry survived a query it does not match: {said:?}"
    );
}
