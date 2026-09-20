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

use mlua::{Lua, Table};
use std::io::{self, Write};

/// Writes the sequence for `text` and flushes it. Flushing is not optional: the sequence has
/// no newline in it, so a line-buffered terminal would otherwise hold it until something
/// else came along.
pub(crate) fn write_to(mut sink: impl Write, text: &str) -> io::Result<()> {
    sink.write_all(&neotmux_term::osc52::set(text))?;
    sink.flush()
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<Table> {
    let clipboard = lua.create_table()?;

    {
        // Standard output, because that is where the renderer draws: if this is not the
        // client's terminal then neotmux has no screen either. Written between frames, from
        // a tick, into the same handle ratatui flushes through, so it cannot land inside a
        // frame's own output.
        let set = lua.create_function(move |_, text: String| {
            Ok(match write_to(io::stdout().lock(), &text) {
                Ok(()) => (true, None),
                Err(err) => (false, Some(err.to_string())),
            })
        })?;
        clipboard.set("set", set)?;
    }

    Ok(clipboard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_reaches_the_sink() {
        let mut out = Vec::new();
        write_to(&mut out, "hi").expect("write");
        assert_eq!(out, b"\x1b]52;c;aGk=\x07");
    }

    /// The sequence carries no newline, so a sink that is only flushed on one would hold it.
    #[test]
    fn a_sink_that_cannot_be_written_to_is_an_error_rather_than_a_silence() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("no terminal"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let err = write_to(Broken, "hi").expect_err("a broken sink reported success");
        assert_eq!(err.to_string(), "no terminal");
    }
}
