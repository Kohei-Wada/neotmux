use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use neotmux_lua::ops::Level;

static LOG_FILE: OnceLock<Mutex<fs::File>> = OnceLock::new();

fn default_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("neotmux/neotmux.log");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local/state/neotmux/neotmux.log")
}

pub fn init(path: Option<&Path>) -> anyhow::Result<PathBuf> {
    let resolved = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&resolved)?;
    LOG_FILE
        .set(Mutex::new(file))
        .map_err(|_| anyhow::anyhow!("logging already initialised"))?;
    Ok(resolved)
}

fn level_tag(level: Level) -> &'static str {
    match level {
        Level::Info => "INFO",
        Level::Warn => "WARN",
        Level::Error => "ERROR",
    }
}

pub fn log(level: Level, tag: &str, text: &str) {
    let Some(mutex) = LOG_FILE.get() else {
        return;
    };
    let Ok(mut file) = mutex.lock() else {
        return;
    };
    let _ = writeln!(file, "[{}] {tag}: {text}", level_tag(level));
}
