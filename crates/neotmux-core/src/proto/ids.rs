use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(u32);

impl PaneId {
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for PaneId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

/// One independent pane tree, and today the only level of grouping above a pane there is.
///
/// tmux has session > window > pane, and this is the window. Only one level is built: what
/// makes a tmux *session* expensive is detach/attach — a server/client split — and without
/// detach a session and a window are indistinguishable from the user's seat, both being "a
/// set of panes you switch between". A second level introduced now would be an unused
/// hierarchy frozen into the ABI.
///
/// Which level the one level *is*, though, is an ABI decision rather than a naming
/// preference, because the session is wanted eventually. Call this a workspace and put a
/// window underneath it later, and `pane.list()` silently changes meaning from "the
/// workspace's panes" to "the current window's panes" — a breaking change to a call that
/// will by then have plugins written against it. Calling it a window puts the later addition
/// *above* it instead: a session containing windows is purely additive, and `pane.list()`
/// keeps its meaning forever.
///
/// It is also what the thing already is. `C-b n` cycles it, and the thing a hand cycles
/// constantly in tmux is the window; the session is the thing you attach to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u32);

impl WindowId {
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for WindowId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_from_and_get() {
        let id: PaneId = 7u32.into();
        assert_eq!(id.get(), 7);
    }

    #[test]
    fn displays_with_percent_prefix() {
        let id = PaneId::from(3);
        assert_eq!(id.to_string(), "%3");
    }

    #[test]
    fn a_window_id_round_trips_and_displays_with_an_at_prefix() {
        let id = WindowId::from(2);
        assert_eq!(id.get(), 2);
        assert_eq!(id.to_string(), "@2");
    }
}
