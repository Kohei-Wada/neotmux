mod draw;
mod notices;
mod overlay;
mod server;
mod signals;
mod start;
mod tty;
mod tui;

use std::env;
use std::path::PathBuf;

use anyhow::Context as _;
use neotmux::logging;
use neotmux_proto::Level;

/// What the invocation asks for, before the name. A name is not a mode.
enum Action {
    /// Attach to a session, starting it where it is not running. The mode with no word.
    Attach,
    /// The running sessions, one per line.
    Ls,
    /// End one of them and its shells.
    Kill,
}

/// What the command line asked for. One binary is both halves of neotmux: with no arguments
/// it is a client, and `--server` is the copy of itself a client starts when nothing is
/// listening on the session's socket.
struct Cli {
    log: Option<PathBuf>,
    action: Action,
    /// The session the action speaks about, or the default where none was said.
    session: Option<String>,
    server: bool,
    help: bool,
    version: bool,
    /// An argument that is a refusal, not a warning: a name that would not be the
    /// session the person meant to name. Warnings are ignored and gone; this is said
    /// and the command is not run.
    refusal: Option<String>,
    warnings: Vec<String>,
}

impl Cli {
    /// The name as the command will run under, default taken.
    fn session_name(&self) -> &str {
        self.session
            .as_deref()
            .unwrap_or(neotmux::sock::DEFAULT_SESSION)
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
    let mut args = args.into_iter().peekable();
    let mut log = None;
    let mut action = Action::Attach;
    let mut session: Option<String> = None;
    let mut server = false;
    let mut help = false;
    let mut version = false;
    let mut refusal: Option<String> = None;
    let mut warnings = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "ls" => {
                action = Action::Ls;
            }
            "kill" => {
                action = Action::Kill;
                match args.next() {
                    // A name for kill must be a name the person clearly meant. One that
                    // starts with `-` is not one, and kill is the one command where
                    // falling back to `default` is the wrong answer: the Review Focus
                    // says kill must not guess.
                    Some(value) if value.starts_with('-') => {
                        refusal = Some(format!(
                            "a name for kill cannot start with '-' ('{value}') — kill does not guess at the default"
                        ));
                    }
                    // An empty word is a word: it lands in `session` and is refused by the
                    // name check after the loop, rather than guessed into `default` here.
                    Some(value) => {
                        if session.is_some() {
                            warnings.push(
                                "a name following 'kill' was ignored, one was already given"
                                    .to_string(),
                            );
                        }
                        session = Some(value);
                    }
                    None => {}
                }
            }
            "--server" => {
                server = true;
                // `--server` is the spawn path: its name is this process's own, set by
                // `spawn`, and never a person's typing. A flag-shaped word after it is a
                // parse error, not a name; the default is what was meant.
                if args.peek().is_some_and(|value| !value.starts_with('-')) {
                    session = Some(args.next().expect("peeked a word"));
                }
            }
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
            "--kill" => {
                refusal = Some(
                    "'--kill' was the old spelling; it is 'kill [name]' now — try 'neotmux kill'"
                        .to_string(),
                )
            }
            "--help" | "-h" => help = true,
            "--version" | "-V" => version = true,
            arg if !arg.starts_with('-') && !arg.is_empty() => {
                if session.is_some() {
                    warnings.push(format!(
                        "unrecognised argument '{arg}' after the session name; ignoring it — try 'neotmux --help'"
                    ));
                } else {
                    session = Some(arg.to_string());
                }
            }
            other => warnings.push(format!(
                "unrecognised argument '{other}'; ignoring it — try 'neotmux --help'"
            )),
        }
    }

    // The name, checked once, after the loop, because a name can arrive from the
    // positional, from `kill`, or from `--server`, and `ls` + a name is a refusal either
    // way it arrived.
    if let Some(why) = session
        .as_deref()
        .and_then(|name| neotmux::sock::validate_session_name(name).err())
    {
        refusal = Some(why);
    }
    if matches!(action, Action::Ls) && session.is_some() && refusal.is_none() {
        refusal = Some("ls lists the running sessions; it does not take a name".to_string());
    }

    Cli {
        log,
        action,
        session,
        server,
        help,
        version,
        refusal,
        warnings,
    }
}

/// Both numbers, because they fail in different ways: the first is what was installed, the
/// second is what a client and a server have to agree on before they will talk at all.
fn version_line() -> String {
    format!(
        "neotmux {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        neotmux_proto::PROTOCOL_VERSION
    )
}

/// What the command line offers. The keys are not here: every one of them belongs to a
/// plugin that can drop or rebind it, so `docs/plugins.md` is where they are written down.
fn usage() -> String {
    format!(
        "\
{}
A terminal multiplexer: it keeps your shells running after you disconnect, and brings
you back to them, one at a time or by name.

usage: neotmux [name | command] [options]

With no command, neotmux attaches to the named session (default, where no name is
given) if it is running, and starts it if it is not. The session survives your
disconnecting, by any means.

commands:
  neotmux [name]      attach to the named session, or start it
  neotmux ls          the running sessions, one per line
  neotmux kill [name] end a session and every shell in it

options:
  --log <path>    write the log somewhere other than the default
  --help, -h      this text
  --version, -V   the version, and the protocol number it speaks
  --server        the half that holds a session; started for you, not by you
",
        version_line()
    )
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
    if cli.help {
        print!("{}", usage());
        return Ok(());
    }
    if cli.version {
        println!("{}", version_line());
        return Ok(());
    }
    if let Some(refusal) = &cli.refusal {
        anyhow::bail!("neotmux: {refusal}");
    }
    if let Err(err) = logging::init(cli.log.as_deref()) {
        eprintln!("neotmux: failed to initialise logging: {err:#}");
    }

    if cli.server {
        return start::serve(cli.session);
    }
    match cli.action {
        Action::Ls => list_sessions(),
        Action::Kill => kill_session(cli.session_name()),
        Action::Attach => client(cli.session.clone()),
    }
}

/// Ends a session's server from outside, for the case the protocol cannot reach: a
/// client built against another version is refused by the handshake, and the session it
/// cannot speak to would otherwise be unreachable and immortal.
fn kill_session(name: &str) -> anyhow::Result<()> {
    let paths = neotmux::sock::Paths::resolve(&|key| env::var_os(key), name)?;
    let pid = match neotmux::sock::holder(&paths)? {
        neotmux::sock::Holder::Nobody => {
            anyhow::bail!("neotmux session '{name}' is not running")
        }
        neotmux::sock::Holder::Unconfirmed => anyhow::bail!(
            "session '{name}'s lock is held, but its pid could not be confirmed; refusing to signal it"
        ),
        neotmux::sock::Holder::Pid(pid) => pid,
    };
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .with_context(|| format!("signal the neotmux server (pid {pid})"))?;
    println!("neotmux: asked the session '{name}' (pid {pid}) to end");
    Ok(())
}

/// The running sessions, the way the socket directory says they are: one file per
/// session, and a connect probe the difference. A socket that answers is a session;
/// one that does not is a file.
fn list_sessions() -> anyhow::Result<()> {
    use std::fs;
    let dir = neotmux::sock::socket_dir(&|key| env::var_os(key));
    let mut running: Vec<(String, Option<i32>)> = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Some(name) = name.strip_suffix(".sock") else {
                continue;
            };
            if neotmux::sock::connect(&dir.join(format!("{name}.sock")))?.is_some() {
                let pid = fs::read_to_string(dir.join(format!("{name}.lock")))
                    .ok()
                    .and_then(|text| text.trim().parse::<i32>().ok());
                running.push((name.to_string(), pid));
            }
        }
    }
    running.sort_by(|a, b| a.0.cmp(&b.0));
    if running.is_empty() {
        anyhow::bail!("no neotmux sessions are running");
    }
    for (name, pid) in running {
        match pid {
            Some(pid) => println!("{name}  pid {pid}"),
            None => println!("{name}"),
        }
    }
    Ok(())
}

fn client(session: Option<String>) -> anyhow::Result<()> {
    // First, and before the socket: until the terminal is in raw mode the line discipline is
    // answering the keyboard, and starting a server is not fast.
    let _raw = RawMode::enter();
    let name = session
        .clone()
        .unwrap_or_else(|| neotmux::sock::DEFAULT_SESSION.to_string());
    let link = start::begin(session)?.finish()?;

    // The attach, and the session's answer, before the screen: a session that a plugin ended
    // while it was loading must not flash the alternate screen on its way past.
    let first = match tui::open(&link, tui::screen_size()?, name)? {
        tui::Opened::Over(transcript) => {
            for line in &transcript {
                eprintln!("neotmux: {line}");
            }
            return Ok(());
        }
        tui::Opened::Running(done) => *done,
    };

    let shutdown = signals::ShutdownSignal::install()?;
    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, &shutdown, &link, first);
    // Not `ratatui::restore`: it reports a failure with `eprintln!`, which panics when the
    // terminal it is trying to restore is the one that has gone away.
    if let Err(err) = ratatui::try_restore() {
        logging::log(
            Level::Warn,
            "neotmux",
            &format!("could not restore the terminal: {err}"),
        );
    }
    let left = result?;
    // The notifications raised while this client was attached, said now that there is a
    // terminal to say them on: a full-screen TUI cannot use stderr while it is running.
    for line in &left.transcript {
        eprintln!("neotmux: {line}");
    }
    match left.signal {
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
            vec!["unrecognised argument '--bogus'; ignoring it — try 'neotmux --help'"]
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

    #[test]
    fn an_unnamed_attach_is_still_the_default_and_warns_on_nothing() {
        let cli = parse_args(args(&[]));
        assert!(matches!(cli.action, Action::Attach));
        assert!(cli.session.is_none());
        assert!(cli.session_name() == "default");
        assert!(cli.warnings.is_empty());
    }

    #[test]
    fn one_positional_is_the_session_it_attaches() {
        let cli = parse_args(args(&["work"]));
        assert!(matches!(cli.action, Action::Attach));
        assert_eq!(cli.session.as_deref(), Some("work"));
        assert!(cli.warnings.is_empty());
    }

    #[test]
    fn ls_is_a_thing_and_takes_no_name() {
        let cli = parse_args(args(&["ls"]));
        assert!(matches!(cli.action, Action::Ls));
        assert!(cli.refusal.is_none());

        let cli = parse_args(args(&["ls", "work"]));
        assert!(
            cli.refusal.is_some(),
            "a name with ls is a mistake, not a filter"
        );
    }

    #[test]
    fn kill_is_a_thing_and_the_name_is_optional() {
        let cli = parse_args(args(&["kill"]));
        assert!(matches!(cli.action, Action::Kill));
        assert!(cli.session.is_none(), "no name is the default session");
        assert!(cli.refusal.is_none());

        let cli = parse_args(args(&["kill", "work"]));
        assert_eq!(cli.session.as_deref(), Some("work"));
        assert!(cli.refusal.is_none());
    }

    #[test]
    fn an_invalid_name_is_refused_and_said_so_not_a_warning() {
        // A refused name is a refusal: the command is not run. In particular `kill` with
        // a name it could not confirm must not fall back to `default`, which is the one
        // case the Review Focus says is dangerous.
        let cases: &[&[&str]] = &[
            ["kill", "a:b"].as_slice(),
            ["kill", "-lead"].as_slice(),
            ["a:b"].as_slice(),
            ["kill", ""].as_slice(),
        ];
        for case in cases {
            let cli = parse_args(args(case));
            assert!(cli.refusal.is_some(), "{case:?} should be refused");
            assert!(
                !cli.warnings.iter().any(|w| w.contains("unrecognised")),
                "{case:?} was filed under warnings, where it would be ignored: {:?}",
                cli.warnings
            );
        }
    }

    #[test]
    fn two_positionals_are_one_name_and_one_misplaced_word() {
        let cli = parse_args(args(&["work", "home"]));
        assert_eq!(cli.session.as_deref(), Some("work"));
        assert_eq!(cli.warnings.len(), 1);
        assert!(cli.warnings[0].contains("home"), "{:?}", cli.warnings);
    }

    #[test]
    fn the_old_kill_flag_is_not_a_flag_anymore() {
        let cli = parse_args(args(&["--kill"]));
        assert!(
            !cli.warnings
                .iter()
                .any(|w| w.contains("--kill") && w.contains("unrecognised"))
                || cli.refusal.is_some(),
            "'--kill' must be either refused or named unrecognised: {:?}",
            cli.warnings
        );
        assert!(
            !matches!(cli.action, Action::Kill),
            "--kill is not the new kill"
        );
    }

    #[test]
    fn the_help_and_version_flags_are_recognised_in_both_spellings() {
        for spelling in ["--help", "-h"] {
            let flag = parse_args(args(&[spelling]));
            assert!(flag.help, "{spelling}");
            assert!(flag.warnings.is_empty(), "{spelling}");
        }
        for spelling in ["--version", "-V"] {
            let flag = parse_args(args(&[spelling]));
            assert!(flag.version, "{spelling}");
            assert!(flag.warnings.is_empty(), "{spelling}");
        }
    }

    /// An unrecognised argument is still a warning rather than a refusal — but now there is
    /// somewhere to send the person who typed it.
    #[test]
    fn an_unrecognised_argument_points_at_the_help() {
        let flag = parse_args(args(&["--wat"]));
        assert_eq!(flag.warnings.len(), 1);
        assert!(flag.warnings[0].contains("--wat"), "{:?}", flag.warnings[0]);
        assert!(
            flag.warnings[0].contains("neotmux --help"),
            "{:?}",
            flag.warnings[0]
        );
    }

    /// The version says both numbers: a client and a server from different commits refuse
    /// each other by protocol number, and this is what the person reads next.
    #[test]
    fn the_version_line_carries_the_protocol_number_too() {
        let said = version_line();
        assert!(said.contains(env!("CARGO_PKG_VERSION")), "{said}");
        assert!(
            said.contains(&neotmux_proto::PROTOCOL_VERSION.to_string()),
            "{said}"
        );
    }

    /// Keys belong to plugins that can rebind or drop them; `docs/plugins.md` is where they
    /// are written down, not here. `C-b d` is the one a person will reach to add next.
    #[test]
    fn the_usage_text_names_no_keybinding() {
        assert!(!usage().contains("C-b"), "{}", usage());
    }
}
