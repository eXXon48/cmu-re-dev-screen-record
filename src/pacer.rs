//! Fixed-rate frame clock on absolute deadlines.

use std::time::{Duration, Instant};

pub struct Pacer {
    period: Duration,
    due: Instant,
}

impl Pacer {
    pub fn new(fps: u32, start: Instant) -> Pacer {
        Pacer { period: Duration::from_nanos(1_000_000_000 / fps as u64), due: start }
    }

    /// Call before each frame: how long to sleep, and how many frames were missed.
    /// More than one period late -> skip the missed slots and restart the clock from `now`.
    pub fn next(&mut self, now: Instant) -> (Duration, u64) {
        if now < self.due {
            let sleep = self.due - now;
            self.due += self.period;
            return (sleep, 0);
        }
        let late = now - self.due;
        if late < self.period {
            self.due += self.period;
            return (Duration::ZERO, 0);
        }
        self.due = now + self.period;
        (Duration::ZERO, (late.as_nanos() / self.period.as_nanos()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Duration = Duration::from_nanos(1_000_000_000 / 24);

    #[test]
    fn pacer_on_time() {
        let t0 = Instant::now();
        let mut p = Pacer::new(24, t0);
        assert_eq!(p.next(t0), (Duration::ZERO, 0)); // first frame at once
        assert_eq!(p.next(t0 + Duration::from_millis(5)), (P - Duration::from_millis(5), 0));
        assert_eq!(p.next(t0 + P), (P, 0));
    }

    #[test]
    fn pacer_small_lateness_is_kept() {
        let t0 = Instant::now();
        let mut p = Pacer::new(24, t0);
        p.next(t0);
        // half a period late: no drop, frame at once, the next slot stays on the grid
        assert_eq!(p.next(t0 + P + P / 2), (Duration::ZERO, 0));
        assert_eq!(p.next(t0 + P + P / 2), (P / 2, 0));
    }

    #[test]
    fn pacer_resyncs_after_stall() {
        let t0 = Instant::now();
        let mut p = Pacer::new(24, t0);
        p.next(t0);
        let late = t0 + P * 10;
        assert_eq!(p.next(late), (Duration::ZERO, 9));
        assert_eq!(p.next(late), (P, 0));
    }
}
