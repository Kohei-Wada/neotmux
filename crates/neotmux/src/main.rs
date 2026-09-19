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

fn parse_log_flag(args: impl Iterator<Item = String>) -> Option<PathBuf> {
    let mut args = args.peekable();
    let mut log_path = None;
    while let Some(arg) = args.next() {
        if arg == "--log" {
            match args.peek() {
                Some(value) if !value.starts_with('-') => {
                    log_path = Some(PathBuf::from(args.next().expect("peeked")));
                }
                _ => {
                    eprintln!(
                        "neotmux: --log requires a path argument; using the default log path"
                    );
                }
            }
        } else {
            eprintln!("neotmux: unrecognised argument '{arg}'; ignoring it");
        }
    }
    log_path
}

fn log_silent_plugins(lua: &LuaSubsystem, already_reported: &HashSet<String>) {
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
        if !registered.contains(&id) && !already_reported.contains(&name) {
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
    let mut failed_plugins = HashSet::new();
    for failure in lua.run_init() {
        logging::log(Level::Warn, &failure.plugin, &failure.message);
        failed_plugins.insert(failure.plugin);
    }
    log_silent_plugins(&lua, &failed_plugins);

    let shutdown = signals::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal, &shutdown, &lua);
    ratatui::restore();
    match result? {
        Some(signal) => signals::die_of(signal),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> impl Iterator<Item = String> + use<> {
        values
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn a_valid_log_flag_is_used() {
        assert_eq!(
            parse_log_flag(args(&["--log", "/tmp/x.log"])),
            Some(PathBuf::from("/tmp/x.log"))
        );
    }

    #[test]
    fn a_trailing_log_flag_with_no_value_falls_back_to_the_default() {
        assert_eq!(parse_log_flag(args(&["--log"])), None);
    }

    #[test]
    fn a_log_flag_followed_by_another_flag_falls_back_to_the_default() {
        assert_eq!(parse_log_flag(args(&["--log", "--foo"])), None);
    }

    #[test]
    fn an_unrecognised_argument_is_ignored() {
        assert_eq!(parse_log_flag(args(&["--bogus"])), None);
    }

    #[test]
    fn no_arguments_at_all_falls_back_to_the_default() {
        assert_eq!(parse_log_flag(args(&[])), None);
    }
}
