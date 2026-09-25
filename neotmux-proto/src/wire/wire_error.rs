use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The peer was built against a different protocol. Distinct from every other error
    /// here, and carrying both numbers, because it is the one a person can act on.
    VersionMismatch { ours: u32, theirs: u32 },
    /// The bytes ran out mid-value.
    Truncated,
    /// The message decoded, and then there were bytes left over.
    Trailing,
    /// A value the format cannot mean: a bad discriminant, a bad `bool`, invalid UTF-8.
    Malformed(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionMismatch { ours, theirs } => write!(
                f,
                "protocol version mismatch (ours {ours}, theirs {theirs})"
            ),
            Self::Truncated => f.write_str("truncated message"),
            Self::Trailing => f.write_str("trailing bytes after the message"),
            Self::Malformed(what) => write!(f, "malformed message: {what}"),
        }
    }
}

impl std::error::Error for WireError {}
