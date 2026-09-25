//! `Commands`, the registry `neotmux.command.add` writes into: a name held by whichever
//! plugin registered it first, resolvable — the whole point of §4 — without entering Lua.
//! Mirrors `registry.rs`'s shape and borrow discipline: every accessor clones what it needs
//! out from under its `RefCell` borrow before the caller can call back into Lua with it.

use crate::plugin::RegisteredCommand;
use crate::plugin::SourceId;
use mlua::{RegistryKey, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

struct Entry {
    callback: Rc<RegistryKey>,
    owner: SourceId,
    desc: Option<String>,
}

#[derive(Default)]
pub struct Commands {
    inner: RefCell<BTreeMap<String, Entry>>,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `name` unless it is already held, in which case the existing owner is
    /// returned so the caller can name who is holding it. The first registration holds; a
    /// config layer's `init.lua` runs before the shipped one, so the user's name always wins.
    pub fn add(
        &self,
        name: String,
        owner: SourceId,
        desc: Option<String>,
        callback: Rc<RegistryKey>,
    ) -> Result<(), SourceId> {
        let mut inner = self.inner.borrow_mut();
        if let Some(existing) = inner.get(&name) {
            return Err(existing.owner);
        }
        inner.insert(
            name,
            Entry {
                callback,
                owner,
                desc,
            },
        );
        Ok(())
    }

    /// In name order, free from the `BTreeMap`.
    pub fn list(&self) -> Vec<RegisteredCommand> {
        self.inner
            .borrow()
            .iter()
            .map(|(name, entry)| RegisteredCommand {
                name: name.clone(),
                desc: entry.desc.clone(),
                owner: entry.owner,
            })
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<(Rc<RegistryKey>, SourceId)> {
        self.inner
            .borrow()
            .get(name)
            .map(|entry| (entry.callback.clone(), entry.owner))
    }
}

/// What a handler answered, turned into the lines it printed — `nil`, a string, or an array
/// of strings, the way the spec's §4 says. Anything else is an error against the plugin.
pub fn lines_from(value: Value) -> Result<Vec<String>, String> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::String(s) => Ok(vec![s.to_string_lossy().to_string()]),
        Value::Table(table) => {
            let refuse = || -> Result<Vec<String>, String> {
                Err("a command must return nil, a string, or an array of strings".to_string())
            };
            // A table is an array of strings only if every key it has is part of its
            // sequence part: `{ foo = 'bar' }` has no sequence at all, and `{ 'a', extra = 1
            // }` has one alongside a key that is not, and both are refused rather than
            // silently answering the sequence prefix they happen to have.
            let sequence_len = table.raw_len();
            let pair_count = table.pairs::<Value, Value>().count();
            if pair_count != sequence_len {
                return refuse();
            }
            let mut lines = Vec::with_capacity(sequence_len);
            for item in table.sequence_values::<Value>() {
                match item {
                    Ok(Value::String(s)) => lines.push(s.to_string_lossy().to_string()),
                    _ => return refuse(),
                }
            }
            Ok(lines)
        }
        _ => Err("a command must return nil, a string, or an array of strings".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{SourceId, Sources};
    use mlua::Table;

    fn key(lua: &mlua::Lua, body: &str) -> Rc<mlua::RegistryKey> {
        let f: mlua::Function = lua.load(body).eval().unwrap();
        Rc::new(lua.create_registry_value(f).unwrap())
    }

    fn setup() -> (mlua::Lua, Commands, SourceId, SourceId) {
        let lua = crate::plugin::new_vm().unwrap();
        let sources = Sources::new();
        let a = sources.intern("alpha");
        let b = sources.intern("beta");
        (lua, Commands::new(), a, b)
    }

    #[test]
    fn a_name_can_be_added_and_found() {
        let (lua, commands, a, _) = setup();
        commands
            .add(
                "scratch".to_string(),
                a,
                Some("a scratch window".to_string()),
                key(&lua, "return function() end"),
            )
            .unwrap();
        let (_, owner) = commands.get("scratch").unwrap();
        assert_eq!(owner, a);
    }

    #[test]
    fn a_second_registration_of_the_same_name_is_refused_and_names_the_holder() {
        let (lua, commands, a, b) = setup();
        commands
            .add(
                "scratch".to_string(),
                a,
                None,
                key(&lua, "return function() end"),
            )
            .unwrap();
        let err = commands
            .add(
                "scratch".to_string(),
                b,
                None,
                key(&lua, "return function() end"),
            )
            .unwrap_err();
        assert_eq!(err, a);
    }

    #[test]
    fn list_is_in_name_order() {
        let (lua, commands, a, _) = setup();
        commands
            .add(
                "zeta".to_string(),
                a,
                None,
                key(&lua, "return function() end"),
            )
            .unwrap();
        commands
            .add(
                "alpha".to_string(),
                a,
                None,
                key(&lua, "return function() end"),
            )
            .unwrap();
        let names: Vec<String> = commands.list().into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["alpha".to_string(), "zeta".to_string()]);
    }

    #[test]
    fn lines_from_nil_is_no_lines() {
        assert_eq!(lines_from(Value::Nil).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn lines_from_anything_else_is_an_error() {
        assert!(lines_from(Value::Boolean(true)).is_err());
    }

    #[test]
    fn lines_from_a_map_shaped_table_is_an_error() {
        let lua = crate::plugin::new_vm().unwrap();
        let table: Table = lua.load("return { foo = 'bar' }").eval().unwrap();
        assert!(lines_from(Value::Table(table)).is_err());
    }

    #[test]
    fn lines_from_a_mixed_table_is_an_error() {
        let lua = crate::plugin::new_vm().unwrap();
        let table: Table = lua.load("return { 'a', extra = 1 }").eval().unwrap();
        assert!(lines_from(Value::Table(table)).is_err());
    }
}
