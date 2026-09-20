use neotmux_core::proto::{Color, GridSnapshot, Size};

use crate::lines::Trailing;
use crate::read::History;

pub struct TermState {
    parser: vt100::Parser,
    /// Lines that have fallen out of the front of the history, counted as they go: nothing
    /// downstream can work it out afterwards. See [`crate::evictions`].
    dropped: usize,
}

fn color_from_vt100(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Idx(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// The smallest grid `vt100` survives arbitrary output on. Measured against vt100 0.16.2:
/// one row underflows in `grid::col_wrap` the first time a line wraps, and one column
/// underflows in `screen::write_grapheme` the first time a wide character is written.
pub const MIN_GRID: Size = Size { rows: 2, cols: 2 };

/// The one place a size is clamped before it reaches `vt100` or the PTY.
pub fn clamp_to_min_grid(size: Size) -> Size {
    Size {
        rows: size.rows.max(MIN_GRID.rows),
        cols: size.cols.max(MIN_GRID.cols),
    }
}

impl TermState {
    pub fn new(size: Size, scrollback: usize) -> Self {
        let size = clamp_to_min_grid(size);
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, scrollback),
            dropped: 0,
        }
    }

    /// Feeds one chunk to the terminal, counting the history lines it pushed out of the
    /// front on the way through — the offset is pinned off zero so `vt100` counts the
    /// appends, and put back before anything else can read it ([`crate::evictions`]).
    pub fn feed(&mut self, bytes: &[u8]) {
        let screen = self.parser.screen_mut();
        let len_before = crate::scrollback::len(screen);
        let resting = screen.scrollback();
        screen.set_scrollback(resting.max(1));
        let offset_before = screen.scrollback();

        self.parser.process(bytes);

        let screen = self.parser.screen_mut();
        let offset_after = screen.scrollback();
        screen.set_scrollback(resting);
        let len_after = crate::scrollback::len(screen);
        self.dropped += crate::evictions::dropped(crate::evictions::Feed {
            len_before,
            len_after,
            offset_before,
            offset_after,
        });
    }

    pub fn resize(&mut self, size: Size) {
        let size = clamp_to_min_grid(size);
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }

    /// Where the cursor is, zero-based, which is what a cursor report is asked for.
    pub fn cursor(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// What the pane is showing, as plain-text lines (`crate::lines`). `last` bounds the work
    /// to the bottom of the screen rather than trimming an answer that was built in full.
    pub fn lines(&self, last: Option<usize>, trailing: Trailing) -> Vec<String> {
        crate::lines::visible(self.parser.screen(), last, trailing)
    }

    /// Whether each of those lines runs on into the next (`crate::wrap`).
    pub fn wrapped(&self, last: Option<usize>) -> Vec<bool> {
        crate::wrap::visible(self.parser.screen(), last)
    }

    /// How many lines the pane holds above its visible screen, and how many have fallen out
    /// of the front of it. `&mut` because reading the length goes through the screen's own
    /// position in the scrollback — see [`crate::scrollback`].
    pub fn history_extent(&mut self) -> History {
        History {
            len: crate::scrollback::len(self.parser.screen_mut()),
            dropped: self.dropped,
        }
    }

    /// Lines from above the visible screen, oldest first, numbered as
    /// [`crate::scrollback`] describes.
    pub fn history(&mut self, from: usize, count: usize, trailing: Trailing) -> Vec<String> {
        crate::scrollback::lines(self.parser.screen_mut(), from, count, trailing)
    }

    /// Whether each of those history lines runs on into the next (`crate::wrap`).
    pub fn history_wrapped(&mut self, from: usize, count: usize) -> Vec<bool> {
        crate::wrap::history(self.parser.screen_mut(), from, count)
    }

    pub fn size(&self) -> Size {
        let (rows, cols) = self.parser.screen().size();
        Size { rows, cols }
    }

    pub fn snapshot_into(&self, out: &mut GridSnapshot) {
        let screen = self.parser.screen();
        let size = self.size();
        out.resize(size);
        out.cursor = screen.cursor_position();
        out.hide_cursor = screen.hide_cursor();
        for row in 0..size.rows {
            for col in 0..size.cols {
                let Some(src) = screen.cell(row, col) else {
                    continue;
                };
                let dst = out.cell_mut(row, col).expect("in bounds");
                dst.text.push_str(src.contents());
                dst.fg = color_from_vt100(src.fgcolor());
                dst.bg = color_from_vt100(src.bgcolor());
                dst.bold = src.bold();
                dst.italic = src.italic();
                dst.underline = src.underline();
                dst.inverse = src.inverse();
                dst.wide = src.is_wide();
                dst.wide_continuation = src.is_wide_continuation();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: &TermState) -> GridSnapshot {
        let mut out = GridSnapshot::default();
        state.snapshot_into(&mut out);
        out
    }

    #[test]
    fn plain_text_lands_in_the_grid() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.feed(b"hi");
        let g = snap(&s);
        assert_eq!(g.cell(0, 0).unwrap().text, "h");
        assert_eq!(g.cell(0, 1).unwrap().text, "i");
        assert_eq!(g.cursor, (0, 2));
    }

    #[test]
    fn sgr_bold_and_indexed_colour_are_carried() {
        let mut s = TermState::new(Size { rows: 1, cols: 4 }, 0);
        s.feed(b"\x1b[1;31mR\x1b[0m");
        let c = snap(&s);
        let cell = c.cell(0, 0).unwrap();
        assert_eq!(cell.text, "R");
        assert!(cell.bold);
        assert_eq!(cell.fg, Color::Idx(1));
    }

    #[test]
    fn truecolour_is_carried() {
        let mut s = TermState::new(Size { rows: 1, cols: 2 }, 0);
        s.feed(b"\x1b[38;2;10;20;30mZ");
        assert_eq!(snap(&s).cell(0, 0).unwrap().fg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn cursor_positioning_and_erase_work() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.feed(b"abcd\x1b[1;1H\x1b[K");
        let g = snap(&s);
        assert_eq!(g.cell(0, 0).unwrap().text, "");
        assert_eq!(g.cursor, (0, 0));
    }

    #[test]
    fn resize_is_reflected_in_the_snapshot() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.resize(Size { rows: 3, cols: 6 });
        assert_eq!(s.size(), Size { rows: 3, cols: 6 });
        assert_eq!(snap(&s).size, Size { rows: 3, cols: 6 });
    }

    #[test]
    fn new_clamps_a_zero_size_up_to_the_minimum_grid() {
        let s = TermState::new(Size { rows: 0, cols: 0 }, 0);
        assert_eq!(s.size(), MIN_GRID);
    }

    #[test]
    fn resize_clamps_a_zero_size_up_to_the_minimum_grid() {
        let mut s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        s.resize(Size { rows: 0, cols: 0 });
        assert_eq!(s.size(), MIN_GRID);
    }

    /// Scrolling shell output, a wrapped line and a wide character are what a real
    /// shell produces immediately; below `MIN_GRID` each of them overflows inside vt100.
    fn feed_scrolling_output(mut s: TermState) {
        s.feed(b"one\r\ntwo\r\nthree\r\nfour\r\n");
        s.feed(b"a long unbroken line that has to wrap several times over");
        s.feed("\u{3042}\u{3042}".as_bytes());
    }

    #[test]
    fn scrolling_output_survives_at_the_minimum_grid() {
        feed_scrolling_output(TermState::new(MIN_GRID, 100));
    }

    #[test]
    fn a_zero_by_zero_terminal_survives_scrolling_output() {
        feed_scrolling_output(TermState::new(Size { rows: 0, cols: 0 }, 100));
    }

    #[test]
    fn a_one_by_one_terminal_survives_scrolling_output() {
        feed_scrolling_output(TermState::new(Size { rows: 1, cols: 1 }, 100));
    }

    #[test]
    fn resizing_below_the_minimum_grid_survives_scrolling_output() {
        let mut s = TermState::new(Size { rows: 24, cols: 80 }, 100);
        s.resize(Size { rows: 1, cols: 1 });
        feed_scrolling_output(s);
    }

    /// A pane two rows tall whose ring holds four lines: the smallest thing that both keeps
    /// a history and runs out of it.
    fn ringed() -> TermState {
        TermState::new(Size { rows: 2, cols: 8 }, 4)
    }

    /// A pane whose ring has room loses nothing, so every line it ever scrolled away is
    /// still numbered from zero.
    #[test]
    fn a_history_with_room_left_has_dropped_nothing() {
        let mut s = ringed();
        s.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        assert_eq!(s.history_extent(), History { len: 3, dropped: 0 });
    }

    /// The number the whole thing exists for: the history stops growing and nothing else
    /// says that every number in it has moved.
    #[test]
    fn a_full_ring_counts_the_lines_it_pushed_out_of_the_front() {
        let mut s = ringed();
        s.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        assert_eq!(s.history_extent(), History { len: 3, dropped: 0 });
        s.feed(b"\r\n6\r\n7\r\n8");
        assert_eq!(
            s.history_extent(),
            History { len: 4, dropped: 2 },
            "the history stopped growing and nothing said the numbers had moved"
        );
        assert_eq!(
            s.history(0, 10, Trailing::Trim),
            vec!["3", "4", "5", "6"],
            "the two oldest lines are the ones that went"
        );
    }

    /// `dropped + index` is the point: an anchor taken on one line keeps naming that line
    /// for as long as the pane holds it.
    #[test]
    fn dropped_plus_an_index_names_the_same_line_across_an_eviction() {
        let mut s = ringed();
        s.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        let before = s.history_extent();
        assert_eq!(s.history(1, 1, Trailing::Trim), vec!["2"]);
        let anchor = before.dropped + 1;

        s.feed(b"\r\n6\r\n7");
        let after = s.history_extent();
        assert_eq!(after.dropped, 1);
        assert_eq!(
            s.history(anchor - after.dropped, 1, Trailing::Trim),
            vec!["2"],
            "the anchor stopped naming the line it was taken on"
        );
    }

    /// An anchor the pane no longer holds is what a plugin has to be able to see, and
    /// `dropped` past it is the whole of the signal.
    #[test]
    fn an_anchor_that_fell_out_of_the_ring_is_visible_as_one() {
        let mut s = ringed();
        s.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        let anchor = s.history_extent().dropped;
        s.feed(b"\r\n6\r\n7");
        assert!(
            anchor < s.history_extent().dropped,
            "the anchor still looks live"
        );
    }

    /// Counting has to survive a feed split anywhere, because a chunk is whatever the reader
    /// thread happened to read.
    #[test]
    fn the_count_does_not_depend_on_where_the_chunks_were_cut() {
        let second = b"\r\n6\r\n7\r\n8";
        let mut whole = ringed();
        whole.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        whole.feed(second);
        for split in 1..second.len() {
            let mut cut = ringed();
            cut.feed(b"1\r\n2\r\n3\r\n4\r\n5");
            cut.feed(&second[..split]);
            cut.feed(&second[split..]);
            assert_eq!(
                cut.history_extent(),
                whole.history_extent(),
                "cutting the feed at {split} changed the count"
            );
        }
    }

    /// The documented limit, pinned rather than hidden: one feed that appends as many lines
    /// as the ring holds saturates the offset the count is read from, and what is reported
    /// from then on is low. Three lines went out of a ring of two; one is what comes back.
    #[test]
    fn a_feed_that_appends_a_whole_ring_undercounts() {
        let mut s = TermState::new(Size { rows: 2, cols: 8 }, 2);
        s.feed(b"1\r\n2\r\n3\r\n4");
        assert_eq!(s.history_extent(), History { len: 2, dropped: 0 });
        s.feed(b"\r\n5\r\n6\r\n7");
        assert_eq!(s.history(0, 10, Trailing::Trim), vec!["4", "5"]);
        assert_eq!(
            s.history_extent(),
            History { len: 2, dropped: 1 },
            "three lines went; this is the undercount the module documents"
        );
    }

    /// The alternate screen keeps no history of its own, so the pane's history goes out of
    /// view and comes back. None of it was evicted.
    #[test]
    fn the_alternate_screen_does_not_look_like_an_eviction() {
        let mut s = TermState::new(Size { rows: 2, cols: 8 }, 100);
        s.feed(b"1\r\n2\r\n3\r\n4");
        let before = s.history_extent();
        s.feed(b"\x1b[?1049h");
        s.feed(b"a\r\nb\r\nc\r\nd");
        s.feed(b"\x1b[?1049l");
        assert_eq!(s.history_extent(), before);
    }

    /// Feeding must leave the pane showing what it was showing: the offset is pinned for the
    /// duration of the feed and put back.
    #[test]
    fn counting_does_not_leave_the_pane_scrolled_back() {
        let mut s = TermState::new(Size { rows: 2, cols: 8 }, 100);
        s.feed(b"1\r\n2\r\n3\r\n4");
        assert_eq!(s.lines(None, Trailing::Trim), vec!["3", "4"]);
        assert_eq!(snap(&s).cell(0, 0).unwrap().text, "3");
    }

    #[test]
    fn a_panes_history_is_what_scrolled_off_its_screen() {
        let mut s = TermState::new(Size { rows: 2, cols: 8 }, 100);
        s.feed(b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(s.history_extent().len, 2);
        assert_eq!(s.history(0, 10, Trailing::Trim), vec!["one", "two"]);
        assert_eq!(s.lines(None, Trailing::Trim), vec!["three", "four"]);
    }

    #[test]
    fn snapshot_into_reuses_the_buffer() {
        let s = TermState::new(Size { rows: 2, cols: 4 }, 0);
        let mut out = GridSnapshot::default();
        s.snapshot_into(&mut out);
        let ptr = out.cells.as_ptr();
        s.snapshot_into(&mut out);
        assert_eq!(out.cells.as_ptr(), ptr);
    }
}
