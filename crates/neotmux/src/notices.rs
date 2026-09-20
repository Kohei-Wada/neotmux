use neotmux_proto::Level;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;

use neotmux_proto::frame::Notice;

fn fg(level: Level) -> Color {
    match level {
        Level::Error => Color::Red,
        Level::Warn => Color::Yellow,
        Level::Info => Color::Cyan,
    }
}

/// Draws a frame's notifications over the top-right of the surface, one to a row, newest
/// first. A host surface: a source's own draw buffer cannot carry a notification about the
/// Lua layer itself failing.
pub fn render(frame: &mut Frame, area: Rect, notices: &[Notice]) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    for (row, item) in notices.iter().enumerate() {
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use neotmux::notify::Notifications;

    fn draw(notifications: &Notifications) -> ratatui::buffer::Buffer {
        let notices = notifications.visible();
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &notices))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn a_pushed_notification_is_drawn_top_right() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        let buf = draw(&n);
        let line: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(line.trim_end().ends_with("clock: hi"), "{line:?}");
    }

    #[test]
    fn an_expired_notification_is_not_drawn() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        n.expire(Instant::now() + Duration::from_secs(10));
        let buf = draw(&n);
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), " ");
        }
    }

    #[test]
    fn a_fresh_notification_survives_expiry() {
        let mut n = Notifications::new();
        n.push(Level::Info, "clock", "hi");
        n.expire(Instant::now());
        let buf = draw(&n);
        let line: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(line.trim_end().ends_with("clock: hi"), "{line:?}");
    }
}
