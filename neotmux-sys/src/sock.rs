//! Where the socket lives, and how one neotmux tells a live server from the file a dead one
//! left behind.
//!
//! The path is `$XDG_RUNTIME_DIR/neotmux/server.sock`, falling back to
//! `/tmp/neotmux-<uid>/server.sock` where that variable is unset. One socket for a user, with
//! the sessions named inside the server behind it, which is tmux's arrangement.
//!
//! It was a file per session, so that killing one, finding one or listing them was `ls`
//! rather than a protocol. What ended that is the thing the sessions are for: a sidebar that
//! shows what each session is doing has to read a session it is not attached to, and a
//! process per session would need a second protocol to answer that. One process needs none,
//! and having one, `ls` and `kill` are two more questions it answers.
//!
//! The permissions are set rather than left to `umask`: the directory is `0700` and the
//! socket `0600`, and a directory that is already there is checked to be a directory, ours,
//! and no more open than that before anything is put in it. On `$XDG_RUNTIME_DIR` that is
//! belt and braces; under `/tmp`, where anyone can create a name, it is the whole defence.

use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::Context as _;

/// What a session name may be: one to sixty-four characters, none of them a control
/// character, and the first not `-`.
///
/// It used to be the filesystem's law as well, because the name was a filename — one socket
/// per session, named after it. It is not one any more: there is one socket for the server and
/// a name is a key in a map and a word on a screen. So the law is what those two need. `-`
/// first, because an argument that looks like a flag is not a name the parser can tell apart
/// from one; no control characters, because a name is printed by `ls`, drawn in a sidebar and
/// written into a log, and a name carrying `ESC` would be writing escape sequences into all
/// three. Everything else is allowed, 日本語 included.
pub fn validate_session_name(name: &str) -> Result<(), String> {
    let chars = name.chars().count();
    if chars == 0 || chars > 64 {
        return Err(format!(
            "session name '{name}' is not one to sixty-four characters long"
        ));
    }
    if name.starts_with('-') {
        return Err(format!("session name '{name}' may not start with '-'"));
    }
    if let Some(bad) = name.chars().find(|c| c.is_control()) {
        return Err(format!(
            "session name '{name}' may not hold a control character (U+{:04X})",
            bad as u32
        ));
    }
    Ok(())
}

const DIR_MODE: u32 = 0o700;
const SOCKET_MODE: u32 = 0o600;

/// The socket and the lock beside it.
pub struct Paths {
    pub socket: PathBuf,
    pub lock: PathBuf,
}

impl Paths {
    /// The socket and lock a client and its server agree on. One pair for a user: the
    /// sessions are inside the server, and finding them is a question it answers.
    pub fn resolve(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> anyhow::Result<Self> {
        let dir = runtime_dir(env);
        make_dir(&dir).with_context(|| format!("prepare {}", dir.display()))?;
        Ok(Self {
            socket: dir.join("server.sock"),
            lock: dir.join("server.lock"),
        })
    }
}

fn runtime_dir(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    match env("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Some(xdg) => xdg.join("neotmux"),
        // Keyed by uid, because `/tmp` is shared and a bare `neotmux` there would be the
        // first user to log in owning the name for everybody.
        None => PathBuf::from(format!("/tmp/neotmux-{}", nix::unistd::getuid())),
    }
}

/// Makes the directory `0700` and refuses one that is not a directory, not ours, or open to
/// anyone else. `create_dir` is subject to `umask`, so the mode is set afterwards either way.
fn make_dir(dir: &Path) -> anyhow::Result<()> {
    match fs::DirBuilder::new().mode(DIR_MODE).create(dir) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err.into()),
    }
    // Not `metadata`: a symlink planted where the directory should be must be seen as a
    // symlink rather than followed to wherever it points.
    let meta = fs::symlink_metadata(dir)?;
    anyhow::ensure!(meta.is_dir(), "{} is not a directory", dir.display());
    anyhow::ensure!(
        meta.uid() == nix::unistd::getuid().as_raw(),
        "{} belongs to uid {} rather than to you",
        dir.display(),
        meta.uid()
    );
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))?;
    Ok(())
}

/// Whether a server is listening on `socket` right now.
///
/// Asked by connecting, which is the only question with no race in it: a listening server
/// answers a connect at every instant of its life, and a socket file with nothing behind it
/// answers `ECONNREFUSED`. Anything else — a full backlog, a permission error — is neither
/// "dead" nor an invitation to delete the file, so it counts as busy.
pub fn connect(socket: &Path) -> io::Result<Option<UnixStream>> {
    match UnixStream::connect(socket) {
        Ok(stream) => Ok(Some(stream)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => Ok(None),
        Err(err) => Err(err),
    }
}

/// The right to be the server for this session, held for as long as the process lives.
///
/// Connecting says whether a server is listening; it does not say that no *other* process is
/// between that question and its own `bind`. This does: whoever holds the lock is the one
/// allowed to unlink a refused socket and put a new one there, so two neotmuxes starting at
/// the same instant cannot both decide the socket is stale and cannot clobber each other —
/// and neither can clobber a live server, because the lock holder connects first and stands
/// down if anything answers.
pub struct Lock {
    _held: nix::fcntl::Flock<fs::File>,
}

/// Opens the lock file and tries to take it, without writing anything — asking whether the
/// lock is held must not itself rewrite who holds it. Only [`Lock::take`], which is actually
/// becoming the server, goes on to write a pid into what this returns.
fn try_lock(paths: &Paths) -> anyhow::Result<Option<nix::fcntl::Flock<fs::File>>> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(SOCKET_MODE)
        .open(&paths.lock)
        .with_context(|| format!("open {}", paths.lock.display()))?;
    match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
        Ok(held) => Ok(Some(held)),
        Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(None),
        Err((_, err)) => Err(err).context("lock the session"),
    }
}

impl Lock {
    /// `None` when somebody else holds it, which means somebody else is already becoming the
    /// server for this session.
    pub fn take(paths: &Paths) -> anyhow::Result<Option<Self>> {
        let Some(mut held) = try_lock(paths)? else {
            return Ok(None);
        };
        use std::io::{Seek, SeekFrom, Write as _};
        held.set_len(0).context("truncate the lock")?;
        held.seek(SeekFrom::Start(0)).context("seek the lock")?;
        writeln!(held, "{}", std::process::id()).context("write the lock's pid")?;
        Ok(Some(Self { _held: held }))
    }
}

/// Who, if anyone, currently holds the session's lock.
#[derive(Debug, PartialEq, Eq)]
pub enum Holder {
    /// The lock is free: taking it would succeed.
    Nobody,
    /// The lock is held, but not by a pid this process could confirm is a live neotmux —
    /// a stale write, a `comm` that raced the holder's exit, or a pid since recycled onto
    /// some unrelated program. Something holds the session; nothing may be signalled on the
    /// strength of a guess about who.
    Unconfirmed,
    /// The lock is held by a live process `/proc` says is neotmux.
    Pid(i32),
}

/// Who holds the session's lock, established without ever handing back a pid this process
/// could not confirm is actually a live neotmux.
///
/// The lock is what answers "is anybody there" — a lock this process can take is a lock
/// nobody holds, whatever a file left behind says. The pid inside it is only a hint, so it
/// is confirmed against `/proc/<pid>/comm` before it is ever named: a pid can be recycled
/// between the holder exiting and this reading the file, and a stranger must never be
/// mistaken for the session `--kill` was asked for.
pub fn holder(paths: &Paths) -> anyhow::Result<Holder> {
    if try_lock(paths)?.is_some() {
        return Ok(Holder::Nobody);
    }
    let text = fs::read_to_string(&paths.lock)
        .with_context(|| format!("read {}", paths.lock.display()))?;
    let Some(pid) = text.trim().parse::<i32>().ok() else {
        return Ok(Holder::Unconfirmed);
    };
    match crate::procinfo::command(pid) {
        Some(comm) if comm == "neotmux" => Ok(Holder::Pid(pid)),
        _ => Ok(Holder::Unconfirmed),
    }
}

/// Binds the session's socket, replacing one a dead server left behind. Only ever called
/// under [`Lock`].
pub fn bind(paths: &Paths) -> anyhow::Result<Option<UnixListener>> {
    if connect(&paths.socket)?.is_some() {
        // Somebody won the race between the lock and here, or was already serving when we
        // took a lock nobody else wanted. Either way this process is not the server.
        return Ok(None);
    }
    match fs::remove_file(&paths.socket) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("replace the stale socket"),
    }
    let listener = UnixListener::bind(&paths.socket)
        .with_context(|| format!("bind {}", paths.socket.display()))?;
    // After the bind rather than through `umask`, which is the caller's and not ours. The
    // window is covered by the directory, which nobody else can even enter.
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(SOCKET_MODE))?;
    Ok(Some(listener))
}

/// Takes the socket away on the way out — but only if it is still the one this server bound.
///
/// A server whose directory was emptied under it, or that outlived a replacement, would
/// otherwise unlink the *next* server's socket on its way out and leave a live server
/// nobody can reach. The name is not the identity; the inode is.
pub fn unbind(paths: &Paths, bound: &UnixListener) {
    let Ok(ours) = bound.local_addr().and_then(|addr| {
        addr.as_pathname()
            .map(fs::symlink_metadata)
            .unwrap_or_else(|| Err(io::Error::other("an unnamed socket")))
    }) else {
        return;
    };
    let Ok(there) = fs::symlink_metadata(&paths.socket) else {
        return;
    };
    if (there.dev(), there.ino()) == (ours.dev(), ours.ino()) {
        let _ = fs::remove_file(&paths.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(value: Option<&str>) -> impl Fn(&str) -> Option<std::ffi::OsString> + use<> {
        let value = value.map(std::ffi::OsString::from);
        move |key| (key == "XDG_RUNTIME_DIR").then(|| value.clone()).flatten()
    }

    /// A name is a word a person types and reads, and no longer a filename. What is left of
    /// the law is what the command line and the screen need.
    #[test]
    fn a_name_is_a_word_a_person_can_type_and_read() {
        for name in [
            "work",
            "default",
            "a",
            "a-b_c9",
            "作業",
            "web-dashboard",
            "a b",
            "a/b",
            "émile",
            &"x".repeat(64),
            &"あ".repeat(64),
        ] {
            assert!(validate_session_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            &"x".repeat(65),
            &"あ".repeat(65),
            "-lead",
            "a\nb",
            "a\x1b[31mb",
            "a\tb",
        ] {
            assert!(
                validate_session_name(name).is_err(),
                "{name:?} should be refused"
            );
        }
    }

    #[test]
    fn resolve_builds_the_one_pair_a_user_has() {
        // `resolve` prepares the directory as its first act, so the test needs a path it
        // can actually create rather than one it cannot.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().to_path_buf();
        let paths = Paths::resolve(&env(Some(dir.to_str().expect("utf8")))).expect("resolve");
        assert_eq!(paths.socket, dir.join("neotmux/server.sock"));
        assert_eq!(paths.lock, dir.join("neotmux/server.lock"));
    }

    #[test]
    fn the_runtime_dir_follows_xdg_when_it_is_set_to_an_absolute_path() {
        assert_eq!(
            runtime_dir(&env(Some("/run/user/1000"))),
            PathBuf::from("/run/user/1000/neotmux")
        );
    }

    /// `/tmp` is shared, so the name has to be one user's or the first person to log in owns
    /// it for everybody. An empty or relative `XDG_RUNTIME_DIR` is no answer at all.
    #[test]
    fn without_xdg_the_directory_is_keyed_by_uid_under_tmp() {
        let uid = nix::unistd::getuid();
        for unusable in [None, Some(""), Some("relative/path")] {
            assert_eq!(
                runtime_dir(&env(unusable)),
                PathBuf::from(format!("/tmp/neotmux-{uid}")),
                "{unusable:?}"
            );
        }
    }

    #[test]
    fn a_missing_directory_is_made_private_rather_than_left_to_the_umask() {
        let parent = tempfile::tempdir().expect("tempdir");
        let dir = parent.path().join("neotmux");
        make_dir(&dir).expect("make the directory");
        let mode = fs::symlink_metadata(&dir)
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, DIR_MODE, "{mode:o}");
    }

    #[test]
    fn a_directory_left_open_to_everyone_is_closed_again() {
        let parent = tempfile::tempdir().expect("tempdir");
        let dir = parent.path().join("neotmux");
        fs::create_dir(&dir).expect("create");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).expect("open it up");
        make_dir(&dir).expect("make the directory");
        let mode = fs::symlink_metadata(&dir)
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, DIR_MODE, "{mode:o}");
    }

    #[test]
    fn something_that_is_not_a_directory_is_refused_rather_than_used() {
        let parent = tempfile::tempdir().expect("tempdir");
        let path = parent.path().join("neotmux");
        fs::write(&path, b"not a directory").expect("write");
        assert!(make_dir(&path).is_err());
    }

    #[test]
    fn a_socket_nobody_is_listening_on_is_reported_as_nobody_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("default.sock");
        assert!(connect(&socket).expect("connect").is_none(), "no file");

        let listener = UnixListener::bind(&socket).expect("bind");
        assert!(connect(&socket).expect("connect").is_some());

        // Given a moment: a connection the listener never accepted is still queued on it as
        // it closes, and the kernel answers the next connect out of that queue until it has
        // torn it down. What matters is that the file it leaves behind is not mistaken for a
        // server, not how many microseconds that takes.
        drop(listener);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while connect(&socket).expect("connect").is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "the file the dead listener left behind was taken for a live server"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_bind_replaces_the_socket_a_dead_server_left_and_makes_it_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        drop(UnixListener::bind(&paths.socket).expect("bind a listener and drop it"));

        let listener = bind(&paths).expect("bind").expect("nobody was listening");
        assert!(connect(&paths.socket).expect("connect").is_some());
        let mode = fs::symlink_metadata(&paths.socket)
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, SOCKET_MODE, "{mode:o}");
        drop(listener);
    }

    #[test]
    fn a_bind_stands_down_rather_than_clobber_a_server_that_is_listening() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        let live = UnixListener::bind(&paths.socket).expect("bind");

        assert!(
            bind(&paths).expect("bind").is_none(),
            "a live server's socket was replaced"
        );
        assert!(connect(&paths.socket).expect("connect").is_some());
        drop(live);
    }

    #[test]
    fn only_one_process_at_a_time_holds_the_right_to_be_the_server() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        let held = Lock::take(&paths).expect("take").expect("nobody held it");
        // Same process, a second descriptor: `flock` is per open file description, so this
        // is the contention another process would see.
        assert!(
            Lock::take(&paths).expect("take").is_none(),
            "two processes could both have become the server"
        );
        drop(held);
        assert!(Lock::take(&paths).expect("take").is_some());
    }

    #[test]
    fn the_lock_is_free_before_being_taken_and_after_being_given_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        assert_eq!(
            holder(&paths).expect("holder"),
            Holder::Nobody,
            "nobody holds it yet"
        );

        let held = Lock::take(&paths).expect("take").expect("nobody held it");
        // Held by this test process, which is not itself neotmux: the honest answer is
        // "somebody, unconfirmed" — never `Nobody`, and never this pid named outright.
        assert_eq!(holder(&paths).expect("holder"), Holder::Unconfirmed);

        drop(held);
        assert_eq!(
            holder(&paths).expect("holder"),
            Holder::Nobody,
            "it was given back"
        );
    }

    /// A lock file naming a live pid is not enough: the pid has to look like neotmux before
    /// `--kill` may be pointed at it, or a stale write — or a recycled pid — could end up
    /// signalling a stranger. Written directly rather than through `Lock::take`, which is
    /// the honest shape of the failure: nothing here says the file's claim was ever true.
    #[test]
    fn a_held_lock_naming_a_live_pid_that_is_not_neotmux_is_not_handed_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        let mut held = try_lock(&paths).expect("try_lock").expect("nobody held it");
        use std::io::Write as _;
        writeln!(held, "{}", std::process::id()).expect("write a live, non-neotmux pid");

        assert_eq!(
            holder(&paths).expect("holder"),
            Holder::Unconfirmed,
            "a held lock naming this test's own pid was handed back as a neotmux holder"
        );
    }

    /// Asking who holds the lock must not itself become the answer: only becoming the
    /// server, through `Lock::take`, may write a pid into the file.
    #[test]
    fn asking_who_holds_a_free_lock_does_not_write_to_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            socket: dir.path().join("default.sock"),
            lock: dir.path().join("default.lock"),
        };
        holder(&paths).expect("holder");
        let text = fs::read_to_string(&paths.lock).unwrap_or_default();
        assert!(
            text.is_empty(),
            "asking who holds a free lock wrote to it: {text:?}"
        );
    }
}
