use std::fs::File;
use std::io::{self, IsTerminal};
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

use neotmux_term::wakeup::Wakeup;
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

/// What one wait on the terminal found.
#[derive(Debug, PartialEq, Eq)]
pub enum Ready {
    /// Bytes are waiting to be read.
    Input,
    /// A pane's reader thread has enqueued output. The frame that draws it is worth starting
    /// now rather than at the next deadline, which is the whole of why the wait watches
    /// anything but the terminal.
    Output,
    /// The deadline passed with nothing to read.
    Idle,
    /// The terminal is gone. `POLLHUP` is the kernel saying the other end of the PTY has been
    /// closed, which is what a terminal emulator exiting, or an ssh session dropping, does.
    HangUp,
}

/// The file neotmux's own input arrives on: stdin when it is a terminal and `/dev/tty`
/// otherwise, which is the rule `crossterm::event` applies to pick the same file. Waiting
/// here rather than in `crossterm::event::poll` is what makes a hangup observable at all —
/// crossterm reacts to one by reading zero bytes in a loop that has no way out.
pub struct Terminal {
    fd: OwnedFd,
}

impl Terminal {
    pub fn open() -> io::Result<Self> {
        let fd = if io::stdin().is_terminal() {
            io::stdin().as_fd().try_clone_to_owned()?
        } else {
            OwnedFd::from(File::options().read(true).write(true).open("/dev/tty")?)
        };
        Ok(Self { fd })
    }

    /// Waits until the terminal has something to read, a pane produces output, the terminal
    /// goes away, or `deadline` passes. A wait cut short by a signal is retried, so the
    /// deadline is the only thing that ends it early — the caller checks for a raised signal
    /// on its own terms.
    ///
    /// `panes` is a parameter rather than a field because half of every frame is spent
    /// deliberately not listening to it: once the frame has drawn, a pane's output is a
    /// reason for the *next* frame, and passing `None` is how the caller says so.
    pub fn wait(&self, panes: Option<&Wakeup>, deadline: Instant) -> io::Result<Ready> {
        loop {
            let tty = PollFd::new(self.fd.as_fd(), PollFlags::POLLIN);
            let mut watched = [tty; 2];
            let watched = match panes {
                Some(panes) => {
                    watched[1] = PollFd::new(panes.as_fd(), PollFlags::POLLIN);
                    &mut watched[..2]
                }
                None => &mut watched[..1],
            };
            match poll(
                watched,
                timeout(deadline.saturating_duration_since(Instant::now())),
            ) {
                Ok(0) => return Ok(Ready::Idle),
                Ok(_) => {
                    let revents = watched[0].revents().unwrap_or_else(PollFlags::empty);
                    // A hangup is reported alongside `POLLIN`, so it has to be looked at first.
                    let gone = PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL;
                    if revents.intersects(gone) {
                        return Ok(Ready::HangUp);
                    }
                    if revents.contains(PollFlags::POLLIN) {
                        return Ok(Ready::Input);
                    }
                    if let Some(panes) = panes {
                        panes.clear();
                    }
                    return Ok(Ready::Output);
                }
                Err(Errno::EINTR) => {
                    if Instant::now() >= deadline {
                        return Ok(Ready::Idle);
                    }
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

/// `poll(2)` counts in whole milliseconds; rounding up is what keeps a sub-millisecond
/// remainder from turning the wait into a spin.
fn timeout(remaining: Duration) -> PollTimeout {
    PollTimeout::from(u16::try_from(remaining.as_micros().div_ceil(1000)).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PTY pair whose master the test can close, which is the whole of what "the terminal
    /// went away" means to the process on the other end.
    fn pty_pair() -> (OwnedFd, OwnedFd) {
        let pty = nix::pty::openpty(None, None).expect("openpty");
        (pty.master, pty.slave)
    }

    fn slave_terminal(slave: &OwnedFd) -> Terminal {
        Terminal {
            fd: slave.as_fd().try_clone_to_owned().expect("clone slave"),
        }
    }

    #[test]
    fn a_quiet_terminal_is_idle_until_the_deadline() {
        let (_master, slave) = pty_pair();
        let tty = slave_terminal(&slave);

        let started = Instant::now();
        let ready = tty
            .wait(None, started + Duration::from_millis(30))
            .expect("wait");

        assert_eq!(ready, Ready::Idle);
        assert!(
            started.elapsed() >= Duration::from_millis(25),
            "the wait returned after {:?}, well before its deadline",
            started.elapsed()
        );
    }

    #[test]
    fn bytes_waiting_on_the_terminal_are_reported_as_input() {
        let (master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        // A line, not a byte: the line discipline holds an unterminated one back.
        nix::unistd::write(master.as_fd(), b"x\n").expect("write");

        assert_eq!(
            tty.wait(None, Instant::now() + Duration::from_secs(5))
                .expect("wait"),
            Ready::Input
        );
    }

    /// The bug this whole file exists for: with the master closed the slave is hung up on,
    /// and nothing downstream may be asked to wait on it again.
    #[test]
    fn a_closed_master_is_reported_as_a_hangup() {
        let (master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        drop(master);

        let started = Instant::now();
        assert_eq!(
            tty.wait(None, started + Duration::from_secs(5))
                .expect("wait"),
            Ready::HangUp
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a hangup should be reported at once, not waited out"
        );
    }

    #[test]
    fn a_hangup_keeps_being_reported_on_every_later_wait() {
        let (master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        drop(master);

        for _ in 0..3 {
            assert_eq!(
                tty.wait(None, Instant::now() + Duration::from_secs(5))
                    .expect("wait"),
                Ready::HangUp
            );
        }
    }

    /// The defect this parameter exists for: a pane that produced output a millisecond into
    /// the frame must not be left unnoticed until the deadline.
    #[test]
    fn a_pane_nudge_ends_the_wait_long_before_the_deadline() {
        let (_master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        let (wakeup, waker) = neotmux_term::wakeup::wakeup().expect("wakeup");
        waker.wake();

        let started = Instant::now();
        let ready = tty
            .wait(Some(&wakeup), started + Duration::from_secs(5))
            .expect("wait");

        assert_eq!(ready, Ready::Output);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "the nudge was noticed only after {:?}",
            started.elapsed()
        );
    }

    /// The nudge is taken as it is reported, so the same output cannot wake the next wait a
    /// second time and turn one chunk into a frame per wait.
    #[test]
    fn a_reported_nudge_does_not_wake_the_next_wait_as_well() {
        let (_master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        let (wakeup, waker) = neotmux_term::wakeup::wakeup().expect("wakeup");
        waker.wake();

        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            tty.wait(Some(&wakeup), deadline).expect("wait"),
            Ready::Output
        );

        let started = Instant::now();
        assert_eq!(
            tty.wait(Some(&wakeup), started + Duration::from_millis(30))
                .expect("wait"),
            Ready::Idle
        );
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    /// Half of every frame is spent not listening for output: once the frame has drawn, a
    /// pane's nudge is a reason for the next frame and must not end this wait.
    #[test]
    fn a_wait_that_was_given_no_wakeup_ignores_a_nudge() {
        let (_master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        let (_wakeup, waker) = neotmux_term::wakeup::wakeup().expect("wakeup");
        waker.wake();

        let started = Instant::now();
        assert_eq!(
            tty.wait(None, started + Duration::from_millis(30))
                .expect("wait"),
            Ready::Idle
        );
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    /// Input wins a tie, and the nudge it beat is still there for the next wait: a keystroke
    /// reaches its pane in the same frame as the output that arrived with it.
    #[test]
    fn input_is_reported_first_and_the_nudge_survives_for_the_next_wait() {
        let (master, slave) = pty_pair();
        let tty = slave_terminal(&slave);
        let (wakeup, waker) = neotmux_term::wakeup::wakeup().expect("wakeup");
        waker.wake();
        nix::unistd::write(master.as_fd(), b"x\n").expect("write");

        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            tty.wait(Some(&wakeup), deadline).expect("wait"),
            Ready::Input
        );

        let mut buf = [0u8; 8];
        nix::unistd::read(std::os::fd::AsRawFd::as_raw_fd(&slave), &mut buf).expect("read");
        assert_eq!(
            tty.wait(Some(&wakeup), deadline).expect("wait"),
            Ready::Output
        );
    }

    #[test]
    fn a_sub_millisecond_remainder_is_rounded_up_rather_than_down_to_a_spin() {
        assert_eq!(timeout(Duration::from_micros(1)), PollTimeout::from(1u16));
        assert_eq!(timeout(Duration::ZERO), PollTimeout::ZERO);
        assert_eq!(timeout(Duration::from_millis(16)), PollTimeout::from(16u16));
    }
}
