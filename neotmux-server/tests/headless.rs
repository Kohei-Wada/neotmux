//! A session driven with no terminal at all: no ratatui, no `/dev/tty`, no signal handler,
//! and no socket either. The two halves M3 puts in two processes — the session and the
//! plugins — are both here, passing the same messages to each other across a function call.
//! This is the property the library exists for, so it is tested as a property rather than
//! through any of the session's internals.

use std::fs;
use std::time::{Duration, Instant};

use neotmux_lua::plugin::Layers;
use neotmux_lua::{KeyOutcome, LuaSubsystem};
use neotmux_proto::screen::Frame;
use neotmux_proto::wire::End;
use neotmux_proto::{DrawCmd, Key, KeyCode, Reserved, SessionId, Size, WindowId};
use neotmux_pty::child::{Wakeup, wakeup};
use neotmux_server::{Plugins, Session, Setup};

const SURFACE: Size = Size { rows: 10, cols: 40 };

/// The one session these two halves are. They are about a session rather than about which of
/// them it is, and the server holds one.
fn one() -> SessionId {
    SessionId::from(1)
}
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
    let terminals = std::rc::Rc::new(neotmux_pty::PaneTerminals::new());
    let session = Session::new(
        &Setup {
            size: SURFACE,
            first_window: WindowId::from(1),
            reserved: Reserved::default(),
            gutter: 0,
            pane_band: 0,
            wants_pane_output: false,
        },
        waker,
        terminals.clone(),
        neotmux_layout::PaneIds::new(),
    )
    .expect("start a session");
    let lua = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        terminals,
    )
    .expect("build the Lua subsystem");
    assert!(lua.run_init().is_empty(), "the layer failed to load");
    let mut plugins = Plugins::new(lua);
    // What the server does the moment the load is over. A load's ops belong to no session —
    // there was not one yet — so this is the one drain that takes them.
    assert!(
        !plugins.drain_load_ops(),
        "the layer asked to quit while loading"
    );
    (dir, wakeup, session, plugins)
}

/// A key as the client would offer it: to the plugins first, and on to the focused pane only
/// if nobody wanted it.
fn key(session: &mut Session, plugins: &Plugins, key: Key) {
    if plugins.key(one(), key).0 == KeyOutcome::Fallthrough {
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
fn frame<'a>(session: &'a mut Session, plugins: &mut Plugins) -> neotmux_server::Pass<'a> {
    let tick = session.tick().expect("tick");
    // The server ops are the set of sessions changing, and there is one session here.
    let answer = plugins.respond(one(), tick);
    let mut pass = session
        .advance(&answer.tock, &answer.views)
        .expect("advance");
    // What the ops did, told to the plugins before anything else happens — the same round
    // the client keeps its keyboard out of.
    plugins.catch_up(one(), std::mem::take(&mut pass.steps));
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
            "neotmux.on_tick(function(ctx)\n\
             \x20 ctx:text(nil, 1, 2, 'painted')\n\
             \x20 for at, one in ipairs(neotmux.notifications.list()) do\n\
             \x20   ctx:text(nil, 0, at - 1, one.level .. ' ' .. one.tag .. ': ' .. one.text)\n\
             \x20 end\n\
             end)\n\
             neotmux.notify('info', 'noticed')\n\
             return {}",
        ),
    ]);
    // `notify` ran at load time, so the startup drain in `halves` took it — into the
    // notification surface, which is where a notice comes from whatever raised it.
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
    // The notification is in the frame because a plugin read it off the host's surface and
    // drew it. It is not a field of the frame any more: what a notification looks like is the
    // same kind of decision as what a status bar looks like, and both are Lua's.
    assert!(
        frame.overlay.iter().any(|cmd| matches!(
            cmd,
            DrawCmd::Text { text, .. } if text == "info plugin/painter: noticed"
        )),
        "the notification never reached a plugin: {:?}",
        frame.overlay
    );
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

/// Frames until the session stops producing them: a shell that has finished starting up has
/// nothing more to say, and from there a frame means something this test did.
fn settle(session: &mut Session, plugins: &mut Plugins) {
    let deadline = Instant::now() + PATIENCE;
    let mut quiet = 0;
    while Instant::now() < deadline && quiet < 3 {
        quiet = match frame(session, plugins).frame {
            Some(_) => 0,
            None => quiet + 1,
        };
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(quiet >= 3, "the session never went quiet");
}

/// A notification going away produces the frame that stops showing it. It used to be a field
/// of the frame, compared per client; it is a plugin's drawing now, and the overlay changing
/// is what the session already watches for.
#[test]
fn the_frame_that_takes_a_cleared_notice_off_the_screen_is_produced() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[(
        "init.lua",
        "neotmux.on_tick(function(ctx)\n\
         \x20 for at, one in ipairs(neotmux.notifications.list()) do\n\
         \x20   ctx:text(nil, 0, at - 1, one.text)\n\
         \x20 end\n\
         end)\n\
         neotmux.keymap.set('N', function() neotmux.notify('info', 'noticed') return true end)\n\
         neotmux.keymap.set('C', function() neotmux.notifications.clear() return true end)\n",
    )]);
    settle(&mut session, &mut plugins);

    for typed in typed("N") {
        key(&mut session, &plugins, typed);
    }
    // The op the key raised is drained after that tick, so the surface has it on the next
    // one and a plugin draws it on the one after. Two frames, not a wait.
    assert!(
        drawn_within(&mut session, &mut plugins, true),
        "the notice was never drawn"
    );

    for typed in typed("C") {
        key(&mut session, &plugins, typed);
    }
    assert!(
        drawn_within(&mut session, &mut plugins, false),
        "the notice is still drawn after it was cleared"
    );
}

/// A plugin winding a pane back into its history is a change to the picture even when it
/// draws nothing different: copy mode's cursor on the bottom row stays on the bottom row as
/// `j` scrolls the view under it, so the overlay is the same and only the view has moved.
#[test]
fn a_view_that_moves_with_the_overlay_unchanged_produces_a_frame() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[(
        "init.lua",
        "local back = 0\n\
         neotmux.keymap.set('V', function() back = 50 return true end)\n\
         neotmux.on_tick(function(ctx)\n\
         \x20 local pane = neotmux.pane.current()\n\
         \x20 if pane then ctx:view(pane, back) end\n\
         end)\n",
    )]);
    for typed in typed("seq 1 100\n") {
        key(&mut session, &plugins, typed);
    }
    assert!(step_until(&mut session, &mut plugins, |frame| {
        screen(frame).lines().any(|row| row.trim() == "100")
    }));
    settle(&mut session, &mut plugins);

    for typed in typed("V") {
        key(&mut session, &plugins, typed);
    }
    let mut moved = false;
    for _ in 0..5 {
        if let Some(shown) = frame(&mut session, &mut plugins).frame {
            moved = screen(&shown).lines().any(|row| row.trim() == "50");
            break;
        }
    }
    assert!(moved, "the view moved and no frame showed it");
}

/// Whether `noticed` is in the overlay, or is not, within a few frames of asking. An op is
/// drained after the tick that raised it, so a change the keyboard made takes a frame or two
/// to reach the picture — and this is the only thing either assertion is waiting for.
fn drawn_within(session: &mut Session, plugins: &mut Plugins, want: bool) -> bool {
    for _ in 0..5 {
        if let Some(shown) = frame(session, plugins).frame {
            let has = shown
                .overlay
                .iter()
                .any(|cmd| matches!(cmd, DrawCmd::Text { text, .. } if text == "noticed"));
            if has == want {
                return true;
            }
        }
    }
    false
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
    let tock = neotmux_proto::wire::Tock {
        ops: Vec::new(),
        overlay: Vec::new(),
        reserved: neotmux_proto::Reserved::default(),
        gutter: 0,
        pane_band: 0,
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
    let first = session.frame_with(&overlay, &Default::default());
    let first_size = first.size;
    let first_panes: Vec<_> = first.panes.iter().map(|pane| pane.pane).collect();
    drop(first);

    let second = session.frame_with(&overlay, &Default::default());
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

/// A source that draws how many times it has ticked, so a frame shows whether it ticked.
const COUNTING: &str = "local n = 0\n\
    neotmux.keymap.set('Q', function() return true end)\n\
    neotmux.on_tick(function(ctx)\n\
      n = n + 1\n\
      ctx:text(nil, 0, 0, 'ticks ' .. n)\n\
    end)\n";

/// With nothing happening, the plugins do not tick: a frame stands on the last one they drew.
/// A key a plugin takes is something happening, and so is a quarter of a second going by —
/// what changes with time alone still changes, a little late.
#[test]
fn a_quiet_session_skips_its_ticks_until_a_key_or_the_idle_interval() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[("init.lua", COUNTING)]);
    settle(&mut session, &mut plugins);

    for _ in 0..5 {
        assert!(
            frame(&mut session, &mut plugins).frame.is_none(),
            "a frame with nothing happening ticked the plugins"
        );
    }

    key(&mut session, &plugins, Key::plain(KeyCode::Char('Q')));
    assert!(
        frame(&mut session, &mut plugins).frame.is_some(),
        "a key a plugin took did not tick it"
    );
    assert!(frame(&mut session, &mut plugins).frame.is_none());

    std::thread::sleep(Duration::from_millis(300));
    assert!(
        frame(&mut session, &mut plugins).frame.is_some(),
        "a quarter of a second went by and nothing ticked"
    );
}

/// A pane band held by a plugin narrows the pane's shell by its columns and moves its grid
/// across, and giving it back puts both back.
#[test]
fn a_pane_band_narrows_the_shell_and_giving_it_back_widens_it() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[(
        "init.lua",
        "local held\n\
         neotmux.keymap.set('B', function()\n\
           if held then held:remove() held = nil\n\
           else held = neotmux.reserve_pane('left', 10) end\n\
           return true\n\
         end)\n",
    )]);
    key(&mut session, &plugins, Key::plain(KeyCode::Char('B')));
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            frame
                .panes
                .iter()
                .all(|pane| pane.rect.x == 10 && pane.grid.size.cols == SURFACE.cols - 10)
        }),
        "the pane was not narrowed"
    );
    key(&mut session, &plugins, Key::plain(KeyCode::Char('B')));
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            frame
                .panes
                .iter()
                .all(|pane| pane.rect.x == 0 && pane.grid.size.cols == SURFACE.cols)
        }),
        "the pane did not get its columns back"
    );
}

/// A pane made while a band is held starts at its content width, not at the width of the
/// rect the tree cut for it.
#[test]
fn a_pane_split_while_a_band_is_held_starts_at_its_content_width() {
    let (_dir, _wakeup, mut session, mut plugins) = halves(&[(
        "init.lua",
        "neotmux.reserve_pane('left', 5)\n\
         neotmux.keymap.set('S', function() neotmux.pane.split('vertical') return true end)\n",
    )]);
    assert!(step_until(&mut session, &mut plugins, |frame| {
        frame.panes.iter().all(|pane| pane.rect.x == 5)
    }));
    key(&mut session, &plugins, Key::plain(KeyCode::Char('S')));
    assert!(
        step_until(&mut session, &mut plugins, |frame| {
            frame.panes.len() == 2
                && frame
                    .panes
                    .iter()
                    .all(|pane| pane.grid.size.cols == pane.rect.w && pane.rect.w == 15)
        }),
        "the new pane's shell is not its content's width"
    );
}
