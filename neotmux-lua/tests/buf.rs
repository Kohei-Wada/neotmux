//! `neotmux.buf`: Neovim's buffer calls, over lines the host keeps.

use neotmux_lua::host::Op;

mod support;

use support::{Built, built};

/// The top-level init.lua ops, filed under no session — `support::logged` drains only the
/// session-bound ones and misses these, as `hl.rs` explains.
fn logged(sub: &Built) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn said(body: &str) -> Vec<String> {
    let (_dir, sub) = built(body);
    logged(&sub)
}

#[test]
fn set_lines_is_neovims_indexing() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             neotmux.buf.set_lines(b, 0, -1, { 'a', 'b', 'c' })\n\
             neotmux.buf.set_lines(b, 1, 2, { 'x', 'y' })\n\
             neotmux.buf.set_lines(b, 0, 0, { 'top' })\n\
             neotmux.log.info(table.concat(neotmux.buf.get_lines(b, 0, -1), ','))\n\
             neotmux.log.info(neotmux.buf.line_count(b))\n"
        ),
        vec!["top,a,x,y,c", "5"]
    );
}

#[test]
fn a_new_buffer_has_one_empty_line_as_neovims_does() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             neotmux.log.info(neotmux.buf.line_count(b) .. ':' .. neotmux.buf.get_lines(b, 0, -1)[1])\n"
        ),
        vec!["1:"]
    );
}

#[test]
fn an_index_past_the_end_a_newline_or_a_non_string_raises() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             neotmux.log.info(tostring(pcall(neotmux.buf.set_lines, b, 5, 6, { 'x' })))\n\
             neotmux.log.info(tostring(pcall(neotmux.buf.set_lines, b, 0, -1, { 'a\\nb' })))\n\
             neotmux.log.info(tostring(pcall(neotmux.buf.set_lines, b, 0, -1, { 1 })))\n"
        ),
        vec!["false", "false", "false"]
    );
}

#[test]
fn a_deleted_buffer_raises_and_delete_is_idempotent() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             neotmux.buf.delete(b)\n\
             neotmux.buf.delete(b)\n\
             neotmux.log.info(tostring(pcall(neotmux.buf.line_count, b)))\n"
        ),
        vec!["false"]
    );
}

#[test]
fn replacing_lines_drops_their_extmarks_and_moves_the_ones_below() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             local ns = neotmux.buf.namespace('t')\n\
             neotmux.buf.set_lines(b, 0, -1, { 'a', 'b', 'c' })\n\
             neotmux.buf.set_extmark(b, ns, 1, 0, { end_col = 1, hl_group = 'x' })\n\
             neotmux.buf.set_extmark(b, ns, 2, 0, { end_col = 1, hl_group = 'y' })\n\
             neotmux.buf.set_lines(b, 0, 2, { 'p', 'q', 'r' })\n\
             for _, m in ipairs(neotmux.buf.get_extmarks(b, ns)) do\n\
               neotmux.log.info(m.row .. ' ' .. m.hl_group)\n\
             end\n"
        ),
        vec!["3 y"]
    );
}

#[test]
fn a_namespace_is_the_same_id_for_the_same_name_and_clear_takes_only_its_own() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             local a, a2, z = neotmux.buf.namespace('a'), neotmux.buf.namespace('a'), neotmux.buf.namespace('z')\n\
             neotmux.log.info(tostring(a == a2) .. ' ' .. tostring(a ~= z))\n\
             neotmux.buf.set_lines(b, 0, -1, { 'abc' })\n\
             neotmux.buf.set_extmark(b, a, 0, 0, { end_col = 1, hl_group = 'x' })\n\
             neotmux.buf.set_extmark(b, z, 0, 1, { end_col = 2, hl_group = 'y' })\n\
             neotmux.buf.clear_namespace(b, a, 0, -1)\n\
             neotmux.log.info(#neotmux.buf.get_extmarks(b, a) .. ' ' .. #neotmux.buf.get_extmarks(b, z))\n"
        ),
        vec!["true true", "0 1"]
    );
}

#[test]
fn an_extmark_on_a_row_that_is_not_there_raises() {
    assert_eq!(
        said(
            "local b = neotmux.buf.create()\n\
             local ns = neotmux.buf.namespace('t')\n\
             neotmux.log.info(tostring(pcall(neotmux.buf.set_extmark, b, ns, 3, 0, { hl_group = 'x' })))\n"
        ),
        vec!["false"]
    );
}

#[test]
fn a_groups_buffer_goes_when_the_group_is_named_again() {
    assert_eq!(
        said(
            "local b = neotmux.group('mine').buf.create()\n\
             neotmux.group('mine')\n\
             neotmux.log.info(tostring(pcall(neotmux.buf.line_count, b)))\n"
        ),
        vec!["false"]
    );
}

#[test]
fn buf_cannot_be_replaced() {
    assert_eq!(
        said("neotmux.log.info(tostring(pcall(function() neotmux.buf.create = nil end)))\n"),
        vec!["false"]
    );
}
