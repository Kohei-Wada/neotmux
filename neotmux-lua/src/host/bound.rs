use neotmux_proto::SessionId;

use crate::host::Ambient;

/// The binding, undone when it goes out of scope — including on the way out of a dispatch that
/// ended in an error, which is the whole reason it is a guard and not two calls.
///
/// It restores to what was bound before rather than to nothing, so a bind inside a bind — a
/// plugin reading another session from inside its own tick — leaves its caller where it was.
pub struct Bound<'a> {
    pub(crate) ambient: &'a Ambient,
    pub(crate) was: Option<SessionId>,
}

impl Drop for Bound<'_> {
    fn drop(&mut self) {
        self.ambient.swap(self.was);
    }
}
