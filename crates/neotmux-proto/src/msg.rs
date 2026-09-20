//! The messages two neotmux processes send each other.
//!
//! The split (§11, M3) puts the session — the panes, the layout, the terminals — in a server
//! and the screen in a client, and it puts the **Lua layer on the client**, because a frozen
//! plugin should cost one client rather than everybody's shells. That means the frame loop is
//! a conversation rather than a function call: the server says what happened and what the
//! session looks like ([`Tick`]), the client runs the tick against it and says what its
//! plugins want ([`Tock`]), and the server applies that and answers with the frame.
//!
//! Everything here is data. [`crate::wire`] turns it into bytes; who reads and writes those
//! bytes is the socket's owner's business.

use std::collections::BTreeMap;

use crate::{
    Dir, DrawCmd, Epoch, Event, Geometry, Notice, PaneId, PaneInfo, Reserved, Size, SplitDir,
    WindowId,
};

/// What the client tells the server when it takes the session over.
///
/// The size and the first window id come from the client because both are its own: the
/// terminal is the client's, and window ids are handed out by the Lua layer, which is too
/// (`window.create()` answers with an id before the window exists). A server that already
/// holds a session will ignore `first_window` when detach/attach lands; today there is
/// exactly one attach, and it is the one that starts the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attach {
    pub size: Size,
    pub first_window: WindowId,
    pub reserved: Reserved,
    pub gutter: u16,
    /// Whether anything is subscribed to `pane_output`. Nothing is, almost always, and a
    /// pane's bytes are the one payload big enough that asking first is worth a bool.
    pub wants_pane_output: bool,
}

/// One thing that happened to the session, in the order it happened.
///
/// A batch of events with one geometry attached would be a lie: a `pane_opened` handler must
/// see the pane it is being told about, and a `pane_closed` handler must not see the dead
/// one, so the geometry moves *between* the events exactly as it does when the two halves
/// are one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Geometry(Geometry),
    Bands { surface: Size, applied: Reserved },
    Event(Event),
}

/// The server's half of a frame: what happened, and what the session looks like now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub steps: Vec<Step>,
    pub epoch: Epoch,
    pub geometry: Geometry,
    /// What every pane is running, when the host's own cadence says it is time to resample.
    /// `None` on the frames in between, which is most of them.
    pub pane_info: Option<BTreeMap<PaneId, PaneInfo>>,
    /// Every pane's read stamp, as of this frame.
    ///
    /// It travels with the frame rather than being asked for, because it is the cheap half
    /// of reading a pane — a watcher asks it every tick for every pane and reads the lines
    /// only when it moved — and a question asked that often must not be a round trip. The
    /// price is that it describes the pane as of this frame rather than this instant, which
    /// is what the stamp already promises: it says the screen *may* have changed.
    pub revisions: BTreeMap<PaneId, u64>,
}

/// An op a plugin raised that the session has to apply. `log` and `notify` are not here:
/// they are raised and answered on the same side of the socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOp {
    Quit,
    PaneSplit { dir: SplitDir },
    PaneFocus { pane: PaneId },
    PaneResize { dir: Dir, cells: u16 },
    WindowCreate { window: WindowId },
    WindowFocus { window: WindowId },
}

/// The client's half of a frame: what the tick asked for, and what it drew.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tock {
    /// Each op with the name of the source that raised it, because a refusal is logged
    /// against whoever asked and the source table is on the client.
    pub ops: Vec<(String, SessionOp)>,
    pub overlay: Vec<DrawCmd>,
    pub notices: Vec<Notice>,
    pub reserved: Reserved,
    pub gutter: u16,
    pub wants_pane_output: bool,
}

/// The end of a frame: what applying the tick's reply did to the session, the picture that
/// came of it, and whether that was the last one.
///
/// It is one message and not three because it is one moment. The client may not look at its
/// keyboard between sending a [`Tock`] and reading this: what a key handler sees has to be
/// the session as it is, and between those two points it is not — the ops have been applied
/// at the far end and nothing here knows it yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done<'a> {
    pub steps: Vec<Step>,
    /// `None` when nothing changed and the last frame still stands.
    pub frame: Option<&'a crate::Frame<'a>>,
    pub end: Option<End>,
}

/// What a decoded [`Done`] is held in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedDone {
    pub steps: Vec<Step>,
    pub frame: Option<crate::OwnedFrame>,
    pub end: Option<End>,
}

/// Why the session is over. A signal is neither: it ends a process, not the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Quit,
    Eof,
}

/// A question the Lua layer asks about a pane's text. The panes are the server's, so these
/// are asked across the socket — and only from inside a tick, where the server is already
/// waiting for the answer to come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    Revision {
        pane: PaneId,
    },
    Content {
        pane: PaneId,
        last: Option<u64>,
        pad: bool,
        wrapped: bool,
    },
    HistoryExtent {
        pane: PaneId,
    },
    History {
        pane: PaneId,
        from: u64,
        count: u64,
        pad: bool,
        wrapped: bool,
    },
    Freeze {
        pane: PaneId,
    },
    Unfreeze {
        pane: PaneId,
    },
    IsFrozen {
        pane: PaneId,
    },
}

/// One read of a pane's text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lines {
    pub lines: Vec<String>,
    /// Whether each line runs on into the next — `None` where it was not asked for.
    pub wrapped: Option<Vec<bool>>,
    pub revision: u64,
}

/// Where a pane's history begins and how long it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Extent {
    pub len: u64,
    pub dropped: u64,
}

/// The answer to a [`Read`]. `None` throughout is a pane that does not exist, which is not
/// an error: a shell that exited between two passes of a watcher's loop is not the caller's
/// mistake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Revision(Option<u64>),
    Text(Option<Lines>),
    Extent(Option<Extent>),
    /// `freeze` and `unfreeze`, which answer whether the pane was there.
    Did(bool),
    /// `frozen`, which answers `nil` for a pane that is not there.
    Frozen(Option<bool>),
}

/// Client to server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToServer {
    Attach(Attach),
    /// A key nothing on the client consumed, for whichever pane has the focus.
    Key(crate::Key),
    /// The terminal changed size. The client owns the terminal, so it owns `SIGWINCH`.
    Resize(Size),
    Read(Read),
    Tock(Tock),
}

/// Server to client. Borrowed, because the two big payloads — a frame's grids and a tick's
/// geometry — are held by the session and encoding writes straight through the borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToClient<'a> {
    Tick(&'a Tick),
    Done(&'a Done<'a>),
    Reading(&'a Reading),
    /// Why this connection is not getting a session: something else already has it.
    Refused(&'a str),
}

/// What a decoded [`ToClient`] is held in, for the same reason [`crate::OwnedFrame`] exists:
/// the far end has nothing to borrow from until it has decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedToClient {
    Tick(Tick),
    Done(OwnedDone),
    Reading(Reading),
    Refused(String),
}
