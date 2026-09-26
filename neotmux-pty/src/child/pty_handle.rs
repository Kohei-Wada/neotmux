use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use anyhow::Context as _;
use neotmux_proto::{LineMode, Size};
use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};

use crate::child::{PtyEvent, Waker};

// `spawn` takes one, so it is part of this crate's surface either way: re-exported so a
// caller does not need its own dependency on portable-pty to name the type.
pub use portable_pty::CommandBuilder;

const OUTPUT_CHANNEL_BOUND: usize = 64;

pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<PtyEvent>,
}

/// Every event is followed by a nudge, because a channel is not something the frame loop
/// can wait on: the nudge is the only reason it looks at this pane before its next frame.
fn read_loop<R: Read>(mut reader: R, tx: SyncSender<PtyEvent>, waker: Waker) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(PtyEvent::Eof);
                waker.wake();
                return;
            }
            Ok(n) => {
                if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                    return;
                }
                waker.wake();
            }
            Err(e) => {
                let _ = tx.send(PtyEvent::Error(e));
                waker.wake();
                return;
            }
        }
    }
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl PtyHandle {
    /// The user's shell, found the way portable-pty's default program finds it, but named by
    /// its path: its default program is spawned as a login shell, which a pane is not.
    pub fn default_shell_command() -> CommandBuilder {
        let mut cmd = CommandBuilder::new(CommandBuilder::new_default_prog().get_shell());
        // Not the server's own TERM: that is the terminal it was launched from, and what the
        // program in the pane is writing to is the emulator here.
        cmd.env("TERM", "xterm-256color");
        cmd
    }

    pub fn spawn(cmd: CommandBuilder, size: Size, waker: Waker) -> anyhow::Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(size))
            .context("open pty")?;
        let child = pair.slave.spawn_command(cmd).context("spawn command")?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;
        let (tx, output) = sync_channel(OUTPUT_CHANNEL_BOUND);
        thread::Builder::new()
            .name("neotmux-pty-reader".into())
            .spawn(move || read_loop(reader, tx, waker))
            .context("spawn reader thread")?;

        Ok(Self {
            master: pair.master,
            writer,
            child,
            output,
        })
    }

    pub fn output(&self) -> &Receiver<PtyEvent> {
        &self.output
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, size: Size) -> anyhow::Result<()> {
        self.master.resize(pty_size(size)).context("resize pty")
    }

    pub fn size(&self) -> anyhow::Result<Size> {
        let size = self.master.get_size().context("get pty size")?;
        Ok(Size {
            rows: size.rows,
            cols: size.cols,
        })
    }

    /// The pty's foreground process group, which is what the user means by "what is this
    /// pane doing": the shell while it is idle, the command it is running otherwise. It is
    /// `tcgetpgrp(3)` on the master — portable-pty implements it that way on unix and answers
    /// `None` everywhere else, which is the same answer a dead child gives.
    pub fn foreground(&self) -> Option<i32> {
        self.master.process_group_leader()
    }

    /// The slave's `ICANON` and `ECHO`, read from the master: on Linux `tcgetattr` on the
    /// master answers for the slave, so the child is never touched and nothing is sent to it.
    /// `None` is a master that would not answer — no fd to ask on, or a platform where the
    /// question does not mean this — and it stays `None` rather than becoming a guess.
    pub fn line_mode(&self) -> Option<LineMode> {
        let fd = self.master.as_raw_fd()?;
        let mut termios = std::mem::MaybeUninit::<libc::termios>::zeroed();
        // Safety: `fd` is the master this handle owns and outlives the call, and `tcgetattr`
        // either fills the struct or reports why it did not.
        let read = unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) };
        if read != 0 {
            return None;
        }
        // Safety: `tcgetattr` answered 0, so it wrote the struct.
        let termios = unsafe { termios.assume_init() };
        Some(LineMode {
            canonical: termios.c_lflag & libc::ICANON != 0,
            echo: termios.c_lflag & libc::ECHO != 0,
        })
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    /// Sends the child SIGHUP and returns at once, saying nothing about when — or whether —
    /// it dies. `Child::kill` cannot be used for this: portable-pty's implementation for a
    /// `std::process::Child` follows the signal with up to 200 ms of polling and then a
    /// SIGKILL. The killer `clone_killer` splits off sends the signal and does nothing else,
    /// which is the whole of what a caller inside the frame loop can afford.
    pub fn hangup(&mut self) -> io::Result<()> {
        self.child.clone_killer().kill()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use portable_pty::CommandBuilder;

    use super::*;

    #[test]
    fn output_channel_stays_bounded_under_a_flooding_child() {
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "yes"]);
        let (_wakeup, waker) = crate::child::wakeup().expect("wakeup");
        let mut pty = PtyHandle::spawn(cmd, Size { rows: 24, cols: 80 }, waker).expect("spawn pty");

        std::thread::sleep(Duration::from_millis(500));

        pty.child.kill().ok();
        pty.child.wait().ok();

        let mut drained = 0;
        while pty.output().try_recv().is_ok() {
            drained += 1;
        }

        assert!(
            drained <= OUTPUT_CHANNEL_BOUND + 1,
            "reader thread buffered {drained} messages, channel bound is {OUTPUT_CHANNEL_BOUND}"
        );
    }

    /// A pane's shell is the user's, but not a login shell, and it is told the terminal it is
    /// in is the emulator here rather than whatever the server inherited from its launcher.
    #[test]
    fn the_default_shell_is_interactive_not_login_and_names_its_terminal() {
        let cmd = PtyHandle::default_shell_command();
        let argv = cmd.get_argv();
        assert_eq!(argv.len(), 1, "{argv:?}");
        assert!(
            !argv[0].to_string_lossy().starts_with('-'),
            "a leading '-' makes it a login shell: {argv:?}"
        );
        assert_eq!(
            cmd.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("boom"))
        }
    }

    #[test]
    fn a_read_error_is_reported_as_error_not_eof() {
        let (tx, rx) = sync_channel(1);
        let (_wakeup, waker) = crate::child::wakeup().expect("wakeup");
        read_loop(FailingReader, tx, waker);

        match rx.recv().expect("event") {
            PtyEvent::Error(e) => assert_eq!(e.kind(), io::ErrorKind::Other),
            PtyEvent::Eof => panic!("a read error must not be reported as Eof"),
            PtyEvent::Output(_) => panic!("unexpected output from a failing reader"),
        }
    }
}
