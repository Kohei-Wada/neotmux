//! The commands the host owns.
//!
//! These control the server — its sessions, their panes, what is written into one and what is
//! on it — and they are answered without the Lua layer being consulted, entered or woken. That
//! is the whole of why they are here rather than in the runtime layer: a control CLI that
//! stops working over how a config is written is not a control CLI.
//!
//! Nothing in this file knows about the socket. A name, its arguments and the sessions go in;
//! lines and whether it worked come out.

/// What a command did.
pub struct Ran {
    pub ok: bool,
    /// stdout where `ok`, and the one sentence that says why not where it is not.
    pub lines: Vec<String>,
}

impl Ran {
    #[allow(dead_code)] // used once a command answers with more than one sentence
    pub fn ok(lines: Vec<String>) -> Self {
        Self { ok: true, lines }
    }

    pub fn failed(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            lines: vec![why.into()],
        }
    }
}

/// Every name the host answers. The CLI reads it to tell a command from a session name.
pub const NAMES: &[&str] = &[];

pub fn unknown(name: &str) -> Ran {
    Ran::failed(format!("unknown command '{name}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_nobody_implements_is_a_failure_that_says_so() {
        let ran = unknown("frobnicate");
        assert!(!ran.ok);
        assert_eq!(ran.lines, vec!["unknown command 'frobnicate'".to_string()]);
    }
}
