use crate::shared::errors::{Failure, Kind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSnapshot {
    pub trace_id: String,
    pub span_id: String,
    pub sampled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRef {
    trace_id: String,
    span_id: String,
    sampled: bool,
}

impl TraceRef {
    pub fn parse(trace_id: &str, span_id: &str, sampled: bool) -> Result<Self, Failure> {
        if !valid(trace_id, 32) || !valid(span_id, 16) {
            return Err(Failure::new(Kind::Invalid, "invalid telemetry context")
                .with_type("telemetry.invalid_context"));
        }
        Ok(Self {
            trace_id: trace_id.to_owned(),
            span_id: span_id.to_owned(),
            sampled,
        })
    }
    pub fn snapshot(&self) -> TraceSnapshot {
        TraceSnapshot {
            trace_id: self.trace_id.clone(),
            span_id: self.span_id.clone(),
            sampled: self.sampled,
        }
    }
}
fn valid(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && value.bytes().any(|b| b != b'0')
}
