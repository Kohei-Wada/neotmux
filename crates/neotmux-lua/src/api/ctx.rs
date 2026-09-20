use std::rc::Rc;

use mlua::{Table, UserData, UserDataMethods, Value};

use neotmux_core::proto::{Color, DrawCmd, PaneId, Rect, Size};

use crate::api::rect;
use crate::draw::Buffers;
use crate::source::SourceId;

/// Scope-bound: created fresh inside `Lua::scope` for one dispatch and dies with it.
pub struct Ctx {
    owner: SourceId,
    buffers: Rc<Buffers>,
    size: Size,
    panes: Vec<(PaneId, Rect)>,
    /// This source's own band, in surface coordinates. Per source: two sources reserving the
    /// same edge get different bands, and a source that reserved nothing gets none.
    band: Option<Rect>,
}

impl Ctx {
    pub fn new(
        owner: SourceId,
        buffers: Rc<Buffers>,
        size: Size,
        panes: Vec<(PaneId, Rect)>,
        band: Option<Rect>,
    ) -> Self {
        Self {
            owner,
            buffers,
            size,
            panes,
            band,
        }
    }

    fn pane_rect(&self, id: PaneId) -> Option<Rect> {
        self.panes
            .iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, rect)| *rect)
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn colour_field(table: &Table, field: &'static str) -> mlua::Result<Color> {
    match table.get(field)? {
        Value::Nil => Ok(Color::Default),
        Value::Integer(idx) => u8::try_from(idx).map(Color::Idx).map_err(|_| {
            mlua::Error::RuntimeError(format!(
                "{field} must be a palette index between 0 and 255, got {idx}"
            ))
        }),
        Value::String(s) => {
            let s = s.to_str()?;
            parse_hex(&s).ok_or_else(|| {
                mlua::Error::RuntimeError(format!(
                    "{field} must be a \"#rrggbb\" string, got {:?}",
                    s.as_ref()
                ))
            })
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "{field} must be a palette index or a \"#rrggbb\" string, got {}",
            other.type_name()
        ))),
    }
}

/// `line` has no background to paint, so a `bg` the caller supplied would be discarded.
/// Say so instead.
/// What a caller who names no glyph gets, unchanged from before `line` took one.
const DEFAULT_LINE_GLYPH: char = '#';

/// A line fills one cell per step, so its glyph has to be one character. Anything longer is
/// refused rather than truncated: a caller who passed "->" meant something the renderer
/// cannot draw.
fn parse_glyph(glyph: Option<String>) -> mlua::Result<char> {
    let Some(glyph) = glyph else {
        return Ok(DEFAULT_LINE_GLYPH);
    };
    let mut chars = glyph.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Ok(ch),
        _ => Err(mlua::Error::RuntimeError(format!(
            "line takes a single character to draw with, got {glyph:?}"
        ))),
    }
}

fn parse_fg_only(colours: Option<Table>) -> mlua::Result<Color> {
    let Some(table) = colours else {
        return Ok(Color::Default);
    };
    if table.get::<Value>("bg")? != Value::Nil {
        return Err(mlua::Error::RuntimeError(
            "line takes no bg: it draws single cells, with nothing behind them".to_string(),
        ));
    }
    colour_field(&table, "fg")
}

/// A channel the caller did not name at all, as distinct from one named `Default`. Only
/// `tint` can tell the two apart: everything else paints a whole cell.
fn optional_colour_field(table: &Table, field: &'static str) -> mlua::Result<Option<Color>> {
    match table.get::<Value>(field)? {
        Value::Nil => Ok(None),
        _ => colour_field(table, field).map(Some),
    }
}

/// `rect` with no colours is a deliberate erase, but a `tint` with no colours would change
/// nothing at all — which is the mistake `tint` exists to make loud rather than silent.
fn parse_tint_colours(colours: Option<Table>) -> mlua::Result<(Option<Color>, Option<Color>)> {
    let fg = match &colours {
        Some(table) => optional_colour_field(table, "fg")?,
        None => None,
    };
    let bg = match &colours {
        Some(table) => optional_colour_field(table, "bg")?,
        None => None,
    };
    if fg.is_none() && bg.is_none() {
        return Err(mlua::Error::RuntimeError(
            "tint takes the colours to change, at least one of fg and bg: it recolours what \
             is already there, and rect is what paints over it"
                .to_string(),
        ));
    }
    Ok((fg, bg))
}

fn parse_colours(colours: Option<Table>) -> mlua::Result<(Color, Color)> {
    let Some(table) = colours else {
        return Ok((Color::Default, Color::Default));
    };
    let fg = colour_field(&table, "fg")?;
    let bg = colour_field(&table, "bg")?;
    Ok((fg, bg))
}

impl UserData for Ctx {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("size", |lua, this, ()| {
            let table = lua.create_table()?;
            table.set("rows", this.size.rows)?;
            table.set("cols", this.size.cols)?;
            Ok(table)
        });

        methods.add_method("pane_rect", |lua, this, pane: u32| {
            match this.pane_rect(PaneId::from(pane)) {
                Some(found) => Ok(Value::Table(rect::to_table(lua, found)?)),
                None => Ok(Value::Nil),
            }
        });

        methods.add_method("reserved", |lua, this, ()| match this.band {
            Some(band) => Ok(Value::Table(rect::to_table(lua, band)?)),
            None => Ok(Value::Nil),
        });

        methods.add_method(
            "text",
            |_,
             this,
             (pane, x, y, text, colours): (
                Option<u32>,
                u16,
                u16,
                String,
                Option<Table>,
            )| {
                let (fg, bg) = parse_colours(colours)?;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Text {
                        pane: pane.map(PaneId::from),
                        x,
                        y,
                        text,
                        fg,
                        bg,
                    },
                );
                Ok(())
            },
        );

        methods.add_method(
            "rect",
            |_,
             this,
             (pane, x, y, w, h, colours): (
                Option<u32>,
                u16,
                u16,
                u16,
                u16,
                Option<Table>,
            )| {
                let (fg, bg) = parse_colours(colours)?;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Rect {
                        pane: pane.map(PaneId::from),
                        rect: Rect::new(x, y, w, h),
                        fg,
                        bg,
                    },
                );
                Ok(())
            },
        );

        methods.add_method(
            "tint",
            |_,
             this,
             (pane, x, y, w, h, colours): (
                Option<u32>,
                u16,
                u16,
                u16,
                u16,
                Option<Table>,
            )| {
                let (fg, bg) = parse_tint_colours(colours)?;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Tint {
                        pane: pane.map(PaneId::from),
                        rect: Rect::new(x, y, w, h),
                        fg,
                        bg,
                    },
                );
                Ok(())
            },
        );

        methods.add_method(
            "line",
            |_,
             this,
             (pane, x1, y1, x2, y2, colours, glyph): (
                Option<u32>,
                u16,
                u16,
                u16,
                u16,
                Option<Table>,
                Option<String>,
            )| {
                let fg = parse_fg_only(colours)?;
                let glyph = parse_glyph(glyph)?;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Line {
                        pane: pane.map(PaneId::from),
                        from: (x1, y1),
                        to: (x2, y2),
                        glyph,
                        fg,
                    },
                );
                Ok(())
            },
        );
    }
}
