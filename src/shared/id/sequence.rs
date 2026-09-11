use super::Id;
use crate::shared::errors::{Failure, Kind};

/// Finite owned fixtures, including intentional duplicates. No clock or entropy.
pub struct Sequence {
    ids: Vec<Id>,
    next: usize,
}
impl Sequence {
    pub fn new(ids: &[Id]) -> Self {
        Self {
            ids: ids.to_vec(),
            next: 0,
        }
    }
    pub fn new_id(&mut self) -> Result<Id, Failure> {
        let value = self.ids.get(self.next).copied().ok_or_else(|| {
            Failure::new(Kind::Unavailable, "ID sequence exhausted")
                .with_type("id.sequence_exhausted")
        })?;
        self.next += 1;
        Ok(value)
    }
}
