use mlua::RegistryKey;
use std::rc::Rc;

/// The callbacks a job was started with. Each is a `RegistryKey` because a Lua function
/// cannot outlive a borrow of the `Lua` it came from.
pub struct Callbacks {
    pub on_stdout: Option<Rc<RegistryKey>>,
    pub on_stderr: Option<Rc<RegistryKey>>,
    pub on_exit: Option<Rc<RegistryKey>>,
}
