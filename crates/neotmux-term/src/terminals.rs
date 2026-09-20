use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_core::proto::{GridSnapshot, PaneId, Size};

use crate::query::Queries;
use crate::state::TermState;

/// One pane's terminal: the screen its child writes into, the reply path that answers the
/// questions it asks, and a stamp that moves whenever either of those has been fed.
struct Terminal {
    state: TermState,
    queries: Queries,
    revision: u64,
}

/// Every pane's terminal, a hidden window's included, behind one shared handle.
///
/// Shared rather than owned by whatever holds the panes, because two very different callers
/// want the same terminals: the host feeds them and draws them, and a plugin reads what they
/// are showing. The alternative — copying every pane's screen out each frame so the plugin
/// layer could have a view of its own — is the per-frame bill this exists to avoid. Nothing
/// is read here until somebody asks.
///
/// Interior mutability, because the host mutates and the plugin layer reads through the same
/// `Rc`. Every borrow begins and ends inside one method: no method here calls out, so a read
/// from a `pane_output` handler can never land inside a feed.
#[derive(Default)]
pub struct PaneTerminals {
    panes: RefCell<BTreeMap<PaneId, Terminal>>,
}

impl PaneTerminals {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&self, pane: PaneId, size: Size, scrollback: usize) {
        self.panes.borrow_mut().insert(
            pane,
            Terminal {
                state: TermState::new(size, scrollback),
                queries: Queries::default(),
                revision: 0,
            },
        );
    }

    /// The pane is gone. Every read about it answers `None` from here on, which is what makes
    /// a watcher's loop survive a shell exiting between one pass and the next.
    pub fn close(&self, pane: PaneId) {
        self.panes.borrow_mut().remove(&pane);
    }

    /// Feeds one chunk of the pane's output to its terminal and returns what the terminal
    /// owes the child back. Nothing for a pane that is not here.
    pub fn feed(&self, pane: PaneId, chunk: &[u8]) -> Vec<u8> {
        let mut panes = self.panes.borrow_mut();
        let Some(terminal) = panes.get_mut(&pane) else {
            return Vec::new();
        };
        terminal.revision += 1;
        terminal.queries.feed(chunk, &mut terminal.state)
    }

    /// A resize moves the revision too: reflowing the screen changes what the pane reads as
    /// without a byte arriving.
    pub fn resize(&self, pane: PaneId, size: Size) {
        let mut panes = self.panes.borrow_mut();
        let Some(terminal) = panes.get_mut(&pane) else {
            return;
        };
        terminal.revision += 1;
        terminal.state.resize(size);
    }

    pub fn size(&self, pane: PaneId) -> Option<Size> {
        self.panes.borrow().get(&pane).map(|t| t.state.size())
    }

    pub fn snapshot_into(&self, pane: PaneId, out: &mut GridSnapshot) {
        if let Some(terminal) = self.panes.borrow().get(&pane) {
            terminal.state.snapshot_into(out);
        }
    }

    /// A stamp that changes whenever the pane's terminal has been fed since it was last read.
    /// It says the screen *may* have changed, not that it did — a chunk that only moved the
    /// cursor moves it too — and that is the whole contract: it is the cheap question a
    /// watcher asks every frame so that the expensive one is asked only when it has to be.
    pub fn revision(&self, pane: PaneId) -> Option<u64> {
        self.panes.borrow().get(&pane).map(|t| t.revision)
    }

    /// What the pane is showing, as lines, together with the revision they were read at.
    /// `None` for a pane that does not exist.
    pub fn content(&self, pane: PaneId, last: Option<usize>) -> Option<(Vec<String>, u64)> {
        let panes = self.panes.borrow();
        let terminal = panes.get(&pane)?;
        Some((terminal.state.lines(last), terminal.revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32) -> PaneId {
        PaneId::from(id)
    }

    fn terminals() -> PaneTerminals {
        let terminals = PaneTerminals::new();
        terminals.open(pane(1), Size { rows: 4, cols: 20 }, 0);
        terminals
    }

    fn lines(terminals: &PaneTerminals, id: u32) -> Vec<String> {
        terminals.content(pane(id), None).expect("a live pane").0
    }

    #[test]
    fn what_a_pane_was_fed_is_what_its_content_says() {
        let terminals = terminals();
        terminals.feed(pane(1), b"hello\r\nworld");
        assert_eq!(lines(&terminals, 1), vec!["hello", "world", "", ""]);
    }

    #[test]
    fn panes_do_not_see_each_others_output() {
        let terminals = terminals();
        terminals.open(pane(2), Size { rows: 2, cols: 20 }, 0);
        terminals.feed(pane(1), b"one");
        terminals.feed(pane(2), b"two");
        assert_eq!(lines(&terminals, 1)[0], "one");
        assert_eq!(lines(&terminals, 2)[0], "two");
    }

    /// The cheap question. A watcher asks it every frame for every pane, and only reads the
    /// lines when the answer moved.
    #[test]
    fn the_revision_moves_when_the_pane_is_fed_and_stays_put_when_it_is_not() {
        let terminals = terminals();
        let first = terminals.revision(pane(1)).expect("a live pane");
        assert_eq!(terminals.revision(pane(1)), Some(first));

        terminals.feed(pane(1), b"x");
        let second = terminals.revision(pane(1)).expect("a live pane");
        assert_ne!(second, first);
        assert_eq!(terminals.revision(pane(1)), Some(second));

        terminals.resize(pane(1), Size { rows: 8, cols: 20 });
        assert_ne!(terminals.revision(pane(1)), Some(second));
    }

    #[test]
    fn the_revision_content_reports_is_the_one_it_was_read_at() {
        let terminals = terminals();
        terminals.feed(pane(1), b"x");
        let (_, revision) = terminals.content(pane(1), None).expect("a live pane");
        assert_eq!(Some(revision), terminals.revision(pane(1)));
    }

    #[test]
    fn a_pane_nobody_has_answers_nothing_rather_than_raising() {
        let terminals = terminals();
        assert_eq!(terminals.content(pane(4242), None), None);
        assert_eq!(terminals.revision(pane(4242)), None);
        assert_eq!(terminals.size(pane(4242)), None);
        assert!(terminals.feed(pane(4242), b"\x1b[c").is_empty());
        terminals.resize(pane(4242), Size { rows: 2, cols: 2 });
        terminals.close(pane(4242));
    }

    /// A shell exiting between one pass of a watcher's loop and the next is not the watcher's
    /// error.
    #[test]
    fn a_pane_that_closes_between_two_reads_answers_nothing_for_the_second() {
        let terminals = terminals();
        assert!(terminals.content(pane(1), None).is_some());
        terminals.close(pane(1));
        assert_eq!(terminals.content(pane(1), None), None);
        assert_eq!(terminals.revision(pane(1)), None);
    }

    /// The terminal answers its child's questions where its screen is in hand, so the reply
    /// describes the pane rather than whatever was last drawn anywhere.
    #[test]
    fn a_query_in_a_panes_output_is_answered_from_that_panes_own_cursor() {
        let terminals = terminals();
        terminals.open(pane(2), Size { rows: 4, cols: 20 }, 0);
        terminals.feed(pane(2), b"abcdef");
        assert_eq!(terminals.feed(pane(1), b"ab\x1b[6n"), b"\x1b[1;3R");
    }

    #[test]
    fn a_reopened_pane_starts_from_an_empty_screen() {
        let terminals = terminals();
        terminals.feed(pane(1), b"gone");
        terminals.open(pane(1), Size { rows: 4, cols: 20 }, 0);
        assert_eq!(lines(&terminals, 1), vec!["", "", "", ""]);
    }

    #[test]
    fn a_resize_is_what_the_pane_reports_afterwards() {
        let terminals = terminals();
        terminals.resize(pane(1), Size { rows: 8, cols: 10 });
        assert_eq!(terminals.size(pane(1)), Some(Size { rows: 8, cols: 10 }));
        assert_eq!(lines(&terminals, 1).len(), 8);
    }

    #[test]
    fn a_snapshot_is_taken_from_the_panes_own_terminal() {
        let terminals = terminals();
        terminals.feed(pane(1), b"hi");
        let mut grid = GridSnapshot::default();
        terminals.snapshot_into(pane(1), &mut grid);
        assert_eq!(grid.cell(0, 0).expect("in bounds").text, "h");
    }
}
