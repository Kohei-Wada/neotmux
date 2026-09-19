use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;

use anyhow::Context as _;
use neotmux_core::proto::Size;
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

pub enum PtyEvent {
    Output(Vec<u8>),
    Eof,
}

const OUTPUT_CHANNEL_BOUND: usize = 64;

pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<PtyEvent>,
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

        let mut reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;
        let (tx, output) = sync_channel(OUTPUT_CHANNEL_BOUND);
        thread::Builder::new()
            .name("neotmux-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            let _ = tx.send(PtyEvent::Eof);
                            return;
                        }
                        Ok(n) => {
                            if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                                return;
                            }
                        }
                    }
                }
            })
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

        let mut drained = 0;
        while pty.output().try_recv().is_ok() {
            drained += 1;
        }

        pty.child.kill().ok();
        pty.child.wait().ok();

        assert!(
            drained <= OUTPUT_CHANNEL_BOUND + 1,
            "reader thread buffered {drained} messages, channel bound is {OUTPUT_CHANNEL_BOUND}"
        );
    }
}
