use neotmux_core::proto::{PaneId, Rect as PRect, Reserved, Size, WorkspaceId};

use crate::panes::PaneLayout;

/// Everything a pane tree has to be laid out against: the whole terminal, the bands taken off
/// its edges, and the cells left between panes. One value because every workspace is assigned
/// onto the same one, and a workspace that was hidden while it changed has to be told all
/// three at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub size: Size,
    pub reserved: Reserved,
    pub gutter: u16,
}

/// What closing a pane did.
pub struct Closed {
    pub workspace: WorkspaceId,
    /// The pane was the last of its workspace, so the workspace went with it.
    pub workspace_closed: bool,
    /// The workspace now being shown, when closing changed which one that is.
    pub switched_to: Option<WorkspaceId>,
}

/// What `show` did, which decides whether anything is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Switched,
    AlreadyCurrent,
    NoSuchWorkspace,
}

struct Workspace {
    id: WorkspaceId,
    layout: PaneLayout,
}

/// Every pane tree there is, and which one is being shown.
///
/// A workspace that is not current has no surface: its layout keeps the rects it was last
/// assigned, and is resurfaced only when it becomes current. That is what keeps a hidden
/// workspace off the frame's bill — a terminal resize costs one reassignment, not one per
/// workspace — and it is why `show` takes the surface rather than reading one.
pub struct Workspaces {
    entries: Vec<Workspace>,
    current: usize,
    /// Handed out across every workspace, never reused. Two trees numbering their panes
    /// apart would make `pane.focus(id)` name two panes, and a hidden pane is still a pane.
    next_pane: u32,
}

impl Workspaces {
    /// The session's first workspace, with its first pane.
    pub fn new(id: WorkspaceId, surface: Surface) -> (Self, PaneId) {
        let mut workspaces = Self {
            entries: Vec::new(),
            current: 0,
            next_pane: 1,
        };
        let first = workspaces.push(id, surface);
        (workspaces, first)
    }

    /// A new workspace with one pane, appended after the others. It is not shown: `show` is
    /// a separate call, so `focus(create())` is what a plugin writes and `create()` alone
    /// leaves a workspace running out of sight.
    pub fn create(&mut self, id: WorkspaceId, surface: Surface) -> PaneId {
        self.push(id, surface)
    }

    fn push(&mut self, id: WorkspaceId, surface: Surface) -> PaneId {
        let pane = self.take_pane_id();
        self.entries.push(Workspace {
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

    /// The workspace ids, in creation order.
    pub fn ids(&self) -> Vec<WorkspaceId> {
        self.entries.iter().map(|entry| entry.id).collect()
    }

    pub fn current_id(&self) -> WorkspaceId {
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
    pub fn show(&mut self, id: WorkspaceId, surface: Surface) -> Shown {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Shown::NoSuchWorkspace;
        };
        if index == self.current {
            return Shown::AlreadyCurrent;
        }
        self.current = index;
        self.resurface(surface);
        Shown::Switched
    }

    /// Where a pane sits, in whichever workspace holds it. A hidden pane's rect is the one it
    /// was last assigned, which is the size its terminal still has.
    pub fn rect(&self, pane: PaneId) -> Option<PRect> {
        self.entries
            .iter()
            .find_map(|entry| entry.layout.rects.get(&pane).copied())
    }

    /// The current workspace only: the hidden ones are caught up when they are next shown.
    pub fn resurface(&mut self, surface: Surface) {
        self.entries[self.current]
            .layout
            .resurface(surface.size, surface.reserved, surface.gutter);
    }

    /// Closes `pane` wherever it lives, hidden workspaces included. `None` for a pane no
    /// workspace holds.
    pub fn close_pane(&mut self, pane: PaneId, surface: Surface) -> Option<Closed> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.layout.rects.contains_key(&pane))?;
        let workspace = self.entries[index].id;
        if self.entries[index].layout.close(pane).is_some() {
            return Some(Closed {
                workspace,
                workspace_closed: false,
                switched_to: None,
            });
        }

        // The last pane of a workspace closing closes the workspace. Emptying the last one
        // leaves nothing to show, and the caller ends the session on it.
        self.entries.remove(index);
        let switched_to = self.settle_after_removing(index, surface);
        Some(Closed {
            workspace,
            workspace_closed: true,
            switched_to,
        })
    }

    /// Keeps `current` pointing at the same workspace it did before `index` was removed, and
    /// moves it to a neighbour when that was the one removed.
    fn settle_after_removing(&mut self, index: usize, surface: Surface) -> Option<WorkspaceId> {
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
        // The workspace being shown is the one that went: the next in creation order takes
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

    fn ws(id: u32) -> WorkspaceId {
        WorkspaceId::from(id)
    }

    fn workspaces() -> Workspaces {
        Workspaces::new(ws(1), surface()).0
    }

    #[test]
    fn the_session_starts_with_one_workspace_holding_one_pane() {
        let (workspaces, first) = Workspaces::new(ws(1), surface());
        assert_eq!(workspaces.ids(), vec![ws(1)]);
        assert_eq!(workspaces.current_id(), ws(1));
        assert_eq!(workspaces.current().focus(), Some(first));
    }

    #[test]
    fn pane_ids_do_not_collide_across_workspaces() {
        let (mut workspaces, first) = Workspaces::new(ws(1), surface());
        let split = workspaces.take_pane_id();
        workspaces
            .current_mut()
            .split(SplitDir::Vertical, split)
            .expect("fits");
        let second = workspaces.create(ws(2), surface());

        let mut ids = vec![first, split, second];
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "an id was handed out twice");
    }

    #[test]
    fn creating_a_workspace_does_not_show_it() {
        let mut workspaces = workspaces();
        workspaces.create(ws(2), surface());
        assert_eq!(workspaces.ids(), vec![ws(1), ws(2)]);
        assert_eq!(workspaces.current_id(), ws(1));
    }

    #[test]
    fn showing_an_unknown_workspace_is_refused() {
        let mut workspaces = workspaces();
        assert_eq!(workspaces.show(ws(9), surface()), Shown::NoSuchWorkspace);
        assert_eq!(workspaces.current_id(), ws(1));
        assert_eq!(workspaces.show(ws(1), surface()), Shown::AlreadyCurrent);
    }

    /// A workspace hidden while the terminal changed has no surface to be told about; it is
    /// caught up on the way in, which is why no resize walks every workspace.
    #[test]
    fn a_workspace_hidden_through_a_resize_is_caught_up_when_it_is_shown() {
        let mut workspaces = workspaces();
        workspaces.create(ws(2), surface());
        let smaller = Surface {
            size: Size { rows: 10, cols: 40 },
            ..surface()
        };
        workspaces.resurface(smaller);

        assert_eq!(workspaces.show(ws(2), smaller), Shown::Switched);
        assert_eq!(workspaces.current().content, smaller.size);
    }

    #[test]
    fn closing_a_pane_with_a_sibling_leaves_the_workspace_alone() {
        let (mut workspaces, first) = Workspaces::new(ws(1), surface());
        let split = workspaces.take_pane_id();
        workspaces
            .current_mut()
            .split(SplitDir::Vertical, split)
            .expect("fits");

        let closed = workspaces.close_pane(split, surface()).expect("known pane");
        assert!(!closed.workspace_closed);
        assert_eq!(closed.workspace, ws(1));
        assert_eq!(workspaces.current().focus(), Some(first));
        assert_eq!(workspaces.ids(), vec![ws(1)]);
    }

    #[test]
    fn closing_the_last_pane_of_the_shown_workspace_closes_it_and_shows_another() {
        let (mut workspaces, first) = Workspaces::new(ws(1), surface());
        workspaces.create(ws(2), surface());

        let closed = workspaces.close_pane(first, surface()).expect("known pane");
        assert!(closed.workspace_closed);
        assert_eq!(closed.workspace, ws(1));
        assert_eq!(closed.switched_to, Some(ws(2)));
        assert_eq!(workspaces.ids(), vec![ws(2)]);
    }

    /// The pane belongs to a workspace nobody is looking at, so what is shown must not move.
    #[test]
    fn closing_a_hidden_workspaces_last_pane_does_not_change_what_is_shown() {
        let mut workspaces = workspaces();
        let hidden = workspaces.create(ws(2), surface());

        let closed = workspaces
            .close_pane(hidden, surface())
            .expect("known pane");
        assert!(closed.workspace_closed);
        assert_eq!(closed.switched_to, None);
        assert_eq!(workspaces.current_id(), ws(1));
    }

    #[test]
    fn closing_the_last_workspace_leaves_nothing() {
        let (mut workspaces, first) = Workspaces::new(ws(1), surface());
        assert!(workspaces.close_pane(first, surface()).is_some());
        assert!(workspaces.is_empty());
    }

    #[test]
    fn closing_a_pane_no_workspace_holds_is_not_a_close() {
        let mut workspaces = workspaces();
        assert!(workspaces.close_pane(PaneId::from(99), surface()).is_none());
    }
}
