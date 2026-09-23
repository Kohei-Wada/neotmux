# The Sessions API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A plugin can see every session, read any of them, and move between them — and the
shipped sidebar uses that to show what each session is doing.

**Architecture:** `neotmux.session.current/list` read the ambient store the Lua layer already
keeps; `with(name, fn)` rebinds the ambient session for the length of a call, so every existing
read works inside it with nothing written per API. `create`, `switch` and `kill` are ops, but
not the kind a session applies: they are about the set of sessions, so they come back from
`Plugins::respond` beside the tock and the frame applies them after the pictures have gone out.

**Tech Stack:** Rust 2024, one workspace of five crates, `mlua` (Lua 5.4), Lua for the sidebar.

**Spec:** `docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md` — stage 3 of §11
as reordered, which is §8 and the sidebar half of §1's fourth row.

## What this makes true

§1's table is the measure. Its fourth row — "all the sessions in one view, and a way into any
of them" — is half true today: the sessions are in one server and `ls` lists them, but nothing
a person looks at says so. After this, the sidebar has a row per session saying what that
session is running, and `C-b s` moves between them. That is the row, and it is written in Lua,
which is the whole argument of the two specs before this one.

## A correction this plan makes to the spec

§8 says `switch` "moves the client this key came from", and that from a tick it moves every
client attached to the current session. The first half needs an op to remember which client
raised it, and an op remembers the session it was raised for and nothing else. Attributing an
op to a client is machinery §9's command surface will need anyway — a command arrives *from*
somewhere — so it comes with that rather than here.

**Ruling:** `switch` moves every client attached to the current session, from a key handler as
much as from a tick. With one client, which is the ordinary case and every case the tests
cover, it is the same behaviour. Task 5 amends §8.

## Global Constraints

- `neotmux-lua` keeps `neotmux-proto` as its only workspace dependency.
- Nothing in `neotmux-lua` learns what a client is. The layer knows sessions; which terminals
  are looking at one is the server's.
- A read of another session through `with` is a read: `ctx` still draws into the session whose
  tick is running, and nothing in this plan changes that.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` green at every commit, read rather than assumed.
- English for code, comments and commits; comments only where the WHY is non-obvious.
- Commit messages in this repo's voice, ending with the two attribution lines recent commits
  carry.

---

## File Structure

| File | Responsibility afterwards |
| --- | --- |
| `crates/neotmux-lua/src/ambient.rs` | Answers which sessions there are and finds one by name |
| `crates/neotmux-lua/src/api/session.rs` | **New.** `neotmux.session.*` |
| `crates/neotmux-lua/src/ops.rs` | The three ops that are about the set of sessions |
| `crates/neotmux/src/plugins.rs` | Hands those back beside the tock |
| `crates/neotmux/src/server.rs` | Applies them |
| `runtime/lua/plugin/sidebar.lua` | A row per session, and `C-b s` |

---

### Task 1: `session.current` and `session.list`

**Files:**
- Modify: `crates/neotmux-lua/src/ambient.rs`
- Create: `crates/neotmux-lua/src/api/session.rs`
- Modify: `crates/neotmux-lua/src/api/mod.rs`
- Test: `crates/neotmux-lua/tests/sessions.rs` (new file)

**Interfaces:**
- Produces: `Ambient::names() -> Vec<String>` (sorted), `Ambient::id_of(&str) -> Option<SessionId>`;
  Lua's `neotmux.session.current()` and `neotmux.session.list()`.

- [ ] **Step 1: Write the failing test**

Create `crates/neotmux-lua/tests/sessions.rs`:

```rust
//! `neotmux.session.*`: the sessions a plugin can see, and the one it is running for.

use std::fs;

use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::{Epoch, Geometry, PaneId, Rect, SessionId, Size};

mod support;

fn built(init: &str) -> (tempfile::TempDir, LuaSubsystem) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), init).unwrap();
    let sub = LuaSubsystem::build(
        Layers::from_paths(vec![dir.path().to_path_buf()]),
        support::panes(),
    )
    .unwrap();
    assert!(sub.run_init().is_empty());
    (dir, sub)
}

/// A session the host has made and told the layer about, with a pane in it.
fn make(sub: &LuaSubsystem, id: SessionId, name: &str, panes: u32) {
    sub.name_session(id, name);
    let size = Size { rows: 24, cols: 80 };
    let panes: Vec<(PaneId, Rect)> = (1..=panes)
        .map(|n| (PaneId::from(n), Rect::new(0, 0, 80, 24)))
        .collect();
    let focus = panes.first().map(|(id, _)| *id);
    sub.begin_frame(
        id,
        Epoch::new(size, panes.iter().map(|(id, _)| *id).collect(), focus),
        Geometry {
            size,
            panes,
            focus,
            ..Default::default()
        },
    );
}

fn logged(sub: &LuaSubsystem, session: Option<SessionId>) -> Vec<String> {
    sub.drain_ops(session)
        .into_iter()
        .filter_map(|(_, op)| match op {
            neotmux_lua::ops::Op::Log { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_plugin_sees_every_session_and_which_one_it_is_running_for() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 neotmux.log.info(neotmux.session.current() .. ' of ' .. table.concat(neotmux.session.list(), ','))\n\
         end)",
    );
    let work = support::one();
    let home = SessionId::from(2);
    make(&sub, work, "work", 1);
    make(&sub, home, "home", 1);

    sub.dispatch_tick(work);
    sub.dispatch_tick(home);

    assert_eq!(logged(&sub, Some(work)), vec!["work of home,work".to_string()]);
    assert_eq!(logged(&sub, Some(home)), vec!["home of home,work".to_string()]);
}

/// A load has no session, so there is nothing to be current and nothing has been named yet.
#[test]
fn a_load_sees_no_sessions_at_all() {
    let (_dir, sub) = built(
        "neotmux.log.info(tostring(neotmux.session.current()) .. ' of ' .. #neotmux.session.list())",
    );
    assert_eq!(logged(&sub, None), vec!["nil of 0".to_string()]);
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux-lua --test sessions`
Expected: FAIL — `neotmux.session` is nil.

- [ ] **Step 3: The ambient answers**

```rust
    /// Every session there is, in name order, which is the order everything that lists them
    /// prints them in.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.names.borrow().values().cloned().collect();
        names.sort();
        names
    }

    pub fn id_of(&self, name: &str) -> Option<SessionId> {
        self.names
            .borrow()
            .iter()
            .find(|(_, held)| held.as_str() == name)
            .map(|(id, _)| *id)
    }
```

- [ ] **Step 4: The module**

`crates/neotmux-lua/src/api/session.rs`:

```rust
use std::rc::Rc;

use mlua::{Lua, Table};

use crate::ambient::Ambient;

/// `neotmux.session`: which sessions there are, which one the work in hand is for, and — from
/// Task 2 — a way to read one that is not.
pub(crate) fn install(lua: &Lua, ambient: Rc<Ambient>) -> mlua::Result<Table> {
    let session = lua.create_table()?;

    {
        let ambient = ambient.clone();
        let current = lua.create_function(move |_lua, ()| {
            Ok::<Option<String>, mlua::Error>(ambient.current_name())
        })?;
        session.set("current", current)?;
    }

    {
        let ambient = ambient.clone();
        let list = lua.create_function(move |lua, ()| lua.create_sequence_from(ambient.names()))?;
        session.set("list", list)?;
    }

    Ok(session)
}
```

Installed in `api/mod.rs` the way `window` is, sealed the same way:

```rust
    {
        let session = session::install(lua, ambient.clone())?;
        let session = seal(lua, session, "neotmux.session")?;
        neotmux.set("session", session)?;
    }
```

`mod session;` goes in `api/mod.rs`'s module list. `neotmux.session_name()` stays as it is:
it is the same answer `session.current()` gives, and plugins already call it.

- [ ] **Step 5: Run it**

Run: `cargo test -p neotmux-lua --test sessions`
Expected: PASS, both.

The sealed-table test in `tests/subsystem.rs` lists every key on `neotmux` in one string —
`"clipboard detach float keymap log notify on_event on_key on_tick opt pane quit reserve
resubscribe session_name width window"`. `session` goes into it, in sort order, between
`resubscribe` and `session_name`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "the sessions a plugin can see"
```

---

### Task 2: `session.with`

**Files:**
- Modify: `crates/neotmux-lua/src/api/session.rs`
- Test: `crates/neotmux-lua/tests/sessions.rs`

**Interfaces:**
- Produces: `neotmux.session.with(name, fn)` — binds that session for the call and returns
  whatever the function returned; `nil` and no call for a name nothing answers to.

- [ ] **Step 1: Write the failing test**

```rust
/// The whole point: a plugin reads a session it is not running for, with every ordinary read,
/// and is put back where it was afterwards.
#[test]
fn with_reads_another_session_and_puts_the_caller_back() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 local theirs = neotmux.session.with('home', function() return #neotmux.pane.list() end)\n\
         \x20 neotmux.log.info('mine=' .. #neotmux.pane.list() .. ' theirs=' .. tostring(theirs)\n\
         \x20   .. ' back=' .. neotmux.session.current())\n\
         end)",
    );
    let work = support::one();
    let home = SessionId::from(2);
    make(&sub, work, "work", 1);
    make(&sub, home, "home", 3);

    sub.dispatch_tick(work);

    assert_eq!(
        logged(&sub, Some(work)),
        vec!["mine=1 theirs=3 back=work".to_string()]
    );
}

/// A name nothing answers to is not an error — a session can end between one tick and the
/// next, and a watcher's loop is not the caller's mistake. The function does not run.
#[test]
fn with_a_name_nothing_answers_to_does_nothing() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 local ran = false\n\
         \x20 local got = neotmux.session.with('gone', function() ran = true return 1 end)\n\
         \x20 neotmux.log.info('got=' .. tostring(got) .. ' ran=' .. tostring(ran))\n\
         end)",
    );
    make(&sub, support::one(), "work", 1);
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["got=nil ran=false".to_string()]
    );
}

/// An error inside the call is the caller's error, and the binding is undone either way: the
/// guard is what restores, so a raise cannot leave the layer reading somebody else's session.
#[test]
fn with_puts_the_caller_back_even_when_the_call_raises() {
    let (_dir, sub) = built(
        "neotmux.on_tick(function()\n\
         \x20 pcall(function() neotmux.session.with('home', function() error('nope') end) end)\n\
         \x20 neotmux.log.info('back=' .. neotmux.session.current())\n\
         end)",
    );
    make(&sub, support::one(), "work", 1);
    make(&sub, SessionId::from(2), "home", 1);
    sub.dispatch_tick(support::one());
    assert_eq!(
        logged(&sub, Some(support::one())),
        vec!["back=work".to_string()]
    );
}
```

- [ ] **Step 2: Run them**

Run: `cargo test -p neotmux-lua --test sessions with_`
Expected: FAIL — `with` is nil.

- [ ] **Step 3: Implement**

```rust
    {
        // The whole of reading another session: everything an ordinary read does, done
        // against a different one. `Bound`'s drop is what puts the caller back, so a raise
        // inside the call restores as surely as a return does.
        let ambient = ambient.clone();
        let with = lua.create_function(
            move |_lua, (name, body): (String, mlua::Function)| -> mlua::Result<mlua::MultiValue> {
                let Some(id) = ambient.id_of(&name) else {
                    return Ok(mlua::MultiValue::new());
                };
                let _bound = ambient.bind(id);
                body.call::<mlua::MultiValue>(())
            },
        )?;
        session.set("with", with)?;
    }
```

- [ ] **Step 4: Run them**

Run: `cargo test -p neotmux-lua --test sessions`
Expected: PASS, all five.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "reading a session that is not the one in hand"
```

---

### Task 3: `create`, `switch` and `kill`

These are not ops a session applies: they are about the set of sessions, and the set is the
server's. So they come back from `Plugins::respond` beside the tock rather than inside it.

**Files:**
- Modify: `crates/neotmux-lua/src/ops.rs`, `crates/neotmux-lua/src/api/session.rs`
- Modify: `crates/neotmux/src/plugins.rs`, `crates/neotmux/src/server.rs`,
  `crates/neotmux/src/session.rs` (a surface size to make a new session at)
- Test: `crates/neotmux/tests/socket.rs`

**Interfaces:**
- Produces: `Op::SessionCreate { name }`, `Op::SessionSwitch { name }`, `Op::SessionKill { name }`;
  `neotmux::plugins::ServerOp` with the same three; `Plugins::respond` returning
  `(Tock, Vec<(String, ServerOp)>)`; `Session::surface_size() -> Size`.

- [ ] **Step 1: Write the failing test**

In `crates/neotmux/tests/socket.rs`:

```rust
/// A plugin makes a session, moves its client into it, and ends the one it came from. All
/// three are the set of sessions changing, which is the server's to do and a plugin's to ask
/// for — and the client that asked ends up looking at the new one.
#[test]
fn a_plugin_can_make_a_session_move_into_it_and_end_the_old_one() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[(
        "init.lua",
        "local done = false\n\
         neotmux.on_key('C-y', function()\n\
           if done then return true end\n\
           done = true\n\
           neotmux.session.create('made')\n\
           neotmux.session.switch('made')\n\
           return true\n\
         end)\n\
         neotmux.on_tick(function()\n\
           neotmux.log.info('tick in ' .. neotmux.session.current())\n\
         end)\n",
    )]);

    let mut client = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    wait_for_log(runtime.path(), "tick in default");

    client.send("\x19"); // C-y
    wait_for_log(runtime.path(), "tick in made");
}
```

`Client::start_with_prefix_and_layer`, `wait_for_socket`, `wait_for_log` and `layer_with` are
all already in that file.

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux --test socket a_plugin_can_make_a_session`
Expected: FAIL — `neotmux.session.create` is nil, so the key handler raises and the log never
says `tick in made`.

- [ ] **Step 3: The ops**

`Op` gains three, each carrying a name:

```rust
    /// The set of sessions changing, which is the server's rather than a session's: these
    /// three do not reach `Session::apply`.
    SessionCreate { name: String },
    SessionSwitch { name: String },
    SessionKill { name: String },
```

and `api/session.rs` pushes them, attributed the way every other op is — read how
`window.rs`'s `create` does it and follow it exactly, `source::caller` included. A name the
host would refuse is refused here, with `neotmux::sock`'s message... except that
`neotmux-lua` cannot see `neotmux::sock`. Validate in the server, where the door already
does, and log the refusal against the source that asked.

- [ ] **Step 4: They come back beside the tock**

In `plugins.rs`:

```rust
/// What a plugin asked of the *server* rather than of a session: the set of sessions is not
/// something a session can change.
pub enum ServerOp {
    Create { name: String },
    Switch { name: String },
    Kill { name: String },
}
```

`drain_ops` splits them out into a second vector, and `respond` returns
`(Tock, Vec<(String, ServerOp)>)`. `drain_load_ops` refuses all three the way it refuses a
split: there is no session to make one beside, and nothing to switch away from.

- [ ] **Step 5: The frame applies them**

In `server.rs`, the session loop collects `(SessionId, String, ServerOp)` and applies them
after the loop — after every picture has gone out, so a client that switches is not sent two
frames in one pass:

```rust
/// What a plugin asked of the set of sessions, applied once every session has had its frame.
fn apply_server_ops(
    asked: Vec<(SessionId, String, ServerOp)>,
    plugins: &Plugins,
    sessions: &mut Sessions,
    clients: &mut Vec<Attached>,
    terminals: &Rc<PaneTerminals>,
    waker: &Waker,
) {
    for (from, tag, op) in asked {
        match op {
            ServerOp::Create { name } => {
                if let Err(why) = neotmux::sock::validate_session_name(&name) {
                    logging::log(Level::Error, &tag, &format!("session.create refused: {why}"));
                    continue;
                }
                if sessions.id_of(&name).is_some() {
                    logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.create('{name}') refused: it already exists"),
                    );
                    continue;
                }
                // At the size of the session that asked: there is no client looking at the
                // new one yet, and a surface has to be some size before a shell can be
                // opened on it.
                let size = match sessions.get_mut(from) {
                    Some(session) => session.surface_size(),
                    None => continue,
                };
                let lua = plugins.lua();
                let setup = Setup {
                    size,
                    first_window: lua.reserve_window_id(),
                    reserved: lua.reserved(),
                    gutter: lua.opt().pane_gutter,
                    wants_pane_output: lua.has_subscriber("pane_output"),
                };
                match Session::new(&setup, waker.clone(), terminals.clone()) {
                    Ok(session) => {
                        let id = sessions.mint();
                        sessions.live.insert(
                            id,
                            Held {
                                name: name.clone(),
                                session,
                            },
                        );
                        plugins.lua().name_session(id, &name);
                    }
                    Err(err) => logging::log(
                        Level::Error,
                        &tag,
                        &format!("session.create('{name}') failed: {err:#}"),
                    ),
                }
            }
            ServerOp::Switch { name } => match sessions.id_of(&name) {
                Some(to) => {
                    for client in clients.iter_mut().filter(|client| client.session == from) {
                        client.session = to;
                    }
                    // Refitted to whoever just arrived, and dirty by construction, so the
                    // next frame is one they are shown.
                    let lua = plugins.lua();
                    let setup = Setup {
                        size: clients
                            .iter()
                            .find(|client| client.session == to)
                            .map(|client| client.size)
                            .unwrap_or_else(|| Size { rows: 24, cols: 80 }),
                        first_window: lua.reserve_window_id(),
                        reserved: lua.reserved(),
                        gutter: lua.opt().pane_gutter,
                        wants_pane_output: lua.has_subscriber("pane_output"),
                    };
                    if let Some(session) = sessions.get_mut(to)
                        && let Err(err) = session.adopt(&setup)
                    {
                        logging::log(
                            Level::Error,
                            &tag,
                            &format!("session.switch('{name}') failed: {err:#}"),
                        );
                    }
                }
                None => logging::log(
                    Level::Error,
                    &tag,
                    &format!("session.switch('{name}') refused: no session by that name"),
                ),
            },
            ServerOp::Kill { name } => match sessions.id_of(&name) {
                Some(id) => end_session(id, plugins, sessions, clients),
                None => logging::log(
                    Level::Error,
                    &tag,
                    &format!("session.kill('{name}') refused: no session by that name"),
                ),
            },
        }
    }
}
```

`Attached` gains `size: Size`, set from the attach and updated by `deliver`'s `Resize` arm —
a switch refits the session it arrives at to the terminal that arrived, which is the same
`window-size latest` rule an attach follows.

`Session::surface_size(&self) -> Size` is one line returning `self.surface.size`.

A `Kill` that empties the map ends the server, which `frame` already decides from
`sessions.live.is_empty()` — make sure that check happens after this call rather than before.

- [ ] **Step 6: Run everything**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS, the new socket test included.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "a plugin can make a session, move into it, and end one"
```

---

### Task 4: The sidebar shows the sessions

**Files:**
- Modify: `runtime/lua/plugin/sidebar.lua`
- Test: `crates/neotmux/tests/socket.rs`

- [ ] **Step 1: Write the failing test**

```rust
/// The payoff, end to end: the shipped sidebar draws a row per session saying what each one is
/// running, including the one nobody is looking at.
#[test]
fn the_sidebar_names_every_session_and_what_it_is_running() {
    let runtime = TestRuntime::new();
    let layer = layer_with(&[(
        "init.lua",
        "require('neotmux.defaults').sidebar = true\n\
         local said = false\n\
         neotmux.on_tick(function()\n\
           if said then return end\n\
           local rows = {}\n\
           for _, name in ipairs(neotmux.session.list()) do\n\
             rows[#rows + 1] = name\n\
           end\n\
           if #rows > 1 then\n\
             said = true\n\
             neotmux.log.info('sessions ' .. table.concat(rows, ','))\n\
           end\n\
         end)\n\
         neotmux.on_key('C-y', function() neotmux.session.create('second') return true end)\n",
    )]);

    let mut client = Client::start_with_prefix_and_layer(runtime.path(), "/bin/cat", layer.path());
    wait_for_socket(&socket_in(runtime.path()));
    client.send("\x19");
    wait_for_log(runtime.path(), "sessions default,second");
}
```

`require('neotmux.defaults').sidebar = true` is how a user turns the sidebar on: `runtime/init.lua`
loops over the shipped names and requires each one whose field in that table is true, and the
shipped layer runs last so a field set from an earlier layer is seen in time. The layer this
test writes is stacked before `shipped_layer()` by `start_with_prefix_and_layer`, so setting
the field there is exactly the supported way round. The `require('plugin.sidebar')` line is
then redundant — `init.lua` will do it — so drop it and let the default path do the work,
which is also what makes this a test of the shipped arrangement rather than of a direct
require.

- [ ] **Step 2: Run it**

Run: `cargo test -p neotmux --test socket the_sidebar_names_every_session`
Expected: FAIL until the plugin is written; the log line is the plugin's.

- [ ] **Step 3: The rows**

In `sidebar.lua`, above the windows, a section per session. Follow the file's own shape: a
`row(marker, indent, number, name, colour)` helper, colours in `M.colors`, markers in
`M.markers`, and `segments.draw` for the run. Each row says the session's name and what its
focused pane is running, read through `with`:

```lua
-- What a session is doing, for a row that is about a session rather than about a pane: how
-- many panes it has, and what the one somebody is looking at is running. Read through `with`,
-- because a session that is not the one this tick is for is still a session this plugin can
-- see.
local function doing(name)
  local panes, command = 0, nil
  neotmux.session.with(name, function()
    panes = #neotmux.pane.list()
    local focus = neotmux.pane.current()
    local info = focus and neotmux.pane.info(focus)
    command = info and info.command
  end)
  return panes, command
end
```

and `C-b s` moves to the next one, bound the way `C-b b` already is:

```lua
-- The next session round, in the order `session.list` gives, which is name order. A picker is
-- what this wants eventually; a cycle is what it can have while a float cannot take the
-- cursor.
function M.next_session()
  local names = neotmux.session.list()
  if #names < 2 then
    return
  end
  local current = neotmux.session.current()
  for at, name in ipairs(names) do
    if name == current then
      neotmux.session.switch(names[at % #names + 1])
      return
    end
  end
end
```

- [ ] **Step 4: Run it**

Run: `cargo test -p neotmux --test socket`
Expected: PASS. The sidebar's existing screen tests in `fixtures.rs` must still pass: a
section above the windows moves the window rows down, so read what those tests assert about
row positions before assuming they do not care.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "the sidebar says what every session is doing"
```

---

### Task 5: The docs and the spec

**Files:**
- Modify: `docs/plugins.md`
- Modify: `docs/superpowers/specs/2026-09-23-sessions-in-the-server-design.md`

- [ ] **Step 1: `docs/plugins.md`**

A section for `neotmux.session`, beside `neotmux.pane` and `neotmux.window`: `current`, `list`,
`with`, `create`, `switch`, `kill`. Say the two things a plugin author cannot guess — `with`
rebinds reads and not drawing, and `switch` moves every client attached to the current session.
The keys section, if there is one, gains `C-b s`.

- [ ] **Step 2: The spec**

§8's `switch` paragraph is corrected by this plan's ruling. §11 stage 3 is marked done. §1's
fourth row becomes true, and saying so is the point of having written the table.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "the spec's fourth row"
```

---

## What this plan does not do

- The command surface: `neotmux.command`, the CLI reaching it, and an op that remembers which
  client raised it. Its own spec, and the thing that makes an agent able to drive its pane.
- A session picker with a search field. It wants a float that can take the cursor, which is
  the palette work.
- Per-session options, several servers, `kill-server`.
