use std::{
    error::Error,
    fmt,
    sync::Mutex,
    time::{Duration, SystemTime},
};

/// A test/configuration mistake: the requested advance cannot be represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvanceError;
impl fmt::Display for AdvanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("clock: invalid advance")
    }
}
impl Error for AdvanceError {}

struct State {
    wall: SystemTime,
    elapsed: Duration,
}

/// Share through Arc when needed. Wall correction never resets elapsed time.
/// Lock poisoning signals an unexpected programming panic, not a domain failure.
pub struct FakeClock {
    state: Mutex<State>,
}
impl FakeClock {
    pub fn new(start: SystemTime) -> Self {
        Self {
            state: Mutex::new(State {
                wall: start,
                elapsed: Duration::ZERO,
            }),
        }
    }
    pub fn now(&self) -> SystemTime {
        self.state.lock().expect("clock state poisoned").wall
    }
    pub fn elapsed(&self) -> Duration {
        self.state.lock().expect("clock state poisoned").elapsed
    }
    pub fn set(&self, wall: SystemTime) {
        self.state.lock().expect("clock state poisoned").wall = wall;
    }
    /// Commit both values only after both additions succeed.
    pub fn advance(&self, delta: Duration) -> Result<(), AdvanceError> {
        let mut state = self.state.lock().expect("clock state poisoned");
        let elapsed = state.elapsed.checked_add(delta).ok_or(AdvanceError)?;
        let wall = state.wall.checked_add(delta).ok_or(AdvanceError)?;
        state.wall = wall;
        state.elapsed = elapsed;
        Ok(())
    }
}
