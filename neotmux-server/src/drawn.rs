use std::collections::BTreeMap;

use neotmux_proto::screen::Notice;
use neotmux_proto::{DrawCmd, PaneId};

/// What a session's plugins drew the last time they ticked, and the notifications they were
/// shown when they did: kept so that a quiet frame can stand on it instead of ticking again.
#[derive(Debug, Clone, Default)]
pub(crate) struct Drawn {
    pub overlay: Vec<DrawCmd>,
    pub views: BTreeMap<PaneId, usize>,
    pub notices: Vec<Notice>,
}
