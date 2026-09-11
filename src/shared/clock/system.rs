use std::time::{Duration, Instant, SystemTime};

/// Production wall time and a private monotonic origin.
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
    pub fn now(&self) -> SystemTime {
        SystemTime::now()
    }
    pub fn elapsed(&self) -> Duration {
        self.origin.elapsed()
    }
}
impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}
