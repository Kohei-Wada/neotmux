use std::cell::Cell;

use neotmux_core::proto::WorkspaceId;

/// Where workspace ids come from.
///
/// Pane ids are handed out by the host when it applies a `pane.split`, because nothing needs
/// to name the new pane before it exists. A workspace is different: `workspace.create()`
/// answers with an id at once while the workspace itself is made after the tick (§4.6), so
/// `workspace.focus(workspace.create())` — the whole of what `C-b c` is — has to have
/// something to name. Allocating here is what lets the call return before the op is applied.
///
/// Ids are never reused, including one burned by a tick whose ops were discarded: a stale
/// `focus(id)` must not land on a workspace that came later.
pub struct WorkspaceIds {
    next: Cell<u32>,
}

impl Default for WorkspaceIds {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceIds {
    pub fn new() -> Self {
        Self { next: Cell::new(1) }
    }

    pub fn take(&self) -> WorkspaceId {
        let id = self.next.get();
        self.next.set(id + 1);
        WorkspaceId::from(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_handed_out_in_order_and_never_repeat() {
        let ids = WorkspaceIds::new();
        let taken: Vec<u32> = (0..3).map(|_| ids.take().get()).collect();
        assert_eq!(taken, vec![1, 2, 3]);
    }
}
