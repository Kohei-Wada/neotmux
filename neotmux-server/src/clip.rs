//! Where a yank waits, now that the Lua layer is in the process with no terminal.
//!
//! OSC 52 is a sequence written to a terminal, and the server's terminals are its clients'.
//! So `neotmux.clipboard.set` queues the text here and the frame hands it to whoever is
//! attached, which is the one place that knows who that is.
//!
//! Each yank is kept with the session it was raised for, and a session's frame takes only its
//! own: one queue drained by whichever session's frame came next sent a yank to the terminals
//! looking at a different session.

use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_proto::SessionId;

#[derive(Default)]
pub struct Clip {
    queued: RefCell<Vec<(Option<SessionId>, String)>>,
    /// How many clients are looking at each session, as of this frame.
    attached: RefCell<BTreeMap<SessionId, usize>>,
}

impl Clip {
    pub fn new() -> Self {
        Self::default()
    }

    /// Who is looking at what, as of this frame: one session per client.
    pub fn attached(&self, sessions: impl Iterator<Item = SessionId>) {
        let mut counts = BTreeMap::new();
        for session in sessions {
            *counts.entry(session).or_insert(0) += 1;
        }
        *self.attached.borrow_mut() = counts;
    }

    /// Everything yanked for `session` since its last frame took it, and anything yanked for no
    /// session at all — a config load — which the first session to ask takes.
    pub fn take(&self, session: SessionId) -> Vec<String> {
        let mut queued = self.queued.borrow_mut();
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *queued)
            .into_iter()
            .partition(|(for_session, _)| for_session.is_none_or(|s| s == session));
        *queued = rest;
        mine.into_iter().map(|(_, text)| text).collect()
    }
}

impl neotmux_lua::host::Clipboard for Clip {
    fn set(&self, session: Option<SessionId>, text: &str) -> std::io::Result<()> {
        let attached = self.attached.borrow();
        let looking = match session {
            Some(session) => attached.get(&session).copied().unwrap_or(0),
            None => attached.values().sum(),
        };
        if looking == 0 {
            return Err(std::io::Error::other(
                "no client is attached, so there is no terminal to write it to",
            ));
        }
        self.queued.borrow_mut().push((session, text.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use neotmux_lua::host::Clipboard as _;

    use super::*;

    fn one() -> SessionId {
        SessionId::from(1)
    }

    fn two() -> SessionId {
        SessionId::from(2)
    }

    #[test]
    fn a_yank_with_a_client_attached_is_queued_once_and_taken_once() {
        let clip = Clip::new();
        clip.attached([one()].into_iter());
        clip.set(Some(one()), "hello")
            .expect("a client is attached");
        assert_eq!(clip.take(one()), vec!["hello".to_string()]);
        assert!(
            clip.take(one()).is_empty(),
            "taking it twice sends it twice"
        );
    }

    /// `Clipboard::set` promises the host did what it does. With no terminal to write to
    /// there is nothing it does, and saying so is the honest answer — the Lua side turns it
    /// into the `false` a plugin can act on.
    #[test]
    fn a_yank_with_nobody_attached_is_refused_rather_than_held() {
        let clip = Clip::new();
        clip.attached(std::iter::empty());
        assert!(clip.set(Some(one()), "hello").is_err());
        assert!(clip.take(one()).is_empty());
    }

    /// #56: a yank is the session's it was raised for. The other session's frame, coming first,
    /// does not take it; its own does.
    #[test]
    fn a_yank_is_taken_only_by_the_session_it_was_raised_for() {
        let clip = Clip::new();
        clip.attached([one(), two()].into_iter());
        clip.set(Some(two()), "for two").expect("two has a client");
        assert!(
            clip.take(one()).is_empty(),
            "session one took session two's yank"
        );
        assert_eq!(clip.take(two()), vec!["for two".to_string()]);
    }

    /// Nobody looking at the session the yank is for is nobody to write it to, however many
    /// are looking at another.
    #[test]
    fn a_yank_for_a_session_nobody_is_looking_at_is_refused() {
        let clip = Clip::new();
        clip.attached([one()].into_iter());
        assert!(clip.set(Some(two()), "hello").is_err());
    }
}
