//! Reads: session resolution into a safe admission projection (U08).
use super::{
    AuthenticatedPrincipal, Config,
    command::{Clock, TokenCodec},
    now_ms, rejected,
};
use crate::{
    domains::identity::domain::token::{TokenDigest, TokenPurpose},
    shared::{errors::Failure, secret::SecretString},
};
use std::future::Future;

/// Storage outcome of resolving a session digest. The resolver owns the
/// transactional recheck of principal status, credential verification, epoch
/// and the idle-extension write; the application adds no cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Admitted(AuthenticatedPrincipal),
    Rejected,
    Absent,
}
pub trait SessionResolver {
    fn resolve(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
        idle_ttl_ms: i64,
    ) -> impl Future<Output = Result<Resolution, Failure>> + Send;
}

pub struct Authenticate<P> {
    ports: P,
    config: Config,
}
impl<P> Authenticate<P>
where
    P: Clock + TokenCodec + SessionResolver + Send + Sync,
{
    pub fn new(ports: P, config: Config) -> Result<Self, Failure> {
        Ok(Self { ports, config })
    }
    /// A malformed token is indistinguishable from a stale one (U08).
    pub async fn authenticate(
        &self,
        token: &SecretString,
    ) -> Result<AuthenticatedPrincipal, Failure> {
        let digest = self
            .ports
            .digest(TokenPurpose::Session, token)
            .map_err(|_| rejected("identity.session_rejected"))?;
        let now = now_ms(&self.ports)?;
        match self
            .ports
            .resolve(&digest, now, self.config.session_idle_ms)
            .await?
        {
            Resolution::Admitted(principal) => Ok(principal),
            Resolution::Rejected | Resolution::Absent => Err(rejected("identity.session_rejected")),
        }
    }
}
