//! One connection, as a stream of messages.
//!
//! [`crate::sock`] says where the socket is; `neotmux_proto::wire` says what a message is
//! worth in bytes. This is the bit in between the two that the codec deliberately leaves
//! out: where one message ends. A four-byte little-endian length in front of each, and a
//! buffer, because a socket hands over whatever happens to have arrived rather than whatever
//! was sent.

use std::collections::VecDeque;
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;

use neotmux_proto::wire::{self, HELLO_LEN};

/// How long a message this end will believe in. A frame of a very large terminal is the
/// biggest thing on the wire and is nowhere near this; anything past it is a peer that has
/// lost the thread, and reading it would be an allocation a stranger chose.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

const HEADER: usize = 4;

/// How long a peer has to say which protocol it speaks. Everything after the greeting waits
/// as long as it has to; this one does not, because there is no work behind it.
const GREETING_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

pub struct Link {
    stream: UnixStream,
    inbox: Vec<u8>,
    /// How much of `inbox` has already been handed out, so a message is not copied to the
    /// front of the buffer for every read.
    taken: usize,
    /// Messages read by somebody who was waiting for a different one, kept in the order they
    /// arrived so that whoever is reading the connection in general still sees them in it.
    stashed: VecDeque<Vec<u8>>,
}

impl Link {
    /// Exchanges the protocol version and hands back the connection.
    ///
    /// Both ends write before either reads, which is what keeps this from being a deadlock,
    /// and the check itself is `decode_hello`'s and nowhere else's. The read is the one on
    /// this connection with a deadline: a peer that connects and then says nothing would
    /// otherwise hold the server here for ever, and four bytes are not slow to send.
    pub fn open(stream: UnixStream) -> io::Result<Self> {
        let mut link = Self {
            stream,
            inbox: Vec::new(),
            taken: 0,
            stashed: VecDeque::new(),
        };
        link.stream.write_all(&wire::encode_hello())?;
        link.stream.flush()?;
        link.stream.set_read_timeout(Some(GREETING_PATIENCE))?;
        let mut theirs = [0u8; HELLO_LEN];
        let greeted = link.stream.read_exact(&mut theirs);
        link.stream.set_read_timeout(None)?;
        greeted?;
        wire::decode_hello(&theirs).map_err(io::Error::other)?;
        Ok(link)
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.stream.as_fd()
    }

    pub fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        let len = u32::try_from(payload.len())
            .map_err(|_| io::Error::other("a message too long to send"))?;
        // One write rather than two: a header that reached the peer without its body would
        // leave the peer blocked on a body that a failure here means never comes.
        let mut framed = Vec::with_capacity(HEADER + payload.len());
        framed.extend_from_slice(&len.to_le_bytes());
        framed.extend_from_slice(payload);
        self.stream.write_all(&framed)?;
        self.stream.flush()
    }

    /// Puts a message back for whoever reads this connection next.
    ///
    /// A pane read is asked in the middle of a key handler, and what comes back first may be
    /// the frame loop's own next message rather than the answer. Setting it aside here is
    /// what keeps the loop from losing it, and keeps the order it arrived in.
    pub fn defer(&mut self, message: Vec<u8>) {
        self.stashed.push_back(message);
    }

    /// The next whole message already in hand, without touching the socket. `None` means the
    /// rest of it has not arrived yet.
    pub fn take(&mut self) -> io::Result<Option<Vec<u8>>> {
        if let Some(stashed) = self.stashed.pop_front() {
            return Ok(Some(stashed));
        }
        let pending = &self.inbox[self.taken..];
        if pending.len() < HEADER {
            return Ok(None);
        }
        let len = u32::from_le_bytes([pending[0], pending[1], pending[2], pending[3]]) as usize;
        if len > MAX_MESSAGE {
            return Err(io::Error::other(format!(
                "the peer announced a {len}-byte message"
            )));
        }
        if pending.len() < HEADER + len {
            return Ok(None);
        }
        let message = pending[HEADER..HEADER + len].to_vec();
        self.taken += HEADER + len;
        Ok(Some(message))
    }

    /// Reads whatever the socket has. `false` means the peer closed it, which is the end of
    /// the conversation rather than an error: a client that went away and a server that
    /// exited both look like this.
    pub fn fill(&mut self) -> io::Result<bool> {
        self.compact();
        let mut chunk = [0u8; 16 * 1024];
        match self.stream.read(&mut chunk) {
            Ok(0) => Ok(false),
            Ok(n) => {
                self.inbox.extend_from_slice(&chunk[..n]);
                Ok(true)
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => Ok(true),
            // A peer that was killed resets the connection rather than closing it politely,
            // and which of the two a read sees is a race. Both are the same event.
            Err(err) if went_away(&err) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Blocks until a whole message is in hand. `None` is the peer having gone away.
    pub fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(message) = self.take()? {
                return Ok(Some(message));
            }
            if !self.fill()? {
                return Ok(None);
            }
        }
    }

    /// Drops what has already been handed out, so the buffer does not grow for the life of
    /// the connection.
    fn compact(&mut self) {
        if self.taken == 0 {
            return;
        }
        self.inbox.drain(..self.taken);
        self.taken = 0;
    }
}

/// Whether an error is the peer having gone, rather than something this end did wrong.
fn went_away(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::NotConnected
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Link, Link) {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let opened = std::thread::spawn(move || Link::open(a).expect("open"));
        let second = Link::open(b).expect("open");
        (opened.join().expect("join"), second)
    }

    #[test]
    fn a_message_comes_back_whole() {
        let (mut a, mut b) = pair();
        a.send(b"hello").expect("send");
        assert_eq!(b.recv().expect("recv").as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn several_messages_come_back_one_at_a_time_and_in_order() {
        let (mut a, mut b) = pair();
        for i in 0..5u8 {
            a.send(&[i; 3]).expect("send");
        }
        for i in 0..5u8 {
            assert_eq!(b.recv().expect("recv").as_deref(), Some(&[i; 3][..]));
        }
    }

    /// The whole reason for a length: a socket hands over whatever arrived, and two messages
    /// written back to back arrive as one read.
    #[test]
    fn two_messages_in_one_read_are_still_two_messages() {
        let (mut a, mut b) = pair();
        a.send(b"one").expect("send");
        a.send(b"two").expect("send");
        assert!(b.fill().expect("fill"));
        assert_eq!(b.take().expect("take").as_deref(), Some(&b"one"[..]));
        assert_eq!(b.take().expect("take").as_deref(), Some(&b"two"[..]));
        assert_eq!(b.take().expect("take"), None);
    }

    #[test]
    fn a_message_split_across_reads_is_not_reported_until_it_is_whole() {
        let (mut a, mut b) = pair();
        let big = vec![7u8; 200 * 1024];
        std::thread::spawn(move || a.send(&big).expect("send"));
        let mut seen = None;
        for _ in 0..1000 {
            if let Some(message) = b.take().expect("take") {
                seen = Some(message.len());
                break;
            }
            assert!(b.fill().expect("fill"));
        }
        assert_eq!(seen, Some(200 * 1024));
    }

    #[test]
    fn a_peer_that_went_away_is_the_end_rather_than_an_error() {
        let (a, mut b) = pair();
        drop(a);
        assert_eq!(b.recv().expect("recv"), None);
    }

    /// A read asked in the middle of a key handler can be answered after the frame loop's
    /// own next message. What it set aside has to come back in the order it arrived.
    #[test]
    fn a_deferred_message_comes_back_before_the_socket_is_read_again() {
        let (mut a, mut b) = pair();
        a.send(b"later").expect("send");
        assert_eq!(b.recv().expect("recv").as_deref(), Some(&b"later"[..]));
        b.defer(b"later".to_vec());
        b.defer(b"and then".to_vec());
        a.send(b"newest").expect("send");

        assert_eq!(b.recv().expect("recv").as_deref(), Some(&b"later"[..]));
        assert_eq!(b.recv().expect("recv").as_deref(), Some(&b"and then"[..]));
        assert_eq!(b.recv().expect("recv").as_deref(), Some(&b"newest"[..]));
    }

    #[test]
    fn an_empty_message_is_a_message() {
        let (mut a, mut b) = pair();
        a.send(b"").expect("send");
        assert_eq!(b.recv().expect("recv").as_deref(), Some(&b""[..]));
    }
}
