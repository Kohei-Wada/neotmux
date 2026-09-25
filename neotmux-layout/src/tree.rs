//! The pane tree. A leaf is a pane. An internal node is a split with an orientation, exactly two
//! children and the fraction of its rect going to the first child. Where each pane lands is not
//! decided here: that is [`assign()`], and nothing else.

use neotmux_proto::{Dir, PaneId, Rect, Size, SplitDir};

use crate::{ResizeRefused, SplitRefused, assign, divide, head, usable};

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Leaf(PaneId),
    Split(Box<Split>),
}

#[derive(Debug, Clone)]
pub(crate) struct Split {
    pub(crate) dir: SplitDir,
    pub(crate) fraction: f32,
    pub(crate) first: Node,
    pub(crate) second: Node,
}

/// A binary split tree over [`PaneId`]s. It owns no PTYs and knows no terminal.
#[derive(Debug, Clone)]
pub struct Tree {
    pub(crate) root: Option<Node>,
}

impl Tree {
    pub fn new(pane: PaneId) -> Self {
        Self {
            root: Some(Node::Leaf(pane)),
        }
    }

    /// The panes left to right, top to bottom.
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            collect(root, &mut out);
        }
        out
    }

    /// Replace `target` with a split of `target` and `new`.
    ///
    /// `min` is the caller's smallest usable pane; the tree has no opinion on what that is,
    /// because it is a property of whatever renders a pane, not of layout.
    ///
    /// `gutter` is the same one [`assign()`] takes: the split it is about to make spends that
    /// many cells, so the refusal has to be decided on the rects the gutter leaves.
    pub fn split(
        &mut self,
        target: PaneId,
        dir: SplitDir,
        new: PaneId,
        surface: Rect,
        min: Size,
        gutter: u16,
    ) -> Result<(), SplitRefused> {
        let root = self.root.as_mut().ok_or(SplitRefused::UnknownPane)?;
        if !insert(root, target, dir, new) {
            return Err(SplitRefused::UnknownPane);
        }
        let fits = assign(self, surface, gutter)
            .iter()
            .filter(|(id, _)| *id == target || *id == new)
            .all(|(_, rect)| rect.w >= min.cols && rect.h >= min.rows);
        if !fits {
            self.close(new);
            return Err(SplitRefused::TooSmall);
        }
        Ok(())
    }

    /// Moves the boundary between `pane` and its neighbour in `dir` by `cells` cells, in
    /// that direction — so the pane grows by exactly that many and its neighbour loses
    /// them. There is no second rule for a pane at the edge of the window: with no
    /// neighbour that way there is no boundary, and the resize is refused.
    ///
    /// The boundary is the *nearest* ancestor split with the right orientation and the pane
    /// on the right side of it. A nested pane can have several matching splits above it, and
    /// the nearest is the one whose boundary is against the pane itself.
    ///
    /// `surface`, `min` and `gutter` are [`assign()`]'s, for the same reason [`split`] takes
    /// them: what the caller asked for is cells, and only the assignment knows what a
    /// fraction is worth in cells.
    ///
    /// [`split`]: Tree::split
    pub fn resize(
        &mut self,
        pane: PaneId,
        dir: Dir,
        cells: u16,
        surface: Rect,
        min: Size,
        gutter: u16,
    ) -> Result<(), ResizeRefused> {
        let root = self.root.as_ref().ok_or(ResizeRefused::UnknownPane)?;
        let mut steps = Vec::new();
        if !descend(root, pane, surface, gutter, &mut steps) {
            return Err(ResizeRefused::UnknownPane);
        }
        let at = steps
            .iter()
            .rposition(|step| {
                step.dir == dir.orientation() && step.into_first == dir.from_first_child()
            })
            .ok_or(ResizeRefused::NoNeighbour)?;
        if cells == 0 {
            return Ok(());
        }

        let taken: Vec<bool> = steps[..at].iter().map(|step| step.into_first).collect();
        let root = self.root.as_mut().ok_or(ResizeRefused::UnknownPane)?;
        let split = split_at(root, &taken).ok_or(ResizeRefused::UnknownPane)?;
        let usable = usable(split.dir, steps[at].rect, gutter);
        // Read back through the same rounding `assign` does rather than off the stored
        // fraction: that is what makes the move exactly `cells` and keeps ten moves of one
        // cell ten cells, however the f32 in between rounds.
        let head = head(usable, split.fraction);
        let moved = if dir.from_first_child() {
            head.checked_add(cells).filter(|head| *head <= usable)
        } else {
            head.checked_sub(cells)
        };
        let Some(moved) = moved else {
            return Err(ResizeRefused::TooSmall);
        };
        let was = split.fraction;
        split.fraction = f32::from(moved) / f32::from(usable);
        let mut affected = Vec::new();
        collect(&split.first, &mut affected);
        collect(&split.second, &mut affected);

        let fits = assign(self, surface, gutter)
            .iter()
            .filter(|(id, _)| affected.contains(id))
            .all(|(_, rect)| rect.w >= min.cols && rect.h >= min.rows);
        if !fits {
            let root = self.root.as_mut().expect("the tree was not emptied");
            if let Some(split) = split_at(root, &taken) {
                split.fraction = was;
            }
            return Err(ResizeRefused::TooSmall);
        }
        Ok(())
    }

    /// Remove `pane`, giving its space to its sibling. Returns the pane focus should move
    /// to, or `None` when that was the last pane and the tree is now empty.
    pub fn close(&mut self, pane: PaneId) -> Option<PaneId> {
        let root = self.root.take()?;
        let (rest, focus) = prune(root, pane);
        self.root = rest;
        focus
    }
}

/// One step of the way down to a pane: the split passed through, the rect *the split* was
/// assigned — which is what its fraction divides — and which child the pane was in.
struct Step {
    dir: SplitDir,
    rect: Rect,
    into_first: bool,
}

/// The path from `node` down to `target`, as the splits crossed to get there. False when the
/// pane is not under this node, in which case `steps` is left as it was found.
fn descend(node: &Node, target: PaneId, rect: Rect, gutter: u16, steps: &mut Vec<Step>) -> bool {
    match node {
        Node::Leaf(id) => *id == target,
        Node::Split(split) => {
            let (first, second) = divide(split, rect, gutter);
            let depth = steps.len();
            for (into_first, child, child_rect) in
                [(true, &split.first, first), (false, &split.second, second)]
            {
                steps.push(Step {
                    dir: split.dir,
                    rect,
                    into_first,
                });
                if descend(child, target, child_rect, gutter, steps) {
                    return true;
                }
                steps.truncate(depth);
            }
            false
        }
    }
}

/// The split reached by taking `steps` from `node`, each step naming the child to go into.
fn split_at<'a>(mut node: &'a mut Node, steps: &[bool]) -> Option<&'a mut Split> {
    for into_first in steps {
        let Node::Split(split) = node else {
            return None;
        };
        node = if *into_first {
            &mut split.first
        } else {
            &mut split.second
        };
    }
    match node {
        Node::Split(split) => Some(split),
        Node::Leaf(_) => None,
    }
}

fn collect(node: &Node, out: &mut Vec<PaneId>) {
    match node {
        Node::Leaf(id) => out.push(*id),
        Node::Split(split) => {
            collect(&split.first, out);
            collect(&split.second, out);
        }
    }
}

fn first_leaf(node: &Node) -> PaneId {
    match node {
        Node::Leaf(id) => *id,
        Node::Split(split) => first_leaf(&split.first),
    }
}

fn insert(node: &mut Node, target: PaneId, dir: SplitDir, new: PaneId) -> bool {
    match node {
        Node::Leaf(id) if *id == target => {
            *node = Node::Split(Box::new(Split {
                dir,
                fraction: 0.5,
                first: Node::Leaf(target),
                second: Node::Leaf(new),
            }));
            true
        }
        Node::Leaf(_) => false,
        Node::Split(split) => {
            insert(&mut split.first, target, dir, new)
                || insert(&mut split.second, target, dir, new)
        }
    }
}

/// Returns the subtree with `pane` gone, and the pane focus should move to.
fn prune(node: Node, pane: PaneId) -> (Option<Node>, Option<PaneId>) {
    match node {
        Node::Leaf(id) if id == pane => (None, None),
        Node::Leaf(id) => (Some(Node::Leaf(id)), None),
        Node::Split(split) => {
            let Split {
                dir,
                fraction,
                first,
                second,
            } = *split;
            let (first, focus) = prune(first, pane);
            let Some(first) = first else {
                let focus = first_leaf(&second);
                return (Some(second), Some(focus));
            };
            if focus.is_some() {
                return (rejoin(dir, fraction, first, second), focus);
            }
            let (second, focus) = prune(second, pane);
            let Some(second) = second else {
                let focus = first_leaf(&first);
                return (Some(first), Some(focus));
            };
            (rejoin(dir, fraction, first, second), focus)
        }
    }
}

fn rejoin(dir: SplitDir, fraction: f32, first: Node, second: Node) -> Option<Node> {
    Some(Node::Split(Box::new(Split {
        dir,
        fraction,
        first,
        second,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;

    #[test]
    fn a_split_the_gutter_leaves_no_room_for_is_refused() {
        // Five columns hold two two-column panes and one gutter; four do not.
        let mut tree = Tree::new(pane(1));
        tree.split(
            pane(1),
            SplitDir::Vertical,
            pane(2),
            Rect::new(0, 0, 5, 24),
            MIN,
            1,
        )
        .expect("five columns hold both panes and the gutter");

        let mut tree = Tree::new(pane(1));
        let tight = Rect::new(0, 0, 4, 24);
        assert_eq!(
            tree.split(pane(1), SplitDir::Vertical, pane(2), tight, MIN, 1),
            Err(SplitRefused::TooSmall),
            "without the gutter four columns would have been enough"
        );
        assert_eq!(tree.panes(), vec![pane(1)]);
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_its_sibling() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("fits");
        assert_eq!(tree.close(pane(1)), Some(pane(2)));
        assert_eq!(assign(&tree, surface, 0), vec![(pane(2), surface)]);
    }

    #[test]
    fn closing_a_pane_focuses_the_siblings_first_leaf() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("fits");
        tree.split(pane(2), SplitDir::Horizontal, pane(3), surface, MIN, 0)
            .expect("fits");
        assert_eq!(tree.close(pane(1)), Some(pane(2)));
        assert_eq!(tree.panes(), vec![pane(2), pane(3)]);
    }

    #[test]
    fn closing_the_last_pane_empties_the_tree() {
        let mut tree = Tree::new(pane(1));
        assert_eq!(tree.close(pane(1)), None);
        assert!(tree.panes().is_empty());
        assert!(assign(&tree, Rect::new(0, 0, 80, 24), 0).is_empty());
    }

    #[test]
    fn closing_an_unknown_pane_changes_nothing() {
        let mut tree = Tree::new(pane(1));
        assert_eq!(tree.close(pane(9)), None);
        assert_eq!(tree.panes(), vec![pane(1)]);
    }

    #[test]
    fn a_surface_of_two_by_two_cannot_be_split() {
        let surface = Rect::new(0, 0, 2, 2);
        for dir in [SplitDir::Vertical, SplitDir::Horizontal] {
            let mut tree = Tree::new(pane(1));
            assert_eq!(
                tree.split(pane(1), dir, pane(2), surface, MIN, 0),
                Err(SplitRefused::TooSmall)
            );
            assert_eq!(assign(&tree, surface, 0), vec![(pane(1), surface)]);
        }
    }

    #[test]
    fn splitting_an_unknown_pane_is_refused() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        assert_eq!(
            tree.split(pane(9), SplitDir::Vertical, pane(2), surface, MIN, 0),
            Err(SplitRefused::UnknownPane)
        );
        assert_eq!(tree.panes(), vec![pane(1)]);
    }

    /// A vertical split of `surface`, then a horizontal split of its right column: pane 1 is
    /// the left column, panes 2 and 3 the top and bottom of the right one.
    fn nested(surface: Rect, gutter: u16) -> Tree {
        let mut tree = Tree::new(pane(1));
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, gutter)
            .expect("fits");
        tree.split(pane(2), SplitDir::Horizontal, pane(3), surface, MIN, gutter)
            .expect("fits");
        tree
    }

    fn rect_of(tree: &Tree, id: PaneId, surface: Rect, gutter: u16) -> Rect {
        assign(tree, surface, gutter)
            .into_iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, rect)| rect)
            .unwrap_or_else(|| panic!("{id} has no rect"))
    }

    /// The whole of what a resize promises: the pane grows by the cells asked for, along the
    /// axis the direction names, and its neighbour pays for them.
    #[test]
    fn a_resize_moves_the_boundary_by_exactly_the_cells_asked() {
        let surface = Rect::new(0, 0, 80, 24);
        for gutter in GUTTERS {
            for (dir, from, orientation) in [
                (Dir::Right, pane(1), SplitDir::Vertical),
                (Dir::Left, pane(2), SplitDir::Vertical),
                (Dir::Down, pane(1), SplitDir::Horizontal),
                (Dir::Up, pane(2), SplitDir::Horizontal),
            ] {
                let mut tree = Tree::new(pane(1));
                tree.split(pane(1), orientation, pane(2), surface, MIN, gutter)
                    .expect("fits");
                let before = rect_of(&tree, from, surface, gutter);
                let other = if from == pane(1) { pane(2) } else { pane(1) };
                let before_other = rect_of(&tree, other, surface, gutter);

                tree.resize(from, dir, 7, surface, MIN, gutter)
                    .expect("there is room for seven cells");

                let after = rect_of(&tree, from, surface, gutter);
                let after_other = rect_of(&tree, other, surface, gutter);
                let (grew, shrank) = match orientation {
                    SplitDir::Vertical => (
                        after.w as i32 - before.w as i32,
                        after_other.w as i32 - before_other.w as i32,
                    ),
                    SplitDir::Horizontal => (
                        after.h as i32 - before.h as i32,
                        after_other.h as i32 - before_other.h as i32,
                    ),
                };
                assert_eq!(grew, 7, "{dir} at gutter {gutter}: {before:?} -> {after:?}");
                assert_eq!(
                    shrank, -7,
                    "{dir} at gutter {gutter}: the neighbour did not pay"
                );
            }
        }
    }

    /// The same, from a pane that is not a child of the root: the boundary moved is the one
    /// against the pane, whichever depth it is at.
    #[test]
    fn a_nested_pane_moves_the_boundary_against_it() {
        let surface = Rect::new(0, 0, 80, 24);
        let mut tree = nested(surface, 1);
        let before = rect_of(&tree, pane(3), surface, 1);
        tree.resize(pane(3), Dir::Left, 6, surface, MIN, 1)
            .expect("there is room");
        let after = rect_of(&tree, pane(3), surface, 1);
        assert_eq!(after.w, before.w + 6);
        assert_eq!(
            after.x,
            before.x - 6,
            "the boundary moved, not the far edge"
        );

        let before = rect_of(&tree, pane(3), surface, 1);
        tree.resize(pane(3), Dir::Up, 4, surface, MIN, 1)
            .expect("there is room");
        let after = rect_of(&tree, pane(3), surface, 1);
        assert_eq!(after.h, before.h + 4);
        assert_eq!(after.y, before.y - 4);
    }

    /// Two splits of the same orientation above the pane. The nearest is the one whose
    /// boundary is against it; taking the outermost instead would move a boundary the user
    /// is nowhere near.
    #[test]
    fn the_boundary_moved_is_the_nearest_one_not_the_outermost() {
        let surface = Rect::new(0, 0, 80, 24);
        // Two vertical splits with pane 1 in the first child of both: the left column is
        // panes 1 and 3, and the whole of it is the first child of the root.
        let mut tree = Tree::new(pane(1));
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("fits");
        tree.split(pane(1), SplitDir::Vertical, pane(3), surface, MIN, 0)
            .expect("fits");
        let right = rect_of(&tree, pane(2), surface, 0);

        tree.resize(pane(1), Dir::Right, 5, surface, MIN, 0)
            .expect("there is room");

        assert_eq!(
            rect_of(&tree, pane(2), surface, 0),
            right,
            "the outer boundary moved, so the split chosen was not the nearest"
        );
        assert_eq!(rect_of(&tree, pane(1), surface, 0).w, 20 + 5);
        assert_eq!(
            rect_of(&tree, pane(3), surface, 0).w,
            20 - 5,
            "the pane on the other side of the nearest boundary did not pay"
        );
    }

    /// Cells in, cells out: the tree stores an f32 fraction, so ten moves of one cell are
    /// where an approximate conversion would show up as nine or eleven.
    #[test]
    fn ten_single_cell_resizes_move_ten_cells() {
        for (w, h) in SIZES.into_iter().filter(|(w, h)| *w >= 40 && *h >= 40) {
            let surface = Rect::new(0, 0, w, h);
            for gutter in GUTTERS {
                let mut tree = nested(surface, gutter);
                let before = rect_of(&tree, pane(3), surface, gutter);
                for step in 0..10 {
                    tree.resize(pane(3), Dir::Left, 1, surface, MIN, gutter)
                        .unwrap_or_else(|why| panic!("step {step} at {w}x{h}: {why}"));
                }
                let after = rect_of(&tree, pane(3), surface, gutter);
                assert_eq!(
                    after.w,
                    before.w + 10,
                    "ten single-cell moves at {w}x{h} gutter {gutter} drifted"
                );
            }
        }
    }

    #[test]
    fn a_resize_that_would_cross_the_minimum_changes_nothing() {
        let surface = Rect::new(0, 0, 80, 24);
        let mut tree = nested(surface, 1);
        let before = assign(&tree, surface, 1);
        // The right column is 39 wide; 38 of them would leave the panes stacked in it one
        // column across, under the two the caller called a pane.
        assert_eq!(
            tree.resize(pane(1), Dir::Right, 38, surface, MIN, 1),
            Err(ResizeRefused::TooSmall)
        );
        assert_eq!(assign(&tree, surface, 1), before);

        assert_eq!(
            tree.resize(pane(1), Dir::Right, 400, surface, MIN, 1),
            Err(ResizeRefused::TooSmall),
            "a move past the far edge is refused, not clamped"
        );
        assert_eq!(assign(&tree, surface, 1), before);
    }

    #[test]
    fn a_pane_with_no_neighbour_that_way_is_refused() {
        let surface = Rect::new(0, 0, 80, 24);
        let mut tree = nested(surface, 0);
        // Pane 1 is the whole left column: nothing is above it, below it, or to its left.
        for dir in [Dir::Left, Dir::Up, Dir::Down] {
            assert_eq!(
                tree.resize(pane(1), dir, 1, surface, MIN, 0),
                Err(ResizeRefused::NoNeighbour),
                "{dir}"
            );
        }
        assert_eq!(
            tree.resize(pane(9), Dir::Left, 1, surface, MIN, 0),
            Err(ResizeRefused::UnknownPane)
        );
        // A lone pane has no boundary at all.
        let mut alone = Tree::new(pane(1));
        assert_eq!(
            alone.resize(pane(1), Dir::Right, 1, surface, MIN, 0),
            Err(ResizeRefused::NoNeighbour)
        );
    }

    /// Whatever the boundary is moved to, the rects still tile the surface: a resize is a
    /// fraction change and nothing else, so the property `assign` is held to must survive it.
    #[test]
    fn a_resized_tree_still_tiles_the_surface() {
        for (w, h) in SIZES.into_iter().filter(|(w, h)| *w >= 40 && *h >= 40) {
            let surface = Rect::new(0, 0, w, h);
            for gutter in GUTTERS {
                let mut tree = nested(surface, gutter);
                tree.resize(pane(3), Dir::Left, 3, surface, MIN, gutter)
                    .expect("there is room");
                tree.resize(pane(3), Dir::Up, 2, surface, MIN, gutter)
                    .expect("there is room");
                assert_tiles(&tree, surface, gutter, &format!("resized {w}x{h}"));
            }
        }
    }
}
