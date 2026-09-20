use std::time::{Duration, Instant};

use neotmux_core::proto::{GridSnapshot, Size};
use neotmux_term::pty::{PtyEvent, PtyHandle};
use neotmux_term::state::TermState;
use neotmux_term::wakeup::wakeup;
use portable_pty::CommandBuilder;

#[test]
fn a_command_running_on_the_pty_reaches_the_grid() {
    let size = Size { rows: 24, cols: 80 };
    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", "printf X"]);
    let (_wakeup, waker) = wakeup().expect("wakeup");
    let mut pty = PtyHandle::spawn(cmd, size, waker).expect("spawn");

    let mut state = TermState::new(size, 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match pty.output().recv_timeout(Duration::from_millis(250)) {
            Ok(PtyEvent::Output(bytes)) => state.feed(&bytes),
            Ok(PtyEvent::Eof) => break,
            Ok(PtyEvent::Error(e)) => panic!("pty read failed: {e}"),
            Err(_) if Instant::now() < deadline => continue,
            Err(e) => panic!("timed out waiting for pty output: {e}"),
        }
    }

    let mut grid = GridSnapshot::default();
    state.snapshot_into(&mut grid);
    assert_eq!(grid.cell(0, 0).unwrap().text, "X");

    let status = pty.wait().expect("wait");
    assert!(status.success());
}

#[test]
fn writing_to_the_pty_reaches_the_child() {
    let size = Size { rows: 24, cols: 80 };
    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", "read line; printf '[%s]' \"$line\""]);
    let (_wakeup, waker) = wakeup().expect("wakeup");
    let mut pty = PtyHandle::spawn(cmd, size, waker).expect("spawn");
    pty.write(b"ok\n").expect("write");

    let mut state = TermState::new(size, 0);
    while let Ok(PtyEvent::Output(bytes)) = pty.output().recv_timeout(Duration::from_secs(10)) {
        state.feed(&bytes);
    }

    let mut grid = GridSnapshot::default();
    state.snapshot_into(&mut grid);
    let mut screen = String::new();
    for r in 0..grid.size.rows {
        for c in 0..grid.size.cols {
            if let Some(cell) = grid.cell(r, c) {
                screen.push_str(&cell.text);
            }
        }
    }
    assert!(screen.contains("[ok]"), "got {screen:?}");

    pty.wait().expect("wait");
}
