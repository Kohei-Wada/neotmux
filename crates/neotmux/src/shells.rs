use std::collections::BTreeMap;

use anyhow::Context as _;
use neotmux_core::proto::{GridSnapshot, PaneId, Rect as PRect, Size};
use neotmux_term::pty::{CommandBuilder, PtyEvent, PtyHandle};
use neotmux_term::state::TermState;

const SCROLLBACK: usize = 10_000;

/// What one pane is running: its child process and the terminal that child writes into.
struct Shell {
    pty: PtyHandle,
    state: TermState,
    grid: GridSnapshot,
}

/// Every pane's child process, keyed by pane. The tree says where a pane is; this says what
/// is running in it, and it is the only thing in the binary that touches a PTY.
///
/// Ordered, not hashed: a pane's output becomes a `pane_output` event, and plugins are owed
/// the same order every frame rather than whatever order a hash map happens to be in.
#[derive(Default)]
pub struct Shells {
    shells: BTreeMap<PaneId, Shell>,
}

/// What one pass over every pane's output found.
pub struct Drained {
    /// Some pane produced output, so the screen needs redrawing.
    pub output: bool,
    /// The panes whose child reached EOF and are to be closed, in pane order.
    pub closed: Vec<PaneId>,
}

impl Shells {
    pub fn open(&mut self, pane: PaneId, cmd: CommandBuilder, size: Size) -> anyhow::Result<()> {
        let pty = PtyHandle::spawn(cmd, size)?;
        self.shells.insert(
            pane,
            Shell {
                pty,
                state: TermState::new(size, SCROLLBACK),
                grid: GridSnapshot::default(),
            },
        );
        Ok(())
    }

    /// Ends the pane's child and drops its PTY. EOF on the master only means no slave fd is
    /// open any more — a child that closed its descriptors and kept running is alive and
    /// sending it — so the child is signalled and then reaped without blocking: one pane's
    /// daemonised child must never be able to stall the frame loop the others share.
    pub fn close(&mut self, pane: PaneId) {
        if let Some(mut shell) = self.shells.remove(&pane) {
            let _ = shell.pty.kill();
            let _ = shell.pty.try_wait();
        }
    }

    /// Reaps every remaining child. The session is over, so nothing is redrawn after this.
    pub fn close_all(&mut self) {
        for pane in self.shells.keys().copied().collect::<Vec<_>>() {
            self.close(pane);
        }
    }

    /// How many panes are running, which is what "was that the last one?" asks.
    pub fn len(&self) -> usize {
        self.shells.len()
    }

    pub fn write(&mut self, pane: PaneId, bytes: &[u8]) -> anyhow::Result<()> {
        let Some(shell) = self.shells.get_mut(&pane) else {
            return Ok(());
        };
        shell.pty.write(bytes).context("write to pty")
    }

    /// Resizes the panes the layout says can be shown. A pane left out keeps the size it
    /// last had, which is §6's rule for a terminal too small to hold the tree.
    pub fn fit(&mut self, showable: impl Iterator<Item = (PaneId, PRect)>) -> anyhow::Result<()> {
        for (pane, rect) in showable {
            let Some(shell) = self.shells.get_mut(&pane) else {
                continue;
            };
            let size = Size {
                rows: rect.h,
                cols: rect.w,
            };
            shell.state.resize(size);
            shell.pty.resize(size)?;
        }
        Ok(())
    }

    /// At most `max_chunks` chunks per pane, so one flooding child cannot starve input or
    /// the other panes. Panes are visited in pane order, so which pane's chunks a plugin
    /// sees first under a flood is a fact rather than an accident.
    pub fn drain(
        &mut self,
        max_chunks: usize,
        mut on_output: impl FnMut(PaneId, Vec<u8>),
    ) -> anyhow::Result<Drained> {
        let mut drained = Drained {
            output: false,
            closed: Vec::new(),
        };
        for (pane, shell) in &mut self.shells {
            for _ in 0..max_chunks {
                match shell.pty.output().try_recv() {
                    Ok(PtyEvent::Output(bytes)) => {
                        shell.state.feed(&bytes);
                        on_output(*pane, bytes);
                        drained.output = true;
                    }
                    Ok(PtyEvent::Eof) => {
                        drained.closed.push(*pane);
                        break;
                    }
                    Ok(PtyEvent::Error(err)) => return Err(err).context("read from pty"),
                    Err(_) => break,
                }
            }
        }
        Ok(drained)
    }

    pub fn snapshot(&mut self) {
        for shell in self.shells.values_mut() {
            shell.state.snapshot_into(&mut shell.grid);
        }
    }

    pub fn grid(&self, pane: PaneId) -> Option<&GridSnapshot> {
        self.shells.get(&pane).map(|shell| &shell.grid)
    }

    #[cfg(test)]
    fn size(&self, pane: PaneId) -> Option<Size> {
        self.shells.get(&pane).map(|shell| shell.state.size())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use neotmux_core::proto::SplitDir;

    use super::*;
    use crate::panes::PaneLayout;

    fn pane(id: u32) -> PaneId {
        PaneId::from(id)
    }

    /// A child that stays alive without producing output, so a test can size its PTY.
    fn sleeper() -> CommandBuilder {
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "sleep 5"]);
        cmd
    }

    /// A child that closes its descriptors and keeps running. The master reaches EOF while
    /// the process is still very much alive, which is the case a blocking wait stalls on.
    fn daemoniser() -> CommandBuilder {
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "exec 0<&- 1>&- 2>&-; sleep 10"]);
        cmd
    }

    fn quick_exit() -> CommandBuilder {
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "true"]);
        cmd
    }

    fn two_panes(layout: &PaneLayout) -> Shells {
        let mut shells = Shells::default();
        for (id, rect) in layout.panes() {
            shells
                .open(
                    id,
                    sleeper(),
                    Size {
                        rows: rect.h,
                        cols: rect.w,
                    },
                )
                .expect("spawn");
        }
        shells
    }

    #[test]
    fn fitting_resizes_every_pane_to_its_own_rect() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 80 }, false, pane(1));
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        let mut shells = two_panes(&layout);

        shells.fit(layout.showable()).expect("fit");

        assert_eq!(shells.size(pane(1)), Some(Size { rows: 24, cols: 40 }));
        assert_eq!(shells.size(pane(2)), Some(Size { rows: 24, cols: 40 }));
    }

    /// §6: when the terminal shrinks below what the tree can hold, the pane that no longer
    /// fits keeps its last valid size instead of being resized to something its child cannot
    /// handle — and it is still there when the terminal grows back.
    #[test]
    fn a_pane_below_the_minimum_keeps_its_last_valid_size() {
        let mut layout = PaneLayout::new(Size { rows: 24, cols: 80 }, false, pane(1));
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        let mut shells = two_panes(&layout);
        shells.fit(layout.showable()).expect("fit");

        layout.resurface(Size { rows: 24, cols: 3 }, false);
        shells.fit(layout.showable()).expect("fit");

        assert_eq!(
            shells.size(pane(1)),
            Some(Size { rows: 24, cols: 2 }),
            "the pane that still fits follows the surface down"
        );
        assert_eq!(
            shells.size(pane(2)),
            Some(Size { rows: 24, cols: 40 }),
            "the hidden pane keeps the size it last had"
        );

        layout.resurface(Size { rows: 24, cols: 80 }, false);
        shells.fit(layout.showable()).expect("fit");
        assert_eq!(shells.size(pane(2)), Some(Size { rows: 24, cols: 40 }));
    }

    /// One pane's child must never be able to stop the frame loop the others share: EOF says
    /// the descriptors are gone, not that the process is, so closing signals and reaps
    /// without waiting.
    #[test]
    fn closing_a_pane_whose_child_outlived_its_descriptors_does_not_block() {
        let mut shells = Shells::default();
        shells
            .open(pane(1), daemoniser(), Size { rows: 24, cols: 80 })
            .expect("spawn");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let drained = shells.drain(8, |_, _| {}).expect("drain");
            if !drained.closed.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the child never closed its descriptors, so the test never reached the EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let started = Instant::now();
        shells.close(pane(1));
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(1),
            "closing waited {took:?} on a child that had only closed its descriptors"
        );
        assert_eq!(shells.len(), 0);
    }

    /// A plugin is owed the same order every frame: which pane's chunks it sees first must
    /// not be whatever order a hash map happens to iterate in.
    #[test]
    fn output_is_dispatched_in_pane_order() {
        let mut shells = Shells::default();
        let ids = [5u32, 2, 6, 1, 4, 3];
        for id in ids {
            let mut cmd = CommandBuilder::new("sh");
            cmd.args(["-c", "printf X; sleep 5"]);
            shells
                .open(pane(id), cmd, Size { rows: 24, cols: 80 })
                .expect("spawn");
        }
        std::thread::sleep(Duration::from_millis(500));

        let mut seen = Vec::new();
        shells.drain(8, |pane, _| seen.push(pane)).expect("drain");
        seen.dedup();
        assert_eq!(
            seen,
            vec![pane(1), pane(2), pane(3), pane(4), pane(5), pane(6)],
            "output was dispatched out of pane order"
        );

        for id in ids {
            shells.close(pane(id));
        }
    }

    #[test]
    fn closing_the_last_shell_empties_the_collection() {
        let mut shells = Shells::default();
        shells
            .open(pane(1), quick_exit(), Size { rows: 24, cols: 80 })
            .expect("spawn");
        assert_eq!(shells.len(), 1);
        shells.close(pane(1));
        assert_eq!(shells.len(), 0);
    }
}
