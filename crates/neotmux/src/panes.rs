use std::collections::HashMap;

use neotmux_core::proto::{Epoch, PaneId, Rect as PRect, Size, SplitDir};
use neotmux_term::state::MIN_GRID;

/// Where each pane currently sits, in surface-local coordinates (the same
/// space `LuaSubsystem::begin_frame` was given).
pub type PaneRects = HashMap<PaneId, PRect>;

/// Everything the host knows about how panes are arranged. M2 replaces `compute` with the
/// pane tree (§6); nothing outside this file decides how many panes there are, which one
/// the PTY writes into, or how much of the surface they get.
pub struct PaneLayout {
    /// The surface the panes share: the terminal minus the status row.
    pub content: Size,
    pub rects: PaneRects,
    /// The pane the single PTY writes into, and the focused one.
    pub output_pane: PaneId,
}

impl PaneLayout {
    /// One PTY-backed pane filling the whole content area. A terminal at the minimum grid
    /// cannot spare a row for the status line.
    pub fn compute(full: Size, has_status: bool) -> Self {
        let content = if has_status && full.rows > MIN_GRID.rows {
            Size {
                rows: full.rows - 1,
                cols: full.cols,
            }
        } else {
            full
        };
        let output_pane = PaneId::from(1);
        let mut rects = PaneRects::new();
        rects.insert(output_pane, PRect::new(0, 0, content.cols, content.rows));
        Self {
            content,
            rects,
            output_pane,
        }
    }

    pub fn focus(&self) -> Option<PaneId> {
        Some(self.output_pane)
    }

    /// Every pane and its rect, in pane order, as `begin_frame` wants them.
    pub fn panes(&self) -> Vec<(PaneId, PRect)> {
        let mut panes: Vec<(PaneId, PRect)> = self.rects.iter().map(|(id, r)| (*id, *r)).collect();
        panes.sort_by_key(|(id, _)| *id);
        panes
    }

    pub fn epoch(&self) -> Epoch {
        Epoch::new(
            self.content,
            self.rects.keys().copied().collect(),
            self.focus(),
        )
    }
}

// Applying a pane op needs the tree M2 introduces (§6). Until then applying one is a no-op
// that says so, so a plugin author can tell from the log that the call arrived.
pub fn split_note(dir: SplitDir) -> String {
    format!("pane.split('{dir}') arrived; there is no pane tree yet, so nothing changed")
}

pub fn focus_note(pane: PaneId) -> String {
    format!("pane.focus({pane}) arrived; there is no pane tree yet, so nothing changed")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The wording is a placeholder M2 deletes; what these pin is that a note names the op's
    // own subject, which is the part a plugin author reads to tell the calls apart.
    #[test]
    fn a_split_note_names_the_direction() {
        let note = split_note(SplitDir::Vertical);
        assert!(note.contains("vertical"), "{note}");
    }

    #[test]
    fn a_focus_note_names_the_pane() {
        let note = focus_note(PaneId::from(1));
        assert!(note.contains("%1"), "{note}");
    }

    #[test]
    fn without_a_status_row_the_pane_gets_the_whole_surface() {
        let full = Size { rows: 24, cols: 80 };
        let layout = PaneLayout::compute(full, false);
        assert_eq!(layout.content, full);
        assert_eq!(
            layout.panes(),
            vec![(layout.output_pane, PRect::new(0, 0, 80, 24))]
        );
    }

    #[test]
    fn a_status_row_costs_the_pane_one_row() {
        let layout = PaneLayout::compute(Size { rows: 24, cols: 80 }, true);
        assert_eq!(layout.content, Size { rows: 23, cols: 80 });
        assert_eq!(
            layout.panes(),
            vec![(layout.output_pane, PRect::new(0, 0, 80, 23))]
        );
    }

    #[test]
    fn a_terminal_at_the_minimum_grid_cannot_spare_a_status_row() {
        let layout = PaneLayout::compute(MIN_GRID, true);
        assert_eq!(layout.content, MIN_GRID);
    }

    #[test]
    fn the_epoch_names_the_content_size_the_panes_and_the_focus() {
        let layout = PaneLayout::compute(Size { rows: 24, cols: 80 }, true);
        assert!(layout.epoch().matches(&Epoch::new(
            Size { rows: 23, cols: 80 },
            vec![layout.output_pane],
            Some(layout.output_pane),
        )));
    }
}
