use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::api::{attribute, make_handle};
use crate::draw::Buffers;
use crate::ops::{Level, Op, OpBuffer};
use crate::registry::{Kind, KindTag, RegistrationId, Registry};
use crate::source::Sources;

/// Canonical order for modifier prefixes: Control, then Meta/Alt, then Shift.
const ORDER: [char; 3] = ['C', 'M', 'S'];

/// Canonical form so `C-b`, `c-b` and `Ctrl-b` compare equal for conflict detection.
pub fn normalise(spec: &str) -> Option<String> {
    if spec.is_empty() {
        return None;
    }

    let mut mods = [false; 3];
    let mut rest = spec;
    while let Some((token, tail)) = rest.split_once('-') {
        let idx = match token.to_ascii_lowercase().as_str() {
            "c" | "ctrl" | "control" => 0,
            "m" | "meta" | "alt" => 1,
            "s" | "shift" => 2,
            _ => break,
        };
        if mods[idx] {
            return None;
        }
        mods[idx] = true;
        rest = tail;
    }

    if rest.is_empty() {
        return None;
    }

    let key = if rest.chars().count() == 1 {
        let c = rest.chars().next()?;
        if c.is_control() {
            return None;
        }
        c.to_ascii_lowercase().to_string()
    } else {
        let mut chars = rest.chars();
        let first = chars.next()?.to_ascii_uppercase();
        let tail: String = chars.as_str().to_ascii_lowercase();
        let key = format!("{first}{tail}");
        if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        key
    };

    let mut out = String::new();
    for (idx, tag) in ORDER.iter().enumerate() {
        if mods[idx] {
            out.push(*tag);
            out.push('-');
        }
    }
    out.push_str(&key);
    Some(out)
}

/// The one scan `holder_of` and `dispatch_key` both need: the registration,
/// if any, bound to this normalised spec.
pub(crate) fn find(registry: &Registry, spec: &str) -> Option<RegistrationId> {
    registry
        .snapshot(KindTag::Keymap)
        .into_iter()
        .find(|id| matches!(registry.kind(*id), Some(Kind::Keymap(existing)) if existing == spec))
}

fn holder_of(registry: &Registry, sources: &Sources, spec: &str) -> Option<String> {
    let owner = registry.owner(find(registry, spec)?)?;
    sources.name(owner)
}

pub fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    registry: Rc<Registry>,
    buffers: Rc<Buffers>,
    ops: Rc<OpBuffer>,
) -> mlua::Result<Table> {
    let keymap = lua.create_table()?;
    let set = lua.create_function(move |lua, (spec, callback): (String, Function)| {
        // A spec the host cannot parse is a programming error, like every other malformed
        // argument in the ABI, so it raises. Losing a conflict is not, so that still returns.
        let Some(normalised) = normalise(&spec) else {
            return Err(mlua::Error::RuntimeError(format!(
                "cannot parse key spec '{spec}'"
            )));
        };

        if let Some(holder) = holder_of(&registry, &sources, &normalised) {
            let attempter = attribute(&sources, &callback);
            let attempted_by = sources.name(attempter).unwrap_or_default();
            let message = format!("'{normalised}' is already bound by {holder}");
            ops.push(
                attempter,
                Op::Log {
                    level: Level::Warn,
                    text: format!(
                        "keymap conflict on '{normalised}': held by {holder}, refused for {attempted_by}"
                    ),
                },
            );
            return Ok((Value::Nil, Value::String(lua.create_string(message)?)));
        }

        let owner = attribute(&sources, &callback);
        let key = Rc::new(lua.create_registry_value(callback)?);
        let id = registry.add(owner, Kind::Keymap(normalised), key);
        let handle = make_handle(lua, registry.clone(), buffers.clone(), id)?;
        Ok((Value::Table(handle), Value::Nil))
    })?;
    keymap.set("set", set)?;
    Ok(keymap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_key_normalises_to_itself_lowercased() {
        assert_eq!(normalise("C-x").as_deref(), Some("C-x"));
        assert_eq!(normalise("c-X").as_deref(), Some("C-x"));
        assert_eq!(normalise("Ctrl-x").as_deref(), Some("C-x"));
    }

    #[test]
    fn modifiers_are_reordered_canonically() {
        assert_eq!(normalise("M-C-x").as_deref(), Some("C-M-x"));
        assert_eq!(normalise("S-M-C-x").as_deref(), Some("C-M-S-x"));
    }

    #[test]
    fn unparseable_specs_are_refused() {
        assert_eq!(normalise("Ctrl+Shift+F13"), None);
        assert_eq!(normalise(""), None);
    }

    #[test]
    fn a_duplicated_modifier_is_refused() {
        assert_eq!(normalise("C-C-x"), None);
    }

    #[test]
    fn punctuation_keys_can_be_bound() {
        assert_eq!(normalise("C-]").as_deref(), Some("C-]"));
        assert_eq!(normalise(".").as_deref(), Some("."));
    }

    #[test]
    fn the_final_dash_before_the_key_is_the_separator_not_the_key() {
        assert_eq!(normalise("C--").as_deref(), Some("C--"));
    }
}
