use crate::shared::errors::{Failure, Kind};
pub use crate::shared::telemetry::Outcome;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    ResponseCompleted,
    PeerClosed,
    Deadline,
    HandlerError,
    WriteError,
    Abandoned,
}
#[derive(Debug, Clone, Copy)]
pub struct CompletionFacts {
    pub status: Option<u16>,
    pub failure_kind: Option<Kind>,
    pub termination: Termination,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub outcome: Outcome,
    pub span_error: bool,
    pub error_type: Option<String>,
}
pub fn normalize_method(value: &str) -> &str {
    match value {
        "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "CONNECT" | "OPTIONS" | "TRACE" | "PATCH" => {
            value
        }
        _ => "_OTHER",
    }
}
pub fn classify_completion(f: CompletionFacts) -> Result<Classification, Failure> {
    if f.status.is_some_and(|s| !(200..=599).contains(&s))
        || (f.termination == Termination::ResponseCompleted && f.status.is_none())
    {
        return Err(Failure::new(Kind::Invalid, "invalid HTTP completion")
            .with_type("http.invalid_completion"));
    }
    let (outcome, reason) = match f.termination {
        Termination::PeerClosed => (Outcome::Canceled, Some("canceled")),
        Termination::Deadline => (Outcome::TimedOut, Some("timeout")),
        Termination::HandlerError => (Outcome::Failed, Some("handler_error")),
        Termination::WriteError => (Outcome::Failed, Some("write_error")),
        Termination::Abandoned => (Outcome::Failed, Some("abandoned")),
        Termination::ResponseCompleted => match f.failure_kind {
            Some(Kind::Timeout) => (Outcome::TimedOut, Some("timeout")),
            Some(Kind::Canceled) => (Outcome::Canceled, Some("canceled")),
            Some(Kind::Internal | Kind::Unavailable) => (Outcome::Failed, None),
            Some(_) => (Outcome::Refused, None),
            None if f.status.unwrap() >= 500 => (Outcome::Failed, None),
            None if f.status.unwrap() >= 400 => (Outcome::Refused, None),
            None => (Outcome::Success, None),
        },
    };
    let span_error = f.termination != Termination::ResponseCompleted
        || f.status.is_some_and(|s| s >= 500)
        || matches!(
            outcome,
            Outcome::Failed | Outcome::TimedOut | Outcome::Canceled
        );
    let error_type = reason.map(str::to_owned).or_else(|| {
        span_error.then(|| match f.status {
            Some(s) if s >= 500 => s.to_string(),
            _ => "handler_error".into(),
        })
    });
    Ok(Classification {
        outcome,
        span_error,
        error_type,
    })
}

impl Termination {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResponseCompleted => "response_completed",
            Self::PeerClosed => "peer_closed",
            Self::Deadline => "deadline",
            Self::HandlerError => "handler_error",
            Self::WriteError => "write_error",
            Self::Abandoned => "abandoned",
        }
    }
}
