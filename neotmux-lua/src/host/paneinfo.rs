use std::collections::BTreeMap;

use neotmux_proto::{PaneId, PaneInfo, PaneTty};

/// What each pane is running, as last sampled by the host — not part of `Geometry` because it
/// is published on its own cadence. The tree changes when a pane opens, closes or takes
/// focus; these change when a command starts or a directory does, and reading them costs the
/// host syscalls rather than a field.
///
/// A pane missing from the map is a pane Lua knows nothing about, which is the same answer as
/// an entry whose facts are all unknown — `pane.info` tells the two apart so a plugin need
/// not.
pub(crate) type PaneInfos = BTreeMap<PaneId, PaneInfo>;

/// Where each pane's cursor is and what mode its program has the terminal in, as of this
/// frame. Published every frame rather than on a cadence: both halves are cheap, and a plugin
/// answering a keystroke cannot use last second's answer.
pub(crate) type PaneTtys = BTreeMap<PaneId, PaneTty>;
