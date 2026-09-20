use std::collections::VecDeque;
use std::io::Write;
use std::time::{Duration, Instant};

use neotmux_proto::Level;

use crate::logging;
use neotmux_proto::frame::Notice;

const TTL: Duration = Duration::from_secs(4);
const MAX_VISIBLE: usize = 5;
/// The exit transcript is held for the life of the session, so a plugin notifying every
/// frame would otherwise grow it without bound. Oldest lines go first.
const MAX_TRANSCRIPT: usize = 1000;

struct Notification {
    level: Level,
    tag: String,
    text: String,
    created: Instant,
}

/// A host-owned surface for notifications: a source's own draw buffer can't
/// carry a notification about the Lua layer itself failing. Expires entries a few seconds
/// after they were pushed and hands the rest to the frame that carries them.
#[derive(Default)]
pub struct Notifications {
    items: Vec<Notification>,
    transcript: VecDeque<String>,
    dropped: usize,
}

impl Notifications {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, level: Level, tag: &str, text: &str) {
        self.transcript.push_back(format!("{tag}: {text}"));
        if self.transcript.len() > MAX_TRANSCRIPT {
            self.transcript.pop_front();
            self.dropped += 1;
            if self.dropped == 1 {
                logging::log(
                    Level::Warn,
                    "neotmux",
                    &format!(
                        "the exit transcript is full at {MAX_TRANSCRIPT} lines; \
                         older notifications are being dropped from it"
                    ),
                );
            }
        }
        self.items.push(Notification {
            level,
            tag: tag.to_string(),
            text: text.to_string(),
            created: Instant::now(),
        });
    }

    /// Every notification raised this session, replayed once the terminal has been restored.
    /// A full-screen TUI cannot use stderr while it is running, so without this a notification
    /// that expired, scrolled past or was never drawn at all is only ever in the log file.
    pub fn flush(&self, out: &mut impl Write) {
        if self.dropped > 0 {
            let _ = writeln!(
                out,
                "neotmux: {} earlier notifications were dropped from this transcript \
                 (capped at {MAX_TRANSCRIPT})",
                self.dropped
            );
        }
        for line in &self.transcript {
            let _ = writeln!(out, "neotmux: {line}");
        }
    }

    pub fn expire(&mut self, now: Instant) {
        self.items
            .retain(|item| now.duration_since(item.created) < TTL);
    }

    /// The notifications a frame carries, newest first and capped at what one surface can
    /// be expected to show. How many there are is this side's business; where they go is
    /// whoever draws them.
    pub fn visible(&self) -> Vec<Notice> {
        self.items
            .iter()
            .rev()
            .take(MAX_VISIBLE)
            .map(|item| Notice {
                level: item.level,
                tag: item.tag.clone(),
                text: item.text.clone(),
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_replays_every_notification_including_expired_ones() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        n.push(Level::Error, "clock", "bye");
        n.expire(Instant::now() + Duration::from_secs(10));
        let mut out = Vec::new();
        n.flush(&mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "neotmux: clock: hi\nneotmux: clock: bye\n"
        );
    }

    #[test]
    fn flush_writes_nothing_when_nothing_was_notified() {
        let mut out = Vec::new();
        Notifications::new().flush(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn the_transcript_is_capped_and_says_so_when_it_drops_lines() {
        let mut n = Notifications::new();
        for i in 0..MAX_TRANSCRIPT + 3 {
            n.push(Level::Info, "loud", &format!("line {i}"));
        }
        let mut out = Vec::new();
        n.flush(&mut out);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), MAX_TRANSCRIPT + 1, "{text}");
        assert!(
            text.lines()
                .next()
                .unwrap()
                .contains("3 earlier notifications were dropped"),
            "{text}"
        );
        assert!(
            !text.contains("line 2\n"),
            "the oldest line survived the cap"
        );
        assert!(text.contains("line 1002"), "the newest line was dropped");
    }
}
