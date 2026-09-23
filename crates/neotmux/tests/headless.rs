//! A session driven with no terminal at all: no ratatui, no `/dev/tty`, no signal handler,
//! and no socket either. The two halves M3 puts in two processes — the session and the
//! plugins — are both here, passing the same messages to each other across a function call.
//! This is the property the library exists for, so it is tested as a property rather than
//! through any of the session's internals.

use std::fs;
use std::time::{Duration, Instant};

use neotmux::plugins::Plugins;
use neotmux::session::{Session, Setup};
use neotmux_lua::KeyOutcome;
use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::Level;
use neotmux_proto::frame::{Frame, Notice};
use neotmux_proto::keys::{Key, KeyCode};
use neotmux_proto::msg::End;
use neotmux_proto::{DrawCmd, Reserved, Size, WindowId};
use neotmux_term::wakeup::{Wakeup, wakeup};

const SURFACE: Size = Size { rows: 10, cols: 40 };
/// Long enough for a shell to start and answer on a loaded machine; a passing run takes a
/// small fraction of it.
const PATIENCE: Duration = Duration::from_secs(20);

/// The two halves, wired to each other: the session with the panes in it, and the plugins
/// that are told about them. A real client builds the Lua layer first and hands it the
/// socket; here the session is built first and the Lua layer reads its terminals directly.
fn halves(files: &[(&str, &str)]) -> (tempfile::TempDir, Wakeup, Session, Plugins) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("create layer dir");
        fs::write(path, body).expect("write layer file");
    }
    let (wakeup, waker) = wakeup().expect("wakeup");
    let terminals = std::rc::Rc::new(neotmux_term::terminals::PaneTerminals::new());
    let session = Session::new(
        &Setup {
            size: SURFACE,
            first_window: WindowId::from(1),
            reserved: Reserved::default(),
            gutter: 0,
            wants_pane_output: false,
        },
        waker,
        terminals.clone(),
    )
    .expect("start a session");
    let lua = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        terminals,
    )
    .expect("build the Lua subsystem");
    assert!(lua.run_init().is_empty(), "the layer failed to load");
    (dir, wakeup, session, Plugins::new(lua))
}

/// A key as the client would offer it: to the plugins first, and on to the focused pane only
/// if nobody wanted it.
fn key(session: &mut Session, plugins: &Plugins, key: Key) {
    if plugins.key(key) == KeyOutcome::Fallthrough {
        session.key(key).expect("key");
    }
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

/// One whole frame: the session says what happened, the plugins tick against it, and the
/// session turns what they asked for into the picture. The two messages are exactly the ones
/// that cross the socket in the real thing.
fn frame<'a>(session: &'a mut Session, plugins: &mut Plugins) -> neotmux::session::Pass<'a> {
    let tick = session.tick().expect("tick");
    let tock = plugins.respond(tick);
    let mut pass = session.advance(&tock).expect("advance");
    // What the ops did, told to the plugins before anything else happens — the same round
    // the client keeps its keyboard out of.
    plugins.catch_up(std::mem::take(&mut pass.steps));
    pass
}

/// Runs frames until `done` accepts one, or patience runs out. The frames go nowhere:
/// nothing draws them, and nothing here has a screen to draw them on.
fn step_until(
    session: &mut Session,
    plugins: &mut Plugins,
    mut done: impl FnMut(&Frame) -> bool,
) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        let pass = frame(session, plugins);
        if let Some(shown) = &pass.frame
            && done(shown)
        {
            return true;
        }
        if pass.end.is_some() {
            return false;
        }
        drop(pass);
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn a_session_runs_with_no_terminal_and_what_was_typed_comes_back_in_a_frame() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[("init.lua", "return {}")]);

    for typed in typed("echo neotmux-is-headless\n") {
        key(&mut session, &plugins, typed);
    }

    // The line the child echoed back is the whole round trip: a key the session was handed
    // reached the shell, and the shell's output reached a frame the session produced.
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            screen(frame).contains("neotmux-is-headless")
        }),
        "nothing the session was typed ever came back in a frame"
    );
}

#[test]
fn a_frame_carries_the_pane_it_is_showing() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[("init.lua", "return {}")]);

    let pass = frame(&mut session, &mut plugins);
    let frame = pass.frame.expect("the first frame is always produced");
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
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[
        ("init.lua", "require('plugin.painter')"),
        (
            "lua/plugin/painter.lua",
            "neotmux.on_tick(function(ctx) ctx:text(nil, 1, 2, 'painted') end)\n\
             neotmux.notify('info', 'noticed')\n\
             return {}",
        ),
    ]);
    // `notify` ran at load time, so its op is waiting where a client's own startup drain
    // would have taken it; the first frame takes it instead.
    let pass = frame(&mut session, &mut plugins);
    let frame = pass.frame.expect("a frame");

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
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[("init.lua", "return {}")]);

    for typed in typed("exit\n") {
        key(&mut session, &plugins, typed);
    }

    let deadline = Instant::now() + PATIENCE;
    let mut ended = None;
    while Instant::now() < deadline && ended.is_none() {
        let pass = frame(&mut session, &mut plugins);
        if pass.end.is_some() {
            ended = Some((matches!(pass.end, Some(End::Eof)), pass.frame.is_some()));
        } else {
            drop(pass);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    assert_eq!(
        ended,
        Some((true, true)),
        "the last shell's exit should end the session with one final frame"
    );
}

/// A frame whose notices are whatever the caller says, so that a notice's whole life can be
/// driven without waiting out the four seconds a real one takes to expire.
fn frame_showing<'a>(
    session: &'a mut Session,
    plugins: &mut Plugins,
    notices: Vec<Notice>,
) -> neotmux::session::Pass<'a> {
    let tick = session.tick().expect("tick");
    let mut tock = plugins.respond(tick);
    tock.notices = notices;
    let mut pass = session.advance(&tock).expect("advance");
    plugins.catch_up(std::mem::take(&mut pass.steps));
    pass
}

/// Frames until the session stops producing them: a shell that has finished starting up has
/// nothing more to say, and from there a frame means something this test did.
fn settle(session: &mut Session, plugins: &mut Plugins) {
    let deadline = Instant::now() + PATIENCE;
    let mut quiet = 0;
    while Instant::now() < deadline && quiet < 3 {
        quiet = match frame_showing(session, plugins, Vec::new()).frame {
            Some(_) => 0,
            None => quiet + 1,
        };
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(quiet >= 3, "the session never went quiet");
}

#[test]
fn the_frame_that_takes_an_expired_notice_off_the_screen_is_produced() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[("init.lua", "return {}")]);
    settle(&mut session, &mut plugins);

    let notice = Notice {
        level: Level::Info,
        tag: "fixture".to_string(),
        text: "noticed".to_string(),
    };
    let pass = frame_showing(&mut session, &mut plugins, vec![notice.clone()]);
    assert_eq!(
        pass.frame
            .as_ref()
            .expect("a frame while the notice is up")
            .notices,
        vec![notice]
    );
    drop(pass);

    // The notice has expired, so this frame is the only thing that can stop it being drawn:
    // without it the last picture the client was given still has it in.
    let pass = frame_showing(&mut session, &mut plugins, Vec::new());
    let shown = pass
        .frame
        .expect("the frame that stops showing an expired notice");
    assert!(
        shown.notices.is_empty(),
        "the notice is still in the frame: {:?}",
        shown.notices
    );
}

/// The split in one test: the part a tock does to a session, which is the part shared,
/// and the part it is shown in, which is the part the server makes per client. Two
/// tocks, one session, both applies, both frames.
#[test]
fn two_applies_are_two_stops_on_the_way_to_two_frames() {
    let (_dir, _wakeup, mut session, _plugins) = halves(&[("init.lua", "")]);

    // A tock with no ops is a clean apply — see the application assertions below — and
    // there is no field on the apply for the picture, which is what the split is, the
    // picture not being the apply's to know about.
    let tock = neotmux_proto::msg::Tock {
        ops: Vec::new(),
        overlay: Vec::new(),
        notices: Vec::new(),
        reserved: neotmux_proto::Reserved::default(),
        gutter: 0,
        wants_pane_output: false,
    };
    // A fresh session hands out with its own steps already queued (`Session::new`:
    // the initial `Bands` and `Geometry`), so the first apply carries those and says
    // nothing about the tock. What the split is: the tock with no ops ADDS nothing —
    // the second apply of it carries empty steps and no end, which is the whole of
    // "a clean apply", and the picture is not among the things it can carry.
    let first = session.apply(&tock).expect("apply");
    assert!(first.end.is_none());
    let second = session.apply(&tock).expect("apply");
    assert!(
        second.steps.is_empty(),
        "a tock with no ops adds no steps of its own"
    );
    assert!(second.end.is_none());

    // `frame_with` takes the overlay and notices as its own: two calls, one session, the
    // same panes and size — the picture is the caller's, not the session's, and is the
    // whole of how two clients see the one it is in. A frame borrows the session it reads
    // off, so each one is asked for, read, and let go before the next is.
    let overlay: Vec<neotmux_proto::DrawCmd> = Vec::new();
    let first = session.frame_with(&overlay, Vec::new());
    let first_size = first.size;
    let first_panes: Vec<_> = first.panes.iter().map(|pane| pane.pane).collect();
    drop(first);

    let second = session.frame_with(&overlay, Vec::new());
    let second_size = second.size;
    let second_panes: Vec<_> = second.panes.iter().map(|pane| pane.pane).collect();
    drop(second);

    assert_eq!(first_size, second_size, "the same session, both frames");
    assert_eq!(
        first_panes.len(),
        second_panes.len(),
        "the session's panes, both times"
    );
    for pane in first_panes {
        assert!(
            second_panes.contains(&pane),
            "{pane:?} in the first, not the second"
        );
    }

    // `wants_output` is the whole of the server's question of it: the session holds what
    // it is told, the OR being the server's to keep, not the session's to keep.
    session.wants_output(true);
    session.wants_output(false);
    let _ = session.is_dirty();
}
