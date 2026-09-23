//! `neotmux.clipboard.set`: what a plugin is told about a clipboard write.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;

mod support;

fn built(body: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    let init = format!("neotmux.on_tick(function()\n{body}\nend)\n");
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn ticked(sub: &LuaSubsystem) -> Vec<String> {
    sub.dispatch_tick();
    sub.drain_ops()
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// Two values, not one: the second is what a plugin has to put in front of a person instead
/// of inventing "copied".
#[test]
fn a_write_answers_with_whether_it_was_written_and_why_not() {
    let (_dir, sub) = built(
        "local ok, err = neotmux.clipboard.set('yanked')\n\
         neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))",
    );
    let line = ticked(&sub).remove(0);
    assert!(
        line == "true nil" || line.starts_with("false "),
        "the call answered something other than a write and its reason: {line:?}"
    );
}

#[test]
fn clipboard_cannot_be_replaced() {
    let (_dir, sub) = built(
        "local ok = pcall(function() neotmux.clipboard.set = function() end end)\n\
         neotmux.log.info(tostring(ok))",
    );
    assert_eq!(ticked(&sub), vec!["false"]);
}

/// The host's implementation is what a write reaches, and what it answers is what the plugin
/// is told. The Lua crate does not own a terminal to write to.
#[test]
fn a_write_reaches_the_clipboard_the_host_gave() {
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Recording(RefCell<Vec<String>>);
    impl neotmux_lua::clipboard::Clipboard for Recording {
        fn set(&self, text: &str) -> std::io::Result<()> {
            self.0.borrow_mut().push(text.to_string());
            Ok(())
        }
    }

    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "neotmux.on_tick(function()\n\
         local ok, err = neotmux.clipboard.set('yanked')\n\
         neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))\n\
         end)\n",
    )
    .unwrap();

    let recording = Rc::new(Recording(RefCell::new(Vec::new())));
    let sub = LuaSubsystem::build_named(
        "default",
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
        recording.clone(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());

    assert_eq!(ticked(&sub), vec!["true nil"]);
    assert_eq!(*recording.0.borrow(), vec!["yanked".to_string()]);
}

/// A subsystem built without one says so rather than claiming a write it did not make. This
/// is what `LuaSubsystem::build` gives, and `build` is for tests and the headless loop.
#[test]
fn without_a_clipboard_the_answer_is_false_and_a_reason() {
    let (_dir, sub) = built(
        "local ok, err = neotmux.clipboard.set('yanked')\n\
         neotmux.log.info(tostring(ok) .. ' ' .. tostring(err))",
    );
    let line = ticked(&sub).remove(0);
    assert_eq!(line, "false this host has no clipboard");
}
