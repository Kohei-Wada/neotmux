use neotmux_proto::SessionId;

/// The session a job or a timer belongs to, which is none only between a load-time start and
/// the first frame: a chunk loaded before any session existed has none to give it.
#[derive(Clone, Copy)]
pub struct SessionSlot(Option<SessionId>);

impl SessionSlot {
    pub fn new(session: Option<SessionId>) -> Self {
        Self(session)
    }

    /// Whether this frame's session is the owner, adopting it if there is none yet: the first
    /// frame to come looking takes it, and it belongs to that session from then on.
    pub fn claim(&mut self, session: SessionId) -> bool {
        match self.0 {
            Some(owner) => owner == session,
            None => {
                self.0 = Some(session);
                true
            }
        }
    }

    /// Whether it belongs to `session` already. One still waiting to be adopted belongs to no
    /// session, so a session that ends leaves it alone.
    pub fn is(&self, session: SessionId) -> bool {
        self.0 == Some(session)
    }
}
