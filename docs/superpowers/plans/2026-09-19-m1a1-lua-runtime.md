# neotmux M1a part 1 — Lua runtime, attribution and registration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** neotmux loads Lua plugins from a runtime path, knows which plugin every callback belongs to, and lets a plugin register and unregister the four kinds of thing the ABI offers — with nothing yet able to draw or consume keys.

**Architecture:** A new `neotmux-lua` crate owns the `mlua` VM and everything Lua-facing; the binary keeps the loop. The VM is constructed sealed (no `io`, no `os.execute`, no C modules, no `debug`, text-only chunks) and loads plugins through a custom searcher that pushes the plugin's identity onto a stack while its chunk runs, so every registration captures who made it. Registrations return disposable handles and are also tracked per plugin, which is what makes unsubscribe and reload possible later. Plugin intent reaches the host through a single op buffer the host drains at one point per frame, so no host binding ever re-enters Lua.

**Tech Stack:** Rust 1.95 edition 2024, `mlua` 0.12 with the `lua54` and `vendored` features, the existing `neotmux-core` / `neotmux-term` / `neotmux` workspace.

**Spec:** `docs/superpowers/specs/2026-09-19-neotmux-design.md` — §4.1, §4.2, §4.3, §4.6, §4.7, §4.9, §4.10. Drawing (§4.5), key consumption (§4.4), resize ordering (§4.8) and the fixture plugin set (§4.11) are part 2. The budget (§5) is M1b.

## Global Constraints

- Edition 2024, rustc 1.95 or newer. Dependency versions are declared once in `[workspace.dependencies]`; members reference them with `workspace = true`
- `mlua = { version = "0.12", features = ["lua54", "vendored"] }`
- The VM is built with `Lua::new_with(libs, options)` — the **safe** constructor. `unsafe_new_with` is forbidden: the safe one is what disables `package.loadlib` and replaces the C searcher, and it rejects `StdLib::DEBUG` outright
- Plugins are fully trusted (spec §7). No sandbox, no capability allow-list. What the sealing in Task 1 removes is the ability to *block*, not the ability to act
- `neotmux-core` and `neotmux-term` stay free of Lua. `neotmux-lua` never touches ratatui or a PTY
- `neotmux-core::proto` carries no behaviour beyond what the container itself is: constructors, accessors, bounds checks, index arithmetic, resizing and `From`/`Display` conversions are in bounds; diffing, applying, I/O and state transitions are not
- Indentation is spaces, never tabs. No code comments unless the *why* is non-obvious, and then one line
- **No `#[allow(...)]` to silence a lint the code can satisfy.** This has been enforced five times on this repo
- Invariant from M0a, still binding: no code path may leave the terminal in raw mode or in the alternate screen
- Every task ends with `make check` (fmt, `clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`) and `cargo test --workspace --release` both green, then a commit

---

## File Structure

| path | responsibility |
|---|---|
| `crates/neotmux-lua/Cargo.toml` | the new crate's manifest |
| `crates/neotmux-lua/src/lib.rs` | crate root, re-exports, `LuaSubsystem` |
| `crates/neotmux-lua/src/vm.rs` | building the sealed VM: stdlib selection, field removal, `load` wrapper |
| `crates/neotmux-lua/src/runtime.rs` | runtime-layer resolution and the `package.path` / searcher wiring |
| `crates/neotmux-lua/src/plugin.rs` | `PluginId`, the plugin registry, the identity stack, chunk loading |
| `crates/neotmux-lua/src/registry.rs` | the four registration kinds, their handles, per-plugin tracking |
| `crates/neotmux-lua/src/ops.rs` | the op buffer: what Lua asks for, drained by the host |
| `crates/neotmux-lua/src/api/mod.rs` | installing the `neotmux` table |
| `crates/neotmux-lua/src/api/log.rs` | `neotmux.log.{info,warn,error}` |
| `crates/neotmux-lua/src/api/opt.rs` | `neotmux.opt.*` host settings |
| `crates/neotmux/src/logging.rs` | log destination, `--log`, pre-raw-mode failure reporting |

Each file has one responsibility and is testable on its own: `vm.rs` needs no runtime path, `runtime.rs` needs no VM behaviour beyond `require`, `registry.rs` needs no filesystem.

---

### Task 1: The sealed VM

**Files:**
- Create: `crates/neotmux-lua/Cargo.toml`, `crates/neotmux-lua/src/lib.rs`, `crates/neotmux-lua/src/vm.rs`
- Modify: `Cargo.toml` (workspace members and `[workspace.dependencies]`)

**Interfaces:**
- Consumes: nothing
- Produces: `vm::new_vm() -> mlua::Result<mlua::Lua>`

Spec §4.5's sealing list, minus the parts the safe constructor already covers. Verified against mlua 0.12.1 sources: `Lua::new_with` returns `Error::SafetyError` if `StdLib::DEBUG` is requested, and calls `disable_c_modules()` whenever `StdLib::PACKAGE` is loaded, which replaces `package.loadlib` with a function that errors and overwrites the C searcher at index 3 (removing index 4 if present). So the crate must *not* re-implement those; it must not use `unsafe_new_with`, which skips them.

What this task does on top: omit `StdLib::IO`; keep `StdLib::OS` but remove the fields that run things or touch the filesystem, because dropping the whole library would take `os.time` / `os.clock` / `os.date` with it and every clock-like plugin needs those; remove `dofile` and `loadfile`, which read a file with `mode = NULL` and therefore accept binary chunks — the same hole `io` was removed for; empty `package.cpath`; and wrap the global `load` so its `mode` argument is forced to `"t"`, since Lua's default is `"bt"` and the manual warns that crafted bytecode can crash the interpreter.

- [ ] **Step 1: Write the failing test**

At the bottom of `crates/neotmux-lua/src/vm.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn is_nil(lua: &mlua::Lua, expr: &str) -> bool {
        lua.load(format!("return ({expr}) == nil"))
            .set_mode(mlua::ChunkMode::Text)
            .eval::<bool>()
            .unwrap()
    }

    #[test]
    fn the_blocking_and_bytecode_doors_are_shut() {
        let lua = new_vm().unwrap();
        for expr in [
            "io",
            "os.execute",
            "os.exit",
            "os.tmpname",
            "os.remove",
            "os.rename",
            "os.setlocale",
            "os.getenv",
            "dofile",
            "loadfile",
            "debug",
        ] {
            assert!(is_nil(&lua, expr), "{expr} should be nil");
        }
        let cpath: String = lua.load("return package.cpath").eval().unwrap();
        assert_eq!(cpath, "");
    }

    #[test]
    fn the_clock_still_works() {
        let lua = new_vm().unwrap();
        let t: i64 = lua.load("return os.time()").eval().unwrap();
        assert!(t > 0);
        lua.load("return os.date('%H:%M')").eval::<String>().unwrap();
        lua.load("return os.clock()").eval::<f64>().unwrap();
    }

    #[test]
    fn load_refuses_a_binary_chunk() {
        let lua = new_vm().unwrap();
        let ok: bool = lua
            .load(
                r#"
                local chunk = string.dump(function() return 1 end)
                local f, err = load(chunk, "bin", "b")
                return f == nil
                "#,
            )
            .eval()
            .unwrap();
        assert!(ok, "load accepted a binary chunk");
    }

    #[test]
    fn load_still_compiles_text() {
        let lua = new_vm().unwrap();
        let n: i64 = lua.load("local f = load('return 7') return f()").eval().unwrap();
        assert_eq!(n, 7);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua`
Expected: the crate does not exist yet, so this fails to resolve; after Step 3's manifest it fails on `new_vm` not being found.

- [ ] **Step 3: Write the manifests and the implementation**

Add to the workspace `Cargo.toml` members list: `"crates/neotmux-lua"`. Add to `[workspace.dependencies]`:

```toml
neotmux-lua = { path = "crates/neotmux-lua" }
mlua = { version = "0.12", features = ["lua54", "vendored"] }
```

`crates/neotmux-lua/Cargo.toml`:

```toml
[package]
name = "neotmux-lua"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
neotmux-core.workspace = true
anyhow.workspace = true
mlua.workspace = true
```

`crates/neotmux-lua/src/lib.rs`:

```rust
pub mod vm;
```

`crates/neotmux-lua/src/vm.rs`, above the tests:

```rust
use mlua::{ChunkMode, Lua, LuaOptions, StdLib, Table, Value};

const SEAL: &str = r#"
do
  local _load = load
  load = function(chunk, chunkname, _mode, env)
    return _load(chunk, chunkname, "t", env)
  end
end
"#;

pub fn new_vm() -> mlua::Result<Lua> {
    let libs = StdLib::COROUTINE
        | StdLib::TABLE
        | StdLib::STRING
        | StdLib::UTF8
        | StdLib::MATH
        | StdLib::OS
        | StdLib::PACKAGE;
    let lua = Lua::new_with(libs, LuaOptions::default())?;

    let globals = lua.globals();
    let os: Table = globals.get("os")?;
    for field in [
        "execute",
        "exit",
        "tmpname",
        "remove",
        "rename",
        "setlocale",
        "getenv",
    ] {
        os.raw_set(field, Value::Nil)?;
    }
    globals.raw_set("dofile", Value::Nil)?;
    globals.raw_set("loadfile", Value::Nil)?;

    let package: Table = globals.get("package")?;
    package.raw_set("cpath", "")?;

    lua.load(SEAL).set_mode(ChunkMode::Text).exec()?;

    Ok(lua)
}
```

The `load` wrapper is written in Lua rather than Rust so the reader-function form of `load` keeps working; `_load` is a local, and with `debug` gone a plugin has no way to reach it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/neotmux-lua
git commit -m "feat(lua): a sealed Lua 5.4 VM for plugins"
```

---

### Task 2: Runtime layers and `require`

**Files:**
- Create: `crates/neotmux-lua/src/runtime.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`

**Interfaces:**
- Consumes: `vm::new_vm`
- Produces:
  - `runtime::Layers(Vec<PathBuf>)` with `Layers::resolve(env: &dyn Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> Layers`
  - `runtime::install(lua: &mlua::Lua, layers: &Layers) -> mlua::Result<()>`

Spec §4.1's layer order: `$NEOTMUX_RUNTIME` (colon-separated) first if set, then `$XDG_CONFIG_HOME/neotmux/` (falling back to `$HOME/.config/neotmux/` when `XDG_CONFIG_HOME` is unset), then the bundled runtime located relative to the executable, then `/usr/share/neotmux/`. Each layer contributes `<layer>/lua/`. No compile-time developer path is baked in — that is a wart ttymap carries and this project does not want.

`install` sets `package.path` to `<layer>/lua/?.lua;<layer>/lua/?/init.lua` for every layer that exists, in order, so `require("plugin.clock")` and `require("neotmux.config")` both resolve by the standard searcher. Layers that do not exist are skipped rather than erroring: **zero resolvable layers is a normal startup with no plugins** (§4.1), because a multiplexer that refuses to start because its config is missing has failed at its only job.

Resolution takes the environment as a closure so the test does not have to mutate process-global state.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-lua/src/runtime.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use super::*;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let owned: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
            .collect();
        move |key: &str| {
            owned
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn explicit_runtime_wins_and_splits_on_colons() {
        let env = env_from(&[("NEOTMUX_RUNTIME", "/a:/b"), ("XDG_CONFIG_HOME", "/cfg")]);
        let layers = Layers::resolve(&env, None);
        assert_eq!(
            layers.paths()[..2],
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn xdg_config_home_comes_before_the_system_layer() {
        let env = env_from(&[("XDG_CONFIG_HOME", "/cfg")]);
        let layers = Layers::resolve(&env, None);
        let paths = layers.paths();
        let cfg = paths.iter().position(|p| p == &PathBuf::from("/cfg/neotmux"));
        let sys = paths
            .iter()
            .position(|p| p == &PathBuf::from("/usr/share/neotmux"));
        assert!(cfg.is_some() && sys.is_some() && cfg < sys);
    }

    #[test]
    fn home_is_the_fallback_when_xdg_is_unset() {
        let env = env_from(&[("HOME", "/home/someone")]);
        let layers = Layers::resolve(&env, None);
        assert!(
            layers
                .paths()
                .contains(&PathBuf::from("/home/someone/.config/neotmux"))
        );
    }

    #[test]
    fn requiring_a_plugin_from_a_layer_works() {
        let dir = tempdir();
        let lua_dir = dir.path().join("lua/plugin");
        fs::create_dir_all(&lua_dir).unwrap();
        fs::write(lua_dir.join("greet.lua"), "return { hello = 'world' }").unwrap();

        let lua = crate::vm::new_vm().unwrap();
        let layers = Layers::from_paths(vec![dir.path().to_path_buf()]);
        install(&lua, &layers).unwrap();

        let got: String = lua
            .load("return require('plugin.greet').hello")
            .eval()
            .unwrap();
        assert_eq!(got, "world");
    }

    #[test]
    fn a_config_holder_lib_resolves_from_the_same_layer() {
        let dir = tempdir();
        let lib_dir = dir.path().join("lua/neotmux");
        fs::create_dir_all(&lib_dir).unwrap();
        fs::write(lib_dir.join("clock.lua"), "return { format = '%H:%M' }").unwrap();

        let lua = crate::vm::new_vm().unwrap();
        let layers = Layers::from_paths(vec![dir.path().to_path_buf()]);
        install(&lua, &layers).unwrap();

        let got: String = lua
            .load("return require('neotmux.clock').format")
            .eval()
            .unwrap();
        assert_eq!(got, "%H:%M");
    }

    #[test]
    fn no_layers_is_a_working_vm_with_no_plugins() {
        let lua = crate::vm::new_vm().unwrap();
        let layers = Layers::from_paths(vec![]);
        install(&lua, &layers).unwrap();
        let path: String = lua.load("return package.path").eval().unwrap();
        assert_eq!(path, "");
        let missing: bool = lua
            .load("local ok = pcall(require, 'plugin.nope') return ok == false")
            .eval()
            .unwrap();
        assert!(missing);
    }
}
```

`tempdir()` is whatever temp-directory helper the workspace already uses; if there is none, add `tempfile` to `[workspace.dependencies]` as a dev-dependency of this crate and use `tempfile::tempdir()`, binding it to a variable so it outlives the test.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua runtime`
Expected: `Layers` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-lua/src/runtime.rs`, above the tests:

```rust
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use mlua::{Lua, Table};

#[derive(Debug, Clone, Default)]
pub struct Layers(Vec<PathBuf>);

impl Layers {
    pub fn from_paths(paths: Vec<PathBuf>) -> Self {
        Self(paths)
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }

    pub fn resolve(env: &dyn Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> Self {
        let mut paths = Vec::new();

        if let Some(explicit) = env("NEOTMUX_RUNTIME") {
            paths.extend(
                explicit
                    .to_string_lossy()
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from),
            );
        }

        if let Some(xdg) = env("XDG_CONFIG_HOME") {
            paths.push(PathBuf::from(xdg).join("neotmux"));
        } else if let Some(home) = env("HOME") {
            paths.push(PathBuf::from(home).join(".config/neotmux"));
        }

        if let Some(dir) = exe_dir {
            paths.push(dir.join("../share/neotmux"));
        }

        paths.push(PathBuf::from("/usr/share/neotmux"));

        Self(paths)
    }
}

pub fn install(lua: &Lua, layers: &Layers) -> mlua::Result<()> {
    let mut entries = Vec::new();
    for layer in layers.paths() {
        let lua_dir = layer.join("lua");
        if !lua_dir.is_dir() {
            continue;
        }
        let base = lua_dir.to_string_lossy();
        entries.push(format!("{base}/?.lua"));
        entries.push(format!("{base}/?/init.lua"));
    }

    let package: Table = lua.globals().get("package")?;
    package.raw_set("path", entries.join(";"))?;
    Ok(())
}
```

`crates/neotmux-lua/src/lib.rs`:

```rust
pub mod runtime;
pub mod vm;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: 10 passed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/neotmux-lua
git commit -m "feat(lua): resolve runtime layers and wire package.path"
```

---

### Task 3: Plugin identity and attribution

**Files:**
- Create: `crates/neotmux-lua/src/plugin.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`, `crates/neotmux-lua/src/runtime.rs`

**Interfaces:**
- Consumes: `runtime::{Layers, install}`, `vm::new_vm`
- Produces:
  - `plugin::PluginId(u32)` with `Display`
  - `plugin::Plugins` — `new()`, `intern(&self, name: &str) -> PluginId`, `name(&self, id: PluginId) -> Option<String>`, `current(&self) -> PluginId`, `push(&self, id: PluginId)`, `pop(&self)`
  - `plugin::ANONYMOUS: PluginId` and `plugin::INIT: PluginId` — the two reserved ids
  - `plugin::install_loader(lua: &mlua::Lua, plugins: Rc<Plugins>) -> mlua::Result<()>`

This is spec §4.2, and it is the task the whole milestone leans on: every budget notification, log line and unsubscribe decision names a plugin, so the host has to know which plugin a callback belongs to. With a single VM and a plain `package.path` searcher it does not — ttymap removed its per-plugin `host_tag` in its issue #240 and its docs now say "no host-side plugin attribution". Stripping `debug` (Task 1) also removes the Lua-side fallback of asking where you are. So attribution is designed in.

The mechanism: a searcher installed **ahead of** the standard Lua searcher for names beginning `plugin.`. It finds the file itself, loads it with `ChunkMode::Text`, and returns a wrapper closure that pushes the plugin's `PluginId`, calls the real chunk, and pops — so the identity is on the stack for exactly as long as the plugin's top-level code runs. Registrations (Task 4) read `Plugins::current()` at registration time.

The rules for what `current()` returns, from §4.2:

- inside a plugin chunk: that plugin
- inside a dispatch (Task 4 pushes the dispatched plugin's id around each callback): the plugin being dispatched, so a registration made from inside a callback inherits it
- otherwise — a bare line in `init.lua` — `INIT`
- a library `require`d outside any plugin chunk: whatever is on the stack, which is `ANONYMOUS` when the stack is empty

`Plugins` is `Rc`-shared and interior-mutable (`RefCell`), because the VM, the loader closure and the host all hold it. It is never `Send`: the VM is single-threaded by design (§4.6).

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-lua/src/plugin.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::fs;
    use std::rc::Rc;

    use super::*;

    fn layer_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua/plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        for (name, body) in files {
            fs::write(plugin_dir.join(format!("{name}.lua")), body).unwrap();
        }
        dir
    }

    fn vm_with(dir: &tempfile::TempDir) -> (mlua::Lua, Rc<Plugins>) {
        let lua = crate::vm::new_vm().unwrap();
        let layers = crate::runtime::Layers::from_paths(vec![dir.path().to_path_buf()]);
        crate::runtime::install(&lua, &layers).unwrap();
        let plugins = Rc::new(Plugins::new());
        install_loader(&lua, plugins.clone()).unwrap();
        (lua, plugins)
    }

    #[test]
    fn outside_any_plugin_the_current_id_is_init() {
        let dir = layer_with(&[]);
        let (_lua, plugins) = vm_with(&dir);
        assert_eq!(plugins.current(), INIT);
    }

    #[test]
    fn a_plugin_chunk_runs_under_its_own_id() {
        let dir = layer_with(&[(
            "recorder",
            "local n = require('neotmux_test_probe') n.seen = true return {}",
        )]);
        let (lua, plugins) = vm_with(&dir);

        let probe = lua.create_table().unwrap();
        lua.globals().set("neotmux_test_probe", probe).unwrap();
        lua.load("package.preload['neotmux_test_probe'] = function() return neotmux_test_probe end")
            .exec()
            .unwrap();

        let seen = Rc::new(std::cell::Cell::new(None));
        let captured = seen.clone();
        let plugins_for_fn = plugins.clone();
        let who = lua
            .create_function(move |_, ()| {
                captured.set(Some(plugins_for_fn.current()));
                Ok(())
            })
            .unwrap();
        lua.globals().set("who", who).unwrap();

        fs::write(
            dir.path().join("lua/plugin/recorder.lua"),
            "who() return {}",
        )
        .unwrap();
        lua.load("require('plugin.recorder')").exec().unwrap();

        let id = seen.get().expect("who() was not called");
        assert_eq!(plugins.name(id).as_deref(), Some("recorder"));
    }

    #[test]
    fn the_identity_is_popped_after_the_chunk() {
        let dir = layer_with(&[("quiet", "return {}")]);
        let (lua, plugins) = vm_with(&dir);
        lua.load("require('plugin.quiet')").exec().unwrap();
        assert_eq!(plugins.current(), INIT);
    }

    #[test]
    fn a_nested_require_restores_the_outer_identity() {
        let dir = layer_with(&[
            ("outer", "require('plugin.inner') who_outer() return {}"),
            ("inner", "who_inner() return {}"),
        ]);
        let (lua, plugins) = vm_with(&dir);

        let seen_inner = Rc::new(std::cell::Cell::new(None));
        let seen_outer = Rc::new(std::cell::Cell::new(None));
        for (name, slot) in [("who_inner", seen_inner.clone()), ("who_outer", seen_outer.clone())] {
            let plugins = plugins.clone();
            let f = lua
                .create_function(move |_, ()| {
                    slot.set(Some(plugins.current()));
                    Ok(())
                })
                .unwrap();
            lua.globals().set(name, f).unwrap();
        }

        lua.load("require('plugin.outer')").exec().unwrap();
        assert_eq!(plugins.name(seen_inner.get().unwrap()).as_deref(), Some("inner"));
        assert_eq!(plugins.name(seen_outer.get().unwrap()).as_deref(), Some("outer"));
        assert_eq!(plugins.current(), INIT);
    }

    #[test]
    fn a_plugin_that_raises_still_pops_its_identity() {
        let dir = layer_with(&[("boom", "error('no')")]);
        let (lua, plugins) = vm_with(&dir);
        let failed = lua.load("local ok = pcall(require, 'plugin.boom') return ok == false")
            .eval::<bool>()
            .unwrap();
        assert!(failed);
        assert_eq!(plugins.current(), INIT);
    }

    #[test]
    fn interning_is_stable() {
        let plugins = Plugins::new();
        let a = plugins.intern("clock");
        let b = plugins.intern("clock");
        let c = plugins.intern("other");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(plugins.name(a).as_deref(), Some("clock"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua plugin`
Expected: `Plugins` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-lua/src/plugin.rs`, above the tests:

```rust
use std::cell::RefCell;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use mlua::{ChunkMode, Function, Lua, Table, Value, Variadic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginId(u32);

pub const ANONYMOUS: PluginId = PluginId(0);
pub const INIT: PluginId = PluginId(1);
const FIRST_DYNAMIC: u32 = 2;

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[derive(Default)]
struct Inner {
    names: Vec<String>,
    stack: Vec<PluginId>,
}

pub struct Plugins {
    inner: RefCell<Inner>,
}

impl Default for Plugins {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugins {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                names: vec!["anonymous".to_string(), "init".to_string()],
                stack: Vec::new(),
            }),
        }
    }

    pub fn intern(&self, name: &str) -> PluginId {
        let mut inner = self.inner.borrow_mut();
        if let Some(pos) = inner.names.iter().position(|n| n == name) {
            return PluginId(pos as u32);
        }
        inner.names.push(name.to_string());
        PluginId(inner.names.len() as u32 - 1)
    }

    pub fn name(&self, id: PluginId) -> Option<String> {
        self.inner.borrow().names.get(id.0 as usize).cloned()
    }

    pub fn current(&self) -> PluginId {
        let inner = self.inner.borrow();
        inner.stack.last().copied().unwrap_or(INIT)
    }

    pub fn push(&self, id: PluginId) {
        self.inner.borrow_mut().stack.push(id);
    }

    pub fn pop(&self) {
        self.inner.borrow_mut().stack.pop();
    }
}

fn find_module(lua: &Lua, modname: &str) -> mlua::Result<Option<PathBuf>> {
    let package: Table = lua.globals().get("package")?;
    let path: String = package.get("path")?;
    let relative = modname.replace('.', "/");
    for pattern in path.split(';').filter(|p| !p.is_empty()) {
        let candidate = PathBuf::from(pattern.replacen('?', &relative, 1));
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

pub fn install_loader(lua: &Lua, plugins: Rc<Plugins>) -> mlua::Result<()> {
    let searcher = lua.create_function(move |lua, modname: String| {
        if !modname.starts_with("plugin.") {
            return Ok(Value::Nil);
        }
        let Some(path) = find_module(lua, &modname)? else {
            return Ok(Value::String(
                lua.create_string(format!("\n\tno plugin '{modname}'"))?,
            ));
        };

        let source = fs::read_to_string(&path).map_err(mlua::Error::external)?;
        let chunk_name = format!("@{}", path.display());
        let inner: Function = lua
            .load(&source)
            .set_name(chunk_name)
            .set_mode(ChunkMode::Text)
            .into_function()?;

        let id = plugins.intern(modname.trim_start_matches("plugin."));
        let plugins = plugins.clone();
        let wrapper = lua.create_function(move |_, args: Variadic<Value>| {
            plugins.push(id);
            let result = inner.call::<Variadic<Value>>(args);
            plugins.pop();
            result
        })?;
        Ok(Value::Function(wrapper))
    })?;

    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package.get("searchers")?;
    searchers.raw_insert(1, searcher)?;
    Ok(())
}
```

`plugins.pop()` runs before the result is inspected, so a chunk that raises still unwinds the stack — which is what `a_plugin_that_raises_still_pops_its_identity` pins.

`crates/neotmux-lua/src/lib.rs`:

```rust
pub mod plugin;
pub mod runtime;
pub mod vm;
```

Add `tempfile` to `[workspace.dependencies]` and as a dev-dependency of `neotmux-lua` if it is not already there.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: 15 passed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/neotmux-lua
git commit -m "feat(lua): attribute every chunk to the plugin that owns it"
```

---

### Task 4: Registrations and their handles

**Files:**
- Create: `crates/neotmux-lua/src/registry.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`

**Interfaces:**
- Consumes: `plugin::{PluginId, Plugins}`
- Produces:
  - `registry::Kind { Tick, Event(String), Keymap(String), Status }`
  - `registry::RegistrationId(u64)`
  - `registry::Registry` — `new()`, `add(&self, owner: PluginId, kind: Kind, callback: Rc<mlua::RegistryKey>) -> RegistrationId`, `remove(&self, id: RegistrationId) -> bool`, `snapshot(&self, tag: KindTag) -> Vec<RegistrationId>`, `callback(&self, id: RegistrationId) -> Option<Rc<mlua::RegistryKey>>`, `owner(&self, id: RegistrationId) -> Option<PluginId>`, `kind(&self, id: RegistrationId) -> Option<Kind>`, `remove_all_of(&self, owner: PluginId) -> usize`, `len(&self)`, `is_empty(&self)`
  - `registry::KindTag { Tick, Event, Keymap, Status }` — the discriminant without the payload, for iteration

Spec §4.3. The load-bearing decisions:

**Every registration returns a handle and `:remove()` is idempotent.** Reload is not in this milestone, but it cannot be added later without breaking the ABI unless the shape is fixed now — ttymap's dynamic-reload issue has been open since May for exactly this reason. Idempotence means the second `:remove()` is a no-op returning `false`, not an error.

**The host keeps the set of registrations per plugin**, which is what `remove_all_of` is for: a plugin that raises during load has its partial registrations withdrawn (§4.10), and M1b's overrun handling removes one plugin's tick subscription without touching its keymaps.

**Dispatch is an id snapshot plus a per-call lookup**, never an iteration over a borrowed vector: a callback may register or remove during dispatch, and the borrow must not be live across the call. This is the reentrancy discipline of §4.6 applied to the registry itself.

Storing the Lua callback: `mlua::RegistryKey` is not `Clone`, and a closure-based accessor (`with_callback(id, f)`) would hold the registry borrow across `f` — which is exactly the reentrancy the dispatcher must avoid. So the stored type is `Rc<mlua::RegistryKey>`: `callback(id)` clones the `Rc` and the borrow ends before the callback is invoked.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-lua/src/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{PluginId, Plugins};

    fn key(lua: &mlua::Lua, body: &str) -> std::rc::Rc<mlua::RegistryKey> {
        let f: mlua::Function = lua.load(body).eval().unwrap();
        std::rc::Rc::new(lua.create_registry_value(f).unwrap())
    }

    fn setup() -> (mlua::Lua, Registry, PluginId, PluginId) {
        let lua = crate::vm::new_vm().unwrap();
        let plugins = Plugins::new();
        let a = plugins.intern("alpha");
        let b = plugins.intern("beta");
        (lua, Registry::new(), a, b)
    }

    #[test]
    fn a_registration_remembers_its_owner_and_kind() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        assert_eq!(reg.owner(id), Some(a));
        assert_eq!(reg.snapshot(KindTag::Tick), vec![id]);
        assert!(reg.snapshot(KindTag::Status).is_empty());
    }

    #[test]
    fn remove_is_idempotent() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        assert!(reg.remove(id));
        assert!(!reg.remove(id));
        assert!(reg.snapshot(KindTag::Tick).is_empty());
    }

    #[test]
    fn removing_one_plugin_leaves_the_others_alone() {
        let (lua, reg, a, b) = setup();
        let a_tick = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        let a_status = reg.add(a, Kind::Status, key(&lua, "return function() end"));
        let b_tick = reg.add(b, Kind::Tick, key(&lua, "return function() end"));
        assert_eq!(reg.remove_all_of(a), 2);
        assert_eq!(reg.snapshot(KindTag::Tick), vec![b_tick]);
        assert!(reg.owner(a_tick).is_none());
        assert!(reg.owner(a_status).is_none());
    }

    #[test]
    fn snapshot_is_stable_while_a_callback_mutates_the_registry() {
        let (lua, reg, a, _) = setup();
        let first = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        let ids = reg.snapshot(KindTag::Tick);
        let added_during = reg.add(a, Kind::Tick, key(&lua, "return function() end"));
        reg.remove(first);
        assert_eq!(ids, vec![first]);
        assert_eq!(reg.snapshot(KindTag::Tick), vec![added_during]);
    }

    #[test]
    fn a_kind_carries_its_payload() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(
            a,
            Kind::Keymap("C-b".to_string()),
            key(&lua, "return function() end"),
        );
        assert_eq!(reg.kind(id), Some(Kind::Keymap("C-b".to_string())));
    }

    #[test]
    fn the_callback_outlives_the_registry_borrow() {
        let (lua, reg, a, _) = setup();
        let id = reg.add(a, Kind::Tick, key(&lua, "return function() return 42 end"));
        let cb = reg.callback(id).unwrap();
        let f: mlua::Function = lua.registry_value(&cb).unwrap();
        assert_eq!(f.call::<i64>(()).unwrap(), 42);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua registry`
Expected: `Registry` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-lua/src/registry.rs`, above the tests:

```rust
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::RegistryKey;

use crate::plugin::PluginId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Tick,
    Event(String),
    Keymap(String),
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindTag {
    Tick,
    Event,
    Keymap,
    Status,
}

impl Kind {
    pub fn tag(&self) -> KindTag {
        match self {
            Kind::Tick => KindTag::Tick,
            Kind::Event(_) => KindTag::Event,
            Kind::Keymap(_) => KindTag::Keymap,
            Kind::Status => KindTag::Status,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegistrationId(u64);

struct Entry {
    owner: PluginId,
    kind: Kind,
    callback: Rc<RegistryKey>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    entries: BTreeMap<RegistrationId, Entry>,
}

#[derive(Default)]
pub struct Registry {
    inner: RefCell<Inner>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, owner: PluginId, kind: Kind, callback: Rc<RegistryKey>) -> RegistrationId {
        let mut inner = self.inner.borrow_mut();
        inner.next += 1;
        let id = RegistrationId(inner.next);
        inner.entries.insert(id, Entry { owner, kind, callback });
        id
    }

    pub fn remove(&self, id: RegistrationId) -> bool {
        self.inner.borrow_mut().entries.remove(&id).is_some()
    }

    pub fn remove_all_of(&self, owner: PluginId) -> usize {
        let mut inner = self.inner.borrow_mut();
        let doomed: Vec<RegistrationId> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.owner == owner)
            .map(|(id, _)| *id)
            .collect();
        for id in &doomed {
            inner.entries.remove(id);
        }
        doomed.len()
    }

    pub fn snapshot(&self, tag: KindTag) -> Vec<RegistrationId> {
        self.inner
            .borrow()
            .entries
            .iter()
            .filter(|(_, e)| e.kind.tag() == tag)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn owner(&self, id: RegistrationId) -> Option<PluginId> {
        self.inner.borrow().entries.get(&id).map(|e| e.owner)
    }

    pub fn kind(&self, id: RegistrationId) -> Option<Kind> {
        self.inner.borrow().entries.get(&id).map(|e| e.kind.clone())
    }

    pub fn callback(&self, id: RegistrationId) -> Option<Rc<RegistryKey>> {
        self.inner
            .borrow()
            .entries
            .get(&id)
            .map(|e| e.callback.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
```

`BTreeMap` rather than `HashMap` so `snapshot` returns registrations in registration order, which makes dispatch deterministic and the tests above meaningful.

`callback` returning `Rc<RegistryKey>` is what lets the dispatcher hold the callback while the registry borrow is released — the reentrancy requirement of §4.6.

`crates/neotmux-lua/src/lib.rs`: add `pub mod registry;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: 21 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): registrations with disposable handles, tracked per plugin"
```

---

### Task 5: The op buffer

**Files:**
- Create: `crates/neotmux-lua/src/ops.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`

**Interfaces:**
- Consumes: `plugin::PluginId`
- Produces:
  - `ops::Op { Quit, Notify { level: ops::Level, text: String }, Log { level: ops::Level, text: String } }`
  - `ops::Level { Info, Warn, Error }`
  - `ops::OpBuffer` — `new()`, `push(&self, from: PluginId, op: Op)`, `drain(&self) -> Vec<(PluginId, Op)>`, `len(&self)`, `is_empty(&self)`

Spec §4.6. Everything a plugin asks the host to *do* goes through one buffer that the host drains at one point in the frame; host bindings never re-enter Lua, and Lua never gets a live mutable borrow of host state. ttymap's issue #294 — "register from anywhere" plus `Rc<RefCell<…>>` host state equals a runtime `BorrowMutError` — is what this avoids, and the op buffer is the conclusion that issue reaches.

`Op` is deliberately small here: part 2 adds the draw commands and the pane operations. What matters in this task is the discipline and the drain point, not the vocabulary.

`drain` takes everything and leaves the buffer empty, so an op pushed *during* the drain (by a callback the host invokes while draining) lands in the next frame rather than extending this one. That is the property the test pins.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/neotmux-lua/src/ops.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Plugins;

    #[test]
    fn ops_come_back_in_order_with_their_owner() {
        let plugins = Plugins::new();
        let a = plugins.intern("alpha");
        let b = plugins.intern("beta");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        buf.push(
            b,
            Op::Notify {
                level: Level::Warn,
                text: "careful".to_string(),
            },
        );
        let drained = buf.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, a);
        assert_eq!(drained[0].1, Op::Quit);
        assert_eq!(drained[1].0, b);
    }

    #[test]
    fn draining_empties_the_buffer() {
        let plugins = Plugins::new();
        let a = plugins.intern("alpha");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        assert_eq!(buf.drain().len(), 1);
        assert!(buf.is_empty());
        assert!(buf.drain().is_empty());
    }

    #[test]
    fn pushing_during_a_drain_lands_in_the_next_drain() {
        let plugins = Plugins::new();
        let a = plugins.intern("alpha");
        let buf = OpBuffer::new();
        buf.push(a, Op::Quit);
        let first = buf.drain();
        buf.push(
            a,
            Op::Log {
                level: Level::Info,
                text: "later".to_string(),
            },
        );
        assert_eq!(first.len(), 1);
        assert_eq!(buf.drain().len(), 1);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua ops`
Expected: `OpBuffer` not found.

- [ ] **Step 3: Write the implementation**

`crates/neotmux-lua/src/ops.rs`, above the tests:

```rust
use std::cell::RefCell;

use crate::plugin::PluginId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Quit,
    Notify { level: Level, text: String },
    Log { level: Level, text: String },
}

#[derive(Default)]
pub struct OpBuffer {
    ops: RefCell<Vec<(PluginId, Op)>>,
}

impl OpBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, from: PluginId, op: Op) {
        self.ops.borrow_mut().push((from, op));
    }

    pub fn drain(&self) -> Vec<(PluginId, Op)> {
        std::mem::take(&mut *self.ops.borrow_mut())
    }

    pub fn len(&self) -> usize {
        self.ops.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
```

`crates/neotmux-lua/src/lib.rs`: add `pub mod ops;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: 24 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): a single op buffer for everything a plugin asks of the host"
```

---

### Task 6: The `neotmux` table and the subsystem

**Files:**
- Create: `crates/neotmux-lua/src/api/mod.rs`, `crates/neotmux-lua/src/api/log.rs`, `crates/neotmux-lua/src/api/opt.rs`
- Modify: `crates/neotmux-lua/src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–5
- Produces:
  - `LuaSubsystem` — `build(layers: runtime::Layers) -> anyhow::Result<LuaSubsystem>`, `run_init(&self) -> Vec<LoadFailure>`, `dispatch_tick(&self)`, `drain_ops(&self) -> Vec<(PluginId, ops::Op)>`, `registry(&self) -> &registry::Registry`, `plugins(&self) -> &plugin::Plugins`, `opt(&self) -> Opts`
  - `LoadFailure { plugin: String, message: String }`
  - `Opts { tick_budget_ms: u64, tick_overrun_limit: u32, prefix: String }` — read by the host; M1b consumes the first two

This is where the pieces become one thing. The Lua surface installed here is what spec §4.3 lists, minus the parts part 2 owns:

```lua
neotmux.on_tick(fn)                -- returns a handle
neotmux.on_event(name, fn)         -- returns a handle
neotmux.keymap.set(key, fn)        -- returns a handle
neotmux.status.provider(fn)        -- returns a handle
neotmux.log.info(msg) / warn / error
neotmux.opt.<name> = value
neotmux.quit()
neotmux.notify(level, text)
```

Each registering call captures `plugins.current()` (Task 3) as the owner, stores the callback through `Registry::add` (Task 4), and returns a Lua table with a `remove` method closing over the `RegistrationId` — so `h:remove()` is idempotent because `Registry::remove` is.

`run_init` executes the `init.lua` chain: for each layer in order, if `<layer>/init.lua` exists, load it with `ChunkMode::Text` and call it. **A raising `init.lua` does not stop startup** (§4.10): the failure is collected into the returned `Vec<LoadFailure>` and the host logs and notifies it, then carries on with whatever loaded before the error. Likewise a plugin that raises during its own load is inert, and its partial registrations are withdrawn with `Registry::remove_all_of`.

`neotmux.opt` is a table with an `__newindex` metamethod that validates the key and stores into host state, so a typo is an error at `init.lua` time rather than a silently ignored setting.

- [ ] **Step 1: Write the failing tests**

Create `crates/neotmux-lua/tests/subsystem.rs`:

```rust
use std::fs;

use neotmux_lua::ops::{Level, Op};
use neotmux_lua::registry::KindTag;
use neotmux_lua::runtime::Layers;
use neotmux_lua::LuaSubsystem;

fn layer(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

#[test]
fn a_plugin_can_register_and_is_attributed() {
    let dir = layer(&[
        ("init.lua", "require('plugin.clock')"),
        (
            "lua/plugin/clock.lua",
            "neotmux.on_tick(function() end) neotmux.status.provider(function() end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert!(failures.is_empty(), "{failures:?}");

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1);
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.plugins().name(owner).as_deref(), Some("clock"));
    assert_eq!(sub.registry().snapshot(KindTag::Status).len(), 1);
}

#[test]
fn a_handle_removes_its_registration_and_is_idempotent() {
    let dir = layer(&[
        ("init.lua", "require('plugin.fickle')"),
        (
            "lua/plugin/fickle.lua",
            "local h = neotmux.on_tick(function() end) h:remove() h:remove() return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert!(sub.registry().snapshot(KindTag::Tick).is_empty());
}

#[test]
fn a_plugin_that_raises_is_inert_and_withdraws_its_registrations() {
    let dir = layer(&[
        ("init.lua", "require('plugin.ok') require('plugin.bad')"),
        ("lua/plugin/ok.lua", "neotmux.on_tick(function() end) return {}"),
        (
            "lua/plugin/bad.lua",
            "neotmux.on_tick(function() end) error('nope')",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("nope"));

    let ticks = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(ticks.len(), 1, "the failing plugin's registration survived");
    let owner = sub.registry().owner(ticks[0]).unwrap();
    assert_eq!(sub.plugins().name(owner).as_deref(), Some("ok"));
}

#[test]
fn a_broken_init_still_leaves_a_working_subsystem() {
    let dir = layer(&[("init.lua", "this is not lua")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(sub.registry().is_empty());
}

#[test]
fn no_layers_at_all_is_a_normal_startup() {
    let sub = LuaSubsystem::build(Layers::from_paths(vec![])).unwrap();
    assert!(sub.run_init().is_empty());
    assert!(sub.registry().is_empty());
}

#[test]
fn ops_reach_the_host_with_their_owner() {
    let dir = layer(&[
        ("init.lua", "require('plugin.loud')"),
        (
            "lua/plugin/loud.lua",
            "neotmux.log.warn('hi') neotmux.quit() return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let ops = sub.drain_ops();
    assert_eq!(ops.len(), 2);
    assert_eq!(sub.plugins().name(ops[0].0).as_deref(), Some("loud"));
    assert_eq!(
        ops[0].1,
        Op::Log {
            level: Level::Warn,
            text: "hi".to_string()
        }
    );
    assert_eq!(ops[1].1, Op::Quit);
}

#[test]
fn opt_sets_host_settings_and_rejects_unknown_keys() {
    let dir = layer(&[("init.lua", "neotmux.opt.tick_budget_ms = 12")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    assert_eq!(sub.opt().tick_budget_ms, 12);

    let dir = layer(&[("init.lua", "neotmux.opt.no_such_option = 1")]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    let failures = sub.run_init();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("no_such_option"));
}

#[test]
fn a_registration_made_during_a_dispatch_inherits_the_dispatched_plugin() {
    let dir = layer(&[
        ("init.lua", "require('plugin.spawner')"),
        (
            "lua/plugin/spawner.lua",
            "neotmux.on_tick(function() neotmux.on_tick(function() end) end) return {}",
        ),
    ]);
    let sub = LuaSubsystem::build(Layers::from_paths(vec![dir.path().to_path_buf()])).unwrap();
    assert!(sub.run_init().is_empty());
    let before = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(before.len(), 1);

    sub.dispatch_tick();

    let after = sub.registry().snapshot(KindTag::Tick);
    assert_eq!(after.len(), 2);
    for id in after {
        let owner = sub.registry().owner(id).unwrap();
        assert_eq!(sub.plugins().name(owner).as_deref(), Some("spawner"));
    }
}
```

The last test requires `LuaSubsystem::dispatch_tick(&self)` — a minimal dispatcher that takes an id snapshot, looks up each callback, pushes the owner's id, calls with no arguments, and pops. Part 2 replaces its body with the real per-frame dispatch that passes a draw context; the identity discipline it establishes is what this task pins.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p neotmux-lua --test subsystem`
Expected: `LuaSubsystem` not found.

- [ ] **Step 3: Write the implementation**

Build `api/mod.rs` to install the `neotmux` table, `api/log.rs` for `neotmux.log.*` (each pushing `Op::Log` with the calling plugin as owner), `api/opt.rs` for the validating `__newindex`, and `lib.rs` for `LuaSubsystem` itself holding `Lua`, `Rc<Plugins>`, `Rc<Registry>`, `Rc<OpBuffer>` and `Rc<RefCell<Opts>>`.

Three things the implementation must get right, because the tests above pin them and the reasons are not local:

- **`dispatch_tick` takes the snapshot first, then resolves each callback** through `Registry::callback`, which hands back an `Rc<RegistryKey>` so no registry borrow is alive during the call. A callback that registers or removes during dispatch must not corrupt the iteration
- **A plugin that raises during load has its registrations withdrawn** — wrap the `require` of each plugin so that on `Err` the host calls `Registry::remove_all_of(that_plugin)` before recording the `LoadFailure`
- **`Opts` validation lives in the `__newindex`**, so an unknown key raises inside `init.lua` and therefore surfaces as a `LoadFailure` rather than being silently dropped

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p neotmux-lua`
Expected: all unit tests plus 8 integration tests passing.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux-lua
git commit -m "feat(lua): the neotmux table, init.lua chain and load-failure handling"
```

---

### Task 7: Logging, and wiring the subsystem into the binary

**Files:**
- Create: `crates/neotmux/src/logging.rs`
- Modify: `crates/neotmux/src/main.rs`, `crates/neotmux/src/app.rs`, `crates/neotmux/Cargo.toml`
- Test: `crates/neotmux/tests/plugin_smoke.rs`

**Interfaces:**
- Consumes: `LuaSubsystem`, `runtime::Layers`, `ops::{Op, Level}`
- Produces: `logging::init(path: Option<&Path>) -> anyhow::Result<PathBuf>`; `app::run` gains a `lua: &LuaSubsystem` parameter

Spec §4.9. A full-screen TUI cannot use stderr once it is running, so:

- the log goes to `$XDG_STATE_HOME/neotmux/neotmux.log`, falling back to `$HOME/.local/state/neotmux/neotmux.log`; `--log <path>` overrides it
- **log initialisation failure is reported on stderr before raw mode is entered**, and startup continues. ttymap's issue #226 is the silent version of this bug
- `Op::Log` entries drained from the op buffer are written with their plugin's name as a tag
- `Op::Notify` has no renderer yet — part 2 owns the notification surface — so for now it is logged at its level and nothing more. Say so in the commit message rather than leaving it to be discovered

`main.rs` resolves the layers from the real environment, builds the subsystem, runs the init chain, reports any `LoadFailure` to the log and continues, and only then enters the terminal. `app::run` drains the op buffer once per frame, at one place, and acts on `Op::Quit` by returning.

The smoke test is the milestone's user-visible proof: a real binary, a real runtime layer in a temp directory, a plugin that logs a line, and the line in the log file after the process exits.

- [ ] **Step 1: Write the failing test**

`crates/neotmux/tests/plugin_smoke.rs`:

```rust
use std::fs;
use std::time::{Duration, Instant};

#[test]
fn a_plugin_loads_and_its_log_line_reaches_the_log_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lua/plugin")).unwrap();
    fs::write(
        dir.path().join("init.lua"),
        "require('plugin.hello')",
    )
    .unwrap();
    fs::write(
        dir.path().join("lua/plugin/hello.lua"),
        "neotmux.log.info('plugin-loaded-marker') return {}",
    )
    .unwrap();

    let log = dir.path().join("neotmux.log");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_neotmux"))
        .arg("--log")
        .arg(&log)
        .env("NEOTMUX_RUNTIME", dir.path())
        .env("SHELL", "/bin/true")
        .env("TERM", "xterm-256color")
        .status()
        .expect("run neotmux");
    assert!(status.success(), "neotmux exited with {status:?}");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = fs::read_to_string(&log) {
            if text.contains("plugin-loaded-marker") {
                assert!(text.contains("hello"), "the log line is not attributed: {text}");
                return;
            }
        }
        assert!(Instant::now() < deadline, "marker never reached {log:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
```

Running the binary with its stdio inherited rather than on a PTY is deliberate here: `ratatui::init()` on a non-tty still works for this purpose and the test is about the plugin path, not the rendering. If it turns out not to, use the PTY harness in `crates/neotmux/tests/terminal_restore.rs` instead and say so in the report.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p neotmux --test plugin_smoke`
Expected: failure — there is no `--log` flag and no plugin loading.

- [ ] **Step 3: Write the implementation**

Wire it as described above. `--log` is the only flag; parse it by hand rather than adding a CLI crate for one argument.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `make check` and `cargo test --workspace --release`
Expected: green, including every M0a and M0b test — in particular `idle.rs` (no bytes while idle: a tick dispatch that draws nothing must not mark the frame dirty), `terminal_restore.rs` and `pty_flood.rs`.

- [ ] **Step 5: Commit**

```bash
git add crates/neotmux Cargo.lock
git commit -m "feat: load Lua plugins at startup and log what they say"
```

---

## Definition of done for M1a part 1

- [ ] A plugin under `<runtime>/lua/plugin/` is `require`d by `init.lua` and runs
- [ ] Every registration is attributed to the plugin that made it, including one made from inside another plugin's dispatch
- [ ] `h:remove()` withdraws a registration and is idempotent
- [ ] A plugin that raises during load is inert, its partial registrations are withdrawn, and the other plugins still work
- [ ] A broken `init.lua`, a missing `init.lua` and zero runtime layers all start normally with no plugins
- [ ] `io`, `os.execute`, `dofile`, `loadfile`, `package.loadlib` and `debug` are absent; `package.cpath` is empty; `load` rejects a binary chunk
- [ ] `neotmux.opt` rejects an unknown key at `init.lua` time
- [ ] A config-holder module at `<runtime>/lua/neotmux/<name>.lua` is `require`-able, which is the §4.7 idiom
- [ ] A plugin's log line reaches the log file, tagged with its name
- [ ] Every M0a and M0b test still passes, `idle.rs` included
- [ ] `make check` and `cargo test --workspace --release` both clean

## What part 2 adds

The draw command buffer and `ctx` (§4.5), the key-consumption contract and prefix ownership (§4.4), the status line, resize ordering (§4.8), the host-owned notification surface, and the fixture plugin set that is §4.11's real exit criterion. M1b then adds the budget.
