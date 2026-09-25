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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::NoRunner;

    /// A host with no way to start a process says so rather than pretending to have started
    /// one, the same way `clipboard::Absent` answers.
    #[test]
    fn a_host_with_no_runner_refuses_to_start_anything() {
        let spec = JobSpec {
            cmd: vec!["true".to_string()],
            cwd: None,
            env: Vec::new(),
        };
        // Not `expect_err`: that needs `T: Debug`, and a trait object standing for a running
        // process has no reason to be one.
        let err = match NoRunner.start(&spec) {
            Ok(_) => panic!("NoRunner started something"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("cannot start"), "{err}");
    }
}
