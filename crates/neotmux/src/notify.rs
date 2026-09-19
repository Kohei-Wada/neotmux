use std::io::Write;
use std::time::{Duration, Instant};

use neotmux_lua::ops::Level;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;

const TTL: Duration = Duration::from_secs(4);
const MAX_VISIBLE: usize = 5;

struct Notification {
    level: Level,
    tag: String,
    text: String,
    created: Instant,
}

/// A host-owned surface for notifications: a source's own draw buffer can't
/// carry a notification about the Lua layer itself failing. Expires
/// entries a few seconds after they were pushed and draws the rest over the
/// top-right of the surface.
#[derive(Default)]
pub struct Notifications {
    items: Vec<Notification>,
    transcript: Vec<String>,
}

impl Notifications {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, level: Level, tag: &str, text: &str) {
        self.transcript.push(format!("{tag}: {text}"));
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
        for line in &self.transcript {
            let _ = writeln!(out, "neotmux: {line}");
        }
    }

    /// Whether there's anything showing that will need a later frame purely
    /// to notice it has expired and stop drawing it.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn expire(&mut self, now: Instant) {
        self.items
            .retain(|item| now.duration_since(item.created) < TTL);
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let fg = |level: Level| match level {
            Level::Error => Color::Red,
            Level::Warn => Color::Yellow,
            Level::Info => Color::Cyan,
        };

        let buf = frame.buffer_mut();
        for (row, item) in self.items.iter().rev().take(MAX_VISIBLE).enumerate() {
            let row = row as u16;
            if row >= area.height {
                break;
            }
            let line = format!("{}: {}", item.tag, item.text);
            let start = line.chars().count().min(usize::from(area.width));
            let text: String = line.chars().take(start).collect();
            let x0 = area.x + area.width - u16::try_from(start).unwrap_or(area.width);
            let y = area.y + row;
            for (i, ch) in text.chars().enumerate() {
                let x = x0 + i as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    let mut tmp = [0u8; 4];
                    cell.set_symbol(ch.encode_utf8(&mut tmp));
                    cell.set_fg(fg(item.level));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn render(notifications: &Notifications) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
        terminal
            .draw(|frame| notifications.render(frame, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn a_pushed_notification_is_drawn_top_right() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        let buf = render(&n);
        let line: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(line.trim_end().ends_with("clock: hi"), "{line:?}");
    }

    #[test]
    fn an_expired_notification_is_not_drawn() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        n.expire(Instant::now() + Duration::from_secs(10));
        let buf = render(&n);
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), " ");
        }
    }

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
    fn a_fresh_notification_survives_expiry() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        n.expire(Instant::now());
        let buf = render(&n);
        let line: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(line.trim_end().ends_with("clock: hi"), "{line:?}");
    }
}
