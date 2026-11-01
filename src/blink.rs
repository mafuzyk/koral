use std::time::{Duration, Instant};

const HALF_PERIOD: Duration = Duration::from_millis(530);
const STOP_AFTER: Duration = Duration::from_secs(15);

/// The cursor's on and off phases, counted from the last reset.
#[derive(Debug, Clone, Copy)]
pub struct Blink {
    start: Instant,
}

impl Blink {
    pub fn new(now: Instant) -> Self {
        Self { start: now }
    }

    pub fn reset(&mut self, now: Instant) {
        self.start = now;
    }

    pub fn visible(&self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.start);
        elapsed >= STOP_AFTER || (elapsed.as_millis() / HALF_PERIOD.as_millis()).is_multiple_of(2)
    }

    pub fn next(&self, now: Instant) -> Option<Instant> {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= STOP_AFTER {
            return None;
        }
        let phases = (elapsed.as_millis() / HALF_PERIOD.as_millis()) as u32 + 1;
        Some(self.start + HALF_PERIOD * phases)
    }
}
