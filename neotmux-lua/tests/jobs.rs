//! `neotmux.job`: a plugin runs a program and is called back with what it said. Every test
//! here drives a fake `Runner`, so nothing spawns and nothing waits — what a real process adds
//! is tested once, in `neotmux-server/tests/job_process.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::Op;
use neotmux_lua::host::{JobChunk, Runner};
use neotmux_lua::plugin::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size};

mod support;

use support::{Built, FakeRunner};

const SIZE: Size = Size { rows: 10, cols: 40 };

fn pane() -> PaneId {
    PaneId::from(1)
}

fn shipped_layer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime")
}

fn built(extra: &str, runner: Rc<FakeRunner>) -> (tempfile::TempDir, Built) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'sync', 'hints' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::host::Absent),
        runner,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    begin(&sub, support::one());
    (dir, Built::new(sub, terminals))
}

/// The geometry one session is drawn against. A test that drives more than one session calls
/// it again for the other, since a session with nothing published has no frame to run.
fn begin(sub: &LuaSubsystem, session: SessionId) {
    sub.begin_frame(
        session,
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            focus: Some(pane()),
            ..Default::default()
        },
    );
}

/// Starts a job when `J` is pressed. Most tests here start from a key rather than from the
/// `init.lua`, so that the session a job's callbacks run in is the obvious one rather than the
/// adoption rule's.
fn on_key(body: &str) -> String {
    format!(
        "neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
         {body}\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

fn keys(sub: &LuaSubsystem, specs: &[&str]) {
    for spec in specs {
        sub.dispatch_key(support::one(), spec);
    }
}

/// One frame: deliver whatever the jobs have said, then tick.
fn frame(sub: &LuaSubsystem) {
    sub.dispatch_jobs(support::one());
    sub.dispatch_tick(support::one());
}

fn logged(sub: &Built) -> Vec<String> {
    logged_in(sub, support::one())
}

fn logged_in(sub: &Built, session: SessionId) -> Vec<String> {
    sub.drain_ops(Some(session))
        .into_iter()
        .filter_map(|(_, op)| match op {
            Op::Log { text, .. } | Op::Notify { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// A job that runs and stops hands its whole output to `on_exit`, unsplit.
#[test]
fn a_buffered_job_answers_with_everything_it_said() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"one\ntw".to_vec())],
        vec![JobChunk::Stdout(b"o\n".to_vec()), JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'echo' }, {\n\
                   on_exit = function(code, out, err_text, err)\n\
                     neotmux.log.info('exit code=' .. tostring(code)\n\
                       .. ' out=[' .. tostring(out) .. ']'\n\
                       .. ' err=' .. tostring(err))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert!(logged(&sub).is_empty(), "on_exit ran before the job ended");

    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line == "exit code=0 out=[one\ntwo\n] err=nil"),
        "{said:?}"
    );
}

/// The command is a list, and it reaches the runner as one.
#[test]
fn the_command_and_its_arguments_reach_the_runner_as_a_list() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &on_key("    neotmux.job.start({ 'git', 'status', '--porcelain' }, { cwd = '/tmp' })"),
        Rc::clone(&runner),
    );
    keys(&sub, &["J"]);

    let started = runner.started.borrow();
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0].cmd, vec!["git", "status", "--porcelain"]);
    assert_eq!(started[0].cwd.as_deref(), Some("/tmp"));
}

/// The `init.lua` body a job started at load time is written in. It runs before any session
/// exists, so `neotmux.session_name()` is nil there and the job has no session of its own.
fn at_load() -> String {
    "neotmux.job.start({ 'echo' }, {\n\
       on_exit = function(code, out)\n\
         neotmux.log.info('exit code=' .. tostring(code)\n\
           .. ' out=[' .. tostring(out) .. ']')\n\
       end,\n\
     })\n"
        .to_string()
}

/// A job has to be startable from an `init.lua`, which is where a plugin most naturally does
/// its setup, and there is no session bound there. The first session whose frame comes looking
/// adopts it, and that is the session its callbacks run in.
#[test]
fn a_job_started_before_any_session_is_adopted_by_the_first_frame() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"loaded\n".to_vec()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(&at_load(), Rc::clone(&runner));

    assert_eq!(runner.started.borrow().len(), 1, "the load started no job");
    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter().any(|line| line == "exit code=0 out=[loaded\n]"),
        "{said:?}"
    );
}

/// Adoption is a handover, not a broadcast: once a session has picked a job up it is that
/// session's, and every other session's frame walks past it without so much as polling it.
/// Polling matters as much as delivering — a poll from the wrong session would eat the chunk
/// the owner was owed.
#[test]
fn a_session_that_did_not_adopt_a_job_never_sees_it() {
    let two = SessionId::from(2);
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"a\n".to_vec())],
        vec![JobChunk::Stdout(b"b\n".to_vec()), JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(&at_load(), Rc::clone(&runner));
    begin(&sub, two);

    // Session two gets there first, so the job is two's from here on.
    sub.dispatch_jobs(two);
    sub.dispatch_jobs(support::one());
    assert!(
        logged_in(&sub, support::one()).is_empty(),
        "session one heard a job it never adopted"
    );

    // If session one had polled, it would have taken this poll's `Exited` and session two
    // would wait forever; if it had merely delivered, `b` would be missing from the output.
    sub.dispatch_jobs(two);
    let said = logged_in(&sub, two);
    assert!(
        said.iter().any(|line| line == "exit code=0 out=[a\nb\n]"),
        "{said:?}"
    );
    assert!(
        logged_in(&sub, support::one()).is_empty(),
        "session one heard the end of a job it never adopted"
    );
}

/// A streaming job is handed complete lines and nothing else: a chunk that stops mid-line is
/// held until the rest of it arrives, and what is still held at the end is flushed as one
/// last line. A plugin never joins a fragment to its remainder because it is never given one.
#[test]
fn a_streaming_job_is_handed_whole_lines_and_the_last_fragment_is_flushed() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"alpha\nbra".to_vec())],
        vec![JobChunk::Stdout(b"vo\ncharlie".to_vec())],
        vec![JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'tail' }, {\n\
                   buffered = false,\n\
                   on_stdout = function(lines)\n\
                     neotmux.log.info('lines=[' .. table.concat(lines, '|') .. ']')\n\
                   end,\n\
                   on_exit = function(code, out)\n\
                     neotmux.log.info('exit out=' .. tostring(out))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(logged(&sub), vec!["lines=[alpha]"]);

    frame(&sub);
    assert_eq!(logged(&sub), vec!["lines=[bravo]"]);

    frame(&sub);
    let said = logged(&sub);
    assert_eq!(
        said,
        vec!["lines=[charlie]", "exit out=nil"],
        "the held fragment was not flushed before on_exit, or on_exit kept a copy"
    );
}

/// `kill` ends it, and the caller hears the end of what it started: `code` nil, `err` set.
#[test]
fn killing_a_job_still_calls_on_exit_once() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let (_dir, sub) = built(
        "local job\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             job = neotmux.job.start({ 'sleep' }, {\n\
               on_exit = function(code, out, errout, err)\n\
                 neotmux.log.info('exit code=' .. tostring(code) .. ' err=' .. tostring(err))\n\
               end,\n\
             })\n\
             return true\n\
           end\n\
           if key == 'K' then\n\
             neotmux.log.info('killed=' .. tostring(job:kill()))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    sub.dispatch_key(support::one(), "K");
    frame(&sub);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "killed=true"), "{said:?}");
    assert_eq!(
        said.iter()
            .filter(|line| line.starts_with("exit code=nil err=killed"))
            .count(),
        1,
        "on_exit did not run exactly once for a killed job: {said:?}"
    );

    // A second kill is not a second ending.
    sub.dispatch_key(support::one(), "K");
    frame(&sub);
    let said = logged(&sub);
    assert!(said.iter().any(|line| line == "killed=false"), "{said:?}");
    assert!(
        !said.iter().any(|line| line.starts_with("exit ")),
        "a dead job ended twice: {said:?}"
    );
}

/// A kill the system refused is not reported as a kill: the program may still be running, and
/// `err = killed` would tell the plugin it is not (#41).
#[test]
fn a_kill_that_failed_does_not_say_killed() {
    let runner = FakeRunner::new();
    runner.cannot_kill("operation not permitted");
    let (_dir, sub) = built(
        "local job\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             job = neotmux.job.start({ 'sleep' }, {\n\
               on_exit = function(code, out, errout, err)\n\
                 neotmux.log.info('exit err=' .. tostring(err))\n\
               end,\n\
             })\n\
             return true\n\
           end\n\
           if key == 'K' then\n\
             job:kill()\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    sub.dispatch_key(support::one(), "K");
    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line == "exit err=killed, but the kill failed: operation not permitted"),
        "{said:?}"
    );
}

/// A job that says more than it is allowed to is killed, and says why.
#[test]
fn a_job_that_overruns_the_output_limit_is_killed_and_says_so() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(vec![b'x'; 64])]]);
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.job_output_bytes = 16\n{}",
            on_key(
                "    neotmux.job.start({ 'yes' }, {\n\
                       on_exit = function(code, out, errout, err)\n\
                         neotmux.log.info('exit code=' .. tostring(code)\n\
                           .. ' err=' .. tostring(err)\n\
                           .. ' kept=' .. tostring(out and #out))\n\
                       end,\n\
                     })",
            )
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line.starts_with("exit code=nil err=output limit")),
        "{said:?}"
    );
    assert!(*runner.killed.borrow(), "the job was not killed");
}

/// The ninth job of one source is refused, and refusing is not raising.
#[test]
fn a_source_may_not_have_more_jobs_than_the_limit() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.job_concurrency = 2\n{}",
            on_key(
                "    local kept = {}\n\
                     for i = 1, 3 do\n\
                       local job, why = neotmux.job.start({ 'sleep' }, {})\n\
                       kept[i] = job\n\
                       neotmux.log.info('start ' .. i .. ' = ' .. tostring(job ~= nil)\n\
                         .. ' why=' .. tostring(why))\n\
                     end",
            )
        ),
        Rc::clone(&runner),
    );
    keys(&sub, &["J"]);

    let said = logged(&sub);
    assert!(
        said.iter().any(|l| l == "start 1 = true why=nil"),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l == "start 2 = true why=nil"),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("start 3 = false why=too many jobs")),
        "{said:?}"
    );
}

/// What a streaming job logs, one delivery per line, so that a test can see both how many
/// deliveries there were and where the breaks fell.
fn streaming(body: &str) -> String {
    on_key(&format!(
        "    neotmux.job.start({{ 'tail' }}, {{\n\
               buffered = false,\n\
               on_stdout = function(lines)\n\
                 neotmux.log.info('out=[' .. table.concat(lines, '|') .. ']')\n\
               end,\n\
               on_stderr = function(lines)\n\
                 neotmux.log.info('err=[' .. table.concat(lines, '|') .. ']')\n\
               end,\n\
             {body}\
             }})",
    ))
}

/// The end of a streaming job, said the way every other test here says it.
const STREAM_EXIT: &str = "    on_exit = function(code, out, errout, err)\n\
                             neotmux.log.info('exit code=' .. tostring(code)\n\
                               .. ' err=' .. tostring(err))\n\
                           end,\n";

/// The limit bounds what a job makes the host hold, not what it has already said. A streaming
/// job's complete lines are handed over and dropped inside the frame they arrive in, so output
/// many times the limit must not end it: streaming mode exists for output that never stops,
/// and a limit that counted delivered lines would defeat its own purpose.
#[test]
fn a_streaming_job_is_not_killed_for_lines_it_has_already_been_handed() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        (0..8)
            .map(|n| JobChunk::Stdout(format!("line-{n}\n").into_bytes()))
            .collect(),
        vec![JobChunk::Stdout(b"done\n".to_vec()), JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.job_output_bytes = 16\n{}",
            streaming(STREAM_EXIT)
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(
        logged(&sub),
        vec!["out=[line-0|line-1|line-2|line-3|line-4|line-5|line-6|line-7]"],
        "the job was cut off for output it had already been handed"
    );

    frame(&sub);
    assert_eq!(logged(&sub), vec!["out=[done]", "exit code=0 err=nil"]);
    assert!(!*runner.killed.borrow(), "the job was killed");
}

/// What the limit does bound in streaming mode is the fragment nobody can be handed: a program
/// that says more than the limit without ever ending a line is a program the host would have to
/// hold all of.
#[test]
fn a_streaming_job_that_holds_a_fragment_past_the_limit_is_killed() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(vec![b'x'; 64])]]);
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.job_output_bytes = 16\n{}",
            streaming(STREAM_EXIT)
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    let said = logged(&sub);
    assert!(
        said.iter()
            .any(|line| line == "exit code=nil err=output limit"),
        "{said:?}"
    );
    assert!(*runner.killed.borrow(), "the job was not killed");
}

/// A program that ends its lines the DOS way means the same lines as one that does not, and a
/// chunk boundary between the `\r` and the `\n` is still not a line boundary.
#[test]
fn a_carriage_return_before_the_break_belongs_to_the_break_across_chunks() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"alpha\r".to_vec())],
        vec![JobChunk::Stdout(b"\nbravo\r\n".to_vec())],
        vec![JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(&streaming(STREAM_EXIT), Rc::clone(&runner));

    keys(&sub, &["J"]);
    frame(&sub);
    assert!(
        logged(&sub).is_empty(),
        "a line whose break had not arrived was handed over anyway"
    );

    frame(&sub);
    assert_eq!(logged(&sub), vec!["out=[alpha|bravo]"]);

    frame(&sub);
    assert_eq!(logged(&sub), vec!["exit code=0 err=nil"]);
}

/// The flush is what is left over, so output that ends on a break leaves nothing: an empty line
/// at the end would be one the program never said. An empty line it did say is a line.
#[test]
fn output_that_ends_on_a_break_invents_no_last_line() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"one\n\nthree\n".to_vec())],
        vec![JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(&streaming(STREAM_EXIT), Rc::clone(&runner));

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(logged(&sub), vec!["out=[one||three]"]);

    frame(&sub);
    assert_eq!(
        logged(&sub),
        vec!["exit code=0 err=nil"],
        "the flush made up a line out of nothing left over"
    );
}

/// Standard error streams the same way standard output does, down to the last fragment: a
/// plugin that reads a program's diagnostics as they come reads them as lines too.
#[test]
fn stderr_streams_in_lines_and_flushes_what_is_left() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stderr(b"warn one\nwarn tw".to_vec())],
        vec![JobChunk::Stderr(b"o".to_vec()), JobChunk::Exited(1)],
    ]);
    let (_dir, sub) = built(&streaming(STREAM_EXIT), Rc::clone(&runner));

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(logged(&sub), vec!["err=[warn one]"]);

    frame(&sub);
    assert_eq!(logged(&sub), vec!["err=[warn two]", "exit code=1 err=nil"]);
}

/// A job that never got as far as exiting still ends, once: the runner's failure is the
/// ending, and `on_exit` is where a plugin hears about it.
#[test]
fn a_job_that_failed_calls_on_exit_once() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Failed("the pipe broke".to_string())]]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'echo' }, {\n\
                   on_exit = function(code, out, errout, err)\n\
                     neotmux.log.info('exit code=' .. tostring(code)\n\
                       .. ' err=' .. tostring(err))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(
        logged(&sub),
        vec!["exit code=nil err=the pipe broke"],
        "a failed job did not end exactly once"
    );

    frame(&sub);
    assert!(logged(&sub).is_empty(), "a failed job ended twice");
}

/// A raising callback is reported and the job keeps running: the next chunk may well be one
/// the plugin handles, and a plugin that wants out calls `kill`.
#[test]
fn a_raising_callback_does_not_end_the_job() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"one\n".to_vec())],
        vec![JobChunk::Stdout(b"two\n".to_vec())],
        vec![JobChunk::Exited(0)],
    ]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'tail' }, {\n\
                   buffered = false,\n\
                   on_stdout = function(lines)\n\
                     neotmux.log.info('saw ' .. lines[1])\n\
                     error('boom')\n\
                   end,\n\
                   on_exit = function() neotmux.log.info('exited') end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    frame(&sub);
    frame(&sub);
    let said = logged(&sub);
    assert!(said.iter().any(|l| l == "saw one"), "{said:?}");
    assert!(said.iter().any(|l| l == "saw two"), "{said:?}");
    assert!(said.iter().any(|l| l == "exited"), "{said:?}");
}

/// A plugin whose load fails takes its jobs with it: the callbacks belong to a chunk that is
/// not there, so nothing is left that could be told how the job ended. The require-failure
/// sweep does it — the same sweep that drops registrations, reservations and floats.
#[test]
fn a_require_that_fails_takes_its_sources_jobs_with_it() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(
        dir.path().join("lua/plugin/worker.lua"),
        "neotmux.job.start({ 'sleep' }, {\n\
           on_exit = function() neotmux.log.info('exited') end,\n\
         })\n\
         error('nope')\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "local defaults = require('neotmux.defaults')\n\
         for _, name in ipairs({ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
           'notify', 'borders', 'sync', 'hints' }) do\n\
           defaults[name] = false\n\
         end\n\
         require('plugin.worker')\n",
    )
    .unwrap();

    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::host::Absent),
        Rc::clone(&runner) as Rc<dyn Runner>,
    )
    .unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1, "the load did not fail: {failures:?}");
    begin(&sub, support::one());
    let sub = Built::new(sub, terminals);

    assert_eq!(runner.started.borrow().len(), 1, "the job never started");
    assert!(
        *runner.killed.borrow(),
        "the failed require left its source's job running"
    );
    frame(&sub);
    assert!(
        logged(&sub).is_empty(),
        "a job whose source is gone reported its ending to it"
    );
}

/// A session that is gone takes the jobs it adopted with it — but `on_exit` still runs, once,
/// as it does for every other way a job can end. The chunk that wrote the callback is alive,
/// and a plugin holding a lock for the length of a job has to hear that the job is over.
#[test]
fn forgetting_a_session_ends_its_jobs_and_says_why() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'sleep' }, {\n\
                   on_exit = function(code, out, errout, err)\n\
                     neotmux.log.info('exit code=' .. tostring(code)\n\
                       .. ' out=[' .. tostring(out) .. ']'\n\
                       .. ' err=' .. tostring(err))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert!(!*runner.killed.borrow(), "a live job was killed");

    sub.forget_session(support::one());
    assert!(*runner.killed.borrow(), "a forgotten session's job ran on");
    let said = logged(&sub);
    assert_eq!(
        said,
        vec!["exit code=nil out=[working\n] err=session ended"],
        "a session ending did not end its job exactly once"
    );

    frame(&sub);
    assert!(logged(&sub).is_empty(), "the job ended twice");
}

/// A plugin that starts a job from inside the `on_exit` a session ending gave it is refused.
/// The session it would be adopted by is the one going away, so nothing would ever sweep it
/// and the child would outlive the whole thing.
#[test]
fn a_job_started_while_a_session_is_being_swept_is_refused() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'sleep' }, {\n\
                   on_exit = function()\n\
                     local job, why = neotmux.job.start({ 'echo' })\n\
                     neotmux.log.info('again=' .. tostring(job) .. ' why=' .. tostring(why))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    sub.forget_session(support::one());

    assert_eq!(
        logged(&sub),
        vec!["again=nil why=the session is ending"],
        "a job was started into a session that is going away"
    );
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "the second start reached the runner anyway"
    );
}

/// A plugin that arms a timer from inside the `on_exit` a session ending gave it is refused,
/// the way a `job.start` there is. The timer would belong to the session going away: no frame
/// of it is left to fire the timer, and the sweep that would have dropped it is already past,
/// so it would sit in the map holding its callback for the life of the server.
#[test]
fn a_timer_armed_while_a_session_is_being_swept_is_refused() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &on_key(
            "    neotmux.job.start({ 'sleep' }, {\n\
                   on_exit = function()\n\
                     local timer, why = neotmux.timer.after(0, function()\n\
                       neotmux.log.info('fired')\n\
                     end)\n\
                     neotmux.log.info('again=' .. tostring(timer)\n\
                       .. ' why=' .. tostring(why))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    sub.forget_session(support::one());

    assert_eq!(
        logged(&sub),
        vec!["again=nil why=the session is ending"],
        "a timer was armed into a session that is going away"
    );

    sub.dispatch_timers(support::one(), Instant::now() + Duration::from_secs(60));
    assert!(
        logged(&sub).is_empty(),
        "a timer outlived the session it was armed in"
    );
}

/// The endings are delivered before the session's timers are dropped, and an `on_exit` that
/// stops one of them is told the truth: `true`, because the call it cancelled really does not
/// happen. Drop the timers first and the same stop answers `false` — there is nothing left
/// for it to have cancelled — which is the reorder this pins.
#[test]
fn a_teardown_on_exit_can_still_stop_the_sessions_timers() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        &on_key(
            "    local timer = neotmux.timer.every(50, function() end)\n\
                 neotmux.job.start({ 'sleep' }, {\n\
                   on_exit = function()\n\
                     neotmux.log.info('stopped=' .. tostring(timer:stop()))\n\
                   end,\n\
                 })",
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    sub.forget_session(support::one());

    assert_eq!(
        logged(&sub),
        vec!["stopped=true"],
        "the timers were dropped before the endings were delivered"
    );
}

/// A streaming job's held fragment is handed over before the session's ending, the way the
/// ordinary ending path hands it over: a session going away is not the one case where the
/// plugin loses the last thing the job said.
#[test]
fn a_session_ending_flushes_what_a_streaming_job_still_held() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"whole\npart".to_vec())]]);
    let (_dir, sub) = built(&streaming(STREAM_EXIT), Rc::clone(&runner));

    keys(&sub, &["J"]);
    frame(&sub);
    assert_eq!(logged(&sub), vec!["out=[whole]"]);

    sub.forget_session(support::one());
    assert_eq!(
        logged(&sub),
        vec!["out=[part]", "exit code=nil err=session ended"],
        "the fragment was lost, or arrived after the ending"
    );
}

/// A job killed by the budget one frame and caught by a session ending the next reports the
/// budget, not the session: the ending it already has is the true one.
#[test]
fn a_session_ending_does_not_overwrite_an_ending_a_job_already_had() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let (_dir, sub) = built(
        "local job\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             job = neotmux.job.start({ 'sleep' }, {\n\
               on_exit = function(code, out, errout, err)\n\
                 neotmux.log.info('exit err=' .. tostring(err))\n\
               end,\n\
             })\n\
             return true\n\
           end\n\
           if key == 'K' then\n\
             job:kill()\n\
             return true\n\
           end\n\
           return false\n\
         end)\n",
        Rc::clone(&runner),
    );

    keys(&sub, &["J", "K"]);
    sub.forget_session(support::one());
    assert_eq!(
        logged(&sub),
        vec!["exit err=killed"],
        "the session ending swallowed the ending the job already had"
    );
}

/// The source sweep stays silent where the session sweep speaks: there is nobody left to tell,
/// because the chunk that wrote `on_exit` is the thing that went away.
#[test]
fn a_failed_load_ends_its_jobs_without_calling_back_into_the_chunk() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![JobChunk::Stdout(b"working\n".to_vec())]]);
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(
        dir.path().join("lua/plugin/worker.lua"),
        "neotmux.job.start({ 'sleep' }, {\n\
           on_exit = function() neotmux.log.info('exited') end,\n\
         })\n\
         error('nope')\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "local defaults = require('neotmux.defaults')\n\
         for _, name in ipairs({ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
           'notify', 'borders', 'sync', 'hints' }) do\n\
           defaults[name] = false\n\
         end\n\
         require('plugin.worker')\n",
    )
    .unwrap();

    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::host::Absent),
        Rc::clone(&runner) as Rc<dyn Runner>,
    )
    .unwrap();
    assert_eq!(sub.run_init().len(), 1, "the load did not fail");
    begin(&sub, support::one());
    sub.forget_session(support::one());
    let sub = Built::new(sub, terminals);

    assert!(*runner.killed.borrow(), "the job was left running");
    assert!(
        !logged(&sub).iter().any(|line| line == "exited"),
        "a dead chunk's on_exit was called anyway"
    );
}

/// A streaming job whose `on_stdout` keeps overrunning is killed, and the plugin hears the one
/// ending every job gets: no code, and the budget named as the reason.
#[test]
fn a_job_whose_callback_keeps_overrunning_is_killed_and_says_the_budget_did_it() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"one\n".to_vec())],
        vec![JobChunk::Stdout(b"two\n".to_vec())],
    ]);
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.tick_budget_ms = 1\n\
             neotmux.opt.tick_overrun_limit = 2\n{}",
            on_key(
                "    neotmux.job.start({ 'tail' }, {\n\
                       buffered = false,\n\
                       on_stdout = function(lines)\n\
                         local t = os.clock()\n\
                         while os.clock() - t < 0.05 do end\n\
                       end,\n\
                       on_exit = function(code, out, errout, err)\n\
                         neotmux.log.info('exit code=' .. tostring(code)\n\
                           .. ' err=' .. tostring(err))\n\
                       end,\n\
                     })",
            )
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    frame(&sub);
    assert!(
        !*runner.killed.borrow(),
        "one overrun was enough to kill it"
    );

    frame(&sub);
    assert!(*runner.killed.borrow(), "the runaway job was not killed");

    frame(&sub);
    let said = logged(&sub);
    assert_eq!(
        said.iter()
            .filter(|line| *line == "exit code=nil err=budget")
            .count(),
        1,
        "a job killed by the budget did not end exactly once, naming it: {said:?}"
    );
}

/// The limit is a run, not a total: a callback that comes back inside its budget wipes its own
/// job's slate, so the next overrun starts counting from one again.
#[test]
fn a_job_callback_inside_its_budget_forgives_the_overruns_before_it() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![JobChunk::Stdout(b"slow\n".to_vec())],
        vec![JobChunk::Stdout(b"fast\n".to_vec())],
        vec![JobChunk::Stdout(b"slow\n".to_vec())],
        vec![JobChunk::Stdout(b"fast\n".to_vec())],
    ]);
    let (_dir, sub) = built(
        &format!(
            "neotmux.opt.tick_budget_ms = 1\n\
             neotmux.opt.tick_overrun_limit = 2\n{}",
            on_key(
                "    neotmux.job.start({ 'tail' }, {\n\
                       buffered = false,\n\
                       on_stdout = function(lines)\n\
                         neotmux.log.info('saw ' .. lines[1])\n\
                         if lines[1] == 'slow' then\n\
                           local t = os.clock()\n\
                           while os.clock() - t < 0.05 do end\n\
                         end\n\
                       end,\n\
                     })",
            )
        ),
        Rc::clone(&runner),
    );

    keys(&sub, &["J"]);
    for _ in 0..4 {
        frame(&sub);
    }
    let said = logged(&sub);
    assert!(
        !*runner.killed.borrow(),
        "two overruns a frame apart, with a\
        callback inside its budget between them, killed the job: {said:?}"
    );
    assert_eq!(
        said.iter().filter(|line| line.starts_with("saw ")).count(),
        4,
        "the job stopped being delivered to: {said:?}"
    );
}
