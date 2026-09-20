mod draw;
mod notices;
mod overlay;
mod server;
mod signals;
mod start;
mod tty;
mod tui;

use std::env;
use std::path::{Path, PathBuf};

use neotmux::plugins::Plugins;
use neotmux::{logging, reads};
use neotmux_lua::LuaSubsystem;
use neotmux_lua::runtime::Layers;
use neotmux_proto::Level;

/// What the command line asked for. One binary is both halves of neotmux: with no arguments
/// it is a client, and `--server` is the copy of itself a client starts when nothing is
/// listening on the session's socket.
struct Cli {
    log: Option<PathBuf>,
    server: bool,
    warnings: Vec<String>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Cli {
    let mut log = None;
    let mut server = false;
    let mut warnings = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = true,
            "--log" => match args.next() {
                Some(value) if !value.starts_with('-') => {
                    log = Some(PathBuf::from(value));
                }
                Some(value) => warnings.push(format!(
                    "--log requires a path argument, got '{value}'; using the default log path"
                )),
                None => warnings
                    .push("--log requires a path argument; using the default log path".to_string()),
            },
            _ => warnings.push(format!("unrecognised argument '{arg}'; ignoring it")),
        }
    }
    Cli {
        log,
        server,
        warnings,
    }
}

/// Raw mode, taken before anything slow and given back however `main` leaves — a panic
/// included, because unwinding runs this drop.
///
/// Until it is taken the line discipline is the one reading the keyboard, and it does not
/// hand over what it understands as editing: `0x17` (`C-w`) is WERASE and is eaten rather
/// than delivered, `0x03` raises SIGINT, and `\r` arrives as `\n`. Starting a server and
/// loading the Lua layer both take long enough on a loaded machine for a key typed at the
/// prompt to land inside that window, so the terminal is claimed first and everything else
/// is done under it.
struct RawMode;

impl RawMode {
    fn enter() -> Self {
        if let Err(err) = ratatui::crossterm::terminal::enable_raw_mode() {
            logging::log(
                Level::Warn,
                "neotmux",
                &format!("could not put the terminal in raw mode: {err}"),
            );
        }
        Self
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
    }
}

fn main() -> anyhow::Result<()> {
    let cli = parse_args(env::args().skip(1));
    for warning in &cli.warnings {
        eprintln!("neotmux: {warning}");
    }
    if let Err(err) = logging::init(cli.log.as_deref()) {
        eprintln!("neotmux: failed to initialise logging: {err:#}");
    }

    if cli.server {
        return start::serve();
    }
    client()
}

fn client() -> anyhow::Result<()> {
    // First, and before the socket: until the terminal is in raw mode the line discipline is
    // answering the keyboard, and starting a server is not fast.
    let _raw = RawMode::enter();

    let link = start::connect()?;

    let exe_dir = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let layers = Layers::resolve(&|key| env::var_os(key), exe_dir.as_deref());
    let panes = std::rc::Rc::new(reads::RemotePanes::new(link.clone()));
    let lua = LuaSubsystem::build(layers, panes.clone())?;
    let mut plugins = Plugins::new(lua);
    for failure in plugins.lua().run_init() {
        logging::log(Level::Warn, &failure.source, &failure.message);
    }

    // Drained here, before `ratatui::init()`, so an init-time `neotmux.quit()` exits before
    // the alternate screen is entered and before the server is asked to start a session.
    if plugins.drain_load_ops() {
        return Ok(());
    }

    let shutdown = signals::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let attach = tui::attach(&terminal, &plugins)?;
    let result = tui::run(
        &mut terminal,
        &shutdown,
        &mut plugins,
        &link,
        &panes,
        attach,
    );
    // Not `ratatui::restore`: it reports a failure with `eprintln!`, which panics when the
    // terminal it is trying to restore is the one that has gone away.
    if let Err(err) = ratatui::try_restore() {
        logging::log(
            Level::Warn,
            "neotmux",
            &format!("could not restore the terminal: {err}"),
        );
    }
    plugins.notifications().flush(&mut std::io::stderr());
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
        let flag = parse_args(args(&["--log", "/tmp/x.log"]));
        assert_eq!(flag.log, Some(PathBuf::from("/tmp/x.log")));
        assert!(flag.warnings.is_empty());
    }

    #[test]
    fn a_trailing_log_flag_with_no_value_falls_back_to_the_default_and_warns() {
        let flag = parse_args(args(&["--log"]));
        assert_eq!(flag.log, None);
        assert_eq!(
            flag.warnings,
            vec!["--log requires a path argument; using the default log path"]
        );
    }

    #[test]
    fn a_log_flag_followed_by_another_flag_is_treated_as_one_typo_not_two_mistakes() {
        let flag = parse_args(args(&["--log", "--foo"]));
        assert_eq!(flag.log, None);
        assert_eq!(
            flag.warnings,
            vec!["--log requires a path argument, got '--foo'; using the default log path"],
            "the flag-shaped token should be consumed as the rejected value, not also reported as its own unrecognised argument"
        );
    }

    #[test]
    fn an_unrecognised_argument_falls_back_to_the_default_and_warns() {
        let flag = parse_args(args(&["--bogus"]));
        assert_eq!(flag.log, None);
        assert_eq!(
            flag.warnings,
            vec!["unrecognised argument '--bogus'; ignoring it"]
        );
    }

    #[test]
    fn no_arguments_at_all_falls_back_to_the_default_with_no_warnings() {
        let flag = parse_args(args(&[]));
        assert_eq!(flag.log, None);
        assert!(flag.warnings.is_empty());
        assert!(!flag.server, "no arguments is a client");
    }

    #[test]
    fn the_server_flag_is_the_other_half_of_the_same_binary() {
        let flag = parse_args(args(&["--server", "--log", "/tmp/x.log"]));
        assert!(flag.server);
        assert_eq!(flag.log, Some(PathBuf::from("/tmp/x.log")));
        assert!(flag.warnings.is_empty());
    }
}
