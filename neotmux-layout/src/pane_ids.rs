use neotmux_proto::PaneId;

/// Pane ids, handed out and never reused.
///
/// Shared rather than owned, because the reason two windows must not number their panes apart
/// is the reason two *sessions* must not either: one server holds them all, a pane's terminal
/// state is looked up by id, and `pane.focus(id)` naming two panes is the same bug whichever
/// two trees it is spread over.
#[derive(Clone, Default)]
pub struct PaneIds(std::rc::Rc<std::cell::Cell<u32>>);

impl PaneIds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&self) -> PaneId {
        let next = self.0.get() + 1;
        self.0.set(next);
        PaneId::from(next)
    }
}
