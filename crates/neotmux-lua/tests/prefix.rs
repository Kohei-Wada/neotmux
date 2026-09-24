//! `plugin.prefix`'s own registration of its verbs as `neotmux.command`s: first-wins is the
//! rule when another layer's `init.lua` claims a name first, and the loss is said out loud
//! rather than only reaching the log — see `runtime/lua/plugin/prefix.lua`'s `register`.

use std::fs;
use std::path::{Path, PathBuf};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Op;
use neotmux_lua::runtime::Layers;

mod support;

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
}

fn built(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        support::panes(),
    )
    .unwrap();
    (dir, sub)
}

fn notified(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(None)
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A layer that claims `detach` ahead of `plugin.prefix` wins it, the same as any other
/// `neotmux.command` conflict — first registration holds. `plugin.prefix` still loses the
/// name silently in its own log-only report from the host; what this test is after is that it
/// *also* says so through `neotmux.notify`, naming both the key that lost its binding and the
/// command name that was already taken.
#[test]
fn a_refused_verb_is_notified_naming_the_key_and_the_name() {
    let (_dir, sub) = built(
        "
neotmux.command.add('detach', function() end)
require('plugin.prefix')
",
    );
    let errors = sub.run_init();
    assert!(errors.is_empty(), "{errors:?}");
    let said = notified(&sub);
    assert!(
        said.iter()
            .any(|l| l.contains("C-b d") && l.contains("detach")),
        "no notification named both the key and the command: {said:?}"
    );
}

/// The other verbs, none of which were contested, are registered as usual — a conflict on one
/// name does not stop `plugin.prefix` from claiming the rest.
#[test]
fn an_uncontested_verb_still_registers() {
    let (_dir, sub) = built(
        "
neotmux.command.add('detach', function() end)
require('plugin.prefix')
",
    );
    let errors = sub.run_init();
    assert!(errors.is_empty(), "{errors:?}");
    let names: Vec<String> = sub.commands().into_iter().map(|c| c.name).collect();
    assert!(names.iter().any(|n| n == "split-vertical"), "{names:?}");
}
