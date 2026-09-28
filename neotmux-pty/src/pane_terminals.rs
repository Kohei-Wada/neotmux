use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use neotmux_proto::screen::{Extent, Hold, Lines, Mark, PaneRead, Want};
use neotmux_proto::{GridSnapshot, PaneId, PaneTty, Size};

use crate::parse::Queries;
use crate::screen::TermState;

/// One pane's terminal: the screen its child writes into, the reply path that answers the
/// questions it asks, and a stamp that moves whenever either of those has been fed.
struct Terminal {
    state: TermState,
    queries: Queries,
    revision: u64,
    /// The revision as it was when the pane was frozen, or `None` while it is live. A frozen
    /// pane answers with this, so the revision a read comes back with is the revision of
    /// what was read: the live one goes on moving as the child prints, and a watcher that
    /// believed it would re-read the same held text on every frame.
    frozen_revision: Option<u64>,
    holds: BTreeSet<u64>,
}

impl Terminal {
    fn read_revision(&self) -> u64 {
        self.frozen_revision.unwrap_or(self.revision)
    }
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
    next_hold: Cell<u64>,
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
                frozen_revision: None,
                holds: BTreeSet::new(),
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

    /// Whether this pane has asked for the cursor keys in their application form. For the
    /// pane, neotmux is the terminal, so the pane's own mode decides what a key is written as.
    pub fn application_cursor(&self, pane: PaneId) -> bool {
        self.panes
            .borrow()
            .get(&pane)
            .is_some_and(|t| t.state.application_cursor())
    }

    /// The half of [`PaneTty`] the screen knows: where the cursor is, whether it is showing,
    /// and what the program has switched on. The line discipline is the pty's half, and the
    /// caller that has both puts them together.
    pub fn screen_tty(&self, pane: PaneId) -> Option<PaneTty> {
        let panes = self.panes.borrow();
        let terminal = panes.get(&pane)?;
        Some(PaneTty {
            cursor: terminal.state.cursor(),
            cursor_visible: terminal.state.cursor_visible(),
            alternate: terminal.state.alternate_screen(),
            application_cursor: terminal.state.application_cursor(),
            line: None,
        })
    }

    pub fn snapshot_into(&self, pane: PaneId, out: &mut GridSnapshot) {
        if let Some(terminal) = self.panes.borrow().get(&pane) {
            terminal.state.snapshot_into(out);
        }
    }

    /// The same, wound `back` lines into the pane's own history for this one snapshot.
    pub fn snapshot_into_at(&self, pane: PaneId, out: &mut GridSnapshot, back: usize) {
        if let Some(terminal) = self.panes.borrow_mut().get_mut(&pane) {
            terminal.state.snapshot_into_at(out, back);
        }
    }

    /// A stamp that changes whenever the pane's terminal has been fed since it was last read.
    /// It says the screen *may* have changed, not that it did — a chunk that only moved the
    /// cursor moves it too — and that is the whole contract: it is the cheap question a
    /// watcher asks every frame so that the expensive one is asked only when it has to be.
    pub fn revision(&self, pane: PaneId) -> Option<u64> {
        self.panes.borrow().get(&pane).map(Terminal::read_revision)
    }

    /// The title the pane's program last set with OSC 0 or OSC 2, or `None` for a pane that
    /// never set one, or that does not exist.
    pub fn title(&self, pane: PaneId) -> Option<String> {
        self.panes
            .borrow()
            .get(&pane)
            .and_then(|terminal| terminal.state.title().map(str::to_owned))
    }

    /// The OSC 133 marks the pane's shell sent on lines `first..=last`, or `None` for a pane
    /// that does not exist.
    pub fn marks(&self, pane: PaneId, first: u64, last: u64) -> Option<Vec<Mark>> {
        self.panes
            .borrow()
            .get(&pane)
            .map(|terminal| terminal.state.marks(first, last))
    }

    pub fn is_frozen(&self, pane: PaneId) -> Option<bool> {
        self.panes.borrow().get(&pane).map(|t| t.state.is_frozen())
    }

    /// Holds the pane still: until every hold on it is released it draws, reads and numbers
    /// its history as it was when the first hold was taken, while its child's output goes on
    /// being parsed into the live screen underneath. A later hold shares that snapshot rather
    /// than taking another, so a caller that has anchored a selection to it does not lose it.
    /// `None` for a pane that does not exist.
    pub fn hold(&self, pane: PaneId) -> Option<Hold> {
        let mut panes = self.panes.borrow_mut();
        let terminal = panes.get_mut(&pane)?;
        if terminal.holds.is_empty() && terminal.state.freeze() {
            terminal.frozen_revision = Some(terminal.revision);
        }
        let id = self.next_hold.get() + 1;
        self.next_hold.set(id);
        terminal.holds.insert(id);
        Some(Hold::new(pane, id))
    }

    /// Lets go of one hold; the pane catches up with everything that arrived while it was
    /// held once no hold is left. `false` if this hold was already released or its pane is
    /// gone.
    pub fn release(&self, hold: Hold) -> bool {
        let mut panes = self.panes.borrow_mut();
        let Some(terminal) = panes.get_mut(&hold.pane) else {
            return false;
        };
        if !terminal.holds.remove(&hold.id()) {
            return false;
        }
        if terminal.holds.is_empty() {
            terminal.state.unfreeze();
            terminal.frozen_revision = None;
        }
        true
    }

    /// How many lines the pane holds above its visible screen, and how many have already
    /// fallen out of the front of it. `None` for a pane that does not exist. This is the cheap half of reading the history, the way `revision` is the
    /// cheap half of reading the screen: it builds no lines.
    pub fn history_extent(&self, pane: PaneId) -> Option<Extent> {
        self.panes
            .borrow_mut()
            .get_mut(&pane)
            .map(|t| t.state.history_extent())
    }

    /// Lines from above the pane's visible screen, oldest first, with the revision they were
    /// read at. The read moves the screen's position in the scrollback and puts it back
    /// before returning, all inside this one borrow — see `crate::screen::scrollback`.
    pub fn history(&self, pane: PaneId, from: usize, count: usize, want: Want) -> Option<Lines> {
        let mut panes = self.panes.borrow_mut();
        let terminal = panes.get_mut(&pane)?;
        Some(Lines {
            lines: terminal.state.history(from, count, want.trailing),
            wrapped: want
                .wrapped
                .then(|| terminal.state.history_wrapped(from, count)),
            // The screen's runs only: a history read in runs is as long as the history.
            runs: None,
            revision: terminal.read_revision(),
        })
    }

    /// What the pane is showing, as lines, together with the revision they were read at.
    /// `None` for a pane that does not exist.
    pub fn content(&self, pane: PaneId, last: Option<usize>, want: Want) -> Option<Lines> {
        let panes = self.panes.borrow();
        let terminal = panes.get(&pane)?;
        Some(Lines {
            lines: terminal.state.lines(last, want.trailing),
            wrapped: want.wrapped.then(|| terminal.state.wrapped(last)),
            runs: want.runs.then(|| terminal.state.runs(last)),
            revision: terminal.read_revision(),
        })
    }
}

/// The read half, as a trait: the Lua layer asks these nine questions and nothing else, and
/// when it is in another process it asks them of a socket instead.
impl PaneRead for PaneTerminals {
    fn revision(&self, pane: PaneId) -> Option<u64> {
        PaneTerminals::revision(self, pane)
    }

    fn title(&self, pane: PaneId) -> Option<String> {
        PaneTerminals::title(self, pane)
    }

    fn content(&self, pane: PaneId, last: Option<u64>, want: Want) -> Option<Lines> {
        PaneTerminals::content(self, pane, last.map(|n| n as usize), want)
    }

    fn history_extent(&self, pane: PaneId) -> Option<Extent> {
        PaneTerminals::history_extent(self, pane)
    }

    fn history(&self, pane: PaneId, from: u64, count: u64, want: Want) -> Option<Lines> {
        PaneTerminals::history(self, pane, from as usize, count as usize, want)
    }

    fn marks(&self, pane: PaneId, first: u64, last: u64) -> Option<Vec<Mark>> {
        PaneTerminals::marks(self, pane, first, last)
    }

    fn is_frozen(&self, pane: PaneId) -> Option<bool> {
        PaneTerminals::is_frozen(self, pane)
    }

    fn hold(&self, pane: PaneId) -> Option<Hold> {
        PaneTerminals::hold(self, pane)
    }

    fn release(&self, hold: Hold) -> bool {
        PaneTerminals::release(self, hold)
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
        terminals
            .content(pane(id), None, Want::default())
            .expect("a live pane")
            .lines
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
        let text = terminals
            .content(pane(1), None, Want::default())
            .expect("a live pane");
        assert_eq!(Some(text.revision), terminals.revision(pane(1)));
    }

    #[test]
    fn a_pane_nobody_has_answers_nothing_rather_than_raising() {
        let terminals = terminals();
        assert_eq!(terminals.content(pane(4242), None, Want::default()), None);
        assert_eq!(terminals.history_extent(pane(4242)), None);
        assert_eq!(terminals.history(pane(4242), 0, 10, Want::default()), None);
        assert_eq!(terminals.revision(pane(4242)), None);
        assert_eq!(terminals.size(pane(4242)), None);
        assert!(terminals.feed(pane(4242), b"\x1b[c").is_empty());
        terminals.resize(pane(4242), Size { rows: 2, cols: 2 });
        terminals.close(pane(4242));
    }

    #[test]
    fn a_panes_history_is_read_without_moving_what_it_is_showing() {
        let terminals = PaneTerminals::new();
        terminals.open(pane(1), Size { rows: 2, cols: 8 }, 100);
        terminals.feed(pane(1), b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(terminals.history_extent(pane(1)).map(|h| h.len), Some(2));
        let history = terminals
            .history(pane(1), 0, 10, Want::default())
            .expect("a live pane");
        assert_eq!(history.lines, vec!["one", "two"]);
        assert_eq!(Some(history.revision), terminals.revision(pane(1)));
        assert_eq!(lines(&terminals, 1), vec!["three", "four"]);
    }

    /// The revision a read comes back with has to be the revision of what was read. A frozen
    /// pane hands back held text, so it hands back the held revision — otherwise a watcher
    /// comparing revisions re-reads the same text on every frame the child says anything.
    #[test]
    fn a_frozen_panes_revision_stands_still_and_catches_up_when_it_is_let_go() {
        let terminals = terminals();
        terminals.feed(pane(1), b"before");
        let held = terminals.revision(pane(1)).expect("a live pane");
        let hold = terminals.hold(pane(1)).expect("a live pane");

        terminals.feed(pane(1), b"\r\nafter");
        assert_eq!(terminals.revision(pane(1)), Some(held));
        let text = terminals
            .content(pane(1), None, Want::default())
            .expect("a frozen pane");
        assert_eq!(text.revision, held);
        assert_eq!(text.lines[0], "before");
        assert_eq!(text.lines[1], "");

        assert!(terminals.release(hold));
        assert_ne!(terminals.revision(pane(1)), Some(held));
        assert_eq!(lines(&terminals, 1)[1], "after");
    }

    /// Freezing is not a lifeline: the pane's child ending closes the pane, and a frozen
    /// pane is a pane.
    #[test]
    fn a_frozen_pane_that_closes_is_gone_like_any_other() {
        let terminals = terminals();
        terminals.feed(pane(1), b"held");
        terminals.hold(pane(1));
        terminals.close(pane(1));
        assert_eq!(terminals.content(pane(1), None, Want::default()), None);
        assert_eq!(terminals.is_frozen(pane(1)), None);
    }

    /// Two holders are independent: the pane moves again when the last one lets go, and
    /// until then it shows the screen the first hold took.
    #[test]
    fn a_pane_moves_again_only_when_its_last_hold_is_released() {
        let terminals = terminals();
        terminals.feed(pane(1), b"first");
        let a = terminals.hold(pane(1)).expect("a live pane");
        terminals.feed(pane(1), b"\r\nsecond");
        let b = terminals.hold(pane(1)).expect("a live pane");
        assert_eq!(
            lines(&terminals, 1),
            vec!["first", "", "", ""],
            "the second hold re-took the view"
        );

        assert!(terminals.release(a));
        assert_eq!(terminals.is_frozen(pane(1)), Some(true));
        assert_eq!(lines(&terminals, 1), vec!["first", "", "", ""]);

        assert!(terminals.release(b));
        assert_eq!(terminals.is_frozen(pane(1)), Some(false));
        assert_eq!(lines(&terminals, 1), vec!["first", "second", "", ""]);
    }

    #[test]
    fn a_hold_releases_once() {
        let terminals = terminals();
        let a = terminals.hold(pane(1)).expect("a live pane");
        let b = terminals.hold(pane(1)).expect("a live pane");
        assert!(terminals.release(a));
        assert!(
            !terminals.release(a),
            "a second release took somebody else's hold"
        );
        assert_eq!(terminals.is_frozen(pane(1)), Some(true));
        assert!(terminals.release(b));
    }

    #[test]
    fn a_hold_on_a_pane_that_closed_releases_nothing() {
        let terminals = terminals();
        let a = terminals.hold(pane(1)).expect("a live pane");
        terminals.close(pane(1));
        assert!(!terminals.release(a));
    }

    #[test]
    fn holding_a_pane_nobody_has_answers_none() {
        let terminals = terminals();
        assert_eq!(terminals.hold(pane(4242)), None);
        assert_eq!(terminals.is_frozen(pane(4242)), None);
    }

    /// A shell exiting between one pass of a watcher's loop and the next is not the watcher's
    /// error.
    #[test]
    fn a_pane_that_closes_between_two_reads_answers_nothing_for_the_second() {
        let terminals = terminals();
        assert!(terminals.content(pane(1), None, Want::default()).is_some());
        terminals.close(pane(1));
        assert_eq!(terminals.content(pane(1), None, Want::default()), None);
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
