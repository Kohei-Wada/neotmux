use crate::wire::End;

/// The end of a frame: the picture that came of it, and whether that was the last one.
///
/// It is one message and not two because it is one moment: a picture and the end it shows are
/// read together or not at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done<'a> {
    /// `None` when nothing changed and the last frame still stands.
    pub frame: Option<&'a crate::Frame<'a>>,
    pub end: Option<End>,
}
