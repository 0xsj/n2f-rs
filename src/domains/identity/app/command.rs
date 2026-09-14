//! Mutations (U04–U07, U09–U13): each struct owns one use case and declares the
//! ports it consumes. Store ports receive validated snapshots, the versions the
//! operation actually verified, explicit time and the safe events to enqueue.
use super::{
    AuthenticatedPrincipal, Config, conflict, deadline, event, hex, new_id, now_ms,
    password_invalid, rejected,
};
use crate::{
    domains::identity::domain::{
        Kind as PrincipalKind, MAX_VERSION, Principal, Snapshot, Status,
        auth_state::AuthState,
        challenge::{Challenge, ChallengeSnapshot},
        credential::{CredentialSnapshot, PasswordCredential},
        email::Email,
        password::{NewPassword, PasswordInput},
        session::{Session, SessionSnapshot},
        token::{TokenDigest, TokenPurpose},
        upgrade_ticket::{UpgradeTicket, UpgradeTicketSnapshot},
    },
    shared::{
        errors::{Failure, Kind},
        events::Envelope,
        id::Id,
        provenance::WorkContext,
        secret::SecretString,
    },
};
use serde_json::json;
use std::{future::Future, time::SystemTime};

// ---- Capabilities shared by several operations ------------------------------

pub trait Clock {
    fn now(&self) -> SystemTime;
}
pub trait IdSource {
    fn new_id(&self) -> Result<Id, Failure>;
}
pub trait PasswordHasher {
    fn hash(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<SecretString, Failure>> + Send;
    /// `true` is a match; a mismatch is a value, never a failure.
    fn verify(
        &self,
        input: &PasswordInput,
        record: &SecretString,
    ) -> impl Future<Output = Result<bool, Failure>> + Send;
    /// Comparable work for an unknown login identifier; always `false`.
    fn verify_absent(
        &self,
        input: &PasswordInput,
    ) -> impl Future<Output = Result<bool, Failure>> + Send;
}
pub trait TokenCodec {
    fn issue(&self, purpose: TokenPurpose) -> Result<(SecretString, TokenDigest), Failure>;
    fn digest(&self, purpose: TokenPurpose, secret: &SecretString) -> Result<TokenDigest, Failure>;
}
pub trait EnrollmentPolicy {
    /// `true` when the password is allowed; a blocklist hit is `false`.
    fn check_blocklist(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<bool, Failure>> + Send;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Permitted,
    Refused { retry_after_ms: i64 },
}
pub trait AttemptLimiter {
    /// The subject is private (an email or an ID); the adapter digests it (A16).
    fn admit(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> impl Future<Output = Result<Admission, Failure>> + Send;
}
pub trait MailDelivery {
    /// Bounded post-commit effect; `false` is a reported non-delivery, never a failure.
    fn send_verification(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send;
    fn send_reset(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send;
}

// ---- Store ports -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    Committed,
    /// The guarded state moved (principal inactive, unverified, version or epoch changed).
    Stale,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterOutcome {
    Created,
    DuplicateEmail,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevokeOutcome {
    Revoked,
    AlreadyInactive,
    Absent,
}
/// One transaction: principal, auth state, credential, challenge and event.
pub struct RegisterRecord {
    pub principal: Snapshot,
    pub auth_epoch: u32,
    pub credential: CredentialSnapshot,
    pub challenge: ChallengeSnapshot,
    pub event: Envelope,
}
pub trait RegisterStore {
    fn register(
        &self,
        record: RegisterRecord,
    ) -> impl Future<Output = Result<RegisterOutcome, Failure>> + Send;
}
pub struct CredentialRecord {
    pub principal: Snapshot,
    pub credential: CredentialSnapshot,
    pub auth_epoch: u32,
}
pub trait CredentialReader {
    fn find_by_email(
        &self,
        email: &Email,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send;
    fn find_by_principal(
        &self,
        principal: Id,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send;
}
pub struct LoginRecord {
    pub session: SessionSnapshot,
    pub expected_password_version: u32,
    pub expected_auth_epoch: u32,
    pub event: Envelope,
}
pub trait LoginStore {
    fn commit_login(
        &self,
        record: LoginRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
}
pub trait SessionRevoker {
    /// Enqueues the event only when a live session was actually revoked.
    fn revoke_session(
        &self,
        session: Id,
        principal: Id,
        now_ms: i64,
        event: Envelope,
    ) -> impl Future<Output = Result<RevokeOutcome, Failure>> + Send;
}
pub trait EpochStore {
    fn revoke_all(
        &self,
        principal: Id,
        expected_epoch: u32,
        now_ms: i64,
        event: Envelope,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
}
pub struct ChallengeRecord {
    pub challenge: ChallengeSnapshot,
    pub credential: CredentialSnapshot,
    pub principal_status: Status,
    pub auth_epoch: u32,
}
pub trait ChallengeReader {
    fn find_by_digest(
        &self,
        purpose: TokenPurpose,
        digest: &TokenDigest,
    ) -> impl Future<Output = Result<Option<ChallengeRecord>, Failure>> + Send;
}
pub struct IssueRecord {
    pub challenge: ChallengeSnapshot,
    pub expected_password_version: u32,
}
pub trait ChallengeStore {
    /// Invalidates the previous outstanding challenge of that purpose atomically.
    fn issue_challenge(
        &self,
        record: IssueRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
}
pub struct VerifyRecord {
    pub challenge_id: Id,
    pub principal_id: Id,
    pub consumed_at_ms: i64,
    pub expected_password_version: u32,
    pub event: Envelope,
}
pub trait VerifyStore {
    fn verify_email(
        &self,
        record: VerifyRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
}
pub struct ChangeRecord {
    pub principal_id: Id,
    pub new_hash: SecretString,
    pub expected_password_version: u32,
    pub expected_auth_epoch: u32,
    pub changed_at_ms: i64,
    pub events: Vec<Envelope>,
}
pub struct ResetRecord {
    pub challenge_id: Id,
    pub principal_id: Id,
    pub new_hash: SecretString,
    pub expected_password_version: u32,
    pub expected_auth_epoch: u32,
    pub consumed_at_ms: i64,
    pub set_verified: bool,
    pub events: Vec<Envelope>,
}
pub trait PasswordStore {
    fn change_password(
        &self,
        record: ChangeRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
    fn reset_password(
        &self,
        record: ResetRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
}
pub struct UpgradeTicketRecord {
    pub ticket: UpgradeTicketSnapshot,
    pub expected_auth_epoch: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpgradeTicketAdmission {
    pub principal_id: Id,
    pub session_id: Id,
    pub auth_epoch: u32,
}
pub trait UpgradeTicketStore {
    fn issue_upgrade_ticket(
        &self,
        record: UpgradeTicketRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send;
    fn consume_upgrade_ticket(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
    ) -> impl Future<Output = Result<Option<UpgradeTicketAdmission>, Failure>> + Send;
}

// ---- Inputs and results -------------------------------------------------------

pub struct RegisterInput {
    pub email: String,
    pub password: SecretString,
    pub source: String,
}
/// Private outcome; transport projects both variants as the same accepted response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterResult {
    pub created: bool,
    pub mail_delivered: bool,
}
pub struct RequestInput {
    pub email: String,
    pub source: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestResult {
    pub issued: bool,
    pub mail_delivered: bool,
}
pub struct VerifyEmailInput {
    pub token: SecretString,
    pub password: SecretString,
}
pub struct LoginInput {
    pub email: String,
    pub password: SecretString,
    pub source: String,
}
pub struct LoginResult {
    pub session_id: Id,
    pub secret: SecretString,
    pub absolute_expires_at_ms: i64,
    pub idle_expires_at_ms: i64,
    pub principal: Snapshot,
}
/// Debug redacts the session secret; the other fields are safe identifiers.
impl std::fmt::Debug for LoginResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginResult")
            .field("session_id", &self.session_id)
            .field("secret", &"[REDACTED]")
            .field("absolute_expires_at_ms", &self.absolute_expires_at_ms)
            .field("idle_expires_at_ms", &self.idle_expires_at_ms)
            .field("principal", &self.principal)
            .finish()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogoutResult {
    pub revoked: bool,
}
pub struct ChangePasswordInput {
    pub current: SecretString,
    pub new: SecretString,
    pub source: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeResult {
    pub sessions_invalidated: bool,
}
pub struct ResetPasswordInput {
    pub token: SecretString,
    pub new: SecretString,
    pub source: String,
}

const REGISTERED: &str = "identity.principal.registered.v1";
const EMAIL_VERIFIED: &str = "identity.email.verified.v1";
const SESSION_CREATED: &str = "identity.session.created.v1";
const SESSION_REVOKED: &str = "identity.session.revoked.v1";
const SESSIONS_REVOKED: &str = "identity.sessions.revoked.v1";
const PASSWORD_CHANGED: &str = "identity.password.changed.v1";

// ---- Shared steps ---------------------------------------------------------------

async fn admit<P: AttemptLimiter + ?Sized>(
    ports: &P,
    operation: &str,
    subject: &SecretString,
    source: &str,
) -> Result<(), Failure> {
    match ports.admit(operation, subject, source).await? {
        Admission::Permitted => Ok(()),
        Admission::Refused { retry_after_ms } => Err(Failure::new(
            Kind::RateLimited,
            "authentication attempts limited",
        )
        .with_type("identity.auth_rate_limited")
        .with_detail("retry_after_ms", retry_after_ms.to_string())),
    }
}
fn email_subject(email: &Email) -> SecretString {
    SecretString::new(email.reveal().to_string())
}
fn digest_subject(digest: &TokenDigest) -> SecretString {
    SecretString::new(hex(&digest.bytes()))
}
/// The principal label until a profile exists (U04): the local part, bounded.
fn display_name(email: &Email) -> String {
    let local = email.reveal().split('@').next().unwrap_or_default();
    local.chars().take(100).collect()
}
fn next_password_version(current: u32) -> Result<u32, Failure> {
    if current >= MAX_VERSION {
        return Err(conflict("identity.version_exhausted"));
    }
    Ok(current + 1)
}
fn next_epoch(principal: Id, current: u32) -> Result<u32, Failure> {
    Ok(AuthState::new(principal, current)?
        .invalidate(current)?
        .epoch())
}
async fn enrol<P: EnrollmentPolicy + ?Sized>(
    ports: &P,
    password: SecretString,
) -> Result<NewPassword, Failure> {
    let password = NewPassword::parse(password)?;
    if !ports.check_blocklist(&password).await? {
        return Err(password_invalid());
    }
    Ok(password)
}
/// The two facts every password replacement promises (A14, A20).
struct PasswordChange {
    principal: Id,
    version: u32,
    epoch: u32,
    reason: &'static str,
    revoke_reason: &'static str,
}
fn password_events<I: IdSource + ?Sized>(
    ids: &I,
    now: i64,
    work: &WorkContext,
    change: PasswordChange,
) -> Result<Vec<Envelope>, Failure> {
    let principal = change.principal.to_string();
    Ok(vec![
        event(
            ids,
            PASSWORD_CHANGED,
            now,
            work,
            json!({"principal_id": principal, "password_version": change.version, "reason": change.reason}),
        )?,
        event(
            ids,
            SESSIONS_REVOKED,
            now,
            work,
            json!({"principal_id": principal, "auth_epoch": change.epoch, "reason": change.revoke_reason}),
        )?,
    ])
}

// ---- Register (U04) ---------------------------------------------------------------

pub struct Register<P> {
    ports: P,
    config: Config,
}
impl<P> Register<P>
where
    P: Clock
        + IdSource
        + PasswordHasher
        + TokenCodec
        + EnrollmentPolicy
        + AttemptLimiter
        + MailDelivery
        + RegisterStore
        + Send
        + Sync,
{
    pub fn new(ports: P, config: Config) -> Result<Self, Failure> {
        Ok(Self { ports, config })
    }
    pub async fn register(
        &self,
        work: &WorkContext,
        input: RegisterInput,
    ) -> Result<RegisterResult, Failure> {
        let email = Email::parse(&input.email)?;
        admit(
            &self.ports,
            "register",
            &email_subject(&email),
            &input.source,
        )
        .await?;
        let password = enrol(&self.ports, input.password).await?;
        let hash = self.ports.hash(&password).await?;
        let now = now_ms(&self.ports)?;
        let principal_id = new_id(&self.ports)?;
        let principal = Principal::register(
            principal_id,
            PrincipalKind::Human,
            display_name(&email),
            now,
        )?
        .snapshot();
        let auth_state = AuthState::new(principal_id, 1)?;
        let credential = PasswordCredential::restore(CredentialSnapshot {
            principal_id,
            email: email.clone(),
            password_hash: hash,
            verified_at_ms: None,
            password_version: 1,
            created_at_ms: now,
            changed_at_ms: now,
        })?
        .snapshot();
        let (token, digest) = self.ports.issue(TokenPurpose::EmailVerification)?;
        let expires = deadline(now, self.config.verification_ttl_ms)?;
        let challenge =
            Challenge::issue(new_id(&self.ports)?, principal_id, digest, 1, now, expires)?
                .snapshot();
        let event = event(
            &self.ports,
            REGISTERED,
            now,
            work,
            json!({"principal_id": principal_id.to_string(), "kind": "human", "origin": "self_registration"}),
        )?;
        let outcome = self
            .ports
            .register(RegisterRecord {
                principal,
                auth_epoch: auth_state.epoch(),
                credential,
                challenge,
                event,
            })
            .await?;
        let created = outcome == RegisterOutcome::Created;
        let mail_delivered = created && self.ports.send_verification(&email, &token, expires).await;
        Ok(RegisterResult {
            created,
            mail_delivered,
        })
    }
}

// ---- Challenge requests (U05, U12) -------------------------------------------------

async fn request_challenge<P>(
    ports: &P,
    config: &Config,
    input: RequestInput,
    purpose: TokenPurpose,
) -> Result<RequestResult, Failure>
where
    P: Clock
        + IdSource
        + TokenCodec
        + AttemptLimiter
        + MailDelivery
        + CredentialReader
        + ChallengeStore
        + Send
        + Sync,
{
    let (operation, ttl) = match purpose {
        TokenPurpose::PasswordReset => ("reset_request", config.reset_ttl_ms),
        _ => ("verification_request", config.verification_ttl_ms),
    };
    let email = Email::parse(&input.email)?;
    admit(ports, operation, &email_subject(&email), &input.source).await?;
    let none = RequestResult {
        issued: false,
        mail_delivered: false,
    };
    let Some(found) = ports.find_by_email(&email).await? else {
        return Ok(none);
    };
    // U05/U12/U17: an inactive principal gets no challenge and no mail, and the
    // response stays accepted so suspension is not disclosed.
    if found.principal.status != Status::Active {
        return Ok(none);
    }
    if purpose == TokenPurpose::EmailVerification && found.credential.verified_at_ms.is_some() {
        return Ok(none);
    }
    let now = now_ms(ports)?;
    let version = found.credential.password_version;
    let (token, digest) = ports.issue(purpose)?;
    let expires = deadline(now, ttl)?;
    let challenge = Challenge::issue(
        new_id(ports)?,
        found.credential.principal_id,
        digest,
        version,
        now,
        expires,
    )?
    .snapshot();
    match ports
        .issue_challenge(IssueRecord {
            challenge,
            expected_password_version: version,
        })
        .await?
    {
        Commit::Stale => Ok(none),
        Commit::Committed => {
            let mail_delivered = match purpose {
                TokenPurpose::PasswordReset => ports.send_reset(&email, &token, expires).await,
                _ => ports.send_verification(&email, &token, expires).await,
            };
            Ok(RequestResult {
                issued: true,
                mail_delivered,
            })
        }
    }
}
pub struct RequestVerification<P> {
    ports: P,
    config: Config,
}
impl<P> RequestVerification<P>
where
    P: Clock
        + IdSource
        + TokenCodec
        + AttemptLimiter
        + MailDelivery
        + CredentialReader
        + ChallengeStore
        + Send
        + Sync,
{
    pub fn new(ports: P, config: Config) -> Result<Self, Failure> {
        Ok(Self { ports, config })
    }
    pub async fn request(&self, input: RequestInput) -> Result<RequestResult, Failure> {
        request_challenge(
            &self.ports,
            &self.config,
            input,
            TokenPurpose::EmailVerification,
        )
        .await
    }
}
pub struct RequestReset<P> {
    ports: P,
    config: Config,
}
impl<P> RequestReset<P>
where
    P: Clock
        + IdSource
        + TokenCodec
        + AttemptLimiter
        + MailDelivery
        + CredentialReader
        + ChallengeStore
        + Send
        + Sync,
{
    pub fn new(ports: P, config: Config) -> Result<Self, Failure> {
        Ok(Self { ports, config })
    }
    pub async fn request(&self, input: RequestInput) -> Result<RequestResult, Failure> {
        request_challenge(
            &self.ports,
            &self.config,
            input,
            TokenPurpose::PasswordReset,
        )
        .await
    }
}

// ---- Verify email (U06) -------------------------------------------------------------

/// Reads and pure-checks a challenge by token; every refusal is `challenge_rejected`.
async fn find_challenge<P>(
    ports: &P,
    purpose: TokenPurpose,
    operation: &str,
    token: &SecretString,
    source: &str,
) -> Result<(ChallengeRecord, TokenDigest), Failure>
where
    P: TokenCodec + AttemptLimiter + ChallengeReader + Send + Sync,
{
    let digest = ports
        .digest(purpose, token)
        .map_err(|_| rejected("identity.challenge_rejected"))?;
    admit(ports, operation, &digest_subject(&digest), source).await?;
    let Some(found) = ports.find_by_digest(purpose, &digest).await? else {
        return Err(rejected("identity.challenge_rejected"));
    };
    if found.principal_status != Status::Active {
        return Err(rejected("identity.challenge_rejected"));
    }
    Ok((found, digest))
}
fn consume(
    challenge: ChallengeSnapshot,
    purpose: TokenPurpose,
    version: u32,
    now: i64,
) -> Result<ChallengeSnapshot, Failure> {
    Ok(Challenge::restore(challenge)?
        .consume(purpose, version, now)
        .map_err(|_| rejected("identity.challenge_rejected"))?
        .snapshot())
}
pub struct VerifyEmail<P> {
    ports: P,
}
impl<P> VerifyEmail<P>
where
    P: Clock
        + IdSource
        + PasswordHasher
        + TokenCodec
        + AttemptLimiter
        + ChallengeReader
        + VerifyStore
        + Send
        + Sync,
{
    pub fn new(ports: P, _config: Config) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
    pub async fn verify(&self, work: &WorkContext, input: VerifyEmailInput) -> Result<(), Failure> {
        let (found, _) = find_challenge(
            &self.ports,
            TokenPurpose::EmailVerification,
            "verify",
            &input.token,
            "",
        )
        .await?;
        let now = now_ms(&self.ports)?;
        let password = PasswordInput::parse(input.password)?;
        if !self
            .ports
            .verify(&password, &found.credential.password_hash)
            .await?
        {
            return Err(rejected("identity.challenge_rejected"));
        }
        let version = found.credential.password_version;
        let consumed = consume(
            found.challenge,
            TokenPurpose::EmailVerification,
            version,
            now,
        )?;
        let event = event(
            &self.ports,
            EMAIL_VERIFIED,
            now,
            work,
            json!({"principal_id": consumed.principal_id.to_string(), "challenge_id": consumed.id.to_string()}),
        )?;
        match self
            .ports
            .verify_email(VerifyRecord {
                challenge_id: consumed.id,
                principal_id: consumed.principal_id,
                consumed_at_ms: now,
                expected_password_version: version,
                event,
            })
            .await?
        {
            Commit::Committed => Ok(()),
            Commit::Stale => Err(rejected("identity.challenge_rejected")),
        }
    }
}

// ---- Login (U07) ---------------------------------------------------------------------

pub struct Login<P> {
    ports: P,
    config: Config,
}
impl<P> Login<P>
where
    P: Clock
        + IdSource
        + PasswordHasher
        + TokenCodec
        + AttemptLimiter
        + CredentialReader
        + LoginStore
        + Send
        + Sync,
{
    pub fn new(ports: P, config: Config) -> Result<Self, Failure> {
        Ok(Self { ports, config })
    }
    pub async fn login(
        &self,
        work: &WorkContext,
        input: LoginInput,
    ) -> Result<LoginResult, Failure> {
        let email = Email::parse(&input.email)?;
        admit(&self.ports, "login", &email_subject(&email), &input.source).await?;
        let password = PasswordInput::parse(input.password)?;
        let now = now_ms(&self.ports)?;
        let Some(found) = self.ports.find_by_email(&email).await? else {
            self.ports.verify_absent(&password).await?;
            return Err(rejected("identity.credentials_rejected"));
        };
        let matched = self
            .ports
            .verify(&password, &found.credential.password_hash)
            .await?;
        if !matched
            || found.credential.verified_at_ms.is_none()
            || found.principal.status != Status::Active
        {
            return Err(rejected("identity.credentials_rejected"));
        }
        let (token, digest) = self.ports.issue(TokenPurpose::Session)?;
        let session_id = new_id(&self.ports)?;
        let absolute = deadline(now, self.config.session_absolute_ms)?;
        let idle = deadline(now, self.config.session_idle_ms)?;
        let session = Session::issue(
            session_id,
            found.principal.id,
            digest,
            found.auth_epoch,
            now,
            absolute,
            idle,
        )?
        .snapshot();
        let event = event(
            &self.ports,
            SESSION_CREATED,
            now,
            work,
            json!({"principal_id": found.principal.id.to_string(), "session_id": session_id.to_string(), "auth_epoch": found.auth_epoch}),
        )?;
        match self
            .ports
            .commit_login(LoginRecord {
                session,
                expected_password_version: found.credential.password_version,
                expected_auth_epoch: found.auth_epoch,
                event,
            })
            .await?
        {
            Commit::Committed => Ok(LoginResult {
                session_id,
                secret: token,
                absolute_expires_at_ms: absolute,
                idle_expires_at_ms: idle,
                principal: found.principal,
            }),
            Commit::Stale => Err(rejected("identity.credentials_rejected")),
        }
    }
}

// ---- Logout (U09) and logout-all (U10) -------------------------------------------

pub struct Logout<P> {
    ports: P,
}
impl<P> Logout<P>
where
    P: Clock + IdSource + SessionRevoker + Send + Sync,
{
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
    pub async fn logout(
        &self,
        work: &WorkContext,
        caller: &AuthenticatedPrincipal,
    ) -> Result<LogoutResult, Failure> {
        let now = now_ms(&self.ports)?;
        let event = event(
            &self.ports,
            SESSION_REVOKED,
            now,
            work,
            json!({"principal_id": caller.principal_id.to_string(), "session_id": caller.session_id.to_string(), "reason": "logout"}),
        )?;
        let outcome = self
            .ports
            .revoke_session(caller.session_id, caller.principal_id, now, event)
            .await?;
        Ok(LogoutResult {
            revoked: outcome == RevokeOutcome::Revoked,
        })
    }
}
pub struct LogoutAll<P> {
    ports: P,
}
impl<P> LogoutAll<P>
where
    P: Clock + IdSource + EpochStore + Send + Sync,
{
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
    pub async fn logout_all(
        &self,
        work: &WorkContext,
        caller: &AuthenticatedPrincipal,
    ) -> Result<(), Failure> {
        let now = now_ms(&self.ports)?;
        let epoch = next_epoch(caller.principal_id, caller.auth_epoch)?;
        let event = event(
            &self.ports,
            SESSIONS_REVOKED,
            now,
            work,
            json!({"principal_id": caller.principal_id.to_string(), "auth_epoch": epoch, "reason": "logout_all"}),
        )?;
        match self
            .ports
            .revoke_all(caller.principal_id, caller.auth_epoch, now, event)
            .await?
        {
            Commit::Committed => Ok(()),
            Commit::Stale => Err(conflict("identity.version_conflict")),
        }
    }
}

// ---- Change password (U11) -----------------------------------------------------------

pub struct ChangePassword<P> {
    ports: P,
}
impl<P> ChangePassword<P>
where
    P: Clock
        + IdSource
        + PasswordHasher
        + EnrollmentPolicy
        + AttemptLimiter
        + CredentialReader
        + PasswordStore
        + Send
        + Sync,
{
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
    pub async fn change(
        &self,
        work: &WorkContext,
        caller: &AuthenticatedPrincipal,
        input: ChangePasswordInput,
    ) -> Result<ChangeResult, Failure> {
        admit(
            &self.ports,
            "password_change",
            &SecretString::new(caller.principal_id.to_string()),
            &input.source,
        )
        .await?;
        let now = now_ms(&self.ports)?;
        let Some(found) = self.ports.find_by_principal(caller.principal_id).await? else {
            return Err(rejected("identity.session_rejected"));
        };
        if found.auth_epoch != caller.auth_epoch || found.principal.status != Status::Active {
            return Err(rejected("identity.session_rejected"));
        }
        let current = PasswordInput::parse(input.current)?;
        if !self
            .ports
            .verify(&current, &found.credential.password_hash)
            .await?
        {
            return Err(rejected("identity.credentials_rejected"));
        }
        let password = enrol(&self.ports, input.new).await?;
        let version = found.credential.password_version;
        let next_version = next_password_version(version)?;
        let epoch = next_epoch(caller.principal_id, found.auth_epoch)?;
        let hash = self.ports.hash(&password).await?;
        let events = password_events(
            &self.ports,
            now,
            work,
            PasswordChange {
                principal: caller.principal_id,
                version: next_version,
                epoch,
                reason: "change",
                revoke_reason: "password_change",
            },
        )?;
        match self
            .ports
            .change_password(ChangeRecord {
                principal_id: caller.principal_id,
                new_hash: hash,
                expected_password_version: version,
                expected_auth_epoch: found.auth_epoch,
                changed_at_ms: now,
                events,
            })
            .await?
        {
            Commit::Committed => Ok(ChangeResult {
                sessions_invalidated: true,
            }),
            Commit::Stale => Err(conflict("identity.version_conflict")),
        }
    }
}

// ---- Reset password (U13) -------------------------------------------------------------

pub struct ResetPassword<P> {
    ports: P,
}

// ---- WebSocket upgrade ticket (A18) -----------------------------------------------

pub struct WebSocketTicket<P> {
    ports: P,
}
pub struct WebSocketTicketResult {
    pub secret: SecretString,
    pub expires_at_ms: i64,
}
impl std::fmt::Debug for WebSocketTicketResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketTicketResult")
            .field("secret", &"[REDACTED]")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}
impl<P> WebSocketTicket<P>
where
    P: Clock + IdSource + TokenCodec + UpgradeTicketStore + Send + Sync,
{
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
    pub async fn issue(
        &self,
        caller: AuthenticatedPrincipal,
    ) -> Result<WebSocketTicketResult, Failure> {
        if caller.auth_epoch == 0 {
            return Err(rejected("identity.session_rejected"));
        }
        let now = now_ms(&self.ports)?;
        let expires_at_ms = deadline(now, 30_000)?;
        let (secret, digest) = self.ports.issue(TokenPurpose::WebSocketUpgrade)?;
        let ticket_id = new_id(&self.ports)?;
        let ticket =
            UpgradeTicket::issue(ticket_id, caller.session_id, digest, now, expires_at_ms)?;
        match self
            .ports
            .issue_upgrade_ticket(UpgradeTicketRecord {
                ticket: ticket.snapshot(),
                expected_auth_epoch: caller.auth_epoch,
            })
            .await?
        {
            Commit::Committed => Ok(WebSocketTicketResult {
                secret,
                expires_at_ms,
            }),
            Commit::Stale => Err(rejected("identity.session_rejected")),
        }
    }
}
impl<P> ResetPassword<P>
where
    P: Clock
        + IdSource
        + PasswordHasher
        + TokenCodec
        + EnrollmentPolicy
        + AttemptLimiter
        + ChallengeReader
        + PasswordStore
        + Send
        + Sync,
{
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
    pub async fn reset(
        &self,
        work: &WorkContext,
        input: ResetPasswordInput,
    ) -> Result<(), Failure> {
        let (found, _) = find_challenge(
            &self.ports,
            TokenPurpose::PasswordReset,
            "reset",
            &input.token,
            &input.source,
        )
        .await?;
        let now = now_ms(&self.ports)?;
        let password = enrol(&self.ports, input.new).await?;
        let version = found.credential.password_version;
        let consumed = consume(found.challenge, TokenPurpose::PasswordReset, version, now)?;
        let next_version = next_password_version(version)?;
        let epoch = next_epoch(consumed.principal_id, found.auth_epoch)?;
        let hash = self.ports.hash(&password).await?;
        let events = password_events(
            &self.ports,
            now,
            work,
            PasswordChange {
                principal: consumed.principal_id,
                version: next_version,
                epoch,
                reason: "reset",
                revoke_reason: "password_reset",
            },
        )?;
        match self
            .ports
            .reset_password(ResetRecord {
                challenge_id: consumed.id,
                principal_id: consumed.principal_id,
                new_hash: hash,
                expected_password_version: version,
                expected_auth_epoch: found.auth_epoch,
                consumed_at_ms: now,
                set_verified: found.credential.verified_at_ms.is_none(),
                events,
            })
            .await?
        {
            Commit::Committed => Ok(()),
            Commit::Stale => Err(rejected("identity.challenge_rejected")),
        }
    }
}
