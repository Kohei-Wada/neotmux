use crate::ServerOp;
use neotmux_proto::PaneId;
use neotmux_proto::wire::Tock;
use std::collections::BTreeMap;

/// One frame's worth of answer from the plugins: what the session has to apply, what the
/// *server* has to apply, and which panes this frame shows at a place in their own history.
pub struct Answer {
    pub tock: Tock,
    pub server: Vec<(String, ServerOp)>,
    pub views: BTreeMap<PaneId, usize>,
}
