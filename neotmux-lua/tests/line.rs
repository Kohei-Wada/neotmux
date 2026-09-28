//! `neotmux.line`: the keys that edit a line of text, and a prompt that edits with them — the
//! one that used to take back a byte with `Backspace` and nothing else.

mod support;

use support::{built, keys, logged};

/// Each key on its own, answered by `line.edit` in Lua.
#[test]
fn each_key_edits_the_line_the_way_a_shell_does() {
    let (_dir, sub) = built(
        "local line = require('neotmux.line')\n\
         neotmux.on_key(function(key)\n\
           if key ~= 'J' then return false end\n\
           local cases = {\n\
             { 'abc', 'Backspace', 'ab' },\n\
             { 'abc', 'C-h', 'ab' },\n\
             { '作業', 'Backspace', '作' },\n\
             { 'git log  ', 'C-w', 'git ' },\n\
             { 'git log', 'C-w', 'git ' },\n\
             { 'one', 'C-w', '' },\n\
             { 'abc', 'C-u', '' },\n\
             { 'ab', 'x', 'abx' },\n\
             { 'ab', '業', 'ab業' },\n\
             { '', 'Backspace', '' },\n\
           }\n\
           for _, c in ipairs(cases) do\n\
             local got = line.edit(c[1], c[2])\n\
             if got ~= c[3] then\n\
               neotmux.log.info(string.format('%q + %s = %q, not %q', c[1], c[2], tostring(got), c[3]))\n\
             end\n\
           end\n\
           neotmux.log.info('not a line key: ' .. tostring(line.edit('ab', 'Up')))\n\
           neotmux.log.info('refused: ' .. line.edit('', '-', function(k) return k ~= '-' end))\n\
           return true\n\
         end)\n",
    );
    keys(&sub, &["J"]);
    assert_eq!(
        logged(&sub),
        vec!["not a line key: nil".to_string(), "refused: ".to_string()]
    );
}

/// A prompt takes every one of them: `C-u`, `C-h` and `C-w` did nothing in one before, and
/// `Backspace` on `作` left a byte of it behind.
#[test]
fn a_prompt_edits_with_every_line_key() {
    let (_dir, sub) = built(
        "neotmux.on_key(function(key)\n\
           if key ~= 'P' then return false end\n\
           require('neotmux.prompt').open({\n\
             title = 'test',\n\
             done = function(text) neotmux.log.info('got ' .. text) end,\n\
           })\n\
           return true\n\
         end)\n",
    );
    keys(
        &sub,
        &[
            "P", "x", "y", "C-u", "g", "i", "t", " ", "l", "o", "g", "C-w",
        ],
    );
    keys(&sub, &["作", "業", "Backspace", "z", "C-h", "Enter"]);
    assert_eq!(logged(&sub), vec!["got git 作".to_string()]);
}
