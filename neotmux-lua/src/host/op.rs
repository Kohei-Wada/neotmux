use neotmux_proto::{Dir, Level, PaneId, SplitDir, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Quit,
    /// Run the config again: the host does it after the frame, never inside the callback that
    /// asked, since that callback is running in the VM being reloaded.
    Reload,
    Detach,
    Notify {
        level: Level,
        text: String,
    },
    Log {
        level: Level,
        text: String,
    },
    PaneSplit {
        dir: SplitDir,
    },
    PaneFocus {
        pane: PaneId,
    },
    PaneResize {
        dir: Dir,
        cells: u16,
    },
    /// Zoom the focused pane, or unzoom.
    PaneZoom,
    PaneSend {
        pane: PaneId,
        text: String,
    },
    /// A keystroke, named the way `on_key` names one. The bytes it becomes are the host's to
    /// work out: they depend on what the pane has asked for, which is a fact about the pane
    /// rather than about the plugin relaying the key.
    PaneKey {
        pane: PaneId,
        spec: String,
    },
    WindowCreate {
        window: WindowId,
    },
    WindowFocus {
        window: WindowId,
    },
    /// A window's name, or `None` to take it away and leave it a number.
    WindowRename {
        window: WindowId,
        name: Option<String>,
    },
    /// The set of sessions changing, which is the server's rather than a session's: these
    /// three never reach `Session::apply`.
    SessionCreate {
        name: String,
    },
    SessionSwitch {
        name: String,
    },
    SessionKill {
        name: String,
    },
    SessionRename {
        from: String,
        to: String,
    },
    /// Take down every notification showing. The surface is the host's — it holds the ones
    /// still inside their time and the transcript of every one ever raised — so clearing it
    /// is a request like any other.
    NoticesClear,
}
