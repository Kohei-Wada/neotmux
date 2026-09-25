use std::env;
use std::path::PathBuf;

use neotmux::logging;

/// What the invocation asks for, before the name. A name is not a mode.
enum Action {
    /// Attach to a session, starting it where it is not running. The mode with no word.
    Attach,
    /// The running sessions, one per line.
    Ls,
    /// End one of them and its shells.
    Kill,
    /// A host command, answered by the server without touching the Lua layer.
    Command { name: String, args: Vec<String> },
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
            // `-t <name>` is read only as the first argument after the command word; every
            // word after that is the command's own payload, taken verbatim. A command that
            // wrote `-t` into its text — `send %1 echo -t MARK` — must not have it read back
            // out as a session name.
            arg if !arg.is_empty() && neotmux::server::command::NAMES.contains(&arg) => {
                let name = arg.to_string();
                if args.peek().is_some_and(|next| next == "-t") {
                    args.next();
                    match args.next() {
                        Some(value) => session = Some(value),
                        None => warnings.push("-t requires a session name argument".to_string()),
                    }
                } else if args
                    .peek()
                    .is_some_and(|next| next == "--help" || next == "-h")
                {
                    // `--help` right after a command word is the same request it is
                    // anywhere else: usage text, exit 0. Anywhere past that position it
                    // is payload, the same as `-t` is — `send %1 git --help` sends the
                    // text, it does not print usage.
                    help = true;
                }
                let cargs: Vec<String> = args.by_ref().collect();
                action = Action::Command { name, args: cargs };
            }
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
  neotmux new [name]  start a session with no client attached
  neotmux rename <old> <new>
                      rename a session
  neotmux windows [name] [-t name]
                      the session's windows, one per line
  neotmux panes [name] [-t name]
                      the session's panes, one per line
  neotmux send <pane> <text> [--enter]
                      write text into that pane's shell
  neotmux capture <pane> [--lines N]
                      what that pane is showing, oldest line first
  neotmux switch <name>
                      move the attached clients to that session

options:
  --log <path>    write the log somewhere other than the default
  --help, -h      this text
  --version, -V   the version, and the protocol number it speaks
  --server        the half that holds a session; started for you, not by you
",
        version_line()
    )
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
        return neotmux::server::run();
    }
    match cli.action {
        Action::Ls => neotmux::client::ls(),
        Action::Kill => neotmux::client::kill(cli.session_name()),
        Action::Attach => neotmux::client::attach(cli.session.clone()),
        Action::Command { name, args } => {
            neotmux::client::command(&name, &args, cli.session.as_deref())
        }
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
        let cli = parse_args(args(&["作業"]));
        assert_eq!(
            cli.session.as_deref(),
            Some("作業"),
            "a name is a word a person types, not a filename"
        );
        assert!(cli.refusal.is_none());

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
            ["kill", "a\nb"].as_slice(),
            ["kill", "-lead"].as_slice(),
            ["a\u{1b}[31mb"].as_slice(),
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
    fn a_command_s_t_right_after_it_names_the_session() {
        let cli = parse_args(args(&["send", "-t", "work", "%1", "echo"]));
        assert_eq!(cli.session.as_deref(), Some("work"));
        match cli.action {
            Action::Command { name, args } => {
                assert_eq!(name, "send");
                assert_eq!(args, vec!["%1".to_string(), "echo".to_string()]);
            }
            _ => panic!("expected a command"),
        }
    }

    #[test]
    fn a_command_s_t_later_in_its_arguments_is_payload_not_a_session() {
        let cli = parse_args(args(&["send", "%1", "echo", "-t", "MARK", "--enter"]));
        assert!(cli.session.is_none());
        match cli.action {
            Action::Command { name, args } => {
                assert_eq!(name, "send");
                assert_eq!(
                    args,
                    vec![
                        "%1".to_string(),
                        "echo".to_string(),
                        "-t".to_string(),
                        "MARK".to_string(),
                        "--enter".to_string(),
                    ],
                    "-t past the first argument must survive verbatim, not be eaten"
                );
            }
            _ => panic!("expected a command"),
        }
    }

    #[test]
    fn a_bare_word_that_is_not_a_command_is_still_the_session_name() {
        let cli = parse_args(args(&["work"]));
        assert!(matches!(cli.action, Action::Attach));
        assert_eq!(cli.session.as_deref(), Some("work"));
    }

    #[test]
    fn a_command_s_remaining_arguments_survive_verbatim() {
        let cli = parse_args(args(&["capture", "%1", "--lines", "3"]));
        match cli.action {
            Action::Command { name, args } => {
                assert_eq!(name, "capture");
                assert_eq!(
                    args,
                    vec!["%1".to_string(), "--lines".to_string(), "3".to_string()]
                );
            }
            _ => panic!("expected a command"),
        }
    }

    #[test]
    fn a_command_s_help_right_after_it_is_usage_but_later_is_payload() {
        let cli = parse_args(args(&["panes", "--help"]));
        assert!(cli.help, "--help right after the command word is usage");

        let cli = parse_args(args(&["send", "%1", "git", "--help"]));
        assert!(
            !cli.help,
            "--help past the first argument is the command's own text, not a request for usage"
        );
        match cli.action {
            Action::Command { name, args } => {
                assert_eq!(name, "send");
                assert_eq!(
                    args,
                    vec!["%1".to_string(), "git".to_string(), "--help".to_string()]
                );
            }
            _ => panic!("expected a command"),
        }
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
