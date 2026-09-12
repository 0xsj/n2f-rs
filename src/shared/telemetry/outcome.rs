use crate::shared::errors::{Failure, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Refused,
    Failed,
    Canceled,
    TimedOut,
}
impl Outcome {
    pub fn parse(value: &str) -> Result<Self, Failure> {
        let outcome = match value {
            "success" => Self::Success,
            "refused" => Self::Refused,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            "timed_out" => Self::TimedOut,
            _ => {
                return Err(Failure::new(Kind::Invalid, "invalid telemetry outcome")
                    .with_type("telemetry.invalid_outcome"));
            }
        };
        Ok(outcome)
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::TimedOut => "timed_out",
        }
    }
}
