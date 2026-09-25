//! The one function that turns a pane tree into rectangles. Geometry lives here and nowhere else,
//! so that a Lua layout callback can later stand in for this one function instead of replacing the
//! tree (issue #27).

use neotmux_proto::{PaneId, Rect, SplitDir};

use crate::{Node, Split, Tree};

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
pub(crate) fn usable(dir: SplitDir, rect: Rect, gutter: u16) -> u16 {
    let along = match dir {
        SplitDir::Vertical => rect.w,
        SplitDir::Horizontal => rect.h,
    };
    along - gutter.min(along)
}

/// What the fraction is worth in cells, with rounding going to the first child. A resize
/// reads its current boundary back through this, so the cells it hands out are the cells
/// [`assign`] will lay down.
pub(crate) fn head(usable: u16, fraction: f32) -> u16 {
    (f32::from(usable) * fraction)
        .round()
        .clamp(0.0, f32::from(usable)) as u16
}

/// One split's share of one rect. [`assign`] is the walk; this is the cut, and nothing else
/// in the tree computes a boundary.
pub(crate) fn divide(split: &Split, rect: Rect, gutter: u16) -> (Rect, Rect) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use neotmux_proto::Size;

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
}
