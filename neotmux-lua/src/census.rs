use neotmux_proto::Reserved;

/// A count of everything the host holds for the Lua layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    /// Every registration's kind, `Debug`-printed and sorted: `Tick`, `Event("resized")`, …
    pub registrations: Vec<String>,
    pub commands: Vec<String>,
    pub reserved: Reserved,
    pub floats: usize,
    pub jobs: usize,
    pub timers: usize,
}
