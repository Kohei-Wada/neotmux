use std::cell::RefCell;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Table, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(u32);

/// The identity of a call with no real chunk behind it: a native function, a `loadstring`d
/// or otherwise anonymous chunk, or a call reached with no Lua frame below it at all.
pub const ANONYMOUS: SourceId = SourceId(0);

const RESERVED: usize = 1;

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Strips a chunk source down to the short, readable form used in logs: the runtime layer
/// prefix and the `.lua` suffix are dropped, so `@/runtime/lua/plugin/clock.lua` becomes
/// `plugin/clock` and `@/runtime/init.lua` becomes `init`. Two layers providing the same
/// relative path collide on this name on purpose — they are the same logical file — and the
/// full source remains available from `Sources` for anything that needs to disambiguate.
fn display_name(source: &str) -> String {
    let source = source.strip_prefix('@').unwrap_or(source);
    let relative = match source.rsplit_once("/lua/") {
        Some((_, rest)) => rest,
        None => source.rsplit('/').next().unwrap_or(source),
    };
    relative
        .strip_suffix(".lua")
        .unwrap_or(relative)
        .to_string()
}

#[derive(Default)]
struct Inner {
    sources: Vec<String>,
}

pub struct Sources {
    inner: RefCell<Inner>,
}

impl Default for Sources {
    fn default() -> Self {
        Self::new()
    }
}

impl Sources {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                sources: vec!["anonymous".to_string()],
            }),
        }
    }

    /// Interns a raw chunk source (typically `@<path>`). Idempotent: the same source string,
    /// however it is reached (a fresh `require`, a cache hit, a direct `Function::info()`
    /// lookup), always yields the same id.
    pub fn intern(&self, source: &str) -> SourceId {
        let mut inner = self.inner.borrow_mut();
        // The reserved "anonymous" slot is a display name only and must never be matched or
        // reassigned, or a real source literally named "anonymous" would alias the sentinel.
        if let Some(pos) = inner
            .sources
            .iter()
            .skip(RESERVED)
            .position(|s| s == source)
        {
            return SourceId((pos + RESERVED) as u32);
        }
        inner.sources.push(source.to_string());
        SourceId(inner.sources.len() as u32 - 1)
    }

    pub fn name(&self, id: SourceId) -> Option<String> {
        let raw = self.inner.borrow().sources.get(id.0 as usize).cloned()?;
        if id == ANONYMOUS {
            return Some(raw);
        }
        Some(display_name(&raw))
    }

    pub fn interned(&self) -> Vec<(SourceId, String)> {
        self.inner
            .borrow()
            .sources
            .iter()
            .enumerate()
            .map(|(idx, source)| (SourceId(idx as u32), source.clone()))
            .filter(|(id, _)| *id != ANONYMOUS)
            .map(|(id, source)| (id, display_name(&source)))
            .collect()
    }

    /// Interns a chunk source that may not name a real chunk. Only `@<path>` is a file the
    /// host can attribute to; a native function, a `loadstring`d chunk or a missing frame
    /// has nothing to intern and is `ANONYMOUS`. Both attribution paths — a registered
    /// callback's `Function::info()` and a call site's stack frame — go through here so the
    /// two cannot drift apart.
    pub fn intern_chunk(&self, source: Option<&str>) -> SourceId {
        match source {
            Some(source) if source.starts_with('@') => self.intern(source),
            _ => ANONYMOUS,
        }
    }
}

/// The owner of an API call is the chunk where the calling code was written (§4.2). Level 0
/// of the Lua stack is the host binding currently running, so the search starts at level 1.
/// C frames are skipped: `pcall(neotmux.quit)`, `xpcall`, `coroutine.wrap` and a callback
/// invoked by `table.sort` or `string.gsub` all put one between the binding and the chunk,
/// and the calling code was plainly still written somewhere. Nothing else is skipped, so the
/// library rule (§4.2) is unchanged — a library frame is a chunk and the walk stops there —
/// and a chunk with no file behind it stays `ANONYMOUS` instead of blaming its caller.
pub fn caller(lua: &Lua, sources: &Sources) -> SourceId {
    let mut level = 1;
    loop {
        let Some(source) = lua.inspect_stack(level, |debug| {
            debug.source().source.map(|source| source.into_owned())
        }) else {
            return ANONYMOUS;
        };
        if source.as_deref() == Some("=[C]") {
            level += 1;
            continue;
        }
        return sources.intern_chunk(source.as_deref());
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

/// The chunk name the loader would give a module, without loading it. Used to recover a
/// module's source after `require` has already failed and there is no `Function` left to ask.
pub(crate) fn resolve_chunk_name(lua: &Lua, modname: &str) -> mlua::Result<Option<String>> {
    Ok(find_module(lua, modname)?.map(|path| format!("@{}", path.display())))
}

pub fn install_loader(lua: &Lua, sources: Rc<Sources>) -> mlua::Result<()> {
    let searcher = lua.create_function(move |lua, modname: String| {
        let Some(path) = find_module(lua, &modname)? else {
            return Ok(Value::String(lua.create_string(format!(
                "\n\tno file matching '{modname}' on package.path"
            ))?));
        };

        let source = fs::read(&path).map_err(mlua::Error::external)?;
        let chunk_name = format!("@{}", path.display());
        let chunk: Function = lua
            .load(source)
            .set_name(chunk_name.clone())
            .set_mode(ChunkMode::Text)
            .into_function()?;

        // Interning at load time, not at first use, is what the draw ordering guarantee rests
        // on: ids are allocated in load order.
        sources.intern(&chunk_name);
        Ok(Value::Function(chunk))
    })?;

    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package.get("searchers")?;
    searchers.raw_insert(2, searcher)?;
    searchers.raw_remove(3)?;
    Ok(())
}

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

    fn vm_with(dir: &tempfile::TempDir) -> (mlua::Lua, Rc<Sources>) {
        let lua = crate::vm::new_vm().unwrap();
        let layers = crate::runtime::Layers::from_paths(vec![dir.path().to_path_buf()]);
        crate::runtime::install(&lua, &layers).unwrap();
        let sources = Rc::new(Sources::new());
        install_loader(&lua, sources.clone()).unwrap();
        (lua, sources)
    }

    /// Pins what `caller` walks past: the host binding it is called from is never the
    /// owner, and neither is the `pcall` in between — the owner is the chunk the call was
    /// written in, whatever levels the stack happens to hold.
    #[test]
    fn the_owner_is_the_first_lua_frame_above_the_host_binding() {
        let dir = layer_with(&[("probe", "who() pcall(who) return {}")]);
        let (lua, sources) = vm_with(&dir);

        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let captured = seen.clone();
        let sources_for_fn = sources.clone();
        let who = lua
            .create_function(move |lua, ()| {
                let owner = caller(lua, &sources_for_fn);
                captured
                    .borrow_mut()
                    .push(sources_for_fn.name(owner).unwrap_or_default());
                Ok(())
            })
            .unwrap();
        lua.globals().set("who", who).unwrap();
        lua.load("require('plugin.probe')").exec().unwrap();

        assert_eq!(
            *seen.borrow(),
            vec!["plugin/probe".to_string(); 2],
            "the direct call and the one through pcall must both name the chunk"
        );
    }

    #[test]
    fn a_call_with_no_lua_frame_below_it_is_anonymous() {
        let dir = layer_with(&[]);
        let (lua, sources) = vm_with(&dir);
        let seen = Rc::new(std::cell::Cell::new(None));
        let captured = seen.clone();
        let sources_for_fn = sources.clone();
        let who = lua
            .create_function(move |lua, ()| {
                captured.set(Some(caller(lua, &sources_for_fn)));
                Ok(())
            })
            .unwrap();
        who.call::<()>(()).unwrap();
        assert_eq!(seen.get().unwrap(), ANONYMOUS);
    }

    #[test]
    fn a_call_from_a_plugin_chunk_is_attributed_to_that_chunk() {
        let dir = layer_with(&[(
            "recorder",
            "local n = require('neotmux_test_probe') n.seen = true return {}",
        )]);
        let (lua, sources) = vm_with(&dir);

        let probe = lua.create_table().unwrap();
        lua.globals().set("neotmux_test_probe", probe).unwrap();
        lua.load(
            "package.preload['neotmux_test_probe'] = function() return neotmux_test_probe end",
        )
        .exec()
        .unwrap();

        let seen = Rc::new(std::cell::Cell::new(None));
        let captured = seen.clone();
        let sources_for_fn = sources.clone();
        let who = lua
            .create_function(move |lua, ()| {
                captured.set(Some(caller(lua, &sources_for_fn)));
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
        assert_eq!(sources.name(id).as_deref(), Some("plugin/recorder"));
    }

    #[test]
    fn a_nested_require_attributes_each_chunk_to_itself() {
        let dir = layer_with(&[
            ("outer", "require('plugin.inner') who_outer() return {}"),
            ("inner", "who_inner() return {}"),
        ]);
        let (lua, sources) = vm_with(&dir);

        let seen_inner = Rc::new(std::cell::Cell::new(None));
        let seen_outer = Rc::new(std::cell::Cell::new(None));
        for (name, slot) in [
            ("who_inner", seen_inner.clone()),
            ("who_outer", seen_outer.clone()),
        ] {
            let sources = sources.clone();
            let f = lua
                .create_function(move |lua, ()| {
                    slot.set(Some(caller(lua, &sources)));
                    Ok(())
                })
                .unwrap();
            lua.globals().set(name, f).unwrap();
        }

        lua.load("require('plugin.outer')").exec().unwrap();
        assert_eq!(
            sources.name(seen_inner.get().unwrap()).as_deref(),
            Some("plugin/inner")
        );
        assert_eq!(
            sources.name(seen_outer.get().unwrap()).as_deref(),
            Some("plugin/outer")
        );
    }

    #[test]
    fn interned_lists_sources_but_not_anonymous() {
        let sources = Sources::new();
        let a = sources.intern("clock");
        let b = sources.intern("watcher");
        let names: Vec<(SourceId, String)> = sources.interned();
        assert_eq!(
            names,
            vec![(a, "clock".to_string()), (b, "watcher".to_string())]
        );
    }

    #[test]
    fn a_source_named_anonymous_does_not_alias_the_sentinel() {
        let sources = Sources::new();
        let named = sources.intern("anonymous");
        assert_ne!(named, ANONYMOUS);
    }

    #[test]
    fn interning_is_stable() {
        let sources = Sources::new();
        let a = sources.intern("clock");
        let b = sources.intern("clock");
        let c = sources.intern("other");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(sources.name(a).as_deref(), Some("clock"));
    }

    #[test]
    fn a_bytecode_file_on_the_search_path_is_refused() {
        let dir = layer_with(&[]);
        let (lua, _sources) = vm_with(&dir);

        let bytecode: mlua::LuaString = lua
            .load("return string.dump(function() return 1 end)")
            .eval()
            .unwrap();
        fs::write(
            dir.path().join("lua/plugin/compiled.lua"),
            &*bytecode.as_bytes(),
        )
        .unwrap();

        let ok: bool = lua
            .load("local ok = pcall(require, 'plugin.compiled') return ok == false")
            .eval()
            .unwrap();
        assert!(ok, "a bytecode file was executed via require");
    }

    #[test]
    fn display_name_strips_the_layer_prefix_and_lua_suffix() {
        assert_eq!(
            display_name("@/runtime/lua/plugin/clock.lua"),
            "plugin/clock"
        );
        assert_eq!(
            display_name("@/runtime/lua/neotmux/config.lua"),
            "neotmux/config"
        );
        assert_eq!(display_name("@/runtime/init.lua"), "init");
        assert_eq!(
            display_name("@/runtime/lua/plugin/nested/thing/init.lua"),
            "plugin/nested/thing/init"
        );
    }

    #[test]
    fn every_chunk_is_interned_including_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let lib_dir = dir.path().join("lua/neotmux");
        fs::create_dir_all(&lib_dir).unwrap();
        fs::write(lib_dir.join("clock.lua"), "return { format = '%H:%M' }").unwrap();
        let (lua, sources) = vm_with(&dir);

        let got: String = lua
            .load("return require('neotmux.clock').format")
            .eval()
            .unwrap();
        assert_eq!(got, "%H:%M");
        let names: Vec<String> = sources.interned().into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, vec!["neotmux/clock".to_string()]);
    }

    #[test]
    fn requiring_plugin_foo_init_and_plugin_foo_share_one_id() {
        let dir = layer_with(&[]);
        let nested = dir.path().join("lua/plugin/foo");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("init.lua"), "return {}").unwrap();
        let (lua, sources) = vm_with(&dir);

        for modname in ["plugin.foo", "plugin.foo.init"] {
            lua.load(format!("require('{modname}')")).exec().unwrap();
        }
        assert_eq!(sources.interned().len(), 1, "{:?}", sources.interned());
    }
}
