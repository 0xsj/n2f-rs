use super::{Classification, CompletionFacts, classify_completion};
use crate::shared::errors::{Failure, Kind};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;
#[derive(Debug, Clone)]
pub struct Completion {
    pub facts: CompletionFacts,
    pub classification: Classification,
    pub elapsed: Duration,
}
pub type Output = Box<dyn FnMut(&Completion) + Send>;
/// Request-local gate; the native adapter owns synchronization when shared.
pub struct Active {
    started: Duration,
    done: bool,
    failed_attempts: u64,
    outputs: Vec<Output>,
}
impl Active {
    pub fn begin(started: Duration, outputs: Vec<Output>) -> Self {
        Self {
            started,
            done: false,
            failed_attempts: 0,
            outputs,
        }
    }
    pub fn finish(&mut self, facts: CompletionFacts, now: Duration) -> Result<bool, Failure> {
        let classification = classify_completion(facts)?;
        if self.done {
            return Ok(false);
        }
        let elapsed = now.checked_sub(self.started).ok_or_else(|| {
            Failure::new(Kind::Invalid, "invalid observation duration")
                .with_type("telemetry.invalid_duration")
        })?;
        self.done = true;
        let completion = Completion {
            facts,
            classification,
            elapsed,
        };
        for output in &mut self.outputs {
            if catch_unwind(AssertUnwindSafe(|| output(&completion))).is_err() {
                self.failed_attempts += 1;
            }
        }
        Ok(true)
    }
    pub fn failed_attempts(&self) -> u64 {
        self.failed_attempts
    }
}
