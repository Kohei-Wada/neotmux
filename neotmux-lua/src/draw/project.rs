use neotmux_proto::screen::width;
use neotmux_proto::{Color, DrawCmd, Rect};

use crate::draw::{HlGroups, TextBuffer};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    fg: Color,
    bg: Color,
    bold: bool,
}

/// `buf` drawn into `rect` (surface coordinates): line 0 at the top row, each line cut at the
/// rect's width with a wide character that would straddle the edge left out, a
/// `line_hl_group` as a `Rect` under its row, and the text as runs split where the colour
/// changes. Later extmarks win per channel.
pub fn project(buf: &TextBuffer, rect: Rect, hl: &HlGroups) -> Vec<DrawCmd> {
    let mut out = Vec::new();
    for (row, line) in buf.lines.iter().enumerate().take(rect.h as usize) {
        let y = rect.y + row as u16;
        let marks: Vec<_> = buf.extmarks.iter().filter(|m| m.row == row).collect();
        let mut base = Cell {
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
        };
        for mark in &marks {
            if let Some(group) = mark.line_hl_group.as_deref().and_then(|name| hl.get(name)) {
                // A group `neotmux.style` published is one colour, carried as `fg`: as a row's
                // group it is that row's background.
                if let Some(bg) = group.bg.or(group.fg) {
                    base.bg = bg;
                    out.push(DrawCmd::Rect {
                        pane: None,
                        rect: Rect::new(rect.x, y, rect.w, 1),
                        fg: Color::Default,
                        bg,
                    });
                }
            }
        }
        let mut col = 0usize;
        let mut run = String::new();
        let mut run_at = 0usize;
        let mut run_cell = base;
        for ch in line.chars() {
            let w = width::of_char(ch);
            if col + w > rect.w as usize {
                break;
            }
            let mut cell = base;
            for mark in &marks {
                let Some(name) = mark.hl_group.as_deref() else {
                    continue;
                };
                let end = mark.end_col.unwrap_or(usize::MAX);
                if col < mark.col || col >= end {
                    continue;
                }
                if let Some(group) = hl.get(name) {
                    if let Some(fg) = group.fg {
                        cell.fg = fg;
                    }
                    if let Some(bg) = group.bg {
                        cell.bg = bg;
                    }
                    if let Some(bold) = group.bold {
                        cell.bold = bold;
                    }
                }
                if let Some(bold) = mark.bold {
                    cell.bold = bold;
                }
            }
            if cell != run_cell && !run.is_empty() {
                out.push(text(
                    rect.x + run_at as u16,
                    y,
                    std::mem::take(&mut run),
                    run_cell,
                ));
            }
            if run.is_empty() {
                run_at = col;
                run_cell = cell;
            }
            run.push(ch);
            col += w;
        }
        if !run.is_empty() {
            out.push(text(rect.x + run_at as u16, y, run, run_cell));
        }
    }
    out
}

fn text(x: u16, y: u16, text: String, cell: Cell) -> DrawCmd {
    DrawCmd::Text {
        pane: None,
        x,
        y,
        text,
        fg: cell.fg,
        bg: cell.bg,
        bold: cell.bold,
    }
}
