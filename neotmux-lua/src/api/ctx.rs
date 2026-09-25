use std::collections::BTreeMap;
use std::rc::Rc;

use mlua::{Function, Table, UserData, UserDataMethods, Value};

use neotmux_proto::{Color, DrawCmd, PaneId, Rect};

use crate::api::rect;
use crate::draw::Buffers;
use crate::float::FloatId;
use crate::source::SourceId;

/// Scope-bound: created fresh inside `Lua::scope` for one dispatch and dies with it.
pub struct Ctx {
    owner: SourceId,
    buffers: Rc<Buffers>,
    /// The pane area, in surface coordinates. `size()` is its size and `area()` is the
    /// whole of it: a plugin that anchors to an edge needs the origin as well, because a
    /// reserved band moves the edge without changing the size.
    area: Rect,
    panes: Vec<(PaneId, Rect)>,
    /// This source's own band, in surface coordinates. Per source: two sources reserving the
    /// same edge get different bands, and a source that reserved nothing gets none.
    band: Option<Rect>,
    /// This source's live floats, as of this tick. Captured per callback, not per frame:
    /// a `move_to`/`resize_to` issued inside a tick takes effect on the next frame, so the
    /// draws this tick saw clip against the rect they started with (matches `band`).
    floats: BTreeMap<FloatId, Rect>,
}

impl Ctx {
    pub fn new(
        owner: SourceId,
        buffers: Rc<Buffers>,
        area: Rect,
        panes: Vec<(PaneId, Rect)>,
        band: Option<Rect>,
        floats: BTreeMap<FloatId, Rect>,
    ) -> Self {
        Self {
            owner,
            buffers,
            area,
            panes,
            band,
            floats,
        }
    }

    fn pane_rect(&self, id: PaneId) -> Option<Rect> {
        self.panes
            .iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, rect)| *rect)
    }
}

/// What the first argument to a drawing method names: nothing (the surface), a pane, or a
/// float handle (`neotmux.float`'s table, read back through its private `__id`).
enum Target {
    Surface,
    Pane(u32),
    Float(FloatId),
}

fn resolve(target: &Value) -> mlua::Result<Target> {
    match target {
        Value::Nil => Ok(Target::Surface),
        Value::Integer(i) => Ok(Target::Pane(*i as u32)),
        Value::Table(table) => {
            let id_fn: Function = table.get("__id")?;
            let id: u32 = id_fn.call(())?;
            Ok(Target::Float(FloatId(id)))
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "a pane, a float, or nil is expected, not {}",
            other.type_name()
        ))),
    }
}

/// `ctx:float_rect`'s target is always a float: a plain number or nil is a type error here,
/// not a pane or the surface — those have no `float_rect` question to answer.
fn resolve_float(target: &Value) -> mlua::Result<FloatId> {
    match resolve(target)? {
        Target::Float(id) => Ok(id),
        _ => Err(mlua::Error::RuntimeError(format!(
            "float_rect takes a float handle, not {}",
            target.type_name()
        ))),
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
            table.set("rows", this.area.h)?;
            table.set("cols", this.area.w)?;
            Ok(table)
        });

        // The same area, with the corner it starts at. What `size` leaves out, and what a
        // plugin drawing into a corner of the surface is off by for every band anybody
        // reserved.
        methods.add_method("area", |lua, this, ()| {
            Ok(Value::Table(rect::to_table(lua, this.area)?))
        });

        // The pane's own cells, at another place in its own history, for this frame only.
        //
        // What a copy mode wants: every colour and attribute the child wrote, which a plugin
        // redrawing the lines itself cannot have — `pane.content` hands back text. The pane
        // is not scrolled, and nothing about the session moves: this is what the frame shows.
        methods.add_method("view", |_, this, (pane, back): (u32, usize)| {
            this.buffers
                .record_view(this.owner, PaneId::from(pane), back);
            Ok(())
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
            |_, this, (target, x, y, text, colours): (Value, u16, u16, String, Option<Table>)| {
                let (fg, bg) = parse_colours(colours)?;
                match resolve(&target)? {
                    Target::Surface => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Text {
                                pane: None,
                                x,
                                y,
                                text,
                                fg,
                                bg,
                            },
                        );
                    }
                    Target::Pane(pane) => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Text {
                                pane: Some(PaneId::from(pane)),
                                x,
                                y,
                                text,
                                fg,
                                bg,
                            },
                        );
                    }
                    Target::Float(id) => {
                        let cmd = DrawCmd::Text {
                            pane: None,
                            x,
                            y,
                            text,
                            fg,
                            bg,
                        };
                        if let Some(rect) = this.floats.get(&id) {
                            this.buffers.record_float(
                                this.owner,
                                id,
                                crate::float::place(&cmd, rect),
                            );
                        }
                    }
                }
                Ok(())
            },
        );

        methods.add_method(
            "rect",
            |_, this, (target, x, y, w, h, colours): (Value, u16, u16, u16, u16, Option<Table>)| {
                let (fg, bg) = parse_colours(colours)?;
                let rect = Rect::new(x, y, w, h);
                match resolve(&target)? {
                    Target::Surface => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Rect {
                                pane: None,
                                rect,
                                fg,
                                bg,
                            },
                        );
                    }
                    Target::Pane(pane) => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Rect {
                                pane: Some(PaneId::from(pane)),
                                rect,
                                fg,
                                bg,
                            },
                        );
                    }
                    Target::Float(id) => {
                        let cmd = DrawCmd::Rect {
                            pane: None,
                            rect,
                            fg,
                            bg,
                        };
                        if let Some(float) = this.floats.get(&id) {
                            this.buffers.record_float(
                                this.owner,
                                id,
                                crate::float::place(&cmd, float),
                            );
                        }
                    }
                }
                Ok(())
            },
        );

        methods.add_method(
            "tint",
            |_, this, (target, x, y, w, h, colours): (Value, u16, u16, u16, u16, Option<Table>)| {
                let (fg, bg) = parse_tint_colours(colours)?;
                let rect = Rect::new(x, y, w, h);
                match resolve(&target)? {
                    Target::Surface => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Tint {
                                pane: None,
                                rect,
                                fg,
                                bg,
                            },
                        );
                    }
                    Target::Pane(pane) => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Tint {
                                pane: Some(PaneId::from(pane)),
                                rect,
                                fg,
                                bg,
                            },
                        );
                    }
                    Target::Float(id) => {
                        let cmd = DrawCmd::Tint {
                            pane: None,
                            rect,
                            fg,
                            bg,
                        };
                        if let Some(float) = this.floats.get(&id) {
                            this.buffers.record_float(
                                this.owner,
                                id,
                                crate::float::place(&cmd, float),
                            );
                        }
                    }
                }
                Ok(())
            },
        );

        methods.add_method(
            "line",
            |_,
             this,
             (target, x1, y1, x2, y2, colours, glyph): (
                Value,
                u16,
                u16,
                u16,
                u16,
                Option<Table>,
                Option<String>,
            )| {
                let fg = parse_fg_only(colours)?;
                let glyph = parse_glyph(glyph)?;
                match resolve(&target)? {
                    Target::Surface => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Line {
                                pane: None,
                                from: (x1, y1),
                                to: (x2, y2),
                                glyph,
                                fg,
                            },
                        );
                    }
                    Target::Pane(pane) => {
                        this.buffers.record(
                            this.owner,
                            DrawCmd::Line {
                                pane: Some(PaneId::from(pane)),
                                from: (x1, y1),
                                to: (x2, y2),
                                glyph,
                                fg,
                            },
                        );
                    }
                    Target::Float(id) => {
                        let cmd = DrawCmd::Line {
                            pane: None,
                            from: (x1, y1),
                            to: (x2, y2),
                            glyph,
                            fg,
                        };
                        if let Some(float) = this.floats.get(&id) {
                            this.buffers.record_float(
                                this.owner,
                                id,
                                crate::float::place(&cmd, float),
                            );
                        }
                    }
                }
                Ok(())
            },
        );

        methods.add_method("float_rect", |lua, this, target: Value| {
            let id = resolve_float(&target)?;
            match this.floats.get(&id) {
                Some(rect) => Ok(Value::Table(rect::to_table(lua, *rect)?)),
                None => Ok(Value::Nil),
            }
        });
    }
}
