//! The wiring, through the binary: the server builds its subsystem with a real runner, and a
//! frame dispatches jobs and timers before it ticks. Neither is visible from `neotmux-lua`'s
//! own tests — they hand the subsystem a fake and call the dispatches themselves — so this is
//! the only thing that fails if `server.rs` goes back to `NoRunner` or the two dispatch lines
//! leave `plugins.rs`.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

mod session;

/// The plugin under test. Its timer starts a job, the job's `on_exit` says what it heard, and
/// the tick is the backstop: with the timer dispatch gone nothing else would ever end the
/// session, and the test would fail by timing out rather than by saying what was missing.
const PLUGIN: &str = "\
local fired, ticks = 0, 0
neotmux.timer.every(16, function()
  fired = fired + 1
  if fired > 1 then return end
  local job, why = neotmux.job.start({ '/bin/sh', '-c', 'echo wired' }, {
    on_exit = function(code, out)
      local said = (out or ''):gsub('%s+$', '')
      neotmux.log.info('wiring-marker code=' .. tostring(code) .. ' out=[' .. said .. ']')
      neotmux.quit()
    end,
  })
  if not job then
    neotmux.log.info('wiring-marker no-job ' .. tostring(why))
    neotmux.quit()
  end
end)
neotmux.on_tick(function()
  ticks = ticks + 1
  if ticks == 300 then
    neotmux.log.info('wiring-marker gave-up fired=' .. fired)
    neotmux.quit()
  end
end)
";

/// The binary, through a PTY — `ratatui::init()` needs a real tty — with a shell that reads
/// its stdin and therefore stays up, so the session lives until the plugin quits it. `/bin/sh`
/// rather than anything else in `/bin`: it is the one path every system has.
fn run_neotmux(runtime_dir: &Path, log: &Path) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let home = tempfile::tempdir().expect("isolated home");
    let _end_session = session::EndsSession::new(home.path());

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.arg("--log");
    cmd.arg(log);
    cmd.env("NEOTMUX_RUNTIME", runtime_dir);
    cmd.env("SHELL", "/bin/sh");
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", home.path());
    cmd.env("XDG_CONFIG_HOME", home.path());
    cmd.env("XDG_RUNTIME_DIR", home.path());
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let collector = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
            if n == 0 {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "neotmux did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(pair.master);
    collector.join().ok();
    assert!(status.success(), "neotmux exited with {status:?}");
}

#[test]
fn a_timer_fires_and_a_job_runs_in_the_server_the_binary_starts() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("init.lua"), PLUGIN).unwrap();

    let log = dir.path().join("neotmux.log");
    run_neotmux(dir.path(), &log);

    let text = fs::read_to_string(&log).unwrap_or_default();
    let marker = text
        .lines()
        .find(|line| line.contains("wiring-marker"))
        .unwrap_or_else(|| panic!("nothing was said about the job or the timer: {text}"));
    assert!(
        marker.contains("code=0 out=[wired]"),
        "the job's output did not reach its callback: {marker}"
    );
}
