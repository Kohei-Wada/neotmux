//! `config-reload` in a real server behind a real client: the shipped layer behind a thin user
//! layer, the way an installed neotmux runs, reloaded while it is being looked at.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod harness;
mod session;
use harness::Neotmux;

const PATIENCE: Duration = Duration::from_secs(10);
const PANE_TOP: u16 = 1;
/// `plugin/sidebar.lua`'s width, herdr's: 25 columns and the rule down its edge.
const SIDEBAR_COLS: u16 = 26;
/// `plugin/copy.lua`'s cursor colour, the one thing on screen that says copy mode is up.
const COPY_CURSOR: (u8, u8, u8) = (0xf9, 0xe2, 0xaf);
/// `config-reload`'s description, which only a palette row shows: the query line holds the
/// name, so the name alone cannot count rows.
const RELOAD_DESC: &str = "run the config again";
const QUIET: &str = "require('neotmux.defaults').chime = false\n";

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

fn start_shipped(init: &str) -> (Neotmux, tempfile::TempDir) {
    let dir = harness::runtime(&[("init.lua", init)]);
    let nt = Neotmux::start_layered(&[dir.path(), &shipped_layer()], "cat");
    (nt, dir)
}

/// `C-b :`, the name, Enter: the reload as a person asks for it.
fn reload_from_the_palette(nt: &mut Neotmux) {
    nt.send("\x02:");
    nt.wait_for_screen(RELOAD_DESC, PATIENCE);
    nt.send("config-reload\r");
}

/// Blocks until `marker` is gone from the screen.
fn wait_until_gone(nt: &mut Neotmux, marker: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let screen = nt.screen_after(Duration::from_millis(20));
        if !screen.contains(marker) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{marker:?} never left the screen:\n{screen}"
        );
    }
}

/// Blocks until a row of the screen starts with `text`: typed into a pane that starts at the
/// left edge, and not into a list's query line.
fn wait_for_row_starting(nt: &mut Neotmux, text: &str) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let screen = nt.screen_after(Duration::from_millis(20));
        if screen.lines().any(|line| line.starts_with(text)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no row starts with {text:?}:\n{screen}"
        );
    }
}

fn wait_for_bg(nt: &mut Neotmux, row: u16, col: u16, bg: Option<(u8, u8, u8)>) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if nt.cell_bg(row, col) == bg {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "cell ({row}, {col}) never had background {bg:?}:\n{}",
            nt.screen_after(Duration::ZERO)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The count of palette rows naming `config-reload`, with the palette narrowed to it.
fn reload_rows(nt: &mut Neotmux) -> usize {
    nt.send("\x02:");
    nt.wait_for_screen(RELOAD_DESC, PATIENCE);
    nt.send("config-reload");
    // The query is on screen once the palette has taken all of it.
    nt.wait_for_screen("config-reload", PATIENCE);
    let screen = nt.screen_after(Duration::from_millis(200));
    let rows = screen
        .lines()
        .filter(|line| line.contains(RELOAD_DESC))
        .count();
    nt.send("\x1b");
    wait_until_gone(nt, RELOAD_DESC, PATIENCE);
    rows
}

/// Checks 1 and 4 of the brief: a reload chosen from the palette, and a key typed in the same
/// write as the Enter that chose it. The key lands in the pane; the bar is still one row, so
/// the pane still starts under it; and the palette lists the command once.
#[test]
fn config_reload_from_the_palette_redraws_once_and_keeps_the_next_key() {
    let (mut nt, _init) = start_shipped("");
    nt.wait_for_status("cat", PATIENCE);
    nt.send("X");
    nt.wait_for_cell(PANE_TOP, 0, "X", PATIENCE);
    nt.send("\r");
    let status_before = nt.status();

    nt.send("\x02:");
    nt.wait_for_screen(RELOAD_DESC, PATIENCE);
    nt.send("config-reload\rAFTER");
    wait_until_gone(&mut nt, RELOAD_DESC, PATIENCE);
    wait_for_row_starting(&mut nt, "AFTER");
    let screen = nt.screen_after(Duration::from_millis(200));
    // A band reserved twice would push the pane down a row; a status bar ticked twice draws
    // the same text into the same row, so where the pane starts is what can be seen.
    assert_eq!(nt.cell(PANE_TOP, 0), "X", "the pane moved:\n{screen}");
    assert_eq!(
        nt.status(),
        status_before,
        "the bar changed across the reload"
    );
    assert_eq!(
        reload_rows(&mut nt),
        1,
        "config-reload is listed other than once"
    );

    let log = nt.log_text();
    assert!(
        !log.contains("WARN") && !log.contains("ERROR"),
        "the reload complained:\n{log}"
    );
}

/// Check 2 of the brief: copy mode is open and the pane frozen under it when a reload comes
/// from outside the keyboard. The CLI's commands are the host's and never enter Lua, so the
/// outside here is a job the test lets go of by creating a file, whose `on_exit` asks for the
/// reload; what reaches the pane from outside meanwhile is `neotmux send`, the real CLI.
#[test]
fn a_reload_while_copy_mode_is_open_thaws_the_pane_and_closes_the_mode() {
    let flag_dir = tempfile::tempdir().expect("flag dir");
    let flag = flag_dir.path().join("go");
    let init = format!(
        "neotmux.group('test.reload').keymap.set('F5', function()\n\
           neotmux.job.start({{ 'sh', '-c', 'while [ ! -e \"$1\" ]; do sleep 0.02; done; rm \"$1\"', 'sh', {flag:?} }}, {{\n\
             on_exit = function() neotmux.reload() end,\n\
           }})\n\
           return true\n\
         end)\n"
    );
    let (mut nt, _init) = start_shipped(&init);
    nt.wait_for_status("cat", PATIENCE);
    nt.send("BEFORE\r");
    nt.wait_for_screen("BEFORE", PATIENCE);

    nt.send("\x1b[15~"); // F5: the job starts waiting
    nt.send("\x02["); // C-b [
    wait_for_bg(&mut nt, PANE_TOP + 2, 0, Some(COPY_CURSOR));

    let sent = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .args(["send", "%1", "FROZEN", "--enter"])
        .env("XDG_RUNTIME_DIR", nt.isolation())
        .env("HOME", nt.isolation())
        .output()
        .expect("run send");
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    let frozen = nt.screen_after(Duration::from_millis(500));
    assert!(
        !frozen.contains("FROZEN"),
        "copy mode did not hold the pane still:\n{frozen}"
    );

    std::fs::write(&flag, "").expect("create flag");
    nt.wait_for_screen("FROZEN", PATIENCE);
    wait_for_bg(&mut nt, PANE_TOP + 2, 0, None);

    // A key copy mode would have taken as a motion reaches the shell.
    nt.send("jjk\r");
    nt.wait_for_screen("jjk", PATIENCE);
}

/// Check 3 of the brief: `neotmux.defaults` is read again, so the sidebar follows the config
/// file across a reload, on and then off.
#[test]
fn the_sidebar_follows_init_lua_across_a_reload() {
    let (mut nt, init) = start_shipped("");
    nt.wait_for_status("cat", PATIENCE);
    nt.send("X");
    nt.wait_for_cell(PANE_TOP, 0, "X", PATIENCE);

    let init_lua = init.path().join("init.lua");
    std::fs::write(
        &init_lua,
        format!("{QUIET}require('neotmux.defaults').sidebar = true\n"),
    )
    .expect("write init.lua");
    reload_from_the_palette(&mut nt);
    nt.wait_for_screen(" spaces", PATIENCE);
    // The `X` already on the screen, not a key typed now: a key typed after `\r` is echoed by
    // the tty while `cat` is still writing its copy of the line, and can land beside it.
    nt.wait_for_cell(PANE_TOP, SIDEBAR_COLS, "X", PATIENCE);

    std::fs::write(&init_lua, QUIET).expect("write init.lua");
    reload_from_the_palette(&mut nt);
    wait_until_gone(&mut nt, " spaces", PATIENCE);
    nt.wait_for_cell(PANE_TOP, 0, "X", PATIENCE);
}
