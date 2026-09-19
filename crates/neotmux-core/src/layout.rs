//! The pane tree, and the one function that turns it into rectangles.
//!
//! A leaf is a pane. An internal node is a split with an orientation, exactly two children
//! and the fraction of its rect going to the first child. Geometry lives in [`assign`] and
//! nowhere else in this module, so that a Lua layout callback can later stand in for that
//! one function instead of replacing the tree.

use crate::proto::{PaneId, Rect, Size, SplitDir};
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
    pub fn split(
        &mut self,
        target: PaneId,
        dir: SplitDir,
        new: PaneId,
        surface: Rect,
        min: Size,
    ) -> Result<(), SplitRefused> {
        let root = self.root.as_mut().ok_or(SplitRefused::UnknownPane)?;
        if !insert(root, target, dir, new) {
            return Err(SplitRefused::UnknownPane);
        }
        let fits = assign(self, surface)
            .iter()
            .filter(|(id, _)| *id == target || *id == new)
            .all(|(_, rect)| rect.w >= min.cols && rect.h >= min.rows);
        if !fits {
            self.close(new);
            return Err(SplitRefused::TooSmall);
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

/// The one place a rect is cut up. Total at every size: the output rects tile `surface`
/// exactly, with rounding going to the first child.
pub fn assign(tree: &Tree, surface: Rect) -> Vec<(PaneId, Rect)> {
    let mut out = Vec::new();
    let Some(root) = tree.root.as_ref() else {
        return out;
    };
    let mut stack = vec![(root, surface)];
    while let Some((node, rect)) = stack.pop() {
        match node {
            Node::Leaf(id) => out.push((*id, rect)),
            Node::Split(split) => {
                let along = match split.dir {
                    SplitDir::Vertical => rect.w,
                    SplitDir::Horizontal => rect.h,
                };
                let head = (f32::from(along) * split.fraction)
                    .round()
                    .clamp(0.0, f32::from(along)) as u16;
                let tail = along - head;
                let (first, second) = match split.dir {
                    SplitDir::Vertical => (
                        Rect::new(rect.x, rect.y, head, rect.h),
                        Rect::new(rect.x + head, rect.y, tail, rect.h),
                    ),
                    SplitDir::Horizontal => (
                        Rect::new(rect.x, rect.y, rect.w, head),
                        Rect::new(rect.x, rect.y + head, rect.w, tail),
                    ),
                };
                stack.push((&split.second, second));
                stack.push((&split.first, first));
            }
        }
    }
    out
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
            assign(&tree, Rect::new(0, 0, 80, 24)),
            vec![(pane(1), Rect::new(0, 0, 80, 24))]
        );
    }

    #[test]
    fn a_vertical_split_halves_the_width() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN)
            .expect("80x24 is big enough");
        assert_eq!(
            assign(&tree, surface),
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
        tree.split(pane(1), SplitDir::Horizontal, pane(2), surface, MIN)
            .expect("80x24 is big enough");
        assert_eq!(
            assign(&tree, surface),
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
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN)
            .expect("81x25 is big enough");
        assert_eq!(
            assign(&tree, surface),
            vec![
                (pane(1), Rect::new(0, 0, 41, 25)),
                (pane(2), Rect::new(41, 0, 40, 25)),
            ]
        );

        let mut tree = Tree::new(pane(1));
        tree.split(pane(1), SplitDir::Horizontal, pane(2), surface, MIN)
            .expect("81x25 is big enough");
        assert_eq!(
            assign(&tree, surface),
            vec![
                (pane(1), Rect::new(0, 0, 81, 13)),
                (pane(2), Rect::new(0, 13, 81, 12)),
            ]
        );
    }

    #[test]
    fn a_split_keeps_the_surfaces_origin() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(3, 1, 10, 6);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN)
            .expect("fits");
        assert_eq!(
            assign(&tree, surface),
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
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN)
            .expect("fits");
        tree.split(pane(2), SplitDir::Horizontal, pane(3), surface, MIN)
            .expect("fits");
        assert_eq!(tree.panes(), vec![pane(1), pane(2), pane(3)]);
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_its_sibling() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN)
            .expect("fits");
        assert_eq!(tree.close(pane(1)), Some(pane(2)));
        assert_eq!(assign(&tree, surface), vec![(pane(2), surface)]);
    }

    #[test]
    fn closing_a_pane_focuses_the_siblings_first_leaf() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        tree.split(pane(1), SplitDir::Vertical, pane(2), surface, MIN)
            .expect("fits");
        tree.split(pane(2), SplitDir::Horizontal, pane(3), surface, MIN)
            .expect("fits");
        assert_eq!(tree.close(pane(1)), Some(pane(2)));
        assert_eq!(tree.panes(), vec![pane(2), pane(3)]);
    }

    #[test]
    fn closing_the_last_pane_empties_the_tree() {
        let mut tree = Tree::new(pane(1));
        assert_eq!(tree.close(pane(1)), None);
        assert!(tree.panes().is_empty());
        assert!(assign(&tree, Rect::new(0, 0, 80, 24)).is_empty());
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
                tree.split(pane(1), dir, pane(2), surface, MIN),
                Err(SplitRefused::TooSmall)
            );
            assert_eq!(assign(&tree, surface), vec![(pane(1), surface)]);
        }
    }

    #[test]
    fn splitting_an_unknown_pane_is_refused() {
        let mut tree = Tree::new(pane(1));
        let surface = Rect::new(0, 0, 80, 24);
        assert_eq!(
            tree.split(pane(9), SplitDir::Vertical, pane(2), surface, MIN),
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
            tree.split(target, *dir, new, roomy, one_cell)
                .expect("the roomy surface fits every shape");
        }
        tree
    }

    /// The property itself, in one place: the rects `assign` produces tile `surface` — one per
    /// pane, none outside it, none overlapping, and the areas adding up to all of it.
    fn assert_tiles(tree: &Tree, surface: Rect, case: &str) {
        let (w, h) = (surface.w, surface.h);
        let rects = assign(tree, surface);
        assert_eq!(
            rects.len(),
            tree.panes().len(),
            "{case}: every pane gets a rect at {w}x{h}"
        );

        let mut area = 0usize;
        for (id, r) in &rects {
            assert!(
                r.x + r.w <= surface.x + surface.w && r.y + r.h <= surface.y + surface.h,
                "{case}: {id} at {r:?} leaves the {w}x{h} surface"
            );
            area += usize::from(r.w) * usize::from(r.h);
        }
        assert_eq!(
            area,
            usize::from(w) * usize::from(h),
            "{case}: the rects cover the {w}x{h} surface"
        );

        for (i, (a_id, a)) in rects.iter().enumerate() {
            for (b_id, b) in &rects[i + 1..] {
                let overlaps =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(
                    !overlaps,
                    "{case}: {a_id} at {a:?} overlaps {b_id} at {b:?}"
                );
            }
        }
    }

    #[test]
    fn assigned_rects_tile_the_surface_exactly() {
        for steps in SHAPES {
            let tree = shape(steps);
            for (w, h) in SIZES {
                assert_tiles(&tree, Rect::new(0, 0, w, h), &format!("table {steps:?}"));
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
    fn assigned_rects_tile_a_generated_shape_at_a_generated_size() {
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
            assert_tiles(
                &tree,
                surface,
                &format!("seed {SEED:#x} case {case} shape {steps:?}"),
            );
        }
    }

    /// The generator is the point of the property, so it has to actually generate: a seed that
    /// only ever produced one pane on one surface would pass everything above and prove
    /// nothing.
    #[test]
    fn the_generator_reaches_a_range_of_shapes_and_sizes() {
        let mut rng = Lcg(SEED);
        let mut leaves = std::collections::BTreeSet::new();
        let mut widths = std::collections::BTreeSet::new();
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
    }
}
