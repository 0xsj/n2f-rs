use crate::shared::errors::{Failure, Kind};
pub(super) fn invalid(code: &str) -> Failure {
    Failure::new(Kind::Invalid, "invalid provenance").with_type(format!("provenance.{code}"))
}
pub(super) fn identity_valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorKind {
    Anonymous,
    User,
    Service,
    System,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    kind: ActorKind,
    identity: String,
}
impl Actor {
    pub fn new(kind: ActorKind, identity: String) -> Result<Self, Failure> {
        if (kind == ActorKind::Anonymous && !identity.is_empty())
            || (kind != ActorKind::Anonymous && !identity_valid(&identity))
        {
            return Err(invalid("invalid_actor"));
        }
        Ok(Self { kind, identity })
    }
    pub fn anonymous() -> Self {
        Self {
            kind: ActorKind::Anonymous,
            identity: String::new(),
        }
    }
    pub fn kind(&self) -> ActorKind {
        self.kind
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub(super) fn named(&self) -> bool {
        self.kind != ActorKind::Anonymous
    }
}
