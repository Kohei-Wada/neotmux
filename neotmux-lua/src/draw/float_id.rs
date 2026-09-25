/// A float's identity. Ids are never recycled, so the grant order is the id order.
/// The field is `pub(crate)` so the sibling modules name it by number (`__id` in Task 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FloatId(pub(crate) u32);
