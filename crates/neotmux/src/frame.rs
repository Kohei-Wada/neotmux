use std::time::{Duration, Instant};

/// How long one pass of the frame loop lasts.
pub const FRAME: Duration = Duration::from_millis(16);

/// Sleeps out whatever is left of the frame. The loop's rate has to be a property of the
/// loop: leaving it to the input wait means any wait that returns sooner than it was asked
/// to — a terminal that hung up, a library that decides not to block — free-runs the CPU.
pub fn sleep_out(frame_end: Instant) {
    std::thread::sleep(frame_end.saturating_duration_since(Instant::now()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleeping_out_a_frame_returns_no_earlier_than_its_end() {
        let end = Instant::now() + Duration::from_millis(30);
        sleep_out(end);
        assert!(
            Instant::now() >= end,
            "the frame's remainder was not slept out"
        );
    }

    #[test]
    fn sleeping_out_a_frame_that_is_already_over_returns_at_once() {
        let started = Instant::now();
        sleep_out(started - Duration::from_secs(1));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "a frame already over should not be waited on: took {:?}",
            started.elapsed()
        );
    }
}
