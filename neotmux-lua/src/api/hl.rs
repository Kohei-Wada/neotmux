//! `neotmux.hl`: Neovim's `nvim_set_hl` / `nvim_get_hl`, for the groups buffers are coloured by.

use std::rc::Rc;

use mlua::{Lua, Table, Value};

use crate::api::ctx::optional_colour_field;
use crate::api::pane::color_value;
use crate::draw::{Hl, HlGroups};

pub(crate) fn install(lua: &Lua, groups: Rc<HlGroups>) -> mlua::Result<Table> {
    let hl = lua.create_table()?;
    let set = {
        let groups = groups.clone();
        lua.create_function(move |_, (name, spec): (String, Table)| {
            let bold = spec.get::<Option<bool>>("bold")?;
            let fg = optional_colour_field(&spec, "fg")?;
            let bg = optional_colour_field(&spec, "bg")?;
            groups.set(&name, Hl { fg, bg, bold });
            Ok(())
        })?
    };
    hl.set("set", set)?;
    let get = lua.create_function(move |lua, name: String| {
        let Some(found) = groups.get(&name) else {
            return Ok(Value::Nil);
        };
        let out = lua.create_table()?;
        if let Some(fg) = found.fg {
            out.set("fg", color_value(lua, fg)?)?;
        }
        if let Some(bg) = found.bg {
            out.set("bg", color_value(lua, bg)?)?;
        }
        if let Some(bold) = found.bold {
            out.set("bold", bold)?;
        }
        Ok(Value::Table(out))
    })?;
    hl.set("get", get)?;
    Ok(hl)
}
