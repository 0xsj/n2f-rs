//! Identity application operations against consumer-owned ports. See CONTRACT.md.
//!
//! Commands own their use case, its refusals and the safe outbox facts they
//! promise; the store ports own transactions. Every port is a small trait declared
//! beside the operations that consume it; nothing here imports an adapter.
#![doc = include_str!("CONTRACT.md")]

pub mod command;
pub mod query;

use crate::shared::{
    errors::{Failure, Kind},
    events::Envelope,
    id::Id,
    provenance::WorkContext,
};
use command::{Clock, IdSource};
use std::time::UNIX_EPOCH;

use super::domain::MAX_TIME_MS;

/// Safe admission projection (A11): identifiers and the admitted epoch only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub principal_id: Id,
    pub session_id: Id,
    pub auth_epoch: u32,
}

/// Product defaults validated once (U02). Values are milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub session_absolute_ms: i64,
    pub session_idle_ms: i64,
    pub verification_ttl_ms: i64,
    pub reset_ttl_ms: i64,
}
impl Config {
    pub fn defaults() -> Self {
        Self {
            session_absolute_ms: 12 * 3600 * 1000,
            session_idle_ms: 30 * 60 * 1000,
            verification_ttl_ms: 24 * 3600 * 1000,
            reset_ttl_ms: 15 * 60 * 1000,
        }
    }
    pub fn new(
        session_absolute_ms: i64,
        session_idle_ms: i64,
        verification_ttl_ms: i64,
        reset_ttl_ms: i64,
    ) -> Result<Self, Failure> {
        let in_range = |v: i64| (1..=MAX_TIME_MS).contains(&v);
        if !in_range(session_absolute_ms)
            || !in_range(session_idle_ms)
            || !in_range(verification_ttl_ms)
            || !in_range(reset_ttl_ms)
            || session_idle_ms > session_absolute_ms
        {
            return Err(
                Failure::new(Kind::Invalid, "invalid authentication configuration")
                    .with_type("identity.auth_configuration"),
            );
        }
        Ok(Self {
            session_absolute_ms,
            session_idle_ms,
            verification_ttl_ms,
            reset_ttl_ms,
        })
    }
}

pub(crate) fn dependency_failed(message: &'static str) -> Failure {
    Failure::new(Kind::Unavailable, message).with_type("identity.auth_dependency_failed")
}
pub(crate) fn rejected(code: &'static str) -> Failure {
    Failure::new(Kind::Unauthenticated, "authentication refused").with_type(code)
}
pub(crate) fn conflict(code: &'static str) -> Failure {
    Failure::new(Kind::Conflict, "authentication state changed").with_type(code)
}
pub(crate) fn password_invalid() -> Failure {
    Failure::new(Kind::Invalid, "password not accepted").with_type("identity.password_invalid")
}
/// Wall time at the start of an operation, bounded to the common range (U01).
pub(crate) fn now_ms<C: Clock + ?Sized>(clock: &C) -> Result<i64, Failure> {
    clock
        .now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .filter(|ms| (0..=MAX_TIME_MS).contains(ms))
        .ok_or_else(|| dependency_failed("clock reading out of range"))
}
pub(crate) fn new_id<I: IdSource + ?Sized>(ids: &I) -> Result<Id, Failure> {
    ids.new_id()
        .map_err(|e| dependency_failed("ID generation failed").with_source(e))
}
/// Deadline arithmetic never wraps or leaves the common range.
pub(crate) fn deadline(now: i64, ttl: i64) -> Result<i64, Failure> {
    now.checked_add(ttl)
        .filter(|t| (0..=MAX_TIME_MS).contains(t))
        .ok_or_else(|| dependency_failed("deadline out of range"))
}
/// A safe outbox fact (U14): fresh ID, the operation's time, the caller's work.
pub(crate) fn event<I: IdSource + ?Sized>(
    ids: &I,
    name: &str,
    at: i64,
    work: &WorkContext,
    payload: serde_json::Value,
) -> Result<Envelope, Failure> {
    Envelope::new(new_id(ids)?, name, at, work, payload)
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
