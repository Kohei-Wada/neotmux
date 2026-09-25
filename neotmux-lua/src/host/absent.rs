use crate::host::Clipboard;

/// The one a subsystem built with [`crate::LuaSubsystem::build`] gets: a host that has no
/// clipboard at all, which answers so rather than claiming a write it did not make.
pub struct Absent;

impl Clipboard for Absent {
    fn set(&self, _text: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("this host has no clipboard"))
    }
}
