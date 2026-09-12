use super::{Reference, ReferenceKind};
use crate::shared::id::Id;
pub struct IncomingCause<'a> {
    pub kind: &'a str,
    pub id: &'a str,
}
pub struct IncomingHints<'a> {
    pub correlation: Option<&'a str>,
    pub causation: Option<IncomingCause<'a>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Fresh,
    Continued,
    Restarted,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub field: &'static str,
    pub reason: &'static str,
}
#[derive(Clone, Debug)]
pub struct IncomingResult {
    pub(super) decision: Disposition,
    pub(super) correlation: Option<Id>,
    pub(super) cause: Option<Reference>,
    issues: Vec<Issue>,
}
impl IncomingResult {
    pub fn decision(&self) -> Disposition {
        self.decision
    }
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }
    pub fn correlation(&self) -> Option<Id> {
        self.correlation
    }
    pub fn cause(&self) -> Option<Reference> {
        self.cause
    }
}
pub fn inspect_incoming(h: IncomingHints<'_>) -> IncomingResult {
    let mut i = IncomingResult {
        decision: Disposition::Fresh,
        correlation: None,
        cause: None,
        issues: Vec::new(),
    };
    if h.correlation.is_none() && h.causation.is_none() {
        return i;
    }
    i.decision = Disposition::Restarted;
    if let Some(s) = h.correlation {
        match Id::parse(s) {
            Ok(v) => {
                i.correlation = Some(v);
                i.decision = Disposition::Continued
            }
            Err(_) => i.issues.push(Issue {
                field: "correlation",
                reason: "invalid_id",
            }),
        }
    }
    if let Some(c) = h.causation {
        let kind = match c.kind {
            "scope" => Some(ReferenceKind::Scope),
            "work" => Some(ReferenceKind::Work),
            "event" => Some(ReferenceKind::Event),
            _ => None,
        };
        let reason = if let Some(k) = kind {
            match Id::parse(c.id) {
                Err(_) => Some("invalid_id"),
                Ok(_) if i.correlation.is_none() => Some("missing_correlation"),
                Ok(id) => {
                    i.cause = Some(Reference::new(k, id).expect("validated reference"));
                    None
                }
            }
        } else {
            Some("invalid_kind")
        };
        if let Some(reason) = reason {
            i.issues.push(Issue {
                field: "causation",
                reason,
            })
        }
    }
    i
}
