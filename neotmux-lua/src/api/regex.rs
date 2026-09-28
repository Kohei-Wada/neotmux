use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use mlua::{Lua, Table};

/// How many compiled patterns are kept. A ruleset is a fixed set of patterns asked on every
/// look, so the cache is what keeps a look from compiling them again; the bound is for a
/// plugin that builds its patterns out of what it read, which would otherwise grow it forever.
const CACHED_PATTERNS: usize = 1024;

/// `neotmux.regex.is_match(pattern, text)`: Rust's `regex` syntax, because Lua's patterns have
/// no alternation, no anchors on a line and no Unicode classes, and the agent rulesets taken
/// from herdr are written in this one. An invalid pattern raises, with the parser's reason.
pub fn install(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let cache: Rc<RefCell<HashMap<String, regex::Regex>>> = Rc::default();
    let is_match = lua.create_function(move |_, (pattern, text): (String, String)| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&pattern) {
            let compiled = regex::Regex::new(&pattern)
                .map_err(|err| mlua::Error::RuntimeError(err.to_string()))?;
            if cache.len() >= CACHED_PATTERNS {
                cache.clear();
            }
            cache.insert(pattern.clone(), compiled);
        }
        Ok(cache[&pattern].is_match(&text))
    })?;
    table.set("is_match", is_match)?;
    Ok(table)
}
