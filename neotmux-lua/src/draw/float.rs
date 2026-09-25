use std::cell::RefCell;
use std::collections::BTreeMap;

use neotmux_proto::{Color, DrawCmd, Rect};

use crate::plugin::SourceId;

/// A float's identity. Ids are never recycled, so the grant order is the id order.
/// The field is `pub(crate)` so the sibling modules name it by number (`__id` in Task 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FloatId(pub(crate) u32);

/// Who has been granted a floating window, and where each one currently sits.
#[derive(Default)]
pub struct Floats {
    entries: RefCell<BTreeMap<FloatId, (SourceId, Rect)>>,
    next: RefCell<u32>,
}

impl Floats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, owner: SourceId, rect: Rect) -> FloatId {
        let mut next = self.next.borrow_mut();
        *next += 1;
        let id = FloatId(*next);
        drop(next);
        self.entries.borrow_mut().insert(id, (owner, rect));
        id
    }

    pub fn rect(&self, id: FloatId) -> Option<Rect> {
        self.entries.borrow().get(&id).map(|(_, rect)| *rect)
    }

    pub fn move_to(&self, id: FloatId, x: u16, y: u16) {
        if let Some((_, rect)) = self.entries.borrow_mut().get_mut(&id) {
            rect.x = x;
            rect.y = y;
        }
    }

    /// Keeps x/y where they are; only the size changes.
    pub fn resize(&self, id: FloatId, w: u16, h: u16) {
        if let Some((_, rect)) = self.entries.borrow_mut().get_mut(&id) {
            rect.w = w;
            rect.h = h;
        }
    }

    pub fn remove(&self, id: FloatId) {
        self.entries.borrow_mut().remove(&id);
    }

    pub fn by_owner(&self, owner: SourceId) -> BTreeMap<FloatId, Rect> {
        self.entries
            .borrow()
            .iter()
            .filter(|(_, (o, _))| *o == owner)
            .map(|(id, (_, rect))| (*id, *rect))
            .collect()
    }

    /// Every owner holding a live float, sorted by the largest float id it holds,
    /// descending — the keyboard order (spec §5).
    pub fn owners_topmost_first(&self) -> Vec<SourceId> {
        let entries = self.entries.borrow();
        let mut latest: BTreeMap<SourceId, FloatId> = BTreeMap::new();
        for (id, (owner, _)) in entries.iter() {
            latest
                .entry(*owner)
                .and_modify(|max| {
                    if *id > *max {
                        *max = *id;
                    }
                })
                .or_insert(*id);
        }
        let mut by_latest: Vec<(FloatId, SourceId)> =
            latest.into_iter().map(|(owner, id)| (id, owner)).collect();
        by_latest.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        by_latest.into_iter().map(|(_, owner)| owner).collect()
    }

    /// The load-failure and unload reap (reserve's convention).
    pub fn remove_all_of(&self, owner: SourceId) {
        self.entries.borrow_mut().retain(|_, (o, _)| *o != owner);
    }

    pub fn count(&self) -> usize {
        self.entries.borrow().len()
    }
}

/// What one float-local draw becomes at surface coordinates: clipped to the float's rect
/// and offset by its origin. Surface clipping is free — the renderer already clips
/// `pane: None` commands to the surface — so only the float-local clip happens here, and
/// every emitted command is `pane: None` at surface coordinates. A dropped draw returns an
/// empty vec. A partially visible line comes back as the runs of its steps that stayed
/// inside.
pub fn place(cmd: &DrawCmd, float: &Rect) -> Vec<DrawCmd> {
    match cmd {
        DrawCmd::Text {
            x, y, text, fg, bg, ..
        } => clip_text(*x, *y, text, *fg, *bg, float),
        DrawCmd::Rect { rect, fg, bg, .. } => clip_rect(rect, *fg, *bg, float),
        DrawCmd::Tint { rect, fg, bg, .. } => clip_tint(rect, *fg, *bg, float),
        DrawCmd::Line {
            from,
            to,
            glyph,
            fg,
            ..
        } => clip_line(from, to, *glyph, *fg, float),
        // Non-exhaustive: an unknown future variant has no local meaning inside a float.
        _ => Vec::new(),
    }
}

/// Same truncation rule as the renderer's `overlay.rs::draw_text`, because both call
/// `neotmux_proto::screen::of_char`: a zero-width (combining) char rides the previous cell rather than
/// being measured, and a wide char that would straddle the cap drops as a whole rather than
/// being half-drawn.
fn clip_text(x: u16, y: u16, text: &str, fg: Color, bg: Color, float: &Rect) -> Vec<DrawCmd> {
    if x >= float.w || y >= float.h {
        return Vec::new();
    }
    let cap = float.w - x;
    let mut acc: u16 = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let width = neotmux_proto::screen::of_char(ch) as u16;
        if width == 0 {
            out.push(ch);
            continue;
        }
        if acc.saturating_add(width) > cap {
            break;
        }
        out.push(ch);
        acc += width;
    }
    if out.is_empty() {
        return Vec::new();
    }
    vec![DrawCmd::Text {
        pane: None,
        x: float.x + x,
        y: float.y + y,
        text: out,
        fg,
        bg,
    }]
}

/// The intersection of `rect` (in float-local coordinates) with the float's own bounds,
/// offset into surface coordinates — or `None` when `rect`'s origin already falls outside
/// the float.
fn clip_and_offset(rect: &Rect, float: &Rect) -> Option<Rect> {
    if rect.x >= float.w || rect.y >= float.h {
        return None;
    }
    let x1 = rect.x.saturating_add(rect.w).min(float.w);
    let y1 = rect.y.saturating_add(rect.h).min(float.h);
    Some(Rect::new(
        float.x + rect.x,
        float.y + rect.y,
        x1 - rect.x,
        y1 - rect.y,
    ))
}

fn clip_rect(rect: &Rect, fg: Color, bg: Color, float: &Rect) -> Vec<DrawCmd> {
    match clip_and_offset(rect, float) {
        Some(rect) => vec![DrawCmd::Rect {
            pane: None,
            rect,
            fg,
            bg,
        }],
        None => Vec::new(),
    }
}

fn clip_tint(rect: &Rect, fg: Option<Color>, bg: Option<Color>, float: &Rect) -> Vec<DrawCmd> {
    match clip_and_offset(rect, float) {
        Some(rect) => vec![DrawCmd::Tint {
            pane: None,
            rect,
            fg,
            bg,
        }],
        None => Vec::new(),
    }
}

/// A line is convex, so both endpoints inside means the whole line is inside: offset and
/// keep it as one `Line`. Otherwise walk the identical Bresenham stepping the renderer
/// walks (`overlay.rs::draw_line`: same `dx`/`dy`/`sx`/`sy`/`err` loop), and emit one
/// single-glyph `Text` per step that lands inside the float — the exact cell the
/// renderer's `draw_char` would paint (glyph + fg, bg `Default`).
fn clip_line(
    from: &(u16, u16),
    to: &(u16, u16),
    glyph: char,
    fg: Color,
    float: &Rect,
) -> Vec<DrawCmd> {
    let inside = |p: (u16, u16)| p.0 < float.w && p.1 < float.h;
    if inside(*from) && inside(*to) {
        return vec![DrawCmd::Line {
            pane: None,
            from: (float.x + from.0, float.y + from.1),
            to: (float.x + to.0, float.y + to.1),
            glyph,
            fg,
        }];
    }
    let (x0, y0) = (i32::from(from.0), i32::from(from.1));
    let (x1, y1) = (i32::from(to.0), i32::from(to.1));
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    let mut out = Vec::new();
    loop {
        if x >= 0 && y >= 0 && (x as u16) < float.w && (y as u16) < float.h {
            out.push(DrawCmd::Text {
                pane: None,
                x: float.x + x as u16,
                y: float.y + y as u16,
                text: glyph.to_string(),
                fg,
                bg: Color::Default,
            });
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Sources;

    fn sources() -> (Sources, SourceId, SourceId) {
        let sources = Sources::new();
        let a = sources.intern("@a.lua");
        let b = sources.intern("@b.lua");
        (sources, a, b)
    }

    #[test]
    fn grant_order_is_id_order() {
        let (_s, a, b) = sources();
        let floats = Floats::new();
        let first = floats.add(a, Rect::new(0, 0, 1, 1));
        let second = floats.add(b, Rect::new(0, 0, 1, 1));
        assert!(first < second);
        assert_eq!(first, FloatId(1));
        assert_eq!(second, FloatId(2));
    }

    #[test]
    fn remove_is_idempotent_and_never_recycles_ids() {
        let (_s, a, _b) = sources();
        let floats = Floats::new();
        let first = floats.add(a, Rect::new(0, 0, 1, 1));
        floats.remove(first);
        floats.remove(first);
        assert_eq!(floats.count(), 0);
        let second = floats.add(a, Rect::new(0, 0, 1, 1));
        assert_eq!(
            second,
            FloatId(2),
            "the removed id is never handed out again"
        );
    }

    #[test]
    fn move_to_is_a_no_op_for_a_missing_id() {
        let (_s, a, _b) = sources();
        let floats = Floats::new();
        let id = floats.add(a, Rect::new(0, 0, 1, 1));
        floats.remove(id);
        floats.move_to(id, 5, 5);
        assert_eq!(floats.rect(id), None);
    }

    #[test]
    fn resize_is_a_no_op_for_a_missing_id() {
        let (_s, a, _b) = sources();
        let floats = Floats::new();
        let id = floats.add(a, Rect::new(0, 0, 1, 1));
        floats.remove(id);
        floats.resize(id, 5, 5);
        assert_eq!(floats.rect(id), None);
    }

    #[test]
    fn resize_keeps_the_origin() {
        let (_s, a, _b) = sources();
        let floats = Floats::new();
        let id = floats.add(a, Rect::new(2, 3, 4, 5));
        floats.resize(id, 10, 20);
        assert_eq!(floats.rect(id), Some(Rect::new(2, 3, 10, 20)));
    }

    #[test]
    fn move_to_keeps_the_size() {
        let (_s, a, _b) = sources();
        let floats = Floats::new();
        let id = floats.add(a, Rect::new(2, 3, 4, 5));
        floats.move_to(id, 9, 8);
        assert_eq!(floats.rect(id), Some(Rect::new(9, 8, 4, 5)));
    }

    #[test]
    fn by_owner_lists_only_that_owners_floats() {
        let (_s, a, b) = sources();
        let floats = Floats::new();
        let a1 = floats.add(a, Rect::new(0, 0, 1, 1));
        let _b1 = floats.add(b, Rect::new(0, 0, 2, 2));
        let a2 = floats.add(a, Rect::new(1, 1, 3, 3));
        let owned = floats.by_owner(a);
        assert_eq!(owned.len(), 2);
        assert_eq!(owned.get(&a1), Some(&Rect::new(0, 0, 1, 1)));
        assert_eq!(owned.get(&a2), Some(&Rect::new(1, 1, 3, 3)));
    }

    #[test]
    fn remove_all_of_empties_the_owners_floats_leaving_others_untouched() {
        let (_s, a, b) = sources();
        let floats = Floats::new();
        floats.add(a, Rect::new(0, 0, 1, 1));
        floats.add(a, Rect::new(0, 0, 1, 1));
        let b1 = floats.add(b, Rect::new(0, 0, 1, 1));
        floats.remove_all_of(a);
        assert!(floats.by_owner(a).is_empty());
        assert_eq!(floats.rect(b1), Some(Rect::new(0, 0, 1, 1)));
        assert_eq!(floats.count(), 1);
    }

    #[test]
    fn owners_topmost_first_orders_by_the_largest_id_each_holds() {
        let (_s, a, b) = sources();
        let floats = Floats::new();
        floats.add(a, Rect::new(0, 0, 1, 1)); // 1
        floats.add(a, Rect::new(0, 0, 1, 1)); // 2
        floats.add(b, Rect::new(0, 0, 1, 1)); // 3
        // a's latest (2) is older than b's only float (3), so b sorts first even though
        // a has held a float for longer.
        assert_eq!(floats.owners_topmost_first(), vec![b, a]);
    }

    #[test]
    fn an_owner_whose_latest_float_is_newer_than_anothers_whole_history_sorts_first() {
        let (_s, a, b) = sources();
        let floats = Floats::new();
        floats.add(a, Rect::new(0, 0, 1, 1)); // 1
        floats.add(a, Rect::new(0, 0, 1, 1)); // 2
        floats.add(a, Rect::new(0, 0, 1, 1)); // 3
        floats.add(b, Rect::new(0, 0, 1, 1)); // 4, newer than any of a's
        assert_eq!(floats.owners_topmost_first(), vec![b, a]);
    }

    fn float() -> Rect {
        Rect::new(3, 2, 5, 3)
    }

    fn text(x: u16, y: u16, text: &str) -> DrawCmd {
        DrawCmd::Text {
            pane: None,
            x,
            y,
            text: text.to_string(),
            fg: Color::Default,
            bg: Color::Default,
        }
    }

    #[test]
    fn text_fully_inside_is_offset_and_unchanged() {
        let placed = place(&text(1, 1, "hi"), &float());
        assert_eq!(
            placed,
            vec![DrawCmd::Text {
                pane: None,
                x: 4,
                y: 3,
                text: "hi".to_string(),
                fg: Color::Default,
                bg: Color::Default,
            }]
        );
    }

    #[test]
    fn text_straddling_the_right_edge_is_truncated_at_the_width_cap() {
        // float is 5 wide, starting at x=1 leaves a cap of 4.
        let placed = place(&text(1, 0, "hello"), &float());
        assert_eq!(
            placed,
            vec![DrawCmd::Text {
                pane: None,
                x: 4,
                y: 2,
                text: "hell".to_string(),
                fg: Color::Default,
                bg: Color::Default,
            }]
        );
    }

    #[test]
    fn a_wide_char_that_would_straddle_the_cap_drops_whole() {
        // float is 5 wide, starting at x=3 leaves a cap of 2: "A" fits (1 col), the
        // 2-column char after it does not (would need col 3 of a 2-col cap).
        let placed = place(&text(3, 0, "A\u{5e45}"), &float());
        assert_eq!(
            placed,
            vec![DrawCmd::Text {
                pane: None,
                x: 6,
                y: 2,
                text: "A".to_string(),
                fg: Color::Default,
                bg: Color::Default,
            }]
        );
    }

    #[test]
    fn text_starting_at_x_equal_w_is_empty() {
        assert_eq!(place(&text(5, 0, "x"), &float()), Vec::new());
    }

    /// An empty text is an empty intersection: it places to nothing, because a `Text`
    /// command carrying no characters is a command the renderer would have nothing to paint.
    #[test]
    fn an_empty_text_places_to_nothing() {
        let placed = place(&text(0, 0, ""), &float());
        assert!(
            placed.is_empty(),
            "a text with an empty intersection must place to zero commands, not a command \\n         carrying nothing: {placed:?}"
        );
    }

    #[test]
    fn text_starting_at_y_equal_h_is_empty() {
        assert_eq!(place(&text(0, 3, "x"), &float()), Vec::new());
    }

    fn rect_cmd(rect: Rect) -> DrawCmd {
        DrawCmd::Rect {
            pane: None,
            rect,
            fg: Color::Default,
            bg: Color::Idx(2),
        }
    }

    #[test]
    fn a_rect_partially_outside_is_clipped_and_offset() {
        let placed = place(&rect_cmd(Rect::new(3, 1, 10, 10)), &float());
        assert_eq!(
            placed,
            vec![DrawCmd::Rect {
                pane: None,
                rect: Rect::new(6, 3, 2, 2),
                fg: Color::Default,
                bg: Color::Idx(2),
            }]
        );
    }

    #[test]
    fn a_rect_fully_outside_is_empty() {
        assert_eq!(
            place(&rect_cmd(Rect::new(5, 0, 1, 1)), &float()),
            Vec::new()
        );
    }

    fn tint_cmd(rect: Rect) -> DrawCmd {
        DrawCmd::Tint {
            pane: None,
            rect,
            fg: None,
            bg: Some(Color::Idx(4)),
        }
    }

    #[test]
    fn a_tint_is_clipped_like_a_rect() {
        let placed = place(&tint_cmd(Rect::new(3, 1, 10, 10)), &float());
        assert_eq!(
            placed,
            vec![DrawCmd::Tint {
                pane: None,
                rect: Rect::new(6, 3, 2, 2),
                fg: None,
                bg: Some(Color::Idx(4)),
            }]
        );
    }

    fn line_cmd(from: (u16, u16), to: (u16, u16)) -> DrawCmd {
        DrawCmd::Line {
            pane: None,
            from,
            to,
            glyph: '#',
            fg: Color::Default,
        }
    }

    #[test]
    fn a_line_with_both_endpoints_inside_is_one_offset_line() {
        let placed = place(&line_cmd((0, 0), (2, 0)), &float());
        assert_eq!(
            placed,
            vec![DrawCmd::Line {
                pane: None,
                from: (3, 2),
                to: (5, 2),
                glyph: '#',
                fg: Color::Default,
            }]
        );
    }

    #[test]
    fn a_line_partially_outside_becomes_per_step_text_at_offset_coordinates() {
        // float is 5x3; a horizontal line from x=2 to x=6 at y=0 runs off the right edge.
        let placed = place(&line_cmd((2, 0), (6, 0)), &float());
        assert_eq!(
            placed,
            vec![
                DrawCmd::Text {
                    pane: None,
                    x: 5,
                    y: 2,
                    text: "#".to_string(),
                    fg: Color::Default,
                    bg: Color::Default,
                },
                DrawCmd::Text {
                    pane: None,
                    x: 6,
                    y: 2,
                    text: "#".to_string(),
                    fg: Color::Default,
                    bg: Color::Default,
                },
                DrawCmd::Text {
                    pane: None,
                    x: 7,
                    y: 2,
                    text: "#".to_string(),
                    fg: Color::Default,
                    bg: Color::Default,
                },
            ],
            "only the steps that stayed inside the float remain"
        );
    }

    /// A `from == to` line is the zero-length limit of the same line: exactly one step, one
    /// paint at `from` — neither zero (a line is a paint, like every other command) nor two
    /// (there is no second step to take). Inside the float the convex fast path keeps it a
    /// single `Line` sitting on that one cell.
    #[test]
    fn a_line_with_from_equal_to_yields_exactly_one_step() {
        // (3, 1) is inside the float, so the fast path applies; the one step is the cell at
        // `from`, offset into surface coordinates.
        let placed = place(&line_cmd((3, 1), (3, 1)), &float());
        assert_eq!(
            placed.len(),
            1,
            "a zero-length line is one step, not zero and not two: {placed:?}"
        );
        assert_eq!(
            placed[0],
            DrawCmd::Line {
                pane: None,
                from: (6, 3),
                to: (6, 3),
                glyph: '#',
                fg: Color::Default,
            },
            "the one step paints exactly the cell at `from`, offset: {placed:?}"
        );
    }
}
