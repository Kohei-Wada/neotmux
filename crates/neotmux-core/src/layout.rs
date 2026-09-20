//! The pane tree, and the one function that turns it into rectangles.
//!
//! A leaf is a pane. An internal node is a split with an orientation, exactly two children
//! and the fraction of its rect going to the first child. Geometry lives in [`assign`] and
//! nowhere else in this module, so that a Lua layout callback can later stand in for that
//! one function instead of replacing the tree.

use neotmux_proto::{Dir, PaneId, Rect, Size, SplitDir};
use std::error::Error;
use std::fmt;

/// Why a split did not happen. The tree is unchanged in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRefused {
    /// The target pane is not in this tree.
    UnknownPane,
    /// One of the two children would be smaller than the caller's minimum.
    TooSmall,
}

impl fmt::Display for SplitRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::UnknownPane => "no such pane",
            Self::TooSmall => "the panes would be too small",
        };
        f.write_str(reason)
    }
}

impl Error for SplitRefused {}

/// Why a resize did not happen. The tree is unchanged in every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeRefused {
    /// The target pane is not in this tree.
    UnknownPane,
    /// Nothing is on that side of the pane, so there is no boundary to move.
    NoNeighbour,
    /// One of the two sides would be smaller than the caller's minimum.
    TooSmall,
}

impl fmt::Display for ResizeRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::UnknownPane => "no such pane",
            Self::NoNeighbour => "no neighbour that way",
            Self::TooSmall => "the panes would be too small",
        };
        f.write_str(reason)
    }
}

impl Error for ResizeRefused {}

#[derive(Debug, Clone)]
enum Node {
    Leaf(PaneId),
    Split(Box<Split>),
}

#[derive(Debug, Clone)]
struct Split {
    dir: SplitDir,
    fraction: f32,
    first: Node,
    second: Node,
}

/// A binary split tree over [`PaneId`]s. It owns no PTYs and knows no terminal.
#[derive(Debug, Clone)]
pub struct Tree {
    root: Option<Node>,
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
    /// `gutter` is the same one [`assign`] takes: the split it is about to make spends that
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
    /// `surface`, `min` and `gutter` are [`assign`]'s, for the same reason [`split`] takes
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

/// The one place a rect is cut up. Total at every size: the output rects lie inside
/// `surface`, never overlap, and together with the gutters between them cover all of it,
/// with rounding going to the first child.
///
/// `gutter` is how many cells to leave between the two children of every split. It is a
/// layout parameter and nothing more: this module has no idea why a caller wants a gap, only
/// that no pane's rect may include one.
pub fn assign(tree: &Tree, surface: Rect, gutter: u16) -> Vec<(PaneId, Rect)> {
    let mut out = Vec::new();
    let Some(root) = tree.root.as_ref() else {
        return out;
    };
    let mut stack = vec![(root, surface)];
    while let Some((node, rect)) = stack.pop() {
        match node {
            Node::Leaf(id) => out.push((*id, rect)),
            Node::Split(split) => {
                let (first, second) = divide(split, rect, gutter);
                stack.push((&split.second, second));
                stack.push((&split.first, first));
            }
        }
    }
    out
}

/// How many cells of `rect` the two children of a split have between them. The gap comes off
/// the top, so what is divided is only what the children may have: a rect too small to hold
/// the gutter spends all of it and leaves the children empty, rather than pushing them off
/// the surface.
fn usable(dir: SplitDir, rect: Rect, gutter: u16) -> u16 {
    let along = match dir {
        SplitDir::Vertical => rect.w,
        SplitDir::Horizontal => rect.h,
    };
    along - gutter.min(along)
}

/// What the fraction is worth in cells, with rounding going to the first child. A resize
/// reads its current boundary back through this, so the cells it hands out are the cells
/// [`assign`] will lay down.
fn head(usable: u16, fraction: f32) -> u16 {
    (f32::from(usable) * fraction)
        .round()
        .clamp(0.0, f32::from(usable)) as u16
}

/// One split's share of one rect. [`assign`] is the walk; this is the cut, and nothing else
/// in the tree computes a boundary.
fn divide(split: &Split, rect: Rect, gutter: u16) -> (Rect, Rect) {
    let usable = usable(split.dir, rect, gutter);
    let gap = match split.dir {
        SplitDir::Vertical => rect.w,
        SplitDir::Horizontal => rect.h,
    } - usable;
    let head = head(usable, split.fraction);
    let tail = usable - head;
    match split.dir {
        SplitDir::Vertical => (
            Rect::new(rect.x, rect.y, head, rect.h),
            Rect::new(rect.x + head + gap, rect.y, tail, rect.h),
        ),
        SplitDir::Horizontal => (
            Rect::new(rect.x, rect.y, rect.w, head),
            Rect::new(rect.x, rect.y + head + gap, rect.w, tail),
        ),
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

    const MIN: Size = Size { rows: 2, cols: 2 };

    fn pane(id: u32) -> PaneId {
        PaneId::from(id)
    }

    #[test]
    fn a_lone_pane_fills_the_surface() {
        let tree = Tree::new(pane(1));
        assert_eq!(
            assign(&tree, Rect::new(0, 0, 80, 24), 0),
            vec![(pane(1), Rect::new(0, 0, 80, 24))]
        );
    }

    #[test]
    fn a_vertical_split_halves_the_width() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("80x24 is big enough");
        assert_eq!(
            assign(&tree, surface, 0),
            vec![
                (pane(1), Rect::new(0, 0, 40, 24)),
                (pane(2), Rect::new(40, 0, 40, 24)),
            ]
        );
    }

    #[test]
    fn a_horizontal_split_halves_the_height() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Horizontal, pane(2), surface, MIN, 0)
            .expect("80x24 is big enough");
        assert_eq!(
            assign(&tree, surface, 0),
            vec![
                (pane(1), Rect::new(0, 0, 80, 12)),
                (pane(2), Rect::new(0, 12, 80, 12)),
            ]
        );
    }

    #[test]
    fn an_odd_size_gives_the_extra_cell_to_the_first_child() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 81, 25);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("81x25 is big enough");
        assert_eq!(
            assign(&tree, surface, 0),
            vec![
                (pane(1), Rect::new(0, 0, 41, 25)),
                (pane(2), Rect::new(41, 0, 40, 25)),
            ]
        );

        let mut tree = Tree::new(pane(1));
        tree.split(pane(1), SplitDir::Horizontal, pane(2), surface, MIN, 0)
            .expect("81x25 is big enough");
        assert_eq!(
            assign(&tree, surface, 0),
            vec![
                (pane(1), Rect::new(0, 0, 81, 13)),
                (pane(2), Rect::new(0, 13, 81, 12)),
            ]
        );
    }

    #[test]
    fn a_gutter_goes_between_the_children_and_into_neither() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 1)
            .expect("80x24 is big enough");
        assert_eq!(
            assign(&tree, surface, 1),
            vec![
                (pane(1), Rect::new(0, 0, 40, 24)),
                (pane(2), Rect::new(41, 0, 39, 24)),
            ],
            "column 40 belongs to neither pane"
        );

        let mut tree = Tree::new(pane(1));
        tree.split(pane(1), SplitDir::Horizontal, pane(2), surface, MIN, 1)
            .expect("80x24 is big enough");
        assert_eq!(
            assign(&tree, surface, 1),
            vec![
                (pane(1), Rect::new(0, 0, 80, 12)),
                (pane(2), Rect::new(0, 13, 80, 11)),
            ],
            "row 12 belongs to neither pane"
        );
    }

    #[test]
    fn every_split_spends_its_own_gutter() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 1)
            .expect("fits");
        tree.split(pane(2), SplitDir::Vertical, pane(3), surface, MIN, 1)
            .expect("fits");
        assert_eq!(
            assign(&tree, surface, 1),
            vec![
                (pane(1), Rect::new(0, 0, 40, 24)),
                (pane(2), Rect::new(41, 0, 19, 24)),
                (pane(3), Rect::new(61, 0, 19, 24)),
            ]
        );
    }

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
    fn a_split_keeps_the_surfaces_origin() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(3, 1, 10, 6);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("fits");
        assert_eq!(
            assign(&tree, surface, 0),
            vec![
                (pane(1), Rect::new(3, 1, 5, 6)),
                (pane(2), Rect::new(8, 1, 5, 6)),
            ]
        );
    }

    #[test]
    fn panes_come_out_left_to_right_top_to_bottom() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN, 0)
            .expect("fits");
        tree.split(pane(2), SplitDir::Horizontal, pane(3), surface, MIN, 0)
            .expect("fits");
        assert_eq!(tree.panes(), vec![pane(1), pane(2), pane(3)]);
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

    /// A fixed set of shapes: each entry splits the pane at that index of `panes()`.
    const SHAPES: [&[(usize, SplitDir)]; 6] = [
        &[],
        &[(0, SplitDir::Vertical)],
        &[(0, SplitDir::Horizontal)],
        &[(0, SplitDir::Vertical), (1, SplitDir::Horizontal)],
        &[
            (0, SplitDir::Horizontal),
            (0, SplitDir::Vertical),
            (2, SplitDir::Vertical),
        ],
        &[
            (0, SplitDir::Vertical),
            (0, SplitDir::Horizontal),
            (1, SplitDir::Vertical),
            (3, SplitDir::Horizontal),
            (0, SplitDir::Vertical),
        ],
    ];

    const SIZES: [(u16, u16); 12] = [
        (1, 1),
        (2, 2),
        (3, 2),
        (2, 3),
        (5, 7),
        (7, 5),
        (9, 9),
        (80, 24),
        (81, 25),
        (13, 60),
        (199, 59),
        (200, 60),
    ];

    fn shape(steps: &[(usize, SplitDir)]) -> Tree {
        // Built on a surface no split can be refused on, so the shapes stay independent of
        // the sizes the property then checks. It has to survive the deepest shape either the
        // table or the generator produces, which is seven splits down one side.
        let roomy = Rect::new(0, 0, 256, 256);
        let one_cell = Size { rows: 1, cols: 1 };
        let mut tree = Tree::new(pane(1));
        for (n, (at, dir)) in steps.iter().enumerate() {
            let target = tree.panes()[*at];
            let new = pane(n as u32 + 2);
            tree.split(target, *dir, new, roomy, one_cell, 0)
                .expect("the roomy surface fits every shape");
        }
        tree
    }

    /// The property itself, in one place. A gutter means the rects no longer tile `surface`,
    /// so what is asserted at every size and every gutter is:
    ///
    /// - one rect per pane, and none of them outside the surface;
    /// - no two rects overlap;
    /// - any two non-empty rects are at least `gutter` cells apart, which is the clause that
    ///   says no rect took a gutter cell for itself;
    /// - every cell of the surface is covered, once the rects are grown by `gutter` along the
    ///   right and bottom edges the gutter is reserved on — that is, the rects and the gutters
    ///   together, and nothing else, cover the surface.
    ///
    /// At `gutter == 0` the growth is the identity and the separation clause is vacuous, so
    /// the four reduce to exactly the old "the rects tile the surface exactly".
    fn assert_tiles(tree: &Tree, surface: Rect, gutter: u16, case: &str) {
        let (w, h) = (surface.w, surface.h);
        let rects = assign(tree, surface, gutter);
        assert_eq!(
            rects.len(),
            tree.panes().len(),
            "{case}: every pane gets a rect at {w}x{h} gutter {gutter}"
        );

        let mut covered = vec![false; usize::from(w) * usize::from(h)];
        for (id, r) in &rects {
            assert!(
                r.x + r.w <= surface.x + surface.w && r.y + r.h <= surface.y + surface.h,
                "{case}: {id} at {r:?} leaves the {w}x{h} surface"
            );
            // An empty rect grows too: a split with no room left divides nothing, and the
            // cells its children stand on are the gutter it spent them on.
            let x1 =
                r.x.saturating_add(r.w)
                    .saturating_add(gutter)
                    .min(surface.x + w);
            let y1 =
                r.y.saturating_add(r.h)
                    .saturating_add(gutter)
                    .min(surface.y + h);
            for y in r.y..y1 {
                for x in r.x..x1 {
                    covered[usize::from(y - surface.y) * usize::from(w)
                        + usize::from(x - surface.x)] = true;
                }
            }
        }
        if let Some(missed) = covered.iter().position(|seen| !seen) {
            let (x, y) = (missed % usize::from(w), missed / usize::from(w));
            panic!(
                "{case}: cell ({x}, {y}) of the {w}x{h} surface is neither a pane nor a gutter \
                 cell, gutter {gutter}: {rects:?}"
            );
        }

        for (i, (a_id, a)) in rects.iter().enumerate() {
            for (b_id, b) in &rects[i + 1..] {
                let overlaps =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(
                    !overlaps,
                    "{case}: {a_id} at {a:?} overlaps {b_id} at {b:?}"
                );
                if a.w == 0 || a.h == 0 || b.w == 0 || b.h == 0 {
                    continue;
                }
                let apart = |a0: u16, a1: u16, b0: u16, b1: u16| {
                    b0.saturating_sub(a1).max(a0.saturating_sub(b1))
                };
                let gap = apart(a.x, a.x + a.w, b.x, b.x + b.w).max(apart(
                    a.y,
                    a.y + a.h,
                    b.y,
                    b.y + b.h,
                ));
                assert!(
                    gap >= gutter,
                    "{case}: {a_id} at {a:?} and {b_id} at {b:?} are {gap} cells apart, \
                     less than the gutter of {gutter}"
                );
            }
        }
    }

    /// The gutters the property is checked at: none, one (what a divider costs), and two.
    const GUTTERS: [u16; 3] = [0, 1, 2];

    #[test]
    fn assigned_rects_and_their_gutters_cover_the_surface_exactly() {
        for steps in SHAPES {
            let tree = shape(steps);
            for (w, h) in SIZES {
                for gutter in GUTTERS {
                    assert_tiles(
                        &tree,
                        Rect::new(0, 0, w, h),
                        gutter,
                        &format!("table {steps:?}"),
                    );
                }
            }
        }
    }

    /// A small linear congruential generator, so "a tree of any shape and a rect of any size"
    /// can be a few hundred actual shapes and sizes without a dependency. Deterministic from
    /// a fixed seed: a failure reproduces exactly, and the table above stays the reproducible
    /// core either way. Constants from Knuth's MMIX; the low bits of an LCG are the weak ones,
    /// so only the top half is used.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    const SEED: u64 = 0x5eed_0000_0000_0001;
    const CASES: usize = 400;
    const MAX_SPLITS: u64 = 8;

    #[test]
    fn assigned_rects_cover_a_generated_shape_at_a_generated_size_and_gutter() {
        let mut rng = Lcg(SEED);
        for case in 0..CASES {
            let mut steps = Vec::new();
            for n in 0..rng.below(MAX_SPLITS) {
                let at = rng.below(n + 1) as usize;
                let dir = if rng.below(2) == 0 {
                    SplitDir::Vertical
                } else {
                    SplitDir::Horizontal
                };
                steps.push((at, dir));
            }
            let tree = shape(&steps);
            let surface = Rect::new(0, 0, rng.below(300) as u16 + 1, rng.below(300) as u16 + 1);
            let gutter = rng.below(GUTTERS.len() as u64) as u16;
            assert_tiles(
                &tree,
                surface,
                gutter,
                &format!("seed {SEED:#x} case {case} shape {steps:?}"),
            );
        }
    }

    /// The generator is the point of the property, so it has to actually generate: a seed that
    /// only ever produced one pane on one surface would pass everything above and prove
    /// nothing.
    #[test]
    fn the_generator_reaches_a_range_of_shapes_sizes_and_gutters() {
        let mut rng = Lcg(SEED);
        let mut leaves = std::collections::BTreeSet::new();
        let mut widths = std::collections::BTreeSet::new();
        let mut gutters = std::collections::BTreeSet::new();
        for _ in 0..CASES {
            let mut count = 1u64;
            for n in 0..rng.below(MAX_SPLITS) {
                rng.below(n + 1);
                rng.below(2);
                count += 1;
            }
            leaves.insert(count);
            widths.insert(rng.below(300) as u16 + 1);
            rng.below(300);
            gutters.insert(rng.below(GUTTERS.len() as u64) as u16);
        }
        assert_eq!(
            leaves,
            (1..=MAX_SPLITS).collect(),
            "the generated shapes do not cover every pane count up to the limit"
        );
        assert!(
            widths.len() > CASES / 2,
            "the generated widths barely vary: {} distinct out of {CASES}",
            widths.len()
        );
        assert_eq!(
            gutters,
            GUTTERS.into_iter().collect(),
            "the generated gutters do not cover every gutter the table checks"
        );
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
