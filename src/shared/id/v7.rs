use super::Id;
use crate::shared::entropy::{Entropy, OsEntropy};
use crate::shared::errors::{Failure, Kind};
use std::time::{SystemTime, UNIX_EPOCH};

/// One generator's ordering state. Exclusive mutable access prevents racing
/// transitions; sharing/synchronization belongs to its caller. Not Clone.
pub struct V7<C, E = OsEntropy> {
    clock: C,
    entropy: E,
    state: Option<(u64, u16)>,
}
impl<C> V7<C, OsEntropy>
where
    C: Fn() -> SystemTime,
{
    pub fn new(clock: C) -> Self {
        Self::with_entropy(clock, OsEntropy)
    }
}
impl<C, E> V7<C, E>
where
    C: Fn() -> SystemTime,
    E: Entropy,
{
    pub fn with_entropy(clock: C, entropy: E) -> Self {
        Self {
            clock,
            entropy,
            state: None,
        }
    }
    pub fn new_id(&mut self) -> Result<Id, Failure> {
        let duration = (self.clock)()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| time_range())?;
        let millis = duration.as_millis();
        if millis > ((1u128 << 48) - 1) {
            return Err(time_range());
        }
        let mut ms = millis as u64;
        let fresh = self.state.is_none_or(|(last, _)| ms > last);
        let mut counter = 0;
        if !fresh {
            let (last, previous) = self.state.expect("nonfresh state exists");
            ms = last;
            if previous == 4095 {
                return Err(Failure::new(Kind::Unavailable, "ID counter exhausted")
                    .with_type("id.exhausted"));
            }
            counter = previous + 1;
        }
        let mut random = [0u8; 10];
        self.entropy.fill(&mut random).map_err(|e| {
            Failure::new(Kind::Unavailable, "ID entropy unavailable")
                .with_type("id.entropy")
                .with_boxed_source(e)
        })?;
        if fresh {
            counter = u16::from_be_bytes([random[0], random[1]]) & 0x07ff;
        }
        let value = Id::v7(ms, counter, &random[2..]);
        self.state = Some((ms, counter));
        Ok(value)
    }
}
fn time_range() -> Failure {
    Failure::new(Kind::Invalid, "ID timestamp out of range").with_type("id.time_range")
}
