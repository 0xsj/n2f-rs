/// Shared categories; transport status codes belong to transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Internal,
    Invalid,
    NotFound,
    Conflict,
    Unauthenticated,
    Forbidden,
    RateLimited,
    Unavailable,
    Timeout,
    Canceled,
}
impl Kind {
    pub const ALL: [Self; 10] = [
        Self::Internal,
        Self::Invalid,
        Self::NotFound,
        Self::Conflict,
        Self::Unauthenticated,
        Self::Forbidden,
        Self::RateLimited,
        Self::Unavailable,
        Self::Timeout,
        Self::Canceled,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::Invalid => "invalid",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::RateLimited => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Canceled => "canceled",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}
