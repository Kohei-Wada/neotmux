use std::rc::Rc;

use mlua::{Table, UserData, UserDataMethods, Value};

use neotmux_core::proto::{Color, DrawCmd, PaneId, Rect, Size, StatusFragment};

use crate::draw::Buffers;
use crate::plugin::PluginId;

/// A per-tick draw surface handed to a plugin's `on_tick` callback. Built fresh for every
/// dispatch and created inside `Lua::scope`, so a plugin that stashes it gets an error from
/// mlua rather than recording into a frame whose epoch has since moved on.
pub struct Ctx {
    owner: PluginId,
    buffers: Rc<Buffers>,
    size: Size,
    panes: Vec<(PaneId, Rect)>,
    focus: Option<PaneId>,
}

impl Ctx {
    pub fn new(
        owner: PluginId,
        buffers: Rc<Buffers>,
        size: Size,
        panes: Vec<(PaneId, Rect)>,
        focus: Option<PaneId>,
    ) -> Self {
        Self {
            owner,
            buffers,
            size,
            panes,
            focus,
        }
    }

    pub fn focus(&self) -> Option<PaneId> {
        self.focus
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
        methods.add_method("size", |_, this, ()| Ok((this.size.cols, this.size.rows)));

        methods.add_method("pane_rect", |lua, this, pane: u32| {
            match this.pane_rect(PaneId::from(pane)) {
                Some(rect) => {
                    let table = lua.create_table()?;
                    table.set("x", rect.x)?;
                    table.set("y", rect.y)?;
                    table.set("w", rect.w)?;
                    table.set("h", rect.h)?;
                    Ok(Value::Table(table))
                }
                None => Ok(Value::Nil),
            }
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
                let mut rect = Rect::default();
                rect.x = x;
                rect.y = y;
                rect.w = w;
                rect.h = h;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Rect {
                        pane: pane.map(PaneId::from),
                        rect,
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
             (pane, x1, y1, x2, y2, colours): (
                Option<u32>,
                u16,
                u16,
                u16,
                u16,
                Option<Table>,
            )| {
                let (fg, _bg) = parse_colours(colours)?;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Line {
                        pane: pane.map(PaneId::from),
                        from: (x1, y1),
                        to: (x2, y2),
                        fg,
                    },
                );
                Ok(())
            },
        );

        methods.add_method(
            "marker",
            |_,
             this,
             (pane, x, y, text, colours): (
                Option<u32>,
                u16,
                u16,
                String,
                Option<Table>,
            )| {
                let (fg, _bg) = parse_colours(colours)?;
                this.buffers.record(
                    this.owner,
                    DrawCmd::Marker {
                        pane: pane.map(PaneId::from),
                        x,
                        y,
                        text,
                        fg,
                    },
                );
                Ok(())
            },
        );

        methods.add_method(
            "status",
            |_, this, (text, colours): (String, Option<Table>)| {
                let (fg, bg) = parse_colours(colours)?;
                this.buffers
                    .status(this.owner, StatusFragment::new(text, fg, bg));
                Ok(())
            },
        );
    }
}
