use crate::shared::{errors::Failure, id::Id};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    Scope,
    Work,
    Event,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    kind: ReferenceKind,
    id: Id,
}
impl Reference {
    pub fn new(kind: ReferenceKind, id: Id) -> Result<Self, Failure> {
        Ok(Self { kind, id })
    }
    pub fn kind(self) -> ReferenceKind {
        self.kind
    }
    pub fn id(self) -> Id {
        self.id
    }
}
