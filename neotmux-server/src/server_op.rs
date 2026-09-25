/// What a plugin asked of the *server* rather than of a session. The set of sessions is not
/// something a session can change, so these do not travel in a tock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerOp {
    Create { name: String },
    Switch { name: String },
    Kill { name: String },
    Rename { from: String, to: String },
    Reload,
}
