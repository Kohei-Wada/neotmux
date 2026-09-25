//! `neotmux.clipboard.set`: hand the client's terminal something to put on the clipboard.
//!
//! **What the return value means, exactly.** `true` says the escape sequence was written to
//! the client's terminal and flushed. It does not say the clipboard holds the text, and
//! nothing here could say that: OSC 52 has no reply, an unknown OSC is discarded rather than
//! refused, and the sequence may be dropped by a terminal that does not implement it, by one
//! that has it turned off — xterm's `allowWindowOps` is off by default in most builds — or
//! by an ssh session or another multiplexer in between.
//!
//! tmux consults terminfo's `Ms` capability and writes nothing at all when it is missing
//! (`tty.c`: `if (!tty_term_has(tty->term, TTYC_MS)) return;`), which is how a "Copied to
//! clipboard" toast ends up over nothing having been copied. neotmux does not do that, in
//! either direction: it does not read terminfo, because `Ms` describes what the terminfo
//! entry for `$TERM` claims rather than what the terminal on the other end did, and a caller
//! reading `true` would take the stronger meaning; and it does not stay silent, because
//! writing an OSC a terminal does not know is harmless. So the sequence always goes out, and
//! the answer is about the write.
//!
//! A plugin that wants to tell a person something should therefore say what it knows — "sent
//! to the terminal" — or say nothing. Whether the paste buffer it keeps beside this got the
//! text is its own question, and one it can answer.

use std::rc::Rc;

use mlua::{Lua, Table};

use crate::clipboard::Clipboard;

pub(crate) fn install(lua: &Lua, clipboard: Rc<dyn Clipboard>) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    {
        // The host's, because the terminal is the host's: this crate has none to write to.
        // Called from a tick, between frames, so whatever the host does with it cannot land
        // inside a frame's own output.
        let set = lua.create_function(move |_, text: String| {
            Ok(match clipboard.set(&text) {
                Ok(()) => (true, None),
                Err(err) => (false, Some(err.to_string())),
            })
        })?;
        table.set("set", set)?;
    }

    Ok(table)
}
