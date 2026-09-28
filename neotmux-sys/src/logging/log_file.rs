use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// How big the log may grow before it is moved aside. A day of a plugin failing on every tick
/// once came to 200 MB; a log is for reading back the last while, not for keeping everything.
pub(crate) const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// The log file, moved to `<path>.1` — replacing the one there — once it passes its limit, and
/// started afresh. One old file is kept, so the lines just before a rotation are still there.
pub(crate) struct LogFile {
    path: PathBuf,
    file: File,
    bytes: u64,
    max: u64,
}

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl LogFile {
    pub(crate) fn open(path: &Path, max: u64) -> io::Result<Self> {
        let file = open(path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            file,
            bytes,
            max,
        })
    }

    pub(crate) fn write_line(&mut self, line: &str) -> io::Result<()> {
        if self.bytes >= self.max {
            self.rotate()?;
        }
        writeln!(self.file, "{line}")?;
        self.bytes += line.len() as u64 + 1;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        let mut old = self.path.clone().into_os_string();
        old.push(".1");
        fs::rename(&self.path, &old)?;
        self.file = open(&self.path)?;
        self.bytes = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_past_its_limit_is_moved_aside_and_started_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("neotmux.log");
        let mut log = LogFile::open(&path, 20).unwrap();
        log.write_line("0123456789").unwrap();
        log.write_line("0123456789").unwrap();
        log.write_line("after").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "after\n");
        assert_eq!(
            fs::read_to_string(dir.path().join("neotmux.log.1")).unwrap(),
            "0123456789\n0123456789\n"
        );
    }

    /// A log that was already big when neotmux started counts from its size, not from zero.
    #[test]
    fn an_existing_log_counts_from_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("neotmux.log");
        fs::write(&path, "x".repeat(30)).unwrap();
        let mut log = LogFile::open(&path, 20).unwrap();
        log.write_line("new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
    }

    #[test]
    fn only_one_old_log_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("neotmux.log");
        let mut log = LogFile::open(&path, 5).unwrap();
        for line in ["first", "second", "third"] {
            log.write_line(line).unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "third\n");
        assert_eq!(
            fs::read_to_string(dir.path().join("neotmux.log.1")).unwrap(),
            "second\n"
        );
    }
}
