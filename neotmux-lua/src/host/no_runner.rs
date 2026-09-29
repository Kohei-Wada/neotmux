use crate::host::{JobSpec, Runner, Running};

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
