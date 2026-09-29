use std::collections::BTreeMap;
use std::rc::Rc;

use neotmux_layout::PaneIds;
use neotmux_proto::wire::{Done, End, ToClient};
use neotmux_proto::{SessionId, Size, wire};
use neotmux_pty::PaneTerminals;
use neotmux_pty::child::Waker;
use neotmux_sys::Link;

use crate::{Plugins, Session, Setup};

/// One client the frame reaches, and everything the server has to remember about it. Which
/// is very little: the picture is the session's, one Lua layer's, so every client attached
/// is shown the same one and there is nothing per client to compare it against.
pub(crate) struct Attached {
    pub(crate) link: Link,
    /// The session this client is looking at. It moves between them without its connection
    /// being touched, which is what `session.switch` is.
    pub(crate) session: SessionId,
    /// How big its terminal is, so a session it switches to can be refitted to it — the same
    /// `window-size latest` rule an attach follows.
    pub(crate) size: Size,
    /// The client has gone, in this frame. Recorded rather than dropped, because the frame's
    /// slot is still a slot the wait is waiting over, and dropping it out from under a wait
    /// is what this flag prevents.
    pub(crate) gone: bool,
    /// A key it sent raised an op, and what it sent after waits for the frame that carries the
    /// op out: typed straight after a switch, it is the new session's, not the old one's.
    pub(crate) held: bool,
}

/// Every session this server holds, and the ids still to come.
///
/// One entry today — the map is the shape the rest of them go in, and everything that used to
/// say "the session" says "this one" through it.
#[derive(Default)]
pub(crate) struct Sessions {
    pub(crate) live: BTreeMap<SessionId, Held>,
    next: u32,
    /// Handed to every session this server makes. A pane's terminal state is looked up by id
    /// out of one table for the whole server, so two sessions numbering their panes apart
    /// would have them sharing a shell — and ending one would close the other's.
    pub(crate) panes: PaneIds,
}

/// One session and the name it answers to.
pub(crate) struct Held {
    pub(crate) name: String,
    pub(crate) session: Session,
}

impl Sessions {
    /// The id a new session takes. Never recycled, so an id names one session for the whole
    /// life of the server.
    pub(crate) fn mint(&mut self) -> SessionId {
        self.next += 1;
        SessionId::from(self.next)
    }

    pub(crate) fn id_of(&self, name: &str) -> Option<SessionId> {
        self.live
            .iter()
            .find(|(_, held)| held.name == name)
            .map(|(id, _)| *id)
    }

    /// Every session's name, in name order, which is the order `ls` prints them in.
    /// What a session asked for by no name is called: the lowest number no running one is,
    /// as tmux numbers them. `rename` gives it a word when it wants one.
    pub(crate) fn next_number(&self) -> String {
        let names = self.names();
        (0..)
            .map(|n: u32| n.to_string())
            .find(|n| !names.contains(n))
            .expect("fewer sessions than numbers")
    }

    pub(crate) fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.live.values().map(|held| held.name.clone()).collect();
        names.sort();
        names
    }

    /// Every session's id. Pane ids are the server's, not a session's, so finding one pane
    /// means walking every session rather than asking the one a command was addressed to.
    pub(crate) fn ids(&self) -> Vec<SessionId> {
        self.live.keys().copied().collect()
    }

    pub(crate) fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.live.get_mut(&id).map(|held| &mut held.session)
    }
}

/// Why [`start_session`] refused. A plugin's log line and a command's `Ran::failed` have
/// never shared a sentence, so the reason is handed back distinguished rather than already
/// formatted, and each caller says it its own way.
pub(crate) enum StartFailure {
    Invalid(String),
    Taken,
    Failed(anyhow::Error),
}

/// A session started with no client looking at it: a plugin's `session.create`, or the `new`
/// host command. The name must be a word nobody else is already using, and past that it is
/// the same session an attach would have made.
pub(crate) fn start_session(
    name: &str,
    size: Size,
    plugins: &Plugins,
    sessions: &mut Sessions,
    terminals: &Rc<PaneTerminals>,
    waker: &Waker,
) -> Result<(), StartFailure> {
    if let Err(why) = neotmux_sys::sock::validate_session_name(name) {
        return Err(StartFailure::Invalid(why));
    }
    if sessions.id_of(name).is_some() {
        return Err(StartFailure::Taken);
    }
    let made = Session::new(
        &setup_for(plugins, size),
        waker.clone(),
        terminals.clone(),
        sessions.panes.clone(),
    );
    match made {
        Ok(session) => {
            let id = sessions.mint();
            sessions.live.insert(
                id,
                Held {
                    name: name.to_string(),
                    session,
                },
            );
            plugins.lua().name_session(id, name);
            Ok(())
        }
        Err(err) => Err(StartFailure::Failed(err)),
    }
}

/// Why [`rename_session`] refused. See [`StartFailure`] for why this is not already a
/// sentence.
pub(crate) enum RenameFailure {
    Invalid(String),
    Taken,
    Missing,
}

/// A session's name changes: a plugin's `session.rename`, or the `rename` host command. The
/// new name must be a word, and one nobody else is already using.
pub(crate) fn rename_session(
    from: &str,
    to: &str,
    plugins: &Plugins,
    sessions: &mut Sessions,
) -> Result<(), RenameFailure> {
    if let Err(why) = neotmux_sys::sock::validate_session_name(to) {
        return Err(RenameFailure::Invalid(why));
    }
    let Some(id) = sessions.id_of(from) else {
        return Err(RenameFailure::Missing);
    };
    if sessions.id_of(to).is_some() {
        return Err(RenameFailure::Taken);
    }
    if let Some(held) = sessions.live.get_mut(&id) {
        held.name = to.to_string();
    }
    // The Lua layer keeps its own copy, for `session.current()` and for the list a picker
    // draws.
    plugins.lua().name_session(id, to);
    Ok(())
}

/// Clients moved to the session named `to`: every one of them where `of` is `None`, or only
/// those looking at `of`'s session where it names one. A plugin's `session.switch` moves the
/// client that asked, so it passes its own session as `of`; the `switch` host command moves
/// whoever is attached to this server, which is what "bring me to that session" means from
/// outside, so it passes `None`.
///
/// Returns how many clients moved. The move itself cannot fail — a client's `session` field is
/// just data — but refitting `to` to whoever just arrived can, the same as an attach's can, so
/// that failure is handed back rather than logged here: a plugin's log line and a command's
/// `Ran::failed` have never shared a sentence, the same reason [`StartFailure`] and
/// [`RenameFailure`] are not already formatted, and each caller says it its own way.
pub(crate) fn switch_clients(
    to: SessionId,
    of: Option<SessionId>,
    clients: &mut [Attached],
    sessions: &mut Sessions,
    plugins: &Plugins,
) -> anyhow::Result<usize> {
    let mut moved = 0usize;
    let mut last_size = None;
    for client in clients
        .iter_mut()
        .filter(|client| of.is_none_or(|from| client.session == from))
    {
        client.session = to;
        last_size = Some(client.size);
        moved += 1;
    }
    // Refitted to whoever just arrived and dirty by construction, so the next frame is one
    // they are shown — the same thing an attach does, for the same reason.
    if let Some(size) = last_size
        && let Some(session) = sessions.get_mut(to)
    {
        session.adopt(&setup_for(plugins, size))?;
    }
    Ok(moved)
}

/// What a session is started or refitted with, out of the Lua layer and a size.
pub(crate) fn setup_for(plugins: &Plugins, size: Size) -> Setup {
    let lua = plugins.lua();
    Setup {
        size,
        first_window: lua.reserve_window_id(),
        reserved: lua.reserved(),
        gutter: lua.opt().pane_gutter,
        pane_band: lua.pane_band(),
        wants_pane_output: lua.has_subscriber("pane_output"),
    }
}

/// A session ends: its clients are told, its shells go with it — `Shells::drop` closes every
/// child — and the Lua layer forgets what it was holding for it.
pub(crate) fn end_session(
    id: SessionId,
    plugins: &mut Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
) {
    for client in clients.iter_mut().filter(|client| client.session == id) {
        let _ = client
            .link
            .send(&wire::encode_to_client(&ToClient::Done(&Done {
                frame: None,
                end: Some(End::Quit),
            })));
        client.gone = true;
    }
    clients.retain(|client| !client.gone);
    // The Lua layer first: a job's `on_exit` runs in here, and a plugin that asks what its
    // panes are showing has to be answered by the session it is being told about rather than
    // by the hole where it was.
    plugins.lua().forget_session(id);
    // What those `on_exit` callbacks raised is filed under a session that has just had its
    // last frame, so no frame will ever come and take it: drained here, or never.
    plugins.drain_teardown_ops(id);
    sessions.live.remove(&id);
}
