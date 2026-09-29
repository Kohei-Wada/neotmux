use std::collections::BTreeMap;
use std::rc::Rc;

use anyhow::Context as _;
use neotmux_proto::{GridSnapshot, LineMode, PaneId, Rect as PRect, Size};
use neotmux_pty::PaneTerminals;
use neotmux_pty::child::{CommandBuilder, PtyEvent, PtyHandle, Waker};

use crate::Drained;

const SCROLLBACK: usize = 10_000;

/// What one pane is running: its child process, and the copy of its screen the renderer
/// draws from. The terminal itself lives in the shared `PaneTerminals`, because Lua reads it
/// too.
struct Shell {
    pty: PtyHandle,
    grid: GridSnapshot,
}

/// Every pane's child process, keyed by pane. The tree says where a pane is; this says what
/// is running in it, and it is the only thing in the binary that touches a PTY.
///
/// Ordered, not hashed: a pane's output becomes a `pane_output` event, and plugins are owed
/// the same order every frame rather than whatever order a hash map happens to be in.
pub struct Shells {
    shells: BTreeMap<PaneId, Shell>,
    /// Every pane's terminal, shared with the Lua layer. Opened and closed here alongside
    /// the child, so a pane that is gone from one is gone from the other.
    terminals: Rc<PaneTerminals>,
    /// Handed to every pane's reader thread, so output is something the frame loop can wait
    /// on rather than something it finds on its next pass.
    waker: Waker,
    /// Children that were hung up on but had not died yet. Nothing here is waited for; each
    /// frame asks once whether they are done. Without it a session that opens and closes
    /// panes accumulates one zombie per closed pane until it exits.
    reaping: Vec<PtyHandle>,
}

impl Shells {
    pub fn new(waker: Waker, terminals: Rc<PaneTerminals>) -> Self {
        Self {
            shells: BTreeMap::new(),
            terminals,
            reaping: Vec::new(),
            waker,
        }
    }

    pub fn open(&mut self, pane: PaneId, cmd: CommandBuilder, size: Size) -> anyhow::Result<()> {
        let pty = PtyHandle::spawn(cmd, size, self.waker.clone())?;
        self.terminals.open(pane, size, SCROLLBACK);
        self.shells.insert(
            pane,
            Shell {
                pty,
                grid: GridSnapshot::default(),
            },
        );
        Ok(())
    }

    /// Ends the pane's child and drops its PTY. EOF on the master only means no slave fd is
    /// open any more — a child that closed its descriptors and kept running is alive and
    /// sending it — so the child is hung up on without blocking: one pane's daemonised child
    /// must never be able to stall the frame loop the others share.
    ///
    /// What that buys and what it costs: SIGHUP goes out and nothing is waited for, so the
    /// call is over in microseconds, but it is the only signal sent. A child that handles or
    /// ignores SIGHUP outlives the pane, and outlives neotmux — init inherits it, which is
    /// what tmux does too. One that dies a moment after the signal is reaped by a later
    /// frame rather than left a zombie for the session.
    pub fn close(&mut self, pane: PaneId) {
        self.terminals.close(pane);
        if let Some(mut shell) = self.shells.remove(&pane) {
            let _ = shell.pty.hangup();
            match shell.pty.try_wait() {
                Ok(Some(_)) => {}
                _ => self.reaping.push(shell.pty),
            }
        }
    }

    /// Asks once, for each child still owed a reap, whether it has died. Never blocks: a
    /// child that is still running simply stays on the list.
    fn reap(&mut self) {
        self.reaping
            .retain_mut(|pty| !matches!(pty.try_wait(), Ok(Some(_))));
    }

    /// How many panes are running, which is what "was that the last one?" asks.
    pub fn len(&self) -> usize {
        self.shells.len()
    }

    /// What the pane's program has told its tty to do with a line. `None` for a pane this
    /// session does not hold, and for one whose master would not answer.
    pub fn line_mode(&self, pane: PaneId) -> Option<LineMode> {
        self.shells.get(&pane)?.pty.line_mode()
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
            self.terminals.resize(pane, size);
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
        self.reap();
        let mut drained = Drained {
            output: false,
            closed: Vec::new(),
        };
        // A local borrow of each field, so the terminals can be fed while the shells are
        // walked.
        let terminals = &self.terminals;
        for (pane, shell) in &mut self.shells {
            for _ in 0..max_chunks {
                match shell.pty.output().try_recv() {
                    Ok(PtyEvent::Output(bytes)) => {
                        // The pane's own terminal answers here, because this is where its
                        // output and its PTY are both in hand.
                        let reply = terminals.feed(*pane, &bytes);
                        if !reply.is_empty() {
                            shell.pty.write(&reply).context("answer a terminal query")?;
                        }
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

    /// Copies out the grid of each named pane. The panes being drawn, not every pane: a
    /// hidden window's terminal goes on taking its child's output either way, and
    /// snapshotting it would put every window on every frame's bill.
    pub fn snapshot(
        &mut self,
        panes: impl Iterator<Item = PaneId>,
        views: &std::collections::BTreeMap<PaneId, usize>,
    ) {
        let terminals = &self.terminals;
        for pane in panes {
            if let Some(shell) = self.shells.get_mut(&pane) {
                match views.get(&pane) {
                    Some(back) => terminals.snapshot_into_at(pane, &mut shell.grid, *back),
                    None => terminals.snapshot_into(pane, &mut shell.grid),
                }
            }
        }
    }

    /// The pane's foreground process group: the shell while the pane is idle, whatever it
    /// launched while it is busy. `None` for a pane that is not here, or whose child has gone.
    pub fn foreground(&self, pane: PaneId) -> Option<i32> {
        self.shells.get(&pane)?.pty.foreground()
    }

    pub fn grid(&self, pane: PaneId) -> Option<&GridSnapshot> {
        self.shells.get(&pane).map(|shell| &shell.grid)
    }

    #[cfg(test)]
    fn size(&self, pane: PaneId) -> Option<Size> {
        self.terminals.size(pane)
    }
}

/// Every way out of the frame loop — the last pane's EOF, `Op::Quit`, a shutdown signal, a
/// PTY read error — ends with this collection being dropped, so reaping the children here is
/// the only arrangement that covers all four.
impl Drop for Shells {
    fn drop(&mut self) {
        for pane in self.shells.keys().copied().collect::<Vec<_>>() {
            self.close(pane);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use neotmux_layout::PaneLayout;
    use neotmux_proto::{Reserved, SplitDir};
    use neotmux_pty::child::{Wakeup, wakeup};
    use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
    use nix::unistd::Pid;

    use super::*;

    /// The wakeup is handed back rather than dropped: a `Shells` whose panes nudge an end
    /// nobody holds is not the arrangement these tests are about.
    fn shells() -> (Wakeup, Shells) {
        let (wakeup, waker) = wakeup().expect("wakeup");
        (wakeup, Shells::new(waker, Rc::new(PaneTerminals::new())))
    }

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

    fn two_panes(layout: &PaneLayout) -> (Wakeup, Shells) {
        let (wakeup, mut shells) = shells();
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
        (wakeup, shells)
    }

    #[test]
    fn fitting_resizes_every_pane_to_its_own_rect() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::default(),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        let (_wakeup, mut shells) = two_panes(&layout);

        shells.fit(layout.showable()).expect("fit");

        assert_eq!(shells.size(pane(1)), Some(Size { rows: 24, cols: 40 }));
        assert_eq!(shells.size(pane(2)), Some(Size { rows: 24, cols: 40 }));
    }

    /// §6: when the terminal shrinks below what the tree can hold, the pane that no longer
    /// fits keeps its last valid size instead of being resized to something its child cannot
    /// handle — and it is still there when the terminal grows back.
    #[test]
    fn a_pane_below_the_minimum_keeps_its_last_valid_size() {
        let mut layout = PaneLayout::new(
            Size { rows: 24, cols: 80 },
            Reserved::default(),
            0,
            0,
            pane(1),
            Size { rows: 2, cols: 2 },
        );
        layout.split(SplitDir::Vertical, pane(2)).expect("fits");
        let (_wakeup, mut shells) = two_panes(&layout);
        shells.fit(layout.showable()).expect("fit");

        layout.resurface(Size { rows: 24, cols: 3 }, Reserved::default(), 0, 0);
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

        layout.resurface(Size { rows: 24, cols: 80 }, Reserved::default(), 0, 0);
        shells.fit(layout.showable()).expect("fit");
        assert_eq!(shells.size(pane(2)), Some(Size { rows: 24, cols: 40 }));
    }

    /// One pane's child must never be able to stop the frame loop the others share: EOF says
    /// the descriptors are gone, not that the process is, so closing signals and reaps
    /// without waiting.
    #[test]
    fn closing_a_pane_whose_child_outlived_its_descriptors_does_not_block() {
        let (_wakeup, mut shells) = shells();
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
        // Well under portable-pty's own 200 ms SIGHUP grace period, which is what `close`
        // sidesteps: a second would pass whether or not the frame loop was being held.
        assert!(
            took < Duration::from_millis(100),
            "closing waited {took:?} on a child that had only closed its descriptors"
        );
        assert_eq!(shells.len(), 0);
    }

    /// A plugin is owed the same order every frame: which pane's chunks it sees first must
    /// not be whatever order a hash map happens to iterate in.
    #[test]
    fn output_is_dispatched_in_pane_order() {
        let (_wakeup, mut shells) = shells();
        let ids = [5u32, 2, 6, 1, 4, 3];
        for id in ids {
            let mut cmd = CommandBuilder::new("sh");
            cmd.args(["-c", "printf X; sleep 5"]);
            shells
                .open(pane(id), cmd, Size { rows: 24, cols: 80 })
                .expect("spawn");
        }
        // Drained until every child has been heard from rather than after a fixed wait: a
        // loaded runner that leaves one child unscheduled would fail a wait, and says
        // nothing about the order.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen: Vec<PaneId> = Vec::new();
        while seen.len() < ids.len() {
            assert!(
                Instant::now() < deadline,
                "only {seen:?} of the six panes ever produced output"
            );
            let mut pass = Vec::new();
            shells.drain(8, |pane, _| pass.push(pane)).expect("drain");
            pass.dedup();
            assert!(
                pass.windows(2).all(|w| w[0] < w[1]),
                "one pass dispatched output out of pane order: {pass:?}"
            );
            for pane in pass {
                if !seen.contains(&pane) {
                    seen.push(pane);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            vec![pane(1), pane(2), pane(3), pane(4), pane(5), pane(6)]
        );

        for id in ids {
            shells.close(pane(id));
        }
    }

    /// Only one path out of the frame loop ever called a reaper explicitly; `Op::Quit`, a
    /// shutdown signal and a PTY read error all returned past it. This pins the end of that
    /// asymmetry: whichever way the loop was left, no child outlives the collection.
    #[test]
    fn dropping_the_collection_ends_the_children_it_still_holds() {
        let dir = tempfile::tempdir().expect("dir");
        let pidfile = dir.path().join("pid");
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", &format!("echo $$ > {}; sleep 30", pidfile.display())]);

        let (_wakeup, mut shells) = shells();
        shells
            .open(pane(1), cmd, Size { rows: 24, cols: 80 })
            .expect("spawn");

        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile)
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                break Pid::from_raw(pid);
            }
            assert!(
                Instant::now() < deadline,
                "the child never reported its pid"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        drop(shells);

        // Anything but `StillAlive` is an exit, or a reap the drop already did.
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Ok(WaitStatus::StillAlive) = waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            assert!(
                Instant::now() < deadline,
                "the child outlived the collection that owned it"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The mutation freezing exists for, at the host's own seam: `snapshot` hands the
    /// renderer the held grid while `drain` goes on reading the pane's PTY into the live
    /// one. Nothing here writes to the terminal — the child does, on its own, throughout.
    #[test]
    fn a_frozen_panes_grid_stops_moving_while_its_pty_keeps_being_read() {
        let (wakeup, waker) = wakeup().expect("wakeup");
        let terminals = Rc::new(PaneTerminals::new());
        let mut shells = Shells::new(waker, terminals.clone());
        let _wakeup = wakeup;

        let mut cmd = CommandBuilder::new("sh");
        cmd.args([
            "-c",
            "i=0; while :; do echo line$i; i=$((i+1)); sleep 0.02; done",
        ]);
        let size = Size { rows: 4, cols: 20 };
        shells.open(pane(1), cmd, size).expect("spawn");

        let drawn = |shells: &Shells| {
            shells
                .grid(pane(1))
                .map(|grid| {
                    (0..grid.size.rows)
                        .flat_map(|row| (0..grid.size.cols).map(move |col| (row, col)))
                        .filter_map(|(row, col)| grid.cell(row, col))
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                })
                .unwrap_or_default()
        };
        let pump = |shells: &mut Shells| {
            let drained = shells.drain(32, |_, _| {}).expect("drain");
            shells.snapshot(std::iter::once(pane(1)), &Default::default());
            drained.output
        };

        // Let the child get two lines out, so the grid has something on it that would keep
        // changing.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !drawn(&shells).contains("line1") {
            assert!(Instant::now() < deadline, "the child never produced output");
            pump(&mut shells);
            std::thread::sleep(Duration::from_millis(5));
        }

        let hold = terminals.hold(pane(1)).expect("a live pane");
        let held = drawn(&shells);
        let mut fed = false;
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            fed |= pump(&mut shells);
            assert_eq!(drawn(&shells), held, "the frozen pane's grid moved");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            fed,
            "the pane's PTY was not read at all while it was frozen"
        );

        assert!(terminals.release(hold));
        pump(&mut shells);
        assert_ne!(
            drawn(&shells),
            held,
            "unfreezing did not show what arrived while the pane was held"
        );

        shells.close(pane(1));
    }

    #[test]
    fn closing_the_last_shell_empties_the_collection() {
        let (_wakeup, mut shells) = shells();
        shells
            .open(pane(1), quick_exit(), Size { rows: 24, cols: 80 })
            .expect("spawn");
        assert_eq!(shells.len(), 1);
        shells.close(pane(1));
        assert_eq!(shells.len(), 0);
    }

    /// A closed pane's child must not sit in the process table for the rest of the session.
    /// `close` hangs up without waiting, so whether the child has died by the time it returns
    /// is a race; a later frame is what reaps it.
    #[test]
    fn a_closed_panes_child_is_reaped_within_a_few_frames() {
        let (_wakeup, mut shells) = shells();
        let size = Size { rows: 4, cols: 20 };
        shells
            .open(pane(1), CommandBuilder::new("true"), size)
            .expect("open");
        shells.close(pane(1));

        let deadline = Instant::now() + Duration::from_secs(5);
        while !shells.reaping.is_empty() {
            assert!(
                Instant::now() < deadline,
                "a closed pane's child was never reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
            shells.reap();
        }
    }
}
