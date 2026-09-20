//! Reading a pane that is in the other process.
//!
//! `neotmux.pane.content` and its neighbours are synchronous calls in Lua, and the terminal
//! they read is the session's, which the server holds — it has to, because a session nobody
//! is looking at goes on parsing its children's output, and from v0.2 a detached one still
//! has a screen to come back to. So the read is a question and an answer on the same socket
//! the frames come over.
//!
//! It is only ever asked from inside a tick, and the server is waiting for that tick's reply
//! when it arrives, so the answer is the next thing on the wire rather than something to be
//! picked out of a stream of frames. A read that fails answers `None`, which in Lua is the
//! `nil` a pane that does not exist already gives: a session ending under a watcher's loop
//! is not the caller's error, and neither is one ending under this.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use neotmux_proto::msg::{Read, Reading, ToServer};
use neotmux_proto::wire;
use neotmux_proto::{OwnedToClient, PaneId};
use neotmux_term::lines::Trailing;
use neotmux_term::read::{History, PaneRead, Text, Want};

use crate::link::Link;

pub struct RemotePanes {
    link: Rc<RefCell<Link>>,
    /// Every pane's read stamp as of the last tick. Answered from here rather than asked
    /// for, because a watcher asks it every tick for every pane: it is the question that
    /// exists so the expensive one is rare, and a round trip would undo that.
    revisions: RefCell<BTreeMap<PaneId, u64>>,
}

impl RemotePanes {
    pub fn new(link: Rc<RefCell<Link>>) -> Self {
        Self {
            link,
            revisions: RefCell::new(BTreeMap::new()),
        }
    }

    /// What the tick said every pane's stamp is.
    pub fn publish(&self, revisions: BTreeMap<PaneId, u64>) {
        *self.revisions.borrow_mut() = revisions;
    }

    fn ask(&self, read: Read) -> Option<Reading> {
        let mut link = self.link.borrow_mut();
        link.send(&wire::encode_to_server(&ToServer::Read(read)))
            .ok()?;
        match wire::decode_to_client(&link.recv().ok()??).ok()? {
            OwnedToClient::Reading(reading) => Some(reading),
            // Nothing else can be in flight: the server is inside its wait for this tick's
            // reply, and a read is the only thing it answers there.
            _ => None,
        }
    }

    fn text(&self, read: Read) -> Option<Text> {
        match self.ask(read)? {
            Reading::Text(text) => text.map(|text| Text {
                lines: text.lines,
                wrapped: text.wrapped,
                revision: text.revision,
            }),
            _ => None,
        }
    }

    fn did(&self, read: Read) -> bool {
        matches!(self.ask(read), Some(Reading::Did(true)))
    }
}

fn pad(want: Want) -> bool {
    matches!(want.trailing, Trailing::Pad)
}

impl PaneRead for RemotePanes {
    fn revision(&self, pane: PaneId) -> Option<u64> {
        self.revisions.borrow().get(&pane).copied()
    }

    fn content(&self, pane: PaneId, last: Option<usize>, want: Want) -> Option<Text> {
        self.text(Read::Content {
            pane,
            last: last.map(|n| n as u64),
            pad: pad(want),
            wrapped: want.wrapped,
        })
    }

    fn history_extent(&self, pane: PaneId) -> Option<History> {
        match self.ask(Read::HistoryExtent { pane })? {
            Reading::Extent(extent) => extent.map(|extent| History {
                len: extent.len as usize,
                dropped: extent.dropped as usize,
            }),
            _ => None,
        }
    }

    fn history(&self, pane: PaneId, from: usize, count: usize, want: Want) -> Option<Text> {
        self.text(Read::History {
            pane,
            from: from as u64,
            count: count as u64,
            pad: pad(want),
            wrapped: want.wrapped,
        })
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
pub fn answer(panes: &dyn PaneRead, read: Read) -> Reading {
    let lines = |text: Option<Text>| {
        Reading::Text(text.map(|text| neotmux_proto::msg::Lines {
            lines: text.lines,
            wrapped: text.wrapped,
            revision: text.revision,
        }))
    };
    let want = |pad: bool, wrapped: bool| Want {
        trailing: if pad { Trailing::Pad } else { Trailing::Trim },
        wrapped,
    };
    match read {
        Read::Revision { pane } => Reading::Revision(panes.revision(pane)),
        Read::Content {
            pane,
            last,
            pad,
            wrapped,
        } => lines(panes.content(pane, last.map(|n| n as usize), want(pad, wrapped))),
        Read::HistoryExtent { pane } => {
            Reading::Extent(
                panes
                    .history_extent(pane)
                    .map(|extent| neotmux_proto::msg::Extent {
                        len: extent.len as u64,
                        dropped: extent.dropped as u64,
                    }),
            )
        }
        Read::History {
            pane,
            from,
            count,
            pad,
            wrapped,
        } => lines(panes.history(pane, from as usize, count as usize, want(pad, wrapped))),
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

        let remote = RemotePanes::new(Rc::new(RefCell::new(client)));
        remote.publish(BTreeMap::from([(pane(), 1)]));
        assert_eq!(remote.revision(pane()), Some(1));
        let text = remote
            .content(pane(), None, Want::default())
            .expect("a live pane");
        assert_eq!(text.lines, vec!["two".to_string(), "three".to_string()]);
        assert_eq!(remote.history_extent(pane()).expect("extent").len, 1);
        let history = remote
            .history(pane(), 0, usize::MAX, Want::default())
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

    /// A server that has gone away answers `nil` rather than taking the client down with it.
    #[test]
    fn a_read_with_nobody_on_the_other_end_is_nil_rather_than_a_panic() {
        let (client, server) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let opened = std::thread::spawn(move || Link::open(server).expect("open"));
        let client = Link::open(client).expect("open");
        drop(opened.join().expect("join"));

        let remote = RemotePanes::new(Rc::new(RefCell::new(client)));
        assert_eq!(remote.content(pane(), None, Want::default()), None);
        assert_eq!(remote.is_frozen(pane()), None);
        assert!(!remote.freeze(pane()));
    }
}
