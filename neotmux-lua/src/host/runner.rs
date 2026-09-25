//! What the host owes a plugin that wants to run a program.
//!
//! A contract rather than a `Command`, for the same reason [`crate::host::Clipboard`] is
//! one: this crate has no threads and no process of its own, and a test that had to spawn
//! something to check what a plugin does with its output would be testing the operating
//! system. The host implements it; a test hands over a script.

use crate::host::JobSpec;
use crate::host::Running;

/// Somewhere to start a program.
pub trait Runner {
    fn start(&self, spec: &JobSpec) -> std::io::Result<Box<dyn Running>>;
}
