/// One read of a pane's text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    pub lines: Vec<String>,
    /// Whether each line runs on into the next, the same length as `lines` — or `None`
    /// where it was not asked for. `None` is "not asked", never "none of them wrap".
    pub wrapped: Option<Vec<bool>>,
    /// Each line as the runs it is drawn in, the same length as `lines` — or `None` where it
    /// was not asked for.
    pub runs: Option<Vec<Vec<crate::screen::Run>>>,
    /// The revision the read was taken at.
    pub revision: u64,
}
