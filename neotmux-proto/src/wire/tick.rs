use crate::wire::Step;
use crate::{Epoch, Geometry, PaneId, PaneInfo, PaneTty};
use std::collections::BTreeMap;

/// The server's half of a frame: what happened, and what the session looks like now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub steps: Vec<Step>,
    pub epoch: Epoch,
    pub geometry: Geometry,
    /// What every pane is running, when the host's own cadence says it is time to resample.
    /// `None` on the frames in between, which is most of them.
    pub pane_info: Option<BTreeMap<PaneId, PaneInfo>>,
    /// Where every pane's cursor is and what mode its program has the terminal in, as of this
    /// frame. Not optional the way `pane_info` is: it is cheap enough to send every time, and
    /// a plugin answering a keystroke cannot use a stale one.
    pub pane_tty: BTreeMap<PaneId, PaneTty>,
    /// Every pane's read stamp, as of this frame.
    ///
    /// It travels with the frame rather than being asked for, because it is the cheap half
    /// of reading a pane — a watcher asks it every tick for every pane and reads the lines
    /// only when it moved — and a question asked that often must not be a round trip. The
    /// price is that it describes the pane as of this frame rather than this instant, which
    /// is what the stamp already promises: it says the screen *may* have changed.
    pub revisions: BTreeMap<PaneId, u64>,
}
