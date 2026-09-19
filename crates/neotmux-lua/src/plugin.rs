use std::cell::RefCell;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Table, Value, Variadic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId(u32);

pub const ANONYMOUS: PluginId = PluginId(0);
pub const INIT: PluginId = PluginId(1);

const RESERVED: usize = 2;

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
        // The reserved sentinel slots (anonymous/init) are display names only and must
        // never be matched or reassigned, or a plugin literally named "init" would alias INIT.
        if let Some(pos) = inner.names.iter().skip(RESERVED).position(|n| n == name) {
            return PluginId((pos + RESERVED) as u32);
        }
        inner.names.push(name.to_string());
        PluginId(inner.names.len() as u32 - 1)
    }

    pub fn name(&self, id: PluginId) -> Option<String> {
        self.inner.borrow().names.get(id.0 as usize).cloned()
    }

    pub fn interned(&self) -> Vec<(PluginId, String)> {
        self.inner
            .borrow()
            .names
            .iter()
            .enumerate()
            .map(|(idx, name)| (PluginId(idx as u32), name.clone()))
            .filter(|(id, _)| *id != ANONYMOUS && *id != INIT)
            .collect()
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

/// Maps a chunk's `FunctionInfo::source` back to the plugin name the searcher would have
/// interned for it, or `None` if the source is not a plugin chunk (bare `init.lua`, a
/// `neotmux.*` library, or an anonymous/string chunk).
pub fn plugin_name_from_source(source: &str) -> Option<String> {
    let source = source.strip_prefix('@').unwrap_or(source);
    // rsplit_once is the correct inverse of the searcher's own path construction: a layer
    // directory whose own path happens to contain "/lua/plugin/" must not be mistaken for
    // the marker that starts the plugin's own dotted name.
    let rest = source.rsplit_once("/lua/plugin/").map(|(_, rest)| rest)?;
    let path = rest
        .strip_suffix("/init.lua")
        .or_else(|| rest.strip_suffix(".lua"))?;
    if path.is_empty() {
        return None;
    }
    Some(path.replace('/', "."))
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
        let Some(path) = find_module(lua, &modname)? else {
            return Ok(Value::String(lua.create_string(format!(
                "\n\tno file matching '{modname}' on package.path"
            ))?));
        };

        let source = fs::read(&path).map_err(mlua::Error::external)?;
        let chunk_name = format!("@{}", path.display());
        // `plugin.foo` and `plugin.foo.init` can both resolve to the same file (`?.lua` and
        // `?/init.lua` are both on package.path), so the id must come from the resolved
        // path, not from whichever spelling the caller typed — otherwise the same plugin
        // ends up interned under two ids depending on how it was required.
        let canonical_name = plugin_name_from_source(&chunk_name);
        let inner: Function = lua
            .load(source)
            .set_name(chunk_name)
            .set_mode(ChunkMode::Text)
            .into_function()?;

        if let Some(plugin_name) = modname.strip_prefix("plugin.") {
            let plugin_name = canonical_name.as_deref().unwrap_or(plugin_name);
            let id = plugins.intern(plugin_name);
            let plugins = plugins.clone();
            let wrapper = lua.create_function(move |_, args: Variadic<Value>| {
                plugins.push(id);
                let result = inner.call::<Variadic<Value>>(args);
                plugins.pop();
                result
            })?;
            return Ok(Value::Function(wrapper));
        }

        Ok(Value::Function(inner))
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
        lua.load(
            "package.preload['neotmux_test_probe'] = function() return neotmux_test_probe end",
        )
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
        for (name, slot) in [
            ("who_inner", seen_inner.clone()),
            ("who_outer", seen_outer.clone()),
        ] {
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
        assert_eq!(
            plugins.name(seen_inner.get().unwrap()).as_deref(),
            Some("inner")
        );
        assert_eq!(
            plugins.name(seen_outer.get().unwrap()).as_deref(),
            Some("outer")
        );
        assert_eq!(plugins.current(), INIT);
    }

    #[test]
    fn a_plugin_that_raises_still_pops_its_identity() {
        let dir = layer_with(&[("boom", "error('no')")]);
        let (lua, plugins) = vm_with(&dir);
        let failed = lua
            .load("local ok = pcall(require, 'plugin.boom') return ok == false")
            .eval::<bool>()
            .unwrap();
        assert!(failed);
        assert_eq!(plugins.current(), INIT);
    }

    #[test]
    fn interned_lists_plugins_but_not_anonymous_or_init() {
        let plugins = Plugins::new();
        let a = plugins.intern("clock");
        let b = plugins.intern("watcher");
        let names: Vec<(PluginId, String)> = plugins.interned();
        assert_eq!(
            names,
            vec![(a, "clock".to_string()), (b, "watcher".to_string())]
        );
    }

    #[test]
    fn a_plugin_named_init_or_anonymous_does_not_alias_the_sentinel() {
        let plugins = Plugins::new();
        let init_named = plugins.intern("init");
        let anon_named = plugins.intern("anonymous");
        assert_ne!(init_named, INIT);
        assert_ne!(anon_named, ANONYMOUS);
        assert_ne!(init_named, anon_named);
    }

    #[test]
    fn a_plugin_named_init_failing_does_not_disturb_inits_own_registrations() {
        let dir = layer_with(&[("init", "error('boom')")]);
        let (lua, plugins) = vm_with(&dir);
        let registry = crate::registry::Registry::new();

        let key = |body: &str| {
            let f: mlua::Function = lua.load(body).eval().unwrap();
            std::rc::Rc::new(lua.create_registry_value(f).unwrap())
        };
        let init_reg = registry.add(
            INIT,
            crate::registry::Kind::Tick,
            key("return function() end"),
        );

        let failed = lua
            .load("local ok = pcall(require, 'plugin.init') return ok == false")
            .eval::<bool>()
            .unwrap();
        assert!(failed);

        let plugin_init_id = plugins.intern("init");
        registry.remove_all_of(plugin_init_id);

        assert_eq!(registry.owner(init_reg), Some(INIT));
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

    #[test]
    fn a_bytecode_file_on_the_search_path_is_refused() {
        let dir = layer_with(&[]);
        let (lua, _plugins) = vm_with(&dir);

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
    fn plugin_name_from_source_recovers_the_name_a_plugin_chunk_would_intern() {
        assert_eq!(
            plugin_name_from_source("@/runtime/lua/plugin/clock.lua"),
            Some("clock".to_string())
        );
        assert_eq!(
            plugin_name_from_source("@/runtime/lua/plugin/nested/thing/init.lua"),
            Some("nested.thing".to_string())
        );
    }

    #[test]
    fn plugin_name_from_source_uses_the_last_lua_plugin_marker() {
        assert_eq!(
            plugin_name_from_source("@/home/kohei/lua/plugin/mylayer/lua/plugin/clock.lua"),
            Some("clock".to_string())
        );
    }

    #[test]
    fn plugin_name_from_source_rejects_non_plugin_chunks() {
        assert_eq!(plugin_name_from_source("@/runtime/init.lua"), None);
        assert_eq!(
            plugin_name_from_source("@/runtime/lua/neotmux/clock.lua"),
            None
        );
        assert_eq!(plugin_name_from_source("=[C]"), None);
        assert_eq!(plugin_name_from_source("[string \"...\"]"), None);
    }

    #[test]
    fn a_library_module_does_not_become_a_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let lib_dir = dir.path().join("lua/neotmux");
        fs::create_dir_all(&lib_dir).unwrap();
        fs::write(lib_dir.join("clock.lua"), "return { format = '%H:%M' }").unwrap();
        let (lua, plugins) = vm_with(&dir);

        let got: String = lua
            .load("return require('neotmux.clock').format")
            .eval()
            .unwrap();
        assert_eq!(got, "%H:%M");
        assert_eq!(plugins.current(), INIT);
    }
}
