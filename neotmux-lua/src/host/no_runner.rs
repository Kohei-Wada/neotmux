use crate::host::JobSpec;
use crate::host::Runner;
use crate::host::Running;

/// The one a subsystem built without a host gets: no processes here.
pub struct NoRunner;

impl Runner for NoRunner {
    fn start(&self, _spec: &JobSpec) -> std::io::Result<Box<dyn Running>> {
        Err(std::io::Error::other("this host cannot start processes"))
    }
}
