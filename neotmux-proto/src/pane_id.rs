use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(u32);

impl PaneId {
    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for PaneId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionId;
    use crate::WindowId;

    #[test]
    fn a_session_id_is_written_the_way_tmux_writes_one() {
        let id: SessionId = 1u32.into();
        assert_eq!(id.get(), 1);
        assert_eq!(id.to_string(), "$1");
    }

    #[test]
    fn round_trips_through_from_and_get() {
        let id: PaneId = 7u32.into();
        assert_eq!(id.get(), 7);
    }

    #[test]
    fn displays_with_percent_prefix() {
        let id = PaneId::from(3);
        assert_eq!(id.to_string(), "%3");
    }

    #[test]
    fn a_window_id_round_trips_and_displays_with_an_at_prefix() {
        let id = WindowId::from(2);
        assert_eq!(id.get(), 2);
        assert_eq!(id.to_string(), "@2");
    }
}
