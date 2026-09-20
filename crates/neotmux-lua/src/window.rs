use std::cell::Cell;

use neotmux_proto::WindowId;

/// Where window ids come from.
///
/// Pane ids are handed out by the host when it applies a `pane.split`, because nothing needs
/// to name the new pane before it exists. A window is different: `window.create()`
/// answers with an id at once while the window itself is made after the tick (§4.6), so
/// `window.focus(window.create())` — the whole of what `C-b c` is — has to have
/// something to name. Allocating here is what lets the call return before the op is applied.
///
/// Ids are never reused, including one burned by a tick whose ops were discarded: a stale
/// `focus(id)` must not land on a window that came later.
pub struct WindowIds {
    next: Cell<u32>,
}

impl Default for WindowIds {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowIds {
    pub fn new() -> Self {
        Self { next: Cell::new(1) }
    }

    pub fn take(&self) -> WindowId {
        let id = self.next.get();
        self.next.set(id + 1);
        WindowId::from(id)
    }

    /// An id this counter did not hand out — a window that was already there when this
    /// client attached. Ids are never reused, so the counter moves past it and never back.
    pub fn seen(&self, id: WindowId) {
        self.next.set(self.next.get().max(id.get() + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_handed_out_in_order_and_never_repeat() {
        let ids = WindowIds::new();
        let taken: Vec<u32> = (0..3).map(|_| ids.take().get()).collect();
        assert_eq!(taken, vec![1, 2, 3]);
    }

    #[test]
    fn an_id_somebody_else_handed_out_is_never_handed_out_again() {
        let ids = WindowIds::new();
        ids.seen(WindowId::from(7));
        assert_eq!(ids.take(), WindowId::from(8));
        // A lower one changes nothing: the counter only ever moves forwards.
        ids.seen(WindowId::from(2));
        assert_eq!(ids.take(), WindowId::from(9));
    }
}
