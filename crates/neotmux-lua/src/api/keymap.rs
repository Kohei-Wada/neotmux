use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::api::{attribute, make_handle};
use crate::draw::Buffers;
use crate::ops::{Level, Op, OpBuffer};
use crate::registry::{Kind, KindTag, RegistrationId, Registry};
use crate::source::Sources;

/// Canonical order for modifier prefixes: Control, then Meta/Alt, then Shift.
const ORDER: [char; 3] = ['C', 'M', 'S'];

fn with_mods(mods: [bool; 3], key: &str) -> String {
    let mut out = String::new();
    for (idx, tag) in ORDER.iter().enumerate() {
        if mods[idx] {
            out.push(*tag);
            out.push('-');
        }
    }
    out.push_str(key);
    out
}

/// Canonical form so `C-b` and `Ctrl-b` compare equal for conflict detection. The modifier
/// prefixes are canonicalised; the key itself is not, because `input::key_spec` reports a
/// character key as the character the terminal produced and `X` is therefore a different key
/// from `x`. The single message for every rejection lives here, so the error a plugin author
/// reads is the same wherever a spec is parsed.
pub fn normalise(spec: &str) -> Result<String, String> {
    let refuse = |why: String| Err(format!("cannot parse key spec '{spec}': {why}"));

    if spec.is_empty() {
        return refuse("it is empty".to_string());
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
            return refuse(format!("the '{token}-' modifier is given twice"));
        }
        mods[idx] = true;
        rest = tail;
    }

    if rest.is_empty() {
        return refuse("it names no key".to_string());
    }

    let key = if rest.chars().count() == 1 {
        let c = rest.chars().next().expect("one character");
        if c.is_control() {
            return refuse("a control character is not a key name".to_string());
        }
        if mods[2] {
            let mut message = "S- does not combine with a single character, because the host \
                               reports a shifted key as the character it produces"
                .to_string();
            // Only a letter has a shifted form we can name; S-1 produces '!' on one layout
            // and something else on the next, so naming a replacement would be a guess.
            if c.is_alphabetic() {
                let instead = with_mods([mods[0], mods[1], false], &c.to_uppercase().to_string());
                message.push_str(&format!("; bind '{instead}' instead"));
            } else {
                message.push_str("; bind the character your terminal actually sends");
            }
            return refuse(message);
        }
        c.to_string()
    } else {
        let mut chars = rest.chars();
        let first = chars
            .next()
            .expect("more than one character")
            .to_ascii_uppercase();
        let tail: String = chars.as_str().to_ascii_lowercase();
        let key = format!("{first}{tail}");
        if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return refuse(format!("'{rest}' is not a key name"));
        }
        key
    };

    Ok(with_mods(mods, &key))
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
        let normalised = normalise(&spec).map_err(mlua::Error::RuntimeError)?;

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

    fn ok(spec: &str) -> String {
        normalise(spec).unwrap()
    }

    fn err(spec: &str) -> String {
        normalise(spec).unwrap_err()
    }

    #[test]
    fn modifier_prefixes_are_canonicalised_but_the_key_is_not() {
        assert_eq!(ok("C-x"), "C-x");
        assert_eq!(ok("c-x"), "C-x");
        assert_eq!(ok("Ctrl-x"), "C-x");
        assert_eq!(ok("CTRL-X"), "C-X");
    }

    #[test]
    fn an_uppercase_character_is_a_different_key_from_its_lowercase() {
        assert_ne!(ok("X"), ok("x"));
        assert_eq!(ok("X"), "X");
        assert_eq!(ok("C-X"), "C-X");
    }

    #[test]
    fn a_named_key_is_still_canonicalised_by_case() {
        assert_eq!(ok("enter"), "Enter");
        assert_eq!(ok("S-TAB"), "S-Tab");
    }

    #[test]
    fn modifiers_are_reordered_canonically() {
        assert_eq!(ok("M-C-x"), "C-M-x");
        assert_eq!(ok("S-M-C-Left"), "C-M-S-Left");
    }

    #[test]
    fn shift_with_a_single_character_is_refused_and_names_the_key_to_use() {
        let message = err("S-x");
        assert!(message.contains("cannot parse key spec 'S-x'"), "{message}");
        assert!(message.contains("bind 'X' instead"), "{message}");
        assert!(err("C-S-x").contains("bind 'C-X' instead"));
    }

    #[test]
    fn unparseable_specs_are_refused_with_a_reason() {
        assert!(err("Ctrl+Shift+F13").contains("is not a key name"));
        assert!(err("").contains("it is empty"));
        assert!(err("C-").contains("it names no key"));
    }

    #[test]
    fn a_duplicated_modifier_is_refused() {
        assert!(err("C-C-x").contains("given twice"));
    }

    #[test]
    fn punctuation_keys_can_be_bound() {
        assert_eq!(ok("C-]"), "C-]");
        assert_eq!(ok("."), ".");
    }

    #[test]
    fn the_final_dash_before_the_key_is_the_separator_not_the_key() {
        assert_eq!(ok("C--"), "C--");
    }
}
