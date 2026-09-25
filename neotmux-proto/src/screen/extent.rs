/// Where a pane's history begins, and how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extent {
    /// How many lines the pane holds above its visible screen.
    pub len: u64,
    /// How many lines have fallen out of the front of it since the pane opened, so history
    /// line `i` is the `dropped + i`th line the pane ever scrolled away. A single feed that
    /// appends a whole ring's worth of lines in one go can undercount this (pinned by
    /// `a_feed_that_appends_a_whole_ring_undercounts` in `neotmux-pty`), so a plugin must
    /// not treat it as exact.
    pub dropped: u64,
}
