use std::collections::VecDeque;
use std::time::{Duration, Instant};

use neotmux_proto::Level;
use neotmux_proto::screen::Notice;
use neotmux_sys::logging;

const TTL: Duration = Duration::from_secs(4);
const MAX_VISIBLE: usize = 5;
/// The transcript is held for the life of the session, so a plugin notifying every frame
/// would otherwise grow it without bound. Oldest lines go first.
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
    /// Everything ever pushed, the dropped included: what `since` counts from, so a mark
    /// stays meaningful after the cap has thrown the line it pointed at away.
    pushed: usize,
}

impl Notifications {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, level: Level, tag: &str, text: &str) {
        self.transcript.push_back(format!("{tag}: {text}"));
        self.pushed += 1;
        if self.transcript.len() > MAX_TRANSCRIPT {
            self.transcript.pop_front();
            self.dropped += 1;
            if self.dropped == 1 {
                logging::log(
                    Level::Warn,
                    "neotmux",
                    &format!(
                        "the transcript is full at {MAX_TRANSCRIPT} lines; \
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

    /// Everything raised since `mark`. The one caller asks from zero: a client whose session
    /// ended while it was loading has no screen to be told on, so it is told on stderr.
    pub fn since(&self, mark: usize) -> Vec<String> {
        let oldest = self.pushed - self.transcript.len();
        let from = mark.saturating_sub(oldest);
        self.transcript.iter().skip(from).cloned().collect()
    }

    /// Everything showing, taken down. The transcript is untouched: what was raised was
    /// raised, and the exit replay is the record of it.
    pub fn clear(&mut self) {
        self.items.clear();
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
    fn a_mark_replays_only_what_was_raised_after_it() {
        let mut n = Notifications::new();
        n.push(Level::Info, "a", "one");
        n.push(Level::Info, "b", "two");
        assert_eq!(n.since(1), vec!["b: two".to_string()]);
        assert_eq!(
            n.since(0),
            vec!["a: one".to_string(), "b: two".to_string()],
            "a mark of zero is the whole of it"
        );
    }

    /// A mark older than the oldest line still held is not an index into nothing: what is
    /// left of the transcript is what that client is told.
    #[test]
    fn a_mark_the_cap_has_overtaken_replays_what_is_left() {
        let mut n = Notifications::new();
        for i in 0..MAX_TRANSCRIPT + 3 {
            n.push(Level::Info, "loud", &format!("line {i}"));
        }
        let lines = n.since(0);
        assert_eq!(lines.len(), MAX_TRANSCRIPT);
        assert_eq!(lines[0], "loud: line 3");
    }
}
