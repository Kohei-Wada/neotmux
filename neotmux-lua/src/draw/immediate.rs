use std::collections::BTreeMap;

use neotmux_proto::DrawCmd;

use crate::draw::FloatId;

/// What the sources drew with `ctx:*` in one session's tick, kept apart from the buffers so a
/// quiet frame can re-project those alone. The committed drawing is one map for the whole
/// VM, left under whichever session ticked last, so it cannot be replayed later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Immediate {
    pub(crate) plain: Vec<DrawCmd>,
    pub(crate) floats: BTreeMap<FloatId, Vec<DrawCmd>>,
}
