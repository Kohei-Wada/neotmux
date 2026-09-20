use neotmux_core::proto::{PaneId, Rect as PRect, Reserved, Size, WindowId};

use crate::panes::PaneLayout;

/// Everything a pane tree has to be laid out against: the whole terminal, the bands taken off
/// its edges, and the cells left between panes. One value because every window is assigned
/// onto the same one, and a window that was hidden while it changed has to be told all
/// three at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub size: Size,
    pub reserved: Reserved,
    pub gutter: u16,
}

/// What closing a pane did.
pub struct Closed {
    pub window: WindowId,
    /// The pane was the last of its window, so the window went with it.
    pub window_closed: bool,
    /// The window now being shown, when closing changed which one that is.
    pub switched_to: Option<WindowId>,
}

/// What `show` did, which decides whether anything is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Switched,
    AlreadyCurrent,
    NoSuchWindow,
}

struct Window {
    id: WindowId,
    layout: PaneLayout,
}

/// Every pane tree there is, and which one is being shown.
///
/// A window that is not current has no surface: its layout keeps the rects it was last
/// assigned, and is resurfaced only when it becomes current. That is what keeps a hidden
/// window off the frame's bill — a terminal resize costs one reassignment, not one per
/// window — and it is why `show` takes the surface rather than reading one.
pub struct Windows {
    entries: Vec<Window>,
    current: usize,
    /// Handed out across every window, never reused. Two trees numbering their panes
    /// apart would make `pane.focus(id)` name two panes, and a hidden pane is still a pane.
    next_pane: u32,
}

impl Windows {
    /// The session's first window, with its first pane.
    pub fn new(id: WindowId, surface: Surface) -> (Self, PaneId) {
        let mut windows = Self {
            entries: Vec::new(),
            current: 0,
            next_pane: 1,
        };
        let first = windows.push(id, surface);
        (windows, first)
    }

    /// A new window with one pane, appended after the others. It is not shown: `show` is
    /// a separate call, so `focus(create())` is what a plugin writes and `create()` alone
    /// leaves a window running out of sight.
    pub fn create(&mut self, id: WindowId, surface: Surface) -> PaneId {
        self.push(id, surface)
    }

    fn push(&mut self, id: WindowId, surface: Surface) -> PaneId {
        let pane = self.take_pane_id();
        self.entries.push(Window {
            id,
            layout: PaneLayout::new(surface.size, surface.reserved, surface.gutter, pane),
        });
        pane
    }

    pub fn take_pane_id(&mut self) -> PaneId {
        let pane = PaneId::from(self.next_pane);
        self.next_pane += 1;
        pane
    }

    /// Every window in creation order, with its panes in layout order. A hidden window's
    /// order is the one it was last assigned, so it is the arrangement it will be shown in
    /// and not a different answer from the one `pane.list` gives once it is.
    pub fn panes(&self) -> Vec<(WindowId, Vec<PaneId>)> {
        self.entries
            .iter()
            .map(|entry| {
                let panes = entry.layout.panes().into_iter().map(|(id, _)| id).collect();
                (entry.id, panes)
            })
            .collect()
    }

    pub fn current_id(&self) -> WindowId {
        self.entries[self.current].id
    }

    pub fn current(&self) -> &PaneLayout {
        &self.entries[self.current].layout
    }

    pub fn current_mut(&mut self) -> &mut PaneLayout {
        &mut self.entries[self.current].layout
    }

    /// Shows `id`, catching its panes up with a surface that may have changed while they
    /// were hidden.
    pub fn show(&mut self, id: WindowId, surface: Surface) -> Shown {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Shown::NoSuchWindow;
        };
        if index == self.current {
            return Shown::AlreadyCurrent;
        }
        self.current = index;
        self.resurface(surface);
        Shown::Switched
    }

    /// Where a pane sits, in whichever window holds it. A hidden pane's rect is the one it
    /// was last assigned, which is the size its terminal still has.
    pub fn rect(&self, pane: PaneId) -> Option<PRect> {
        self.entries
            .iter()
            .find_map(|entry| entry.layout.rects.get(&pane).copied())
    }

    /// The current window only: the hidden ones are caught up when they are next shown.
    pub fn resurface(&mut self, surface: Surface) {
        self.entries[self.current]
            .layout
            .resurface(surface.size, surface.reserved, surface.gutter);
    }

    /// Closes `pane` wherever it lives, hidden windows included. `None` for a pane no
    /// window holds.
    pub fn close_pane(&mut self, pane: PaneId, surface: Surface) -> Option<Closed> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.layout.rects.contains_key(&pane))?;
        let window = self.entries[index].id;
        if self.entries[index].layout.close(pane).is_some() {
            return Some(Closed {
                window,
                window_closed: false,
                switched_to: None,
            });
        }

        // The last pane of a window closing closes the window. Emptying the last one
        // leaves nothing to show, and the caller ends the session on it.
        self.entries.remove(index);
        let switched_to = self.settle_after_removing(index, surface);
        Some(Closed {
            window,
            window_closed: true,
            switched_to,
        })
    }

    /// Keeps `current` pointing at the same window it did before `index` was removed, and
    /// moves it to a neighbour when that was the one removed.
    fn settle_after_removing(&mut self, index: usize, surface: Surface) -> Option<WindowId> {
        if self.entries.is_empty() {
            self.current = 0;
            return None;
        }
        if index > self.current {
            return None;
        }
        if index < self.current {
            self.current -= 1;
            return None;
        }
        // The window being shown is the one that went: the next in creation order takes
        // its place, or the last one when it was the last.
        self.current = index.min(self.entries.len() - 1);
        self.resurface(surface);
        Some(self.current_id())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use neotmux_core::proto::SplitDir;

    use super::*;

    fn surface() -> Surface {
        Surface {
            size: Size { rows: 24, cols: 80 },
            reserved: Reserved::default(),
            gutter: 0,
        }
    }

    fn ws(id: u32) -> WindowId {
        WindowId::from(id)
    }

    fn ids(windows: &Windows) -> Vec<WindowId> {
        windows.panes().into_iter().map(|(id, _)| id).collect()
    }

    fn windows() -> Windows {
        Windows::new(ws(1), surface()).0
    }

    #[test]
    fn the_session_starts_with_one_window_holding_one_pane() {
        let (windows, first) = Windows::new(ws(1), surface());
        assert_eq!(ids(&windows), vec![ws(1)]);
        assert_eq!(windows.current_id(), ws(1));
        assert_eq!(windows.current().focus(), Some(first));
    }

    #[test]
    fn pane_ids_do_not_collide_across_windows() {
        let (mut windows, first) = Windows::new(ws(1), surface());
        let split = windows.take_pane_id();
        windows
            .current_mut()
            .split(SplitDir::Vertical, split)
            .expect("fits");
        let second = windows.create(ws(2), surface());

        let mut ids = vec![first, split, second];
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "an id was handed out twice");
    }

    #[test]
    fn creating_a_window_does_not_show_it() {
        let mut windows = windows();
        windows.create(ws(2), surface());
        assert_eq!(ids(&windows), vec![ws(1), ws(2)]);
        assert_eq!(windows.current_id(), ws(1));
    }

    /// What `window.panes` is built on: a window that is not being shown still reports the
    /// panes it holds, in the order they are laid out.
    #[test]
    fn a_hidden_windows_panes_are_reported_in_layout_order() {
        let (mut windows, first) = Windows::new(ws(1), surface());
        let hidden = windows.create(ws(2), surface());
        let split = windows.take_pane_id();
        windows.show(ws(2), surface());
        windows
            .current_mut()
            .split(SplitDir::Vertical, split)
            .expect("fits");
        windows.show(ws(1), surface());

        assert_eq!(
            windows.panes(),
            vec![(ws(1), vec![first]), (ws(2), vec![hidden, split])]
        );
    }

    #[test]
    fn showing_an_unknown_window_is_refused() {
        let mut windows = windows();
        assert_eq!(windows.show(ws(9), surface()), Shown::NoSuchWindow);
        assert_eq!(windows.current_id(), ws(1));
        assert_eq!(windows.show(ws(1), surface()), Shown::AlreadyCurrent);
    }

    /// A window hidden while the terminal changed has no surface to be told about; it is
    /// caught up on the way in, which is why no resize walks every window.
    #[test]
    fn a_window_hidden_through_a_resize_is_caught_up_when_it_is_shown() {
        let mut windows = windows();
        windows.create(ws(2), surface());
        let smaller = Surface {
            size: Size { rows: 10, cols: 40 },
            ..surface()
        };
        windows.resurface(smaller);

        assert_eq!(windows.show(ws(2), smaller), Shown::Switched);
        assert_eq!(windows.current().content, smaller.size);
    }

    #[test]
    fn closing_a_pane_with_a_sibling_leaves_the_window_alone() {
        let (mut windows, first) = Windows::new(ws(1), surface());
        let split = windows.take_pane_id();
        windows
            .current_mut()
            .split(SplitDir::Vertical, split)
            .expect("fits");

        let closed = windows.close_pane(split, surface()).expect("known pane");
        assert!(!closed.window_closed);
        assert_eq!(closed.window, ws(1));
        assert_eq!(windows.current().focus(), Some(first));
        assert_eq!(ids(&windows), vec![ws(1)]);
    }

    #[test]
    fn closing_the_last_pane_of_the_shown_window_closes_it_and_shows_another() {
        let (mut windows, first) = Windows::new(ws(1), surface());
        windows.create(ws(2), surface());

        let closed = windows.close_pane(first, surface()).expect("known pane");
        assert!(closed.window_closed);
        assert_eq!(closed.window, ws(1));
        assert_eq!(closed.switched_to, Some(ws(2)));
        assert_eq!(ids(&windows), vec![ws(2)]);
    }

    /// The pane belongs to a window nobody is looking at, so what is shown must not move.
    #[test]
    fn closing_a_hidden_windows_last_pane_does_not_change_what_is_shown() {
        let mut windows = windows();
        let hidden = windows.create(ws(2), surface());

        let closed = windows.close_pane(hidden, surface()).expect("known pane");
        assert!(closed.window_closed);
        assert_eq!(closed.switched_to, None);
        assert_eq!(windows.current_id(), ws(1));
    }

    #[test]
    fn closing_the_last_window_leaves_nothing() {
        let (mut windows, first) = Windows::new(ws(1), surface());
        assert!(windows.close_pane(first, surface()).is_some());
        assert!(windows.is_empty());
    }

    #[test]
    fn closing_a_pane_no_window_holds_is_not_a_close() {
        let mut windows = windows();
        assert!(windows.close_pane(PaneId::from(99), surface()).is_none());
    }
}
