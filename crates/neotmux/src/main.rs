mod app;
mod input;
mod logging;
mod render;
mod signals;

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use neotmux_lua::LuaSubsystem;
use neotmux_lua::ops::Level;
use neotmux_lua::registry::KindTag;
use neotmux_lua::runtime::Layers;

fn parse_log_flag(mut args: impl Iterator<Item = String>) -> Option<PathBuf> {
    while let Some(arg) = args.next() {
        if arg == "--log" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn log_silent_plugins(lua: &LuaSubsystem) {
    let mut registered: HashSet<_> = HashSet::new();
    for tag in [
        KindTag::Tick,
        KindTag::Event,
        KindTag::Keymap,
        KindTag::Status,
    ] {
        for id in lua.registry().snapshot(tag) {
            if let Some(owner) = lua.registry().owner(id) {
                registered.insert(owner);
            }
        }
    }
    for (id, name) in lua.plugins().interned() {
        if !registered.contains(&id) {
            logging::log(Level::Warn, &name, "registered nothing");
        }
    }
}

fn main() -> anyhow::Result<()> {
    let log_override = parse_log_flag(env::args().skip(1));
    if let Err(err) = logging::init(log_override.as_deref()) {
        eprintln!("neotmux: failed to initialise logging: {err:#}");
    }

    let exe_dir = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let layers = Layers::resolve(&|key| env::var_os(key), exe_dir.as_deref());
    let lua = LuaSubsystem::build(layers)?;
    for failure in lua.run_init() {
        logging::log(Level::Warn, &failure.plugin, &failure.message);
    }
    log_silent_plugins(&lua);

    let shutdown = signals::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, &shutdown, &lua);
    ratatui::restore();
    match result? {
        Some(signal) => signals::die_of(signal),
        None => Ok(()),
    }
}
