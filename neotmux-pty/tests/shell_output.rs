use std::time::{Duration, Instant};

use neotmux_proto::{GridSnapshot, Size};
use neotmux_pty::pty::{PtyEvent, PtyHandle};
use neotmux_pty::state::TermState;
use neotmux_pty::wakeup::wakeup;
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

/// The claim `pane.mode` rests on: `tcgetattr` on the **master** answers for the slave, so the
/// host can tell a program that reads lines from one that edits for itself without touching
/// the child. Three states, one pty, driven by `stty` in the child itself.
///
/// Read after the child has answered, never before: the flags are the child's, and a child
/// that has not run yet has not set them.
#[test]
fn the_master_answers_for_the_slave_s_line_discipline() {
    let size = Size { rows: 24, cols: 80 };
    let mut cmd = CommandBuilder::new("sh");
    // Cooked, then a password prompt's shape, then raw — each announced once it is in force.
    cmd.args([
        "-c",
        "stty icanon echo; echo COOKED; sleep 0.4; \
         stty icanon -echo; echo SECRET; sleep 0.4; \
         stty -icanon -echo; echo RAW; sleep 0.4",
    ]);
    let (_wakeup, waker) = wakeup().expect("wakeup");
    let pty = PtyHandle::spawn(cmd, size, waker).expect("spawn");

    let mut seen = String::new();
    let mut modes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while modes.len() < 3 && Instant::now() < deadline {
        match pty.output().recv_timeout(Duration::from_millis(250)) {
            Ok(PtyEvent::Output(bytes)) => {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                for marker in ["COOKED", "SECRET", "RAW"] {
                    if seen.contains(marker) && !modes.iter().any(|(at, _)| *at == marker) {
                        modes.push((marker, pty.line_mode().expect("the master answered")));
                    }
                }
            }
            Ok(PtyEvent::Eof) => break,
            Ok(PtyEvent::Error(e)) => panic!("pty read failed: {e}"),
            Err(_) => continue,
        }
    }

    let flags: Vec<(&str, bool, bool)> = modes
        .iter()
        .map(|(at, mode)| (*at, mode.canonical, mode.echo))
        .collect();
    assert_eq!(
        flags,
        vec![
            ("COOKED", true, true),
            ("SECRET", true, false),
            ("RAW", false, false),
        ],
        "{seen:?}"
    );
}
