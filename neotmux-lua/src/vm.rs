use mlua::chunk::ChunkMode;
use mlua::{Lua, LuaOptions, StdLib, Table, Value};

const SEAL: &str = r#"
do
  local _load = load
  -- Lua distinguishes an absent 4th argument from an explicit nil one, so forward the
  -- environment only when the caller supplied it; otherwise the chunk gets a nil _ENV.
  load = function(chunk, chunkname, _mode, ...)
    if select('#', ...) == 0 then
      return _load(chunk, chunkname, "t")
    end
    return _load(chunk, chunkname, "t", ...)
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
    package.raw_set("path", "")?;
    package.raw_set("cpath", "")?;
    package.raw_set("loadlib", Value::Nil)?;

    lua.load(SEAL).set_mode(ChunkMode::Text).exec()?;

    Ok(lua)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_nil(lua: &mlua::Lua, expr: &str) -> bool {
        lua.load(format!("return ({expr}) == nil"))
            .set_mode(mlua::chunk::ChunkMode::Text)
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
            "package.loadlib",
        ] {
            assert!(is_nil(&lua, expr), "{expr} should be nil");
        }
        let cpath: String = lua.load("return package.cpath").eval().unwrap();
        assert_eq!(cpath, "");
        let path: String = lua.load("return package.path").eval().unwrap();
        assert_eq!(path, "");
    }

    #[test]
    fn require_cannot_reach_a_lua_file_in_the_working_directory() {
        let dir = std::env::temp_dir().join(format!(
            "neotmux-lua-require-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.lua"), "return 1").unwrap();

        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let lua = new_vm().unwrap();
        let ok: bool = lua
            .load("local ok, _ = pcall(require, 'probe') return ok")
            .eval()
            .unwrap();
        std::env::set_current_dir(&original).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert!(!ok, "require found probe.lua via the working directory");
    }

    #[test]
    fn the_clock_still_works() {
        let lua = new_vm().unwrap();
        let t: i64 = lua.load("return os.time()").eval().unwrap();
        assert!(t > 0);
        lua.load("return os.date('%H:%M')")
            .eval::<String>()
            .unwrap();
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
    fn load_refuses_a_binary_chunk_through_the_three_argument_shape() {
        let lua = new_vm().unwrap();
        let ok: bool = lua
            .load(
                r#"
                local chunk = string.dump(function() return 1 end)
                local f = load(chunk, "bin")
                return f == nil
                "#,
            )
            .eval()
            .unwrap();
        assert!(ok, "load accepted a binary chunk");
    }

    #[test]
    fn load_refuses_a_binary_chunk_through_the_four_argument_shape() {
        let lua = new_vm().unwrap();
        let ok: bool = lua
            .load(
                r#"
                local chunk = string.dump(function() return 1 end)
                local f = load(chunk, "bin", "b", _ENV)
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
        let n: i64 = lua
            .load("local f = load('return 7 + #tostring(7)') return f()")
            .eval()
            .unwrap();
        assert_eq!(n, 8);
    }

    #[test]
    fn a_loaded_chunk_can_reach_the_globals() {
        let lua = new_vm().unwrap();
        let kind: String = lua
            .load("local f = load('return type(string)') return f()")
            .eval()
            .unwrap();
        assert_eq!(kind, "table");
    }

    #[test]
    fn an_explicitly_supplied_environment_replaces_the_globals() {
        let lua = new_vm().unwrap();
        let n: i64 = lua
            .load("local f = load('return x', 'c', 't', {x = 7}) return f()")
            .eval()
            .unwrap();
        assert_eq!(n, 7);

        let hidden: bool = lua
            .load("local f = load('return string', 'c', 't', {}) return f() == nil")
            .eval()
            .unwrap();
        assert!(
            hidden,
            "the supplied environment should not see the globals"
        );
    }
}
