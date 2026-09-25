/// A group, as `neotmux.group` minted it. The name is the plugin's; the id is what the host
/// keys the group by, so a name is looked up once per call rather than once per registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupId(pub u64);
