//! A session driven with no terminal at all: no ratatui, no `/dev/tty`, no signal handler.
//! It is told how big its surface is, handed keys, stepped a frame at a time, and read back
//! out of the frames it produces. This is the property the library exists for, so it is
//! tested as a property rather than through any of the session's internals.

use std::fs;
use std::time::{Duration, Instant};

use neotmux::frame::Frame;
use neotmux::keys::{Key, KeyCode};
use neotmux::notify::Notifications;
use neotmux::session::{End, Session};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::{DrawCmd, Size};
use neotmux_term::wakeup::{Wakeup, wakeup};

const SURFACE: Size = Size { rows: 10, cols: 40 };
/// Long enough for a shell to start and answer on a loaded machine; a passing run takes a
/// small fraction of it.
const PATIENCE: Duration = Duration::from_secs(20);

fn lua(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("create layer dir");
        fs::write(path, body).expect("write layer file");
    }
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()]))
        .expect("build the Lua subsystem");
    assert!(sub.run_init().is_empty(), "the layer failed to load");
    (dir, sub)
}

fn session(lua: &LuaSubsystem) -> (Wakeup, Session) {
    let (wakeup, waker) = wakeup().expect("wakeup");
    let session = Session::new(SURFACE, lua, waker).expect("start a session");
    (wakeup, session)
}

fn typed(text: &str) -> Vec<Key> {
    text.chars()
        .map(|ch| match ch {
            '\n' => Key::plain(KeyCode::Enter),
            ch => Key::plain(KeyCode::Char(ch)),
        })
        .collect()
}

/// Every cell of every pane the frame shows, as lines.
fn screen(frame: &Frame) -> String {
    let mut out = String::new();
    for pane in &frame.panes {
        for row in 0..pane.grid.size.rows {
            for col in 0..pane.grid.size.cols {
                match pane.grid.cell(row, col) {
                    Some(cell) if !cell.text.is_empty() => out.push_str(&cell.text),
                    _ => out.push(' '),
                }
            }
            out.push('\n');
        }
    }
    out
}

/// Steps the session until `done` accepts a frame it produced, or patience runs out. The
/// frames go nowhere: nothing draws them, and nothing here has a screen to draw them on.
fn step_until(
    session: &mut Session,
    lua: &LuaSubsystem,
    notifications: &mut Notifications,
    mut done: impl FnMut(&Frame) -> bool,
) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let step = session.advance(lua, notifications).expect("advance");
        if let Some(frame) = &step.frame
            && done(frame)
        {
            return true;
        }
        if step.end.is_some() {
            return false;
        }
        drop(step);
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn a_session_runs_with_no_terminal_and_what_was_typed_comes_back_in_a_frame() {
    let (_dir, lua) = lua(&[("init.lua", "return {}")]);
    let (_wakeup, mut session) = session(&lua);
    let mut notifications = Notifications::new();

    for key in typed("echo neotmux-is-headless\n") {
        session.key(&lua, key).expect("key");
    }

    // The line the child echoed back is the whole round trip: a key the session was handed
    // reached the shell, and the shell's output reached a frame the session produced.
    assert!(
        step_until(&mut session, &lua, &mut notifications, |frame| {
            screen(frame).contains("neotmux-is-headless")
        }),
        "nothing the session was typed ever came back in a frame"
    );
}

#[test]
fn a_frame_carries_the_pane_it_is_showing() {
    let (_dir, lua) = lua(&[("init.lua", "return {}")]);
    let (_wakeup, mut session) = session(&lua);
    let mut notifications = Notifications::new();

    let step = session.advance(&lua, &mut notifications).expect("advance");
    let frame = step.frame.expect("the first frame is always produced");
    assert_eq!(frame.size, SURFACE);
    assert_eq!(frame.panes.len(), 1, "one shell, one pane");
    let pane = &frame.panes[0];
    assert_eq!(pane.rect.w, SURFACE.cols);
    assert_eq!(pane.rect.h, SURFACE.rows);
    assert_eq!(pane.grid.size, SURFACE);
    assert_eq!(frame.cursor, Some(pane.pane), "the one pane has the cursor");
    assert_eq!(frame.rects.get(&pane.pane), Some(&pane.rect));
}

#[test]
fn a_frame_carries_what_the_sources_drew_and_what_they_notified() {
    let (_dir, lua) = lua(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 1, 2, 'painted') end)\n\
             neotmux.notify('info', 'noticed')\n\
             return {}",
        ),
    ]);
    let (_wakeup, mut session) = session(&lua);
    let mut notifications = Notifications::new();
    // `notify` ran at load time, so its op is waiting where a host's own startup drain
    // would have taken it; the first step takes it instead.
    let step = session.advance(&lua, &mut notifications).expect("advance");
    let frame = step.frame.expect("a frame");

    assert!(
        frame.overlay.iter().any(|cmd| matches!(
            cmd,
            DrawCmd::Text { x: 1, y: 2, text, .. } if text == "painted"
        )),
        "the tick's drawing is not in the frame: {:?}",
        frame.overlay
    );
    assert_eq!(frame.notices.len(), 1);
    assert_eq!(frame.notices[0].text, "noticed");
}

#[test]
fn a_session_ends_when_its_last_shell_does_and_still_produces_that_frame() {
    let (_dir, lua) = lua(&[("init.lua", "return {}")]);
    let (_wakeup, mut session) = session(&lua);
    let mut notifications = Notifications::new();

    for key in typed("exit\n") {
        session.key(&lua, key).expect("key");
    }

    let deadline = Instant::now() + PATIENCE;
    let mut ended = None;
    while Instant::now() < deadline && ended.is_none() {
        let step = session.advance(&lua, &mut notifications).expect("advance");
        if step.end.is_some() {
            ended = Some((matches!(step.end, Some(End::Eof)), step.frame.is_some()));
        } else {
            drop(step);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    assert_eq!(
        ended,
        Some((true, true)),
        "the last shell's exit should end the session with one final frame"
    );
}
