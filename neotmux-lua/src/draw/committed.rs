use neotmux_proto::{DrawCmd, Epoch};

#[derive(Debug, Clone, Default)]
pub struct Committed {
    pub epoch: Epoch,
    pub cmds: Vec<DrawCmd>,
}
