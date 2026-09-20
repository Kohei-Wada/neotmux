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
}
