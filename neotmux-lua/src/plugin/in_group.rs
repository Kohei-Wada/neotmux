use crate::plugin::{GroupId, Groups};

/// The span of one `g.*` call. Whatever is registered while it lives is the group's. It puts
/// back what was there before rather than clearing, so a `g.*` reached from inside another is
/// still attributed to the inner group and the outer one resumes after.
pub struct InGroup<'a> {
    pub(crate) groups: &'a Groups,
    pub(crate) was: Option<GroupId>,
}

impl Drop for InGroup<'_> {
    fn drop(&mut self) {
        self.groups.current.set(self.was);
    }
}
