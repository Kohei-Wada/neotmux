//! What the host owes a plugin that wants to run a program.
//!
//! A contract rather than a `Command`, for the same reason [`crate::host::Clipboard`] is
//! one: this crate has no threads and no process of its own, and a test that had to spawn
//! something to check what a plugin does with its output would be testing the operating
//! system. The host implements it; a test hands over a script.

/// What to run. The command is a list because a shell is the caller's to ask for by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    /// Added to whatever the host already has, never a replacement: a plugin cannot read the
    /// environment, so it has no way to compose a whole one.
    pub env: Vec<(String, String)>,
}

/// One thing a running job has said. Bytes, not lines: where the line breaks fall is
/// [`crate::jobs`]'s business, and a chunk boundary is not a line boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobChunk {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// The program exited on its own, with this status.
    Exited(i32),
    /// It never got that far: the pipe broke, the child could not be waited for. One line.
    Failed(String),
}

/// A job that has started. Dropping it must not leave a process behind.
pub trait Running {
    /// Whatever has arrived since the last call, and nothing else. Never blocks: this is
    /// called from inside a frame, once per job, and a frame that waited on a child would be
    /// a frame every session waits for.
    fn poll(&mut self) -> Vec<JobChunk>;

    /// Ends it now, without waiting. Idempotent — the caller may already have seen
    /// [`JobChunk::Exited`].
    fn kill(&mut self);
}

/// Somewhere to start a program.
pub trait Runner {
    fn start(&self, spec: &JobSpec) -> std::io::Result<Box<dyn Running>>;
}

/// The one a subsystem built without a host gets: no processes here.
pub struct NoRunner;

impl Runner for NoRunner {
    fn start(&self, _spec: &JobSpec) -> std::io::Result<Box<dyn Running>> {
        Err(std::io::Error::other("this host cannot start processes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
