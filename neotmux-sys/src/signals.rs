use signal_hook::consts::SIGHUP;

/// What a terminal that has gone away is reported as, so a hangup neotmux notices itself
/// ends the session exactly the way the kernel's own SIGHUP does.
pub const HANGUP: i32 = SIGHUP;

pub fn die_of(signal: i32) -> anyhow::Result<()> {
    signal_hook::low_level::emulate_default_handler(signal)?;
    Ok(())
}
