use std::rc::Rc;

use mlua::RegistryKey;

/// The callbacks a job was started with. Each is a `RegistryKey` because a Lua function
/// cannot outlive a borrow of the `Lua` it came from.
pub struct Callbacks {
    pub on_stdout: Option<Rc<RegistryKey>>,
    pub on_stderr: Option<Rc<RegistryKey>>,
    pub on_exit: Option<Rc<RegistryKey>>,
}
