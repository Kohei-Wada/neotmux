//! Reading a pane that is in the other process.
//!
//! `neotmux.pane.content` and its neighbours are synchronous calls in Lua, and the terminal
//! they read is the session's, which the server holds — it has to, because a session nobody
//! is looking at goes on parsing its children's output, and from v0.2 a detached one still
//! has a screen to come back to. So the read is a question and an answer on the same socket
//! the frames come over.
//!
//! It can be asked from anywhere a plugin runs — a tick, an event, a key handler — and the
//! server answers it wherever it is in its own frame. So the answer is not necessarily the
//! next thing on the wire: a tick the server had already begun sending can arrive first, and
//! that one is set aside for the loop whose message it is rather than dropped. A read that
//! fails answers `None`, which in Lua is the `nil` a pane that does not exist already gives:
//! a session ending under a watcher's loop is not the caller's error, and neither is one
//! ending under this.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use neotmux_proto::msg::{Read, Reading, ToServer};
use neotmux_proto::read::{Extent, Lines, PaneRead, Want};
use neotmux_proto::wire;
use neotmux_proto::{OwnedToClient, PaneId};

use crate::link::Link;

#[derive(Default)]
pub struct RemotePanes {
    /// Set once the server answers. The Lua layer is built before there is a connection —
    /// the two are started together — and before there is one there are no panes to read,
    /// which is the `nil` a read already means.
    link: RefCell<Option<Rc<RefCell<Link>>>>,
    /// Every pane's read stamp as of the last tick. Answered from here rather than asked
    /// for, because a watcher asks it every tick for every pane: it is the question that
    /// exists so the expensive one is rare, and a round trip would undo that.
    revisions: RefCell<BTreeMap<PaneId, u64>>,
}

impl RemotePanes {
    pub fn new() -> Self {
        Self::default()
    }

    /// The connection the reads go over, once the client has one.
    pub fn attach(&self, link: Rc<RefCell<Link>>) {
        *self.link.borrow_mut() = Some(link);
    }

    /// What the tick said every pane's stamp is.
    pub fn publish(&self, revisions: BTreeMap<PaneId, u64>) {
        *self.revisions.borrow_mut() = revisions;
    }

    fn ask(&self, read: Read) -> Option<Reading> {
        let connected = self.link.borrow();
        let mut link = connected.as_ref()?.borrow_mut();
        link.send(&wire::encode_to_server(&ToServer::Read(read)))
            .ok()?;
        // Held rather than put back one at a time: putting one back is putting it where the
        // next read of this connection looks first, which is here, and this call would take
        // it again and again instead of ever reaching the socket.
        let mut theirs = Vec::new();
        let reading = loop {
            let Some(message) = link.recv().ok().flatten() else {
                break None;
            };
            match wire::decode_to_client(&message).ok() {
                Some(OwnedToClient::Reading(reading)) => break Some(reading),
                // The frame loop's, not this call's.
                Some(_) => theirs.push(message),
                None => break None,
            }
        };
        // Back in the order they arrived, for the loop whose messages they are.
        for message in theirs {
            link.defer(message);
        }
        reading
    }

    fn text(&self, pane: PaneId, read: Read) -> Option<Lines> {
        match self.ask(read)? {
            Reading::Text(text) => text.map(|text| Lines {
                lines: text.lines,
                wrapped: text.wrapped,
                // This frame's stamp, not the one the pane is at as the answer is written.
                // The two are the same question asked at two instants, and a plugin that
                // holds one and compares it against `pane.revision` — which is the whole of
                // what the stamp is for — has to be comparing like with like. A pane fed
                // since is not lost by this: its stamp moves in the next tick, and the
                // comparison says so then.
                revision: self.revision(pane).unwrap_or(text.revision),
            }),
            _ => None,
        }
    }

    fn did(&self, read: Read) -> bool {
        matches!(self.ask(read), Some(Reading::Did(true)))
    }
}

impl PaneRead for RemotePanes {
    fn revision(&self, pane: PaneId) -> Option<u64> {
        self.revisions.borrow().get(&pane).copied()
    }

    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines> {
        self.text(pane, Read::Content { pane, last, want })
    }

    fn history_extent(&self, pane: PaneId) -> Option<Extent> {
        match self.ask(Read::HistoryExtent { pane })? {
            Reading::Extent(extent) => extent,
            _ => None,
        }
    }

    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines> {
        self.text(
            pane,
            Read::History {
                pane,
                from,
                count,
                want,
            },
        )
    }

    fn freeze(&self, pane: PaneId) -> bool {
        self.did(Read::Freeze { pane })
    }

    fn unfreeze(&self, pane: PaneId) -> bool {
        self.did(Read::Unfreeze { pane })
    }

    fn is_frozen(&self, pane: PaneId) -> Option<bool> {
        match self.ask(Read::IsFrozen { pane })? {
            Reading::Frozen(frozen) => frozen,
            _ => None,
        }
    }
}

/// The other side of [`RemotePanes`]: one read, answered out of the session's own terminals.
///
/// One line per question, because there is one vocabulary now: what the socket asks in is
/// what the terminals answer in.
pub fn answer(panes: &dyn PaneRead, read: Read) -> Reading {
    match read {
        Read::Revision { pane } => Reading::Revision(panes.revision(pane)),
        Read::Content { pane, last, want } => Reading::Text(panes.content(pane, last, want)),
        Read::HistoryExtent { pane } => Reading::Extent(panes.history_extent(pane)),
        Read::History {
            pane,
            from,
            count,
            want,
        } => Reading::Text(panes.history(pane, from, count, want)),
        Read::Freeze { pane } => Reading::Did(panes.freeze(pane)),
        Read::Unfreeze { pane } => Reading::Did(panes.unfreeze(pane)),
        Read::IsFrozen { pane } => Reading::Frozen(panes.is_frozen(pane)),
    }
}

#[cfg(test)]
mod tests {
    use neotmux_proto::Size;
    use neotmux_term::terminals::PaneTerminals;

    use super::*;

    const SIZE: Size = Size { rows: 2, cols: 8 };

    fn pane() -> PaneId {
        PaneId::from(1)
    }

    /// Every read asked of a `RemotePanes` comes back exactly as the same read asked of the
    /// terminals directly, which is the whole contract: a plugin cannot tell that the pane
    /// it is reading is in another process.
    #[test]
    fn a_read_over_the_socket_answers_what_the_terminals_would_have() {
        let (client, server) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let opened = std::thread::spawn(move || Link::open(server).expect("open"));
        let client = Link::open(client).expect("open");
        let mut server = opened.join().expect("join");

        let terminals = PaneTerminals::new();
        terminals.open(pane(), SIZE, 100);
        terminals.feed(pane(), b"one\r\ntwo\r\nthree");

        let served = std::thread::spawn(move || {
            while let Some(message) = server.recv().expect("recv") {
                let ToServer::Read(read) = wire::decode_to_server(&message).expect("decode") else {
                    panic!("the reader sent something that is not a read");
                };
                let reading = answer(&terminals, read);
                server
                    .send(&wire::encode_to_client(&neotmux_proto::ToClient::Reading(
                        &reading,
                    )))
                    .expect("send");
            }
        });

        let remote = RemotePanes::new();
        remote.attach(Rc::new(RefCell::new(client)));
        remote.publish(BTreeMap::from([(pane(), 1)]));
        assert_eq!(remote.revision(pane()), Some(1));
        let text = remote
            .content(pane(), None, Want::default())
            .expect("a live pane");
        assert_eq!(text.lines, vec!["two".to_string(), "three".to_string()]);
        assert_eq!(remote.history_extent(pane()).expect("extent").len, 1);
        let history = remote
            .history(pane(), 0, u64::MAX, Want::default())
            .expect("a live pane");
        assert_eq!(history.lines, vec!["one".to_string()]);
        assert!(remote.freeze(pane()));
        assert_eq!(remote.is_frozen(pane()), Some(true));
        assert!(remote.unfreeze(pane()));
        assert_eq!(remote.is_frozen(pane()), Some(false));

        assert_eq!(remote.revision(PaneId::from(99)), None);
        assert_eq!(
            remote.content(PaneId::from(99), None, Want::default()),
            None
        );
        assert!(!remote.freeze(PaneId::from(99)));

        drop(remote);
        served.join().expect("join");
    }

    /// A read before there is anything to read from — the Lua layer exists first — and one
    /// after the server has gone: both answer `nil` rather than taking the client with them.
    #[test]
    fn a_read_with_nobody_on_the_other_end_is_nil_rather_than_a_panic() {
        let (client, server) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let opened = std::thread::spawn(move || Link::open(server).expect("open"));
        let client = Link::open(client).expect("open");
        drop(opened.join().expect("join"));

        let unconnected = RemotePanes::new();
        assert_eq!(unconnected.content(pane(), None, Want::default()), None);
        assert_eq!(unconnected.revision(pane()), None);

        let remote = RemotePanes::new();
        remote.attach(Rc::new(RefCell::new(client)));
        assert_eq!(remote.content(pane(), None, Want::default()), None);
        assert_eq!(remote.is_frozen(pane()), None);
        assert!(!remote.freeze(pane()));
    }
}
