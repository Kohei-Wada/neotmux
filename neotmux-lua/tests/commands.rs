//! `neotmux.command`: a plugin registers a named callback, anything can list what is
//! registered, and anything can call one by name — the in-process half of the command
//! surface spec's §4.

use std::fs;

use neotmux_lua::host::Op;
use neotmux_lua::plugin::Layers;
use neotmux_lua::{CommandOutcome, LuaSubsystem};

mod support;

fn built(files: &[(&str, &str)]) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

fn logged(sub: &LuaSubsystem) -> Vec<String> {
    sub.drain_ops(None)
        .iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_command_can_be_added_and_listed_with_its_description() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('scratch', function(args) end, { desc = 'open a scratch window' })",
    )]);
    let commands = sub.commands();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "scratch");
    assert_eq!(commands[0].desc.as_deref(), Some("open a scratch window"));
    assert_eq!(commands[0].source, "init");
}

#[test]
fn a_command_runs_and_its_lines_come_back() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('scratch', function(args) return { 'line one', 'line two' } end)",
    )]);
    assert_eq!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Ran(vec!["line one".to_string(), "line two".to_string()])
    );
}

#[test]
fn a_name_nobody_registered_is_unknown() {
    let (_dir, sub) = built(&[("init.lua", "")]);
    assert_eq!(
        sub.run_command(support::one(), "nope", &[]),
        CommandOutcome::Unknown
    );
}

#[test]
fn a_handler_that_raises_is_a_failure_naming_its_source() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('boom', function(args) error('kaboom') end)",
    )]);
    let CommandOutcome::Failed(why) = sub.run_command(support::one(), "boom", &[]) else {
        panic!("a raising handler did not fail");
    };
    assert!(why.contains("init"), "the failure did not name init: {why}");
    assert!(why.contains("kaboom"), "{why}");
}

#[test]
fn a_map_shaped_table_is_a_failure_naming_its_source() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('m', function() return { foo = 'bar' } end)",
    )]);
    let CommandOutcome::Failed(why) = sub.run_command(support::one(), "m", &[]) else {
        panic!("a map-shaped return did not fail");
    };
    assert!(why.contains("init"), "{why}");
}

#[test]
fn a_mixed_table_is_a_failure_naming_its_source() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('m', function() return { 'a', extra = 1 } end)",
    )]);
    let CommandOutcome::Failed(why) = sub.run_command(support::one(), "m", &[]) else {
        panic!("a mixed table return did not fail");
    };
    assert!(why.contains("init"), "{why}");
}

#[test]
fn an_array_holding_a_number_is_a_failure_naming_its_source() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('m', function() return { 1, 2, 3 } end)",
    )]);
    let CommandOutcome::Failed(why) = sub.run_command(support::one(), "m", &[]) else {
        panic!("an array of numbers did not fail");
    };
    assert!(why.contains("init"), "{why}");
}

#[test]
fn a_second_registration_from_another_source_is_refused_and_the_first_holds() {
    let (_dir, sub) = built(&[
        (
            "lua/plugin/first.lua",
            "neotmux.command.add('scratch', function() return 'first' end)",
        ),
        (
            "lua/plugin/second.lua",
            "local ok, err = neotmux.command.add('scratch', function() return 'second' end)
             assert(ok == nil, 'the second registration should have been refused')
             neotmux.log.info(err)",
        ),
        (
            "init.lua",
            "require('plugin.first') require('plugin.second')",
        ),
    ]);
    assert_eq!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Ran(vec!["first".to_string()])
    );
    let lines = logged(&sub);
    assert!(
        lines.iter().any(|l| l.contains("already registered")),
        "{lines:?}"
    );
}

/// A plugin run again registers its commands again, and they are its own: the second one is
/// the one that answers.
#[test]
fn a_second_registration_from_the_same_source_replaces_the_first() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "assert(neotmux.command.add('scratch', function() return 'first' end))
         assert(neotmux.command.add('scratch', function() return 'second' end))",
    )]);
    assert_eq!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Ran(vec!["second".to_string()])
    );
}

#[test]
fn a_removed_command_is_unknown_and_a_second_remove_says_so() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('scratch', function() return 'x' end)
         assert(neotmux.command.remove('scratch') == true)
         assert(neotmux.command.remove('scratch') == false)",
    )]);
    assert!(matches!(
        sub.run_command(support::one(), "scratch", &[]),
        CommandOutcome::Unknown
    ));
}

#[test]
fn the_arguments_reach_the_handler_as_an_array_of_strings() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('echo', function(args) return table.concat(args, ',') end)",
    )]);
    assert_eq!(
        sub.run_command(
            support::one(),
            "echo",
            &["a".to_string(), "b c".to_string()]
        ),
        CommandOutcome::Ran(vec!["a,b c".to_string()])
    );
}

#[test]
fn a_command_can_be_called_from_lua_by_name() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('helper', function(args)
           neotmux.log.info('helper ran with ' .. args[1])
         end)
         local ok, why = neotmux.command.run('helper', { 'x' })
         assert(ok == true, tostring(why))",
    )]);
    let lines = logged(&sub);
    assert!(lines.iter().any(|l| l == "helper ran with x"), "{lines:?}");
}

#[test]
fn run_answers_the_lines_a_handler_returned_as_its_third_value() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('lines', function() return { 'a', 'b' } end)
         neotmux.command.add('empty', function() end)
         neotmux.command.add('boom', function() error('kaboom') end)

         local ok1, err1, lines1 = neotmux.command.run('lines')
         assert(ok1 == true, tostring(err1))
         assert(err1 == nil, 'a success carried an error message')
         assert(#lines1 == 2 and lines1[1] == 'a' and lines1[2] == 'b', 'wrong lines')

         local ok2, err2, lines2 = neotmux.command.run('empty')
         assert(ok2 == true, tostring(err2))
         assert(type(lines2) == 'table' and #lines2 == 0, 'a handler with nothing to say did not answer an empty table')

         local ok3, err3, lines3 = neotmux.command.run('boom')
         assert(ok3 == false, 'a raising handler answered ok')
         assert(err3 ~= nil, 'a failure carried no message')
         assert(lines3 == nil, 'a failure carried lines')

         neotmux.log.info('checked')",
    )]);
    let lines = logged(&sub);
    assert!(lines.iter().any(|l| l == "checked"), "{lines:?}");
}

#[test]
fn list_answers_in_name_order() {
    let (_dir, sub) = built(&[(
        "init.lua",
        "neotmux.command.add('zeta', function() end)
         neotmux.command.add('alpha', function() end)
         local names = {}
         for _, c in ipairs(neotmux.command.list()) do names[#names + 1] = c.name end
         neotmux.notify('info', table.concat(names, ' '))",
    )]);
    let ops = sub.drain_ops(None);
    let (_, op) = ops
        .into_iter()
        .find(|(_, op)| matches!(op, Op::Notify { .. }))
        .expect("no notify op");
    let Op::Notify { text, .. } = op else {
        unreachable!()
    };
    assert_eq!(text, "alpha zeta");
}
