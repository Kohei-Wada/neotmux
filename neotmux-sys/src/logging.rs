use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use neotmux_proto::Level;

mod log_file;
mod repeats;

use log_file::{LogFile, MAX_BYTES};
use repeats::Repeats;

/// The file, and the last warning or error each tag wrote to it: one that only repeats is
/// counted, not written, and the file is moved aside past its limit, so nothing a plugin does
/// on every tick can fill the disk.
static LOG_FILE: OnceLock<Mutex<(LogFile, Repeats)>> = OnceLock::new();

fn non_empty_absolute(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn default_path() -> PathBuf {
    if let Some(xdg) = non_empty_absolute("XDG_STATE_HOME") {
        return xdg.join("neotmux/neotmux.log");
    }
    if let Some(home) = non_empty_absolute("HOME") {
        return home.join(".local/state/neotmux/neotmux.log");
    }
    PathBuf::from("/tmp/neotmux/neotmux.log")
}

pub fn init(path: Option<&Path>) -> anyhow::Result<PathBuf> {
    let resolved = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = LogFile::open(&resolved, MAX_BYTES)?;
    LOG_FILE
        .set(Mutex::new((file, Repeats::default())))
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

// Howard Hinnant's civil_from_days: days since the Unix epoch to a (year, month, day) civil
// date, avoiding a chrono/time dependency for a one-line log timestamp.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let time_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day / 60) % 60,
        time_of_day % 60
    )
}

pub fn log(level: Level, tag: &str, text: &str) {
    let Some(mutex) = LOG_FILE.get() else {
        return;
    };
    let Ok(mut guard) = mutex.lock() else {
        return;
    };
    let (file, repeats) = &mut *guard;
    // Only what went wrong is counted: an info line is what a plugin chose to say, and one that
    // says the same thing every second is saying it on purpose.
    let lines = match level {
        Level::Info => vec![text.to_string()],
        Level::Warn | Level::Error => repeats.admit(tag, text, Instant::now()),
    };
    for line in lines {
        let _ = file.write_line(&format!(
            "{} [{}] {tag}: {line}",
            timestamp(),
            level_tag(level)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn timestamp_has_the_expected_shape() {
        let ts = timestamp();
        assert_eq!(ts.len(), 20, "{ts}");
        assert_eq!(ts.as_bytes()[4], b'-');
        assert_eq!(ts.as_bytes()[7], b'-');
        assert_eq!(ts.as_bytes()[10], b' ');
        assert_eq!(ts.as_bytes()[13], b':');
        assert_eq!(ts.as_bytes()[16], b':');
        assert_eq!(
            ts.as_bytes()[19],
            b'Z',
            "{ts} is missing its UTC zone marker"
        );
    }
}
