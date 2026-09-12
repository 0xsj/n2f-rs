use super::{Reference, ReferenceKind, actor::invalid};
use crate::shared::errors::Failure;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Input,
    ReplayOf,
    PreviousAttempt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub relation: Relation,
    pub target: Reference,
}
#[derive(Clone, Debug)]
pub struct LinkSet(Vec<Link>);
impl LinkSet {
    pub fn new(input: Vec<Link>) -> Result<Self, Failure> {
        if input.len() > 32 {
            return Err(invalid("too_many_links"));
        }
        let mut out = Vec::new();
        for l in input {
            if l.relation == Relation::PreviousAttempt && l.target.kind() != ReferenceKind::Scope {
                return Err(invalid("invalid_reference"));
            }
            if !out.contains(&l) {
                out.push(l)
            }
        }
        Ok(Self(out))
    }
    pub fn values(&self) -> &[Link] {
        &self.0
    }
}
