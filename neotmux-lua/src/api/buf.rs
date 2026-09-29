//! `neotmux.buf`: Neovim's `nvim_create_buf`, `nvim_buf_set_lines` and extmarks, over the
//! lines the host keeps and draws in whatever window shows them.

use std::rc::Rc;

use mlua::{Lua, Table, Value};
use neotmux_proto::screen::width;

use crate::draw::{BufId, Extmark, TextBuffers};
use crate::plugin::{self, Groups, Held, Sources};

fn missing(buf: u32) -> mlua::Error {
    mlua::Error::RuntimeError(format!("buffer {buf} does not exist"))
}

/// Neovim's `start`/`end`: negative counts from past the end, so -1 is the end itself.
fn index(at: i64, len: usize, what: &str) -> mlua::Result<usize> {
    let resolved = if at < 0 { len as i64 + 1 + at } else { at };
    if resolved < 0 || resolved as usize > len {
        return Err(mlua::Error::RuntimeError(format!(
            "{what} {at} is out of range for a buffer of {len} lines"
        )));
    }
    Ok(resolved as usize)
}

fn lines_arg(lines: Table) -> mlua::Result<Vec<String>> {
    let mut out = Vec::new();
    for value in lines.sequence_values::<Value>() {
        match value? {
            Value::String(s) => {
                let s = s.to_str()?.to_string();
                if s.contains('\n') {
                    return Err(mlua::Error::RuntimeError(
                        "a line may not hold a newline; pass it as two lines".into(),
                    ));
                }
                out.push(s);
            }
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "a line is a string, got {}",
                    other.type_name()
                )));
            }
        }
    }
    Ok(out)
}

pub(crate) fn install(
    lua: &Lua,
    sources: Rc<Sources>,
    bufs: Rc<TextBuffers>,
    groups: Rc<Groups>,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    let create = {
        let bufs = bufs.clone();
        lua.create_function(move |lua, ()| {
            let owner = plugin::caller(lua, &sources);
            let id = bufs.create(owner);
            groups.note(Held::Buf(id));
            Ok(id.0)
        })?
    };
    t.set("create", create)?;

    let delete = {
        let bufs = bufs.clone();
        lua.create_function(move |_, buf: u32| {
            bufs.delete(BufId(buf));
            Ok(())
        })?
    };
    t.set("delete", delete)?;

    let set_lines = {
        let bufs = bufs.clone();
        lua.create_function(move |_, (buf, start, end, lines): (u32, i64, i64, Table)| {
            let lines = lines_arg(lines)?;
            let len = bufs
                .read(BufId(buf), |b| b.lines.len())
                .ok_or_else(|| missing(buf))?;
            let start = index(start, len, "start")?;
            let end = index(end, len, "end")?;
            if start > end {
                return Err(mlua::Error::RuntimeError(format!(
                    "start {start} is past end {end}"
                )));
            }
            bufs.with(BufId(buf), |b| b.set_lines(start, end, lines));
            Ok(())
        })?
    };
    t.set("set_lines", set_lines)?;

    let get_lines = {
        let bufs = bufs.clone();
        lua.create_function(move |lua, (buf, start, end): (u32, i64, i64)| {
            let lines = bufs
                .read(BufId(buf), |b| b.lines.clone())
                .ok_or_else(|| missing(buf))?;
            let start = index(start, lines.len(), "start")?;
            let end = index(end, lines.len(), "end")?;
            lua.create_sequence_from(lines[start..end.max(start)].iter().cloned())
        })?
    };
    t.set("get_lines", get_lines)?;

    let line_count = {
        let bufs = bufs.clone();
        lua.create_function(move |_, buf: u32| {
            bufs.read(BufId(buf), |b| b.lines.len())
                .ok_or_else(|| missing(buf))
        })?
    };
    t.set("line_count", line_count)?;

    let namespace = {
        let bufs = bufs.clone();
        lua.create_function(move |_, name: String| Ok(bufs.namespace(&name)))?
    };
    t.set("namespace", namespace)?;

    let set_extmark = {
        let bufs = bufs.clone();
        lua.create_function(
            move |_, (buf, ns, row, col, opts): (u32, u32, usize, usize, Option<Table>)| {
                let rows = bufs
                    .read(BufId(buf), |b| b.lines.len())
                    .ok_or_else(|| missing(buf))?;
                if row >= rows {
                    return Err(mlua::Error::RuntimeError(format!(
                        "row {row} is past the buffer's {rows} lines"
                    )));
                }
                let opts = match opts {
                    Some(opts) => opts,
                    None => {
                        return Err(mlua::Error::RuntimeError(
                            "set_extmark takes { hl_group = } or { line_hl_group = }".into(),
                        ));
                    }
                };
                let hl_group: Option<String> = opts.get("hl_group")?;
                let line_hl_group: Option<String> = opts.get("line_hl_group")?;
                if hl_group.is_none() && line_hl_group.is_none() {
                    return Err(mlua::Error::RuntimeError(
                        "set_extmark takes { hl_group = } or { line_hl_group = }".into(),
                    ));
                }
                let end_col: Option<usize> = opts.get("end_col")?;
                let bold: Option<bool> = opts.get("bold")?;
                let id = bufs
                    .with(BufId(buf), |b| {
                        // A column past the line's end is clamped to it.
                        let cols = width::of_str(&b.lines[row]);
                        let id = b.next_extmark;
                        b.next_extmark += 1;
                        b.extmarks.push(Extmark {
                            id,
                            ns,
                            row,
                            col: col.min(cols),
                            end_col: end_col.map(|e| e.min(cols)),
                            hl_group,
                            line_hl_group,
                            bold,
                        });
                        id
                    })
                    .ok_or_else(|| missing(buf))?;
                Ok(id)
            },
        )?
    };
    t.set("set_extmark", set_extmark)?;

    let get_extmarks = {
        let bufs = bufs.clone();
        lua.create_function(move |lua, (buf, ns): (u32, u32)| {
            let mut marks = bufs
                .read(BufId(buf), |b| {
                    b.extmarks
                        .iter()
                        .filter(|m| m.ns == ns)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .ok_or_else(|| missing(buf))?;
            marks.sort_by_key(|m| (m.row, m.col, m.id));
            let out = lua.create_table()?;
            for mark in marks {
                let m = lua.create_table()?;
                m.set("id", mark.id)?;
                m.set("row", mark.row)?;
                m.set("col", mark.col)?;
                m.set("end_col", mark.end_col)?;
                m.set("hl_group", mark.hl_group)?;
                m.set("line_hl_group", mark.line_hl_group)?;
                out.push(m)?;
            }
            Ok(out)
        })?
    };
    t.set("get_extmarks", get_extmarks)?;

    let clear_namespace = {
        let bufs = bufs.clone();
        lua.create_function(move |_, (buf, ns, start, end): (u32, u32, i64, i64)| {
            let len = bufs
                .read(BufId(buf), |b| b.lines.len())
                .ok_or_else(|| missing(buf))?;
            let start = index(start, len, "start")?;
            let end = index(end, len, "end")?;
            bufs.with(BufId(buf), |b| {
                b.extmarks
                    .retain(|m| m.ns != ns || m.row < start || m.row >= end)
            });
            Ok(())
        })?
    };
    t.set("clear_namespace", clear_namespace)?;

    Ok(t)
}
