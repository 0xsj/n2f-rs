use crate::shared::errors::{Kind, PublicInfo};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub problem_type: String,
    pub title: String,
    pub detail: String,
    pub kind: String,
    pub status: u16,
    pub code: Option<String>,
    pub fields: BTreeMap<String, String>,
}
pub fn problem_of(failure: Option<&PublicInfo>) -> Option<Problem> {
    let mut i = failure?.clone();
    if i.kind == Kind::Internal {
        i.message = "internal error".into();
        i.error_type = None;
        i.fields.clear();
    } else if i.message.is_empty() {
        i.message = "request failed".into();
    }
    let s = match i.kind {
        Kind::Invalid => 400,
        Kind::NotFound => 404,
        Kind::Conflict => 409,
        Kind::Unauthenticated => 401,
        Kind::Forbidden => 403,
        Kind::RateLimited => 429,
        Kind::Unavailable | Kind::Timeout | Kind::Canceled => 503,
        _ => 500,
    };
    Some(Problem {
        problem_type: format!("urn:n2f:problem:{}", i.kind.as_str()),
        title: match s {
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            409 => "Conflict",
            429 => "Too Many Requests",
            503 => "Service Unavailable",
            _ => "Internal Server Error",
        }
        .into(),
        detail: i.message,
        kind: i.kind.as_str().into(),
        status: s,
        code: i.error_type,
        fields: i.fields,
    })
}

impl Problem {
    pub fn json(&self) -> serde_json::Value {
        let mut v = serde_json::json!({"type":self.problem_type,"title":self.title,"status":self.status,"detail":self.detail,"kind":self.kind});
        if let Some(code) = &self.code {
            v["code"] = code.clone().into();
        }
        if !self.fields.is_empty() {
            v["fields"] = serde_json::json!(self.fields);
        }
        v
    }
}
