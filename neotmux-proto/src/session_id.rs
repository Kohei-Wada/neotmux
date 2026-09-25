use std::fmt;

/// One session: a set of windows, and the thing a client attaches to.
///
/// `WindowId`'s note above called a level above it purely additive, and this is that level:
/// a window keeps meaning what it meant and this sits over it. The sigil is tmux's, which
/// spells a pane `%1`, a window `@1` and a session `$1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u32);

impl SessionId {
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SessionId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.0)
    }
}
