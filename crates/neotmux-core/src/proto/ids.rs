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

/// One independent pane tree, and the only level of grouping above a pane there is.
///
/// tmux has session > window > pane. What makes a tmux *session* expensive is detach/attach —
/// a server/client split, which is not built here; what makes it useful day to day is
/// switching between independent sets of panes, which is just having several trees and a
/// current one. Without detach a session and a window are indistinguishable from the user's
/// seat — both are "a set of panes you switch between" — so a second level introduced now
/// would be an unused hierarchy frozen into the ABI. One level is deliberate: a detach, when
/// it comes, attaches *to* workspaces, and a level above them is easy to add later and
/// impossible to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(u32);

impl WorkspaceId {
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for WorkspaceId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.0)
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
    fn a_workspace_id_round_trips_and_displays_with_a_dollar_prefix() {
        let id = WorkspaceId::from(2);
        assert_eq!(id.get(), 2);
        assert_eq!(id.to_string(), "$2");
    }
}
