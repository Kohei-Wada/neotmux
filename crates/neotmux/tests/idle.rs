use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const IDLE_BYTE_BOUND: usize = 64;

#[test]
fn produces_no_output_while_idle() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let isolation = tempfile::tempdir().expect("isolation dir");
    let log = isolation.path().join("neotmux.log");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("SHELL", "/bin/sh");
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
    cmd.arg("--log");
    cmd.arg(&log);
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let (tx, rx) = mpsc::channel::<usize>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(n).is_err() {
                        break;
                    }
                }
            }
        }
    });

    std::thread::sleep(Duration::from_millis(700));
    while rx.try_recv().is_ok() {}

    let mut idle_bytes = 0usize;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(n) => idle_bytes += n,
            Err(_) => break,
        }
    }

    child.kill().ok();
    child.wait().ok();

    assert!(
        idle_bytes <= IDLE_BYTE_BOUND,
        "expected no redraw while nothing changed, got {idle_bytes} bytes"
    );
}

/// What a plugin reading every pane on every tick costs, measured as the multiplexer's own
/// CPU time while nothing at all is happening.
///
/// Four idle panes, because "every pane" with one pane says very little, and `/bin/cat` in
/// each so that nothing but neotmux is running.
const SPLITTER: &str = r#"
local made = 0
neotmux.on_tick(function()
  if made < 3 then
    made = made + 1
    neotmux.pane.split('vertical')
  end
end)
"#;

/// The discipline the shipped watcher follows: ask the cheap question every tick, build the
/// lines only for a pane that has been fed since.
const GATED: &str = r#"
local seen = {}
neotmux.on_tick(function()
  for _, id in ipairs(neotmux.pane.list()) do
    local revision = neotmux.pane.revision(id)
    if revision ~= nil and revision ~= seen[id] then
      seen[id] = revision
      neotmux.pane.content(id)
    end
  end
end)
"#;

/// The same plugin with the gate taken out: every pane's whole screen, every tick, for ever.
const UNGATED: &str = r#"
neotmux.on_tick(function()
  for _, id in ipairs(neotmux.pane.list()) do
    neotmux.pane.content(id)
  end
end)
"#;

/// The process's own user + system time so far, in seconds. Fields 14 and 15 of
/// `/proc/<pid>/stat`, read after the command name so a shell called `a b c` cannot shift
/// them.
fn cpu_seconds(pid: u32) -> f64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read /proc stat");
    // Everything after the last ')' — a command name can hold spaces and parentheses, and
    // splitting before it would shift every field.
    let after_comm = stat.rsplit_once(')').expect("a comm field").1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let ticks: u64 =
        fields[11].parse::<u64>().expect("utime") + fields[12].parse::<u64>().expect("stime");
    // `sysconf(_SC_CLK_TCK)`, which is 100 on every Linux that matters and is what
    // `/proc/<pid>/stat` is denominated in.
    ticks as f64 / 100.0
}

/// Spawns neotmux over a PTY with `plugins` as its runtime layer and reports the CPU seconds
/// it spends over `window` once the session has settled.
fn idle_cpu(plugins: &[(&str, &str)], window: Duration) -> f64 {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let isolation = tempfile::tempdir().expect("isolation dir");
    let runtime = tempfile::tempdir().expect("runtime dir");
    for (path, body) in plugins {
        let full = runtime.path().join(path);
        std::fs::create_dir_all(full.parent().expect("a parent")).expect("create dir");
        std::fs::write(full, body).expect("write plugin");
    }

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_neotmux"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("SHELL", "/bin/cat");
    cmd.env("HOME", isolation.path());
    cmd.env("XDG_CONFIG_HOME", isolation.path());
    cmd.env("NEOTMUX_RUNTIME", runtime.path());
    cmd.arg("--log");
    cmd.arg(isolation.path().join("neotmux.log"));
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let pid = child.process_id().expect("a pid");

    // The output is drained so the multiplexer is never blocked on a full pipe.
    let mut reader = pair.master.try_clone_reader().expect("reader");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
    });

    // The splits, the shells and the first frames are startup, not idle.
    std::thread::sleep(Duration::from_millis(1500));
    let before = cpu_seconds(pid);
    std::thread::sleep(window);
    let after = cpu_seconds(pid);

    child.kill().ok();
    child.wait().ok();
    after - before
}

/// The cost control has to be doing something. A plugin that asks about every pane on every
/// tick must cost close to what no plugin costs, and taking the gate out has to be visible —
/// otherwise the gate is decoration and a watcher is paying for every pane's screen sixty
/// times a second.
///
/// The claim is about the *plugin's* share, not the absolute seconds, which belong to
/// whatever runner this is and to an unoptimised test build.
#[test]
fn reading_every_pane_every_tick_costs_about_nothing_while_the_session_is_idle() {
    const WINDOW: Duration = Duration::from_secs(3);

    let base = idle_cpu(&[("init.lua", SPLITTER)], WINDOW);
    let gated = idle_cpu(&[("init.lua", &format!("{SPLITTER}\n{GATED}"))], WINDOW);
    let ungated = idle_cpu(&[("init.lua", &format!("{SPLITTER}\n{UNGATED}"))], WINDOW);
    println!("idle cpu over {WINDOW:?}: none {base:.3}s gated {gated:.3}s ungated {ungated:.3}s");

    let with_gate = gated - base;
    let without = ungated - base;
    assert!(
        without > 0.0 && with_gate <= without / 2.0,
        "the revision gate saved nothing: {with_gate:.3}s of CPU over {WINDOW:?} with it, \
         {without:.3}s without, over a {base:.3}s baseline"
    );
    // A loose absolute floor under the ratio, so a run where both are noise cannot pass.
    assert!(
        with_gate <= 0.25,
        "reading every pane every tick cost {with_gate:.3}s of CPU over {WINDOW:?} above the \
         {base:.3}s baseline"
    );
}
