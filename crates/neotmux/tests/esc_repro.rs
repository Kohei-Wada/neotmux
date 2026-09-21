//! Escape-key (ctl-[ / 0x1b) delivery to the pane's program.
//!
//! The pane runs a probe child that puts the pane's PTY in raw mode and prints the hex of
//! every byte it actually receives, one token at a time. Asserting on the child's input
//! (not the terminal's echo) makes "did it arrive at the program" a fact.
//!
//! The two tests:
//!   * a bare 0x1b reaches the program (regression guard — the simple path works);
//!   * Escape sent in the kitty/CSI-u keyboard-protocol form, `\x1b[27u`, reaches it
//!     (currently FAILS: the decoder drops unknown final bytes, so the pane's program sees
//!     nothing. Once `decode::csi` maps `CSI u` to a key this must pass).

mod harness;
mod session;

use harness::Neotmux;
use std::time::Duration;

/// The pane's shell: a python child that raw-reads its stdin and prints the hex of each byte.
/// Written into the test's own tempdir so the test is self-contained.
fn start(dir: &tempfile::TempDir) -> Neotmux {
    let probe = dir.path().join("probe.py");
    std::fs::write(
        &probe,
        r#"
import os, sys, termios, tty
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
out = sys.stdout
try:
    while True:
        b = os.read(fd, 1)
        if not b:
            break
        out.write(f"{b[0]:02x} ")
        out.flush()
finally:
    termios.tcsetattr(fd, termios.TCSADRAIN, old)
"#,
    )
    .unwrap();
    let shell = dir.path().join("probe.sh");
    std::fs::write(&shell, "#!/bin/sh\nexec /usr/bin/python3 PROBE\n".replace("PROBE", &probe.to_string_lossy())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    Neotmux::start_with_shell(dir, shell.to_str().unwrap())
}

/// Let the probe child reach its raw read loop before we type.
fn settle(nt: &mut Neotmux) {
    nt.settle(Duration::from_millis(300));
}

/// A bare Escape (0x1b) — the key the user calls ctl-[ — must reach the pane's program as a
/// 0x1b byte. The probe prints "1b" for the byte it received; `wait_for_screen` blocks until
/// it is on screen, so a pass means the byte arrived at the program.
#[test]
fn bare_escape_reaches_the_pane_program() {
    let dir = tempfile::tempdir().unwrap();
    let mut nt = start(&dir);
    settle(&mut nt);

    nt.send("\x1b"); // ctl-[
    nt.wait_for_screen("1b", Duration::from_secs(5));
}

/// Escape delivered the way a keyboard-protocol (kitty / CSI-u) terminal sends it:
/// `\x1b[27u` — CSI <unicode-codepoint> u, 27 being Escape's codepoint. A regression guard
/// for the "ctl-[ never arrives" bug: `decode::csi()` maps `CSI u` to a key (27 → Esc) and the
/// byte reaches the pane's program.
#[test]
fn csi_u_escape_reaches_the_pane_program() {
    let dir = tempfile::tempdir().unwrap();
    let mut nt = start(&dir);
    settle(&mut nt);

    nt.send("\x1b[27u");
    nt.wait_for_screen("1b", Duration::from_secs(5));
}

/// A keyboard-protocol character key: `\x1b[97u` = codepoint 97 = `a`, arriving as `61`.
/// And a modified one: `\x1b[97;2u` = `a` with the kitty alt bit, which re-encodes to the
/// conventional alt form `\x1ba` (probe prints `1b 61`) rather than being dropped.
#[test]
fn csi_u_character_and_modified_reach_the_pane_program() {
    let dir = tempfile::tempdir().unwrap();
    let mut nt = start(&dir);
    settle(&mut nt);

    nt.send("\x1b[97u"); // plain a
    nt.wait_for_screen("61", Duration::from_secs(5));

    nt.settle(Duration::from_millis(100));
    nt.send("\x1b[97;2u"); // alt-a: the 0x1b prefix must also arrive
    nt.wait_for_screen("1b 61", Duration::from_secs(5));
}
