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
        let non_empty = |key: &str| env(key).filter(|v| !v.is_empty());

        let mut paths = Vec::new();

        if let Some(explicit) = non_empty("NEOTMUX_RUNTIME") {
            paths.extend(
                explicit
                    .to_string_lossy()
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from),
            );
        }

        if let Some(xdg) = non_empty("XDG_CONFIG_HOME") {
            paths.push(PathBuf::from(xdg).join("neotmux"));
        } else if let Some(home) = non_empty("HOME") {
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let owned: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
            .collect();
        move |key: &str| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
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
        let cfg = paths
            .iter()
            .position(|p| p == &PathBuf::from("/cfg/neotmux"));
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
    fn an_empty_xdg_config_home_falls_back_to_home() {
        let env = env_from(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/someone")]);
        let layers = Layers::resolve(&env, None);
        assert!(
            layers
                .paths()
                .contains(&PathBuf::from("/home/someone/.config/neotmux"))
        );
    }

    #[test]
    fn an_empty_home_contributes_no_home_layer() {
        let env = env_from(&[("HOME", "")]);
        let layers = Layers::resolve(&env, None);
        assert_eq!(layers.paths(), &[PathBuf::from("/usr/share/neotmux")]);
    }

    #[test]
    fn no_resolved_layer_is_a_relative_path() {
        let env = env_from(&[
            ("NEOTMUX_RUNTIME", "/a:/b"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("HOME", "/home/someone"),
        ]);
        let layers = Layers::resolve(&env, Some(Path::new("/opt/neotmux/bin")));
        for p in layers.paths() {
            assert!(p.is_absolute(), "{p:?} is not absolute");
        }
    }

    #[test]
    fn requiring_a_plugin_from_a_layer_works() {
        let dir = tempdir().unwrap();
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
        let dir = tempdir().unwrap();
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
