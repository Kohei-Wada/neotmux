use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use anyhow::Context as _;
use neotmux_core::proto::Size;
use portable_pty::{Child, ExitStatus, MasterPty, PtySize, native_pty_system};

// `spawn` takes one, so it is part of this crate's surface either way: re-exported so a
// caller does not need its own dependency on portable-pty to name the type.
pub use portable_pty::CommandBuilder;

pub enum PtyEvent {
    Output(Vec<u8>),
    Eof,
    Error(io::Error),
}

const OUTPUT_CHANNEL_BOUND: usize = 64;

pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<PtyEvent>,
}

fn read_loop<R: Read>(mut reader: R, tx: SyncSender<PtyEvent>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(PtyEvent::Eof);
                return;
            }
            Ok(n) => {
                if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(PtyEvent::Error(e));
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
    pub fn default_shell_command() -> CommandBuilder {
        CommandBuilder::new_default_prog()
    }

    pub fn spawn(cmd: CommandBuilder, size: Size) -> anyhow::Result<Self> {
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
            .spawn(move || read_loop(reader, tx))
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
        let mut pty = PtyHandle::spawn(cmd, Size { rows: 24, cols: 80 }).expect("spawn pty");

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

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("boom"))
        }
    }

    #[test]
    fn a_read_error_is_reported_as_error_not_eof() {
        let (tx, rx) = sync_channel(1);
        read_loop(FailingReader, tx);

        match rx.recv().expect("event") {
            PtyEvent::Error(e) => assert_eq!(e.kind(), io::ErrorKind::Other),
            PtyEvent::Eof => panic!("a read error must not be reported as Eof"),
            PtyEvent::Output(_) => panic!("unexpected output from a failing reader"),
        }
    }
}
