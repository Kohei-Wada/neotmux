use crate::Level;

/// A notification as the host's surface holds it: what it says, not where it goes or what
/// colour it is drawn in. It reaches a plugin through `neotmux.notifications.list()`, and
/// whoever draws it decides the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub tag: String,
    pub text: String,
}
