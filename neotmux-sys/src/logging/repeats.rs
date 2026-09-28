use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a line that keeps repeating is held back before it is written again, with the count
/// of what was held: often enough to show it is still happening, rarely enough that a plugin
/// failing on every tick costs a line a minute rather than four a second.
pub(crate) const REPEAT_EVERY: Duration = Duration::from_secs(60);

struct Seen {
    text: String,
    held: u64,
    written: Instant,
}

/// The last line each tag wrote, so a line that only repeats it is counted rather than written.
/// Per tag, not per log: two plugins failing on alternate ticks are two repeats, not a
/// sequence of changes.
#[derive(Default)]
pub(crate) struct Repeats {
    by_tag: HashMap<String, Seen>,
}

impl Repeats {
    /// What to write for `text` from `tag` at `now`, oldest first: nothing while it repeats the
    /// tag's last line and that was written less than [`REPEAT_EVERY`] ago; otherwise a note of
    /// how many repeats were held back, if any were, then the line itself.
    pub(crate) fn admit(&mut self, tag: &str, text: &str, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(seen) = self.by_tag.get_mut(tag) {
            let same = seen.text == text;
            if same && now.duration_since(seen.written) < REPEAT_EVERY {
                seen.held += 1;
                return out;
            }
            if seen.held > 0 {
                out.push(format!("the line before repeated {} more times", seen.held));
            }
        }
        out.push(text.to_string());
        self.by_tag.insert(
            tag.to_string(),
            Seen {
                text: text.to_string(),
                held: 0,
                written: now,
            },
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_line_is_written() {
        let mut repeats = Repeats::default();
        assert_eq!(repeats.admit("a", "boom", Instant::now()), vec!["boom"]);
    }

    /// A plugin raising the same error on every tick: one line, then silence, then — when it says
    /// something else — how many it held back, before the new line.
    #[test]
    fn a_line_repeated_is_counted_and_said_when_the_tag_moves_on() {
        let mut repeats = Repeats::default();
        let now = Instant::now();
        assert_eq!(repeats.admit("a", "boom", now), vec!["boom"]);
        for _ in 0..3 {
            assert!(repeats.admit("a", "boom", now).is_empty());
        }
        assert_eq!(
            repeats.admit("a", "fine", now),
            vec!["the line before repeated 3 more times", "fine"]
        );
    }

    #[test]
    fn a_line_still_repeating_a_minute_on_is_written_again_with_its_count() {
        let mut repeats = Repeats::default();
        let start = Instant::now();
        repeats.admit("a", "boom", start);
        assert!(repeats.admit("a", "boom", start).is_empty());
        let later = start + REPEAT_EVERY;
        assert_eq!(
            repeats.admit("a", "boom", later),
            vec!["the line before repeated 1 more times", "boom"]
        );
    }

    /// Two plugins failing on alternate ticks are two repeats, not a line changing every time.
    #[test]
    fn each_tag_repeats_on_its_own() {
        let mut repeats = Repeats::default();
        let now = Instant::now();
        repeats.admit("a", "boom", now);
        repeats.admit("b", "bang", now);
        assert!(repeats.admit("a", "boom", now).is_empty());
        assert!(repeats.admit("b", "bang", now).is_empty());
    }
}
