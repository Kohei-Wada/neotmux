//! `neotmux.jev`: a pane no ruleset in `neotmux.detect.agents` answers is asked of the Jev
//! model, and the answer it comes back with is what `neotmux.state` tells the next layer.
//!
//! Every test here drives a fake `Runner`, so no `curl` runs and no key is needed: the job
//! is the whole of the wire, and what a real process adds is `neotmux-server`'s business.
//! The `built` is this file's own rather than `support`'s, because the tick reaches the
//! panes through `neotmux.window`, and a geometry with no window has none to reach.

use std::fs;
use std::rc::Rc;
use std::time::Duration;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::host::JobChunk;
use neotmux_lua::plugin::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, Size, Window, WindowId};
use tempfile::TempDir;

mod support;

use support::{Built, FakeRunner, logged, notified, running};

const SIZE: Size = Size { rows: 10, cols: 40 };

/// The one pane the harness publishes, in the one window it holds.
fn pane() -> PaneId {
    PaneId::from(1)
}

fn window_id() -> WindowId {
    WindowId::from(1)
}

fn built(extra: impl AsRef<str>, runner: Rc<FakeRunner>) -> (TempDir, Built) {
    let extra = extra.as_ref();
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("init.lua"),
        format!(
            "local defaults = require('neotmux.defaults')\n\
             for _, name in ipairs({{ 'status', 'prefix', 'copy', 'sessions', 'palette',\n\
               'notify', 'borders', 'guard', 'sync', 'watch', 'sidebar', 'footer',\n\
               'hints', 'chime' }}) do\n\
               defaults[name] = false\n\
             end\n\
             {extra}"
        ),
    )
    .unwrap();
    let terminals = support::panes();
    let sub = LuaSubsystem::build_with_runner(
        Layers::from_paths(vec![dir.path().to_path_buf(), support::shipped_layer()]),
        terminals.clone(),
        Rc::new(neotmux_lua::host::Absent),
        runner,
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    sub.begin_frame(
        support::one(),
        Epoch::new(SIZE, vec![pane()], Some(pane())),
        Geometry {
            area: Rect::new(0, 0, SIZE.cols, SIZE.rows),
            panes: vec![(pane(), Rect::new(0, 0, SIZE.cols, SIZE.rows))],
            outer: Vec::new(),
            focus: Some(pane()),
            windows: vec![Window {
                id: window_id(),
                panes: vec![pane()],
                name: None,
            }],
            window: Some(window_id()),
            zoomed: None,
        },
    );
    (dir, Built::new(sub, terminals))
}

/// One frame: deliver whatever the job has said, then tick.
fn frame(sub: &LuaSubsystem) {
    sub.dispatch_jobs(support::one());
    sub.dispatch_tick(support::one());
}

/// The API's answer for this pane, the way it comes off the wire: the decision, its
/// distribution, and the confidence the layer is told to weigh.
fn response(choice: &str, confidence: f64) -> String {
    format!(
        r#"{{"model":"jev-1.13.0","answers":{{"p1":{{"type":"choice","choice":"{choice}","probabilities":{{"blocked":{confidence},"working":0.05,"idle":0.05}},"confidence":{confidence}}}}},"usage":{{"input_tokens":300,"output_tokens":20}}}}
200"#
    )
}

/// A question the pattern list below has no words for: digits where the list expects a yes,
/// and no "do you want to" anywhere. So `blocked` can only be the model's.
const SCREEN: &[u8] = b"Select an option (1-3):\n";

/// The layer under test, on and fast, with a key that reads `state.of` on `J`. `extra` is
/// what a test puts in between: a narrower confidence, a shorter fallback clock.
fn init(extra: &str) -> String {
    format!(
        "local jev = require('neotmux.jev')\n\
         jev.enabled = true\n\
         jev.settle_ms = 1\n\
         {extra}\
         local state = require('neotmux.state')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('of=' .. tostring(state.of(1)))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    )
}

/// Body of a test that only reads: everything on `J`, because the log the test asserts on
/// must be the session's rather than init's.
fn reader(body: &str) -> String {
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

/// The harness's key, this file's own: the helper in `support` reads the same dispatch and
/// belongs to the one-pane, no-window harness this file does not use.
fn keys(sub: &LuaSubsystem) {
    assert_eq!(
        sub.dispatch_key(support::one(), "J"),
        neotmux_lua::KeyOutcome::Consumed,
        "the test's own on_key did not take 'J'"
    );
}

/// The whole point: a pane nobody's rules know gets the model's word, and the word lands in
/// `state.of` for the pane it was about.
#[test]
fn a_pane_no_ruleset_claims_gets_the_model_answer() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(response("blocked", 0.89).into_bytes()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(init(""), runner.clone());
    running(&sub, "bash");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), SCREEN);

    frame(&sub); // marks what it has just seen
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub); // the settle has passed, and the request goes out
    keys(&sub);
    assert!(
        !logged(&sub).iter().any(|l| l == "of=blocked"),
        "blocked before the answer came back"
    );

    frame(&sub); // the answer arrives
    keys(&sub);
    assert!(
        logged(&sub).iter().any(|l| l == "of=blocked"),
        "the model's answer did not land in state"
    );
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "a settled, decided pane was asked again"
    );
}

/// Below the threshold the answer is not believed: the pane falls through to whatever is
/// under the layer, and the layer is the one holding it.
#[test]
fn an_answer_below_the_confidence_is_not_believed() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(response("blocked", 0.3).into_bytes()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(init(""), runner.clone());
    running(&sub, "bash");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), SCREEN);

    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub);
    frame(&sub); // the answer arrives, and is let go
    keys(&sub);
    let log = logged(&sub);
    assert!(
        !log.iter().any(|l| l == "of=blocked"),
        "a decision the layer is not to make was made"
    );
    assert!(
        log.iter().any(|l| l == "of=working"),
        "the pane fell through to nothing: the fallback under the layer stopped answering"
    );
}

/// A refused key is a wall, not a beat: the layer says so once and stops asking, and a
/// changed screen does not send it out again.
#[test]
fn a_refused_key_turns_the_tier_off_for_the_session() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(b"{}\n401".to_vec()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(init(""), runner.clone());
    running(&sub, "bash");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), SCREEN);

    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub);
    frame(&sub); // the refusal arrives
    assert!(
        notified(&sub).iter().any(|t| t.contains("401")),
        "the refused key was not said out loud"
    );

    sub.terminals().feed(pane(), b"Something else to answer:\n");
    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub);
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "a disabled tier still sends"
    );
}

/// An answer about a screen the pane has since left is about a past: it is dropped, and the
/// pane is left to whatever answers it now.
#[test]
fn an_answer_about_a_screen_the_pane_has_left_is_dropped() {
    let runner = FakeRunner::new();
    runner.will_say(vec![
        vec![],
        vec![
            JobChunk::Stdout(response("blocked", 0.9).into_bytes()),
            JobChunk::Exited(0),
        ],
    ]);
    let (_dir, sub) = built(
        init("local state = require('neotmux.state')\nstate.working_for = 1\n"),
        runner.clone(),
    );
    running(&sub, "bash");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), SCREEN);

    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub); // the request is out
    sub.terminals().feed(pane(), b"\r\n1\n"); // and the pane answers it before the answer comes back
    frame(&sub); // the first poll: nothing yet
    frame(&sub); // the answer arrives, for a screen the pane has left
    keys(&sub);
    assert!(
        !logged(&sub).iter().any(|l| l == "of=blocked"),
        "a decision about a past screen was kept"
    );
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "a second request went out for a pane this test has not settled again"
    );
}

/// Two panes wanting, one request out: the second waits for the first to be answered rather
/// than going over it.
#[test]
fn a_request_out_keeps_the_next_one_waiting() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![]]); // nothing yet, still running
    let (_dir, sub) = built(init(""), runner.clone());
    running(&sub, "bash");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), SCREEN);

    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub); // the request goes out
    assert_eq!(runner.started.borrow().len(), 1);

    sub.terminals().feed(pane(), b"More waiting:\n");
    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub);
    assert_eq!(
        runner.started.borrow().len(),
        1,
        "a second request went out over the first"
    );
}

/// What goes over the wire: the pane's own text under its field, the model the user is
/// paying for, and a shell line that carries the body and the endpoint as arguments rather
/// than spliced into itself, with the key the shell's to expand.
#[test]
fn the_request_carries_the_pane_and_its_question() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(response("blocked", 0.9).into_bytes()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(init(""), runner.clone());
    running(&sub, "bash");
    sub.terminals().open(pane(), SIZE, 100);
    sub.terminals().feed(pane(), SCREEN);

    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub);

    let started = runner.started.borrow();
    assert_eq!(started.len(), 1);
    let cmd = &started[0].cmd;
    assert_eq!(cmd[0], "sh");
    assert_eq!(cmd[1], "-c");
    assert!(
        cmd[2].contains("TYPESAFE_API_KEY"),
        "the key is not the shell's to expand"
    );
    assert!(cmd[2].contains("curl"), "the wire is not a curl");
    let body = &cmd[4];
    assert!(
        body.contains("Select an option"),
        "the pane's own text is not on the wire"
    );
    assert!(
        body.contains(r#""model":"jev-latest""#),
        "the model is not the one the user is paying for"
    );
    assert!(body.contains(r#""p1""#), "the pane's field is not named");
    assert!(
        cmd[2].contains("\"$1\"") && cmd[2].contains("\"$2\""),
        "the body and the endpoint are spliced into the shell line"
    );
    assert_eq!(
        cmd[5], "https://api.typesafe.ai/v1/systemone",
        "the endpoint is not the request's"
    );
}

/// The decoder and encoder the layer stands on, against the API's own shape: nested, typed,
/// and a character that is not on the ASCII side of the table.
#[test]
fn the_json_module_reads_and_writes_the_api_shape() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        reader(
            r#"local json = require('neotmux.json')
local value, err = json.decode('{"a":[1,2.5,"x\\u00e9"],"b":true,"c":null,"d":{"e":-3}}')
neotmux.log.info('err=' .. tostring(err))
neotmux.log.info('a2=' .. tostring(value.a[2]))
neotmux.log.info('a3=' .. value.a[3])
neotmux.log.info('b=' .. tostring(value.b))
neotmux.log.info('c=' .. tostring(value.c))
neotmux.log.info('e=' .. tostring(value.d.e))
local bad, where = json.decode('{"a":')
neotmux.log.info('bad=' .. tostring(bad) .. ' at ' .. tostring(where))
local out, oerr = json.encode({ s = 'a\nb"c', list = {1, 2}, t = true })
neotmux.log.info('enc-err=' .. tostring(oerr))
neotmux.log.info('enc=' .. out)"#,
        ),
        runner.clone(),
    );
    keys(&sub);
    let log = logged(&sub);
    assert!(
        log.iter().any(|l| l == "err=nil"),
        "the shape did not decode"
    );
    assert!(log.iter().any(|l| l == "a2=2.5"));
    assert!(
        log.iter().any(|l| l == "a3=x\u{e9}"),
        "the \\u escape did not come back"
    );
    assert!(log.iter().any(|l| l == "b=true"));
    assert!(
        log.iter().any(|l| l == "c=nil"),
        "null is not the word for nothing"
    );
    assert!(log.iter().any(|l| l == "e=-3"));
    assert!(
        log.iter().any(|l| l == "bad=nil at 6"),
        "a value that stops mid-way did not say where"
    );
    assert!(log.iter().any(|l| l == "enc-err=nil"));
    assert!(
        log.iter()
            .any(|l| l == r#"enc={"list":[1,2],"s":"a\nb\"c","t":true}"#),
        "the encoder did not write what the decoder will read"
    );
}

/// The round trip, the direction the layer actually lives in: a request the encoder wrote is
/// a value the decoder reads back the same.
#[test]
fn the_json_module_round_trips_the_request_shape() {
    let runner = FakeRunner::new();
    let (_dir, sub) = built(
        reader(
            r#"local json = require('neotmux.json')
local request = { state = { p1 = 'pane 1 (running: bash)\nSelect an option (1-3):' }, model = 'jev-latest' }
request.questions = { p1 = { type = 'choice', instructions = 'Judging only the state field `p1`', criteria = { working = 'w', blocked = 'b', idle = 'i' } } }
local out = assert(json.encode(request))
local back, err = json.decode(out)
neotmux.log.info('err=' .. tostring(err))
neotmux.log.info('same=' .. tostring(back.state.p1 == request.state.p1 and back.questions.p1.type == 'choice'))
neotmux.log.info('crit=' .. tostring(back.questions.p1.criteria.blocked == 'b'))"#,
        ),
        runner.clone(),
    );
    keys(&sub);
    let log = logged(&sub);
    assert!(
        log.iter().any(|l| l == "err=nil"),
        "the round trip lost the shape"
    );
    assert!(log.iter().any(|l| l == "same=true"));
    assert!(log.iter().any(|l| l == "crit=true"));
}

/// The seam in `neotmux.state`: the rules win where they answer, and the model's word stands
/// where they do not — in one `state.of`, against the same screen.
#[test]
fn state_asks_the_model_only_where_the_rules_do_not_answer() {
    let runner = FakeRunner::new();
    runner.will_say(vec![vec![
        JobChunk::Stdout(response("blocked", 0.9).into_bytes()),
        JobChunk::Exited(0),
    ]]);
    let (_dir, sub) = built(init(""), runner.clone());
    running(&sub, "claude");
    sub.terminals().open(pane(), SIZE, 100);
    // A live turn: `live_turn_working` answers it, so this pane is the ruleset's.
    sub.terminals().feed(
        pane(),
        "\u{273b} Herding\u{2026} (12s \u{00b7} esc to interrupt)\n".as_bytes(),
    );

    frame(&sub);
    std::thread::sleep(Duration::from_millis(20));
    frame(&sub);
    keys(&sub);
    assert!(
        logged(&sub).iter().any(|l| l == "of=working"),
        "the ruleset stopped answering for its own pane"
    );
    assert!(
        !logged(&sub).iter().any(|l| l == "of=blocked"),
        "the model's word went over the ruleset's for the pane it claims"
    );
}
