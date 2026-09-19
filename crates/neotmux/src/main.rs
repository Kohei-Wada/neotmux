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

struct LogFlag {
    path: Option<PathBuf>,
    warnings: Vec<String>,
}

fn parse_log_flag(mut args: impl Iterator<Item = String>) -> LogFlag {
    let mut path = None;
    let mut warnings = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--log" {
            match args.next() {
                Some(value) if !value.starts_with('-') => {
                    path = Some(PathBuf::from(value));
                }
                Some(value) => warnings.push(format!(
                    "--log requires a path argument, got '{value}'; using the default log path"
                )),
                None => warnings
                    .push("--log requires a path argument; using the default log path".to_string()),
            }
        } else {
            warnings.push(format!("unrecognised argument '{arg}'; ignoring it"));
        }
    }
    LogFlag { path, warnings }
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
    let flag = parse_log_flag(env::args().skip(1));
    for warning in &flag.warnings {
        eprintln!("neotmux: {warning}");
    }
    if let Err(err) = logging::init(flag.path.as_deref()) {
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
    fn a_valid_log_flag_is_used_with_no_warnings() {
        let flag = parse_log_flag(args(&["--log", "/tmp/x.log"]));
        assert_eq!(flag.path, Some(PathBuf::from("/tmp/x.log")));
        assert!(flag.warnings.is_empty());
    }

    #[test]
    fn a_trailing_log_flag_with_no_value_falls_back_to_the_default_and_warns() {
        let flag = parse_log_flag(args(&["--log"]));
        assert_eq!(flag.path, None);
        assert_eq!(
            flag.warnings,
            vec!["--log requires a path argument; using the default log path"]
        );
    }

    #[test]
    fn a_log_flag_followed_by_another_flag_is_treated_as_one_typo_not_two_mistakes() {
        let flag = parse_log_flag(args(&["--log", "--foo"]));
        assert_eq!(flag.path, None);
        assert_eq!(
            flag.warnings,
            vec!["--log requires a path argument, got '--foo'; using the default log path"],
            "the flag-shaped token should be consumed as the rejected value, not also reported as its own unrecognised argument"
        );
    }

    #[test]
    fn an_unrecognised_argument_falls_back_to_the_default_and_warns() {
        let flag = parse_log_flag(args(&["--bogus"]));
        assert_eq!(flag.path, None);
        assert_eq!(
            flag.warnings,
            vec!["unrecognised argument '--bogus'; ignoring it"]
        );
    }

    #[test]
    fn no_arguments_at_all_falls_back_to_the_default_with_no_warnings() {
        let flag = parse_log_flag(args(&[]));
        assert_eq!(flag.path, None);
        assert!(flag.warnings.is_empty());
    }
}
