//! Identity PostgreSQL store; see CONTRACT.md. Owns SQL, the identity migration
//! and row-to-domain restoration. SQLx types never leave this module.
#![doc = include_str!("CONTRACT.md")]
use crate::{
    domains::identity::{
        app::{
            AuthenticatedPrincipal,
            command::{
                ChallengeReader, ChallengeRecord, ChallengeStore, ChangeRecord, Commit,
                CredentialReader, CredentialRecord, EpochStore, IssueRecord, LoginRecord,
                LoginStore, PasswordStore, RegisterOutcome, RegisterRecord, RegisterStore,
                ResetRecord, RevokeOutcome, SessionRevoker, UpgradeTicketAdmission,
                UpgradeTicketRecord, UpgradeTicketStore, VerifyRecord, VerifyStore,
            },
            query::{Resolution, SessionResolver},
        },
        domain::{
            Kind as PrincipalKind, MAX_VERSION, Principal, Snapshot, Status,
            auth_state::AuthState,
            challenge::{Challenge, ChallengeSnapshot},
            credential::{CredentialSnapshot, PasswordCredential},
            email::Email,
            session::{Session, SessionSnapshot},
            token::{TokenDigest, TokenPurpose},
            upgrade_ticket::{UpgradeTicket, UpgradeTicketSnapshot},
        },
    },
    shared::{
        errors::{Classified, Failure, Kind},
        events::{Envelope, postgres::enqueue},
        id::Id,
        postgres::{Database, Migration, map},
        secret::SecretString,
    },
};
use sqlx::{PgConnection, Row, postgres::PgRow};
use std::sync::Arc;

pub fn migration(version: i64) -> Migration {
    Migration {
        version,
        sql: include_str!("migrations/0001_identity.sql").into(),
    }
}
pub fn upgrade_ticket_migration(version: i64) -> Migration {
    Migration {
        version,
        sql: include_str!("migrations/0002_upgrade_tickets.sql").into(),
    }
}

/// Implements every stage 4 store port. Shared by reference through the pool.
pub struct Store {
    database: Arc<Database>,
}
impl Store {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }
}

// ---- Outcome markers: an Err inside the callback rolls the transaction back ---
const STALE: &str = "identity.store.stale";
const DUPLICATE: &str = "identity.store.duplicate_email";
fn marker(code: &str) -> Failure {
    Failure::new(Kind::Conflict, "guarded state moved").with_type(code)
}
fn is_marker(e: &Failure, code: &str) -> bool {
    e.classification().and_then(|c| c.error_type) == Some(code)
}
fn commit_of(r: Result<(), Failure>) -> Result<Commit, Failure> {
    match r {
        Ok(()) => Ok(Commit::Committed),
        Err(e) if is_marker(&e, STALE) => Ok(Commit::Stale),
        Err(e) => Err(e),
    }
}
fn corrupt(e: Failure) -> Failure {
    let t = e
        .classification()
        .and_then(|c| c.error_type)
        .unwrap_or("")
        .to_string();
    if t == "identity.credential_corrupt" {
        return e;
    }
    Failure::new(Kind::Internal, "stored identity record is corrupt")
        .with_type("identity.record_corrupt")
        .with_field("domain_type", t)
}
fn guard_failed() -> Failure {
    Failure::new(Kind::Internal, "guarded update affected no row under lock")
        .with_type("identity.store_guard_failed")
}
fn exhausted() -> Failure {
    Failure::new(Kind::Conflict, "version ceiling reached").with_type("identity.version_exhausted")
}

// ---- Text forms and row restoration -----------------------------------------
fn kind_text(k: PrincipalKind) -> &'static str {
    match k {
        PrincipalKind::Human => "human",
        PrincipalKind::Service => "service",
    }
}
fn status_text(s: Status) -> &'static str {
    match s {
        Status::Active => "active",
        Status::Suspended => "suspended",
    }
}
fn version_in(v: u32) -> i32 {
    v.min(MAX_VERSION) as i32
}
fn version_out(v: i32) -> Result<u32, Failure> {
    u32::try_from(v).map_err(|_| {
        corrupt(
            Failure::new(Kind::Invalid, "negative version").with_type("identity.version_invalid"),
        )
    })
}
fn id_out(row: &PgRow, column: &str) -> Result<Id, Failure> {
    let text: String = row.try_get(column).map_err(map)?;
    Id::parse(&text).map_err(corrupt)
}
fn principal_out(row: &PgRow) -> Result<Snapshot, Failure> {
    let kind = match row.try_get::<String, _>("kind").map_err(map)?.as_str() {
        "human" => PrincipalKind::Human,
        "service" => PrincipalKind::Service,
        _ => {
            return Err(corrupt(
                Failure::new(Kind::Invalid, "unknown kind").with_type("identity.principal_invalid"),
            ));
        }
    };
    let status = match row.try_get::<String, _>("status").map_err(map)?.as_str() {
        "active" => Status::Active,
        "suspended" => Status::Suspended,
        _ => {
            return Err(corrupt(
                Failure::new(Kind::Invalid, "unknown status")
                    .with_type("identity.principal_invalid"),
            ));
        }
    };
    let snapshot = Snapshot {
        id: id_out(row, "p_id")?,
        kind,
        display_name: row.try_get("display_name").map_err(map)?,
        status,
        created_at_ms: row.try_get("p_created_at_ms").map_err(map)?,
        updated_at_ms: row.try_get("p_updated_at_ms").map_err(map)?,
        version: version_out(row.try_get("p_version").map_err(map)?)?,
    };
    Ok(Principal::restore(snapshot).map_err(corrupt)?.snapshot())
}
fn credential_out(row: &PgRow) -> Result<CredentialSnapshot, Failure> {
    let email: String = row.try_get("canonical_email").map_err(map)?;
    let snapshot = CredentialSnapshot {
        principal_id: id_out(row, "c_principal_id")?,
        email: Email::parse(&email).map_err(corrupt)?,
        password_hash: SecretString::new(row.try_get("password_hash").map_err(map)?),
        verified_at_ms: row.try_get("verified_at_ms").map_err(map)?,
        password_version: version_out(row.try_get("password_version").map_err(map)?)?,
        created_at_ms: row.try_get("c_created_at_ms").map_err(map)?,
        changed_at_ms: row.try_get("changed_at_ms").map_err(map)?,
    };
    Ok(PasswordCredential::restore(snapshot)
        .map_err(corrupt)?
        .snapshot())
}
fn epoch_out(row: &PgRow) -> Result<u32, Failure> {
    let epoch = version_out(row.try_get("auth_epoch").map_err(map)?)?;
    AuthState::new(id_out(row, "a_principal_id")?, epoch).map_err(corrupt)?;
    Ok(epoch)
}
fn purpose_out(text: &str) -> Result<TokenPurpose, Failure> {
    TokenPurpose::parse(text).map_err(corrupt)
}
fn session_out(row: &PgRow) -> Result<SessionSnapshot, Failure> {
    let digest: Vec<u8> = row.try_get("token_digest").map_err(map)?;
    let snapshot = SessionSnapshot {
        id: id_out(row, "s_id")?,
        principal_id: id_out(row, "s_principal_id")?,
        token_digest: TokenDigest::parse(TokenPurpose::Session, &digest).map_err(corrupt)?,
        auth_epoch: version_out(row.try_get("s_auth_epoch").map_err(map)?)?,
        issued_at_ms: row.try_get("issued_at_ms").map_err(map)?,
        last_seen_at_ms: row.try_get("last_seen_at_ms").map_err(map)?,
        absolute_expires_at_ms: row.try_get("absolute_expires_at_ms").map_err(map)?,
        idle_expires_at_ms: row.try_get("idle_expires_at_ms").map_err(map)?,
        revoked_at_ms: row.try_get("revoked_at_ms").map_err(map)?,
    };
    Ok(Session::restore(snapshot).map_err(corrupt)?.snapshot())
}
fn challenge_out(row: &PgRow) -> Result<ChallengeSnapshot, Failure> {
    let digest: Vec<u8> = row.try_get("ch_token_digest").map_err(map)?;
    let purpose = purpose_out(&row.try_get::<String, _>("purpose").map_err(map)?)?;
    let snapshot = ChallengeSnapshot {
        id: id_out(row, "ch_id")?,
        principal_id: id_out(row, "ch_principal_id")?,
        token_digest: TokenDigest::parse(purpose, &digest).map_err(corrupt)?,
        password_version: version_out(row.try_get("ch_password_version").map_err(map)?)?,
        issued_at_ms: row.try_get("ch_issued_at_ms").map_err(map)?,
        expires_at_ms: row.try_get("expires_at_ms").map_err(map)?,
        consumed_at_ms: row.try_get("consumed_at_ms").map_err(map)?,
        invalidated_at_ms: row.try_get("invalidated_at_ms").map_err(map)?,
    };
    Ok(Challenge::restore(snapshot).map_err(corrupt)?.snapshot())
}
fn upgrade_ticket_out(row: &PgRow) -> Result<UpgradeTicketSnapshot, Failure> {
    let digest: Vec<u8> = row.try_get("ut_token_digest").map_err(map)?;
    let snapshot = UpgradeTicketSnapshot {
        id: id_out(row, "ut_id")?,
        session_id: id_out(row, "ut_session_id")?,
        token_digest: TokenDigest::parse(TokenPurpose::WebSocketUpgrade, &digest)
            .map_err(corrupt)?,
        issued_at_ms: row.try_get("ut_issued_at_ms").map_err(map)?,
        expires_at_ms: row.try_get("ut_expires_at_ms").map_err(map)?,
        consumed_at_ms: row.try_get("ut_consumed_at_ms").map_err(map)?,
    };
    Ok(UpgradeTicket::restore(snapshot)
        .map_err(corrupt)?
        .snapshot())
}

// ---- Column lists ------------------------------------------------------------
const PRINCIPAL_COLUMNS: &str = "p.id::text AS p_id, p.kind, p.display_name, p.status, p.created_at_ms AS p_created_at_ms, p.updated_at_ms AS p_updated_at_ms, p.version AS p_version";
const CREDENTIAL_COLUMNS: &str = "c.principal_id::text AS c_principal_id, c.canonical_email, c.password_hash, c.verified_at_ms, c.password_version, c.created_at_ms AS c_created_at_ms, c.changed_at_ms";
const AUTH_COLUMNS: &str = "a.principal_id::text AS a_principal_id, a.auth_epoch";
const SESSION_COLUMNS: &str = "s.id::text AS s_id, s.principal_id::text AS s_principal_id, s.token_digest, s.auth_epoch AS s_auth_epoch, s.issued_at_ms, s.last_seen_at_ms, s.absolute_expires_at_ms, s.idle_expires_at_ms, s.revoked_at_ms";
const CHALLENGE_COLUMNS: &str = "ch.id::text AS ch_id, ch.principal_id::text AS ch_principal_id, ch.purpose, ch.token_digest AS ch_token_digest, ch.password_version AS ch_password_version, ch.issued_at_ms AS ch_issued_at_ms, ch.expires_at_ms, ch.consumed_at_ms, ch.invalidated_at_ms";
const UPGRADE_TICKET_COLUMNS: &str = "ut.id::text AS ut_id, ut.session_id::text AS ut_session_id, ut.token_digest AS ut_token_digest, ut.issued_at_ms AS ut_issued_at_ms, ut.expires_at_ms AS ut_expires_at_ms, ut.consumed_at_ms AS ut_consumed_at_ms";

// ---- Locks in the S04 order --------------------------------------------------
async fn lock_epoch(tx: &mut PgConnection, principal: Id) -> Result<Option<u32>, Failure> {
    let q = format!(
        "SELECT {AUTH_COLUMNS} FROM public.n2f_identity_auth_states a WHERE a.principal_id=$1::uuid FOR UPDATE"
    );
    let row = sqlx::query(&q)
        .bind(principal.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map)?;
    row.as_ref().map(epoch_out).transpose()
}
async fn lock_principal(tx: &mut PgConnection, principal: Id) -> Result<Option<Snapshot>, Failure> {
    let q = format!(
        "SELECT {PRINCIPAL_COLUMNS} FROM public.n2f_identity_principals p WHERE p.id=$1::uuid FOR UPDATE"
    );
    let row = sqlx::query(&q)
        .bind(principal.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map)?;
    row.as_ref().map(principal_out).transpose()
}
async fn lock_credential(
    tx: &mut PgConnection,
    principal: Id,
) -> Result<Option<CredentialSnapshot>, Failure> {
    let q = format!(
        "SELECT {CREDENTIAL_COLUMNS} FROM public.n2f_identity_credentials c WHERE c.principal_id=$1::uuid FOR UPDATE"
    );
    let row = sqlx::query(&q)
        .bind(principal.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map)?;
    row.as_ref().map(credential_out).transpose()
}
async fn lock_session(
    tx: &mut PgConnection,
    session: Id,
    principal: Id,
) -> Result<Option<SessionSnapshot>, Failure> {
    let q = format!(
        "SELECT {SESSION_COLUMNS} FROM public.n2f_identity_sessions s WHERE s.id=$1::uuid AND s.principal_id=$2::uuid FOR UPDATE"
    );
    let row = sqlx::query(&q)
        .bind(session.to_string())
        .bind(principal.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map)?;
    row.as_ref().map(session_out).transpose()
}
async fn lock_session_by_digest(
    tx: &mut PgConnection,
    digest: &[u8],
    principal: Id,
) -> Result<Option<SessionSnapshot>, Failure> {
    let q = format!(
        "SELECT {SESSION_COLUMNS} FROM public.n2f_identity_sessions s JOIN public.n2f_identity_upgrade_tickets ut ON ut.session_id=s.id WHERE ut.token_digest=$1 AND s.principal_id=$2::uuid FOR UPDATE"
    );
    let row = sqlx::query(&q)
        .bind(digest)
        .bind(principal.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map)?;
    row.as_ref().map(session_out).transpose()
}
/// Locks auth state, principal and credential in order; None when any is missing.
async fn lock_identity(
    tx: &mut PgConnection,
    principal: Id,
) -> Result<Option<(u32, Snapshot, CredentialSnapshot)>, Failure> {
    let Some(epoch) = lock_epoch(tx, principal).await? else {
        return Ok(None);
    };
    let Some(p) = lock_principal(tx, principal).await? else {
        return Ok(None);
    };
    let Some(c) = lock_credential(tx, principal).await? else {
        return Ok(None);
    };
    Ok(Some((epoch, p, c)))
}
async fn enqueue_all(tx: &mut PgConnection, events: &[Envelope]) -> Result<(), Failure> {
    for e in events {
        enqueue(tx, e).await?;
    }
    Ok(())
}
async fn guarded(
    tx: &mut PgConnection,
    q: sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments>,
) -> Result<(), Failure> {
    let affected = q.execute(&mut *tx).await.map_err(map)?.rows_affected();
    if affected != 1 {
        return Err(guard_failed());
    }
    Ok(())
}
async fn invalidate_outstanding(
    tx: &mut PgConnection,
    principal: Id,
    purpose: Option<TokenPurpose>,
    at: i64,
) -> Result<(), Failure> {
    let q = match purpose {
        Some(_) => {
            "UPDATE public.n2f_identity_challenges SET invalidated_at_ms=$2 WHERE principal_id=$1::uuid AND purpose=$3 AND consumed_at_ms IS NULL AND invalidated_at_ms IS NULL"
        }
        None => {
            "UPDATE public.n2f_identity_challenges SET invalidated_at_ms=$2 WHERE principal_id=$1::uuid AND consumed_at_ms IS NULL AND invalidated_at_ms IS NULL"
        }
    };
    let mut query = sqlx::query(q).bind(principal.to_string()).bind(at);
    if let Some(p) = purpose {
        query = query.bind(p.as_str());
    }
    query.execute(&mut *tx).await.map_err(map)?;
    Ok(())
}
/// One guarded consume; zero rows means the challenge moved (S12).
async fn consume_challenge(
    tx: &mut PgConnection,
    challenge: Id,
    principal: Id,
    purpose: TokenPurpose,
    consumed_at: i64,
) -> Result<(), Failure> {
    let affected = sqlx::query("UPDATE public.n2f_identity_challenges SET consumed_at_ms=$3 WHERE id=$1::uuid AND principal_id=$2::uuid AND purpose=$4 AND consumed_at_ms IS NULL AND invalidated_at_ms IS NULL AND issued_at_ms<=$3 AND $3<expires_at_ms")
        .bind(challenge.to_string())
        .bind(principal.to_string())
        .bind(consumed_at)
        .bind(purpose.as_str())
        .execute(&mut *tx)
        .await
        .map_err(map)?
        .rows_affected();
    if affected != 1 {
        return Err(marker(STALE));
    }
    Ok(())
}
/// Password replacement shared by change and reset (S13/S14).
async fn replace_password(
    tx: &mut PgConnection,
    principal: Id,
    epoch: u32,
    credential: &CredentialSnapshot,
    new_hash: &SecretString,
    changed_at: i64,
    set_verified: bool,
) -> Result<(), Failure> {
    if credential.password_version >= MAX_VERSION {
        return Err(exhausted());
    }
    let next_epoch = AuthState::new(principal, epoch)?.invalidate(epoch)?.epoch();
    guarded(tx, sqlx::query("UPDATE public.n2f_identity_credentials SET password_hash=$2, password_version=password_version+1, changed_at_ms=$3, verified_at_ms=CASE WHEN $5 THEN COALESCE(verified_at_ms,$3) ELSE verified_at_ms END WHERE principal_id=$1::uuid AND password_version=$4")
        .bind(principal.to_string())
        .bind(new_hash.reveal())
        .bind(changed_at)
        .bind(version_in(credential.password_version))
        .bind(set_verified)).await?;
    guarded(tx, sqlx::query("UPDATE public.n2f_identity_auth_states SET auth_epoch=$2 WHERE principal_id=$1::uuid AND auth_epoch=$3")
        .bind(principal.to_string())
        .bind(version_in(next_epoch))
        .bind(version_in(epoch))).await?;
    invalidate_outstanding(tx, principal, None, changed_at).await
}

// ---- Reads without locks -----------------------------------------------------
fn credential_record(row: &PgRow) -> Result<CredentialRecord, Failure> {
    Ok(CredentialRecord {
        principal: principal_out(row)?,
        credential: credential_out(row)?,
        auth_epoch: epoch_out(row)?,
    })
}
const IDENTITY_JOIN: &str = "FROM public.n2f_identity_credentials c JOIN public.n2f_identity_principals p ON p.id=c.principal_id JOIN public.n2f_identity_auth_states a ON a.principal_id=c.principal_id";

impl CredentialReader for Store {
    async fn find_by_email(&self, email: &Email) -> Result<Option<CredentialRecord>, Failure> {
        let key = email.reveal().to_owned();
        self.database.transaction(move |tx| Box::pin(async move {
            let q = format!("SELECT {PRINCIPAL_COLUMNS}, {CREDENTIAL_COLUMNS}, {AUTH_COLUMNS} {IDENTITY_JOIN} WHERE c.canonical_email=$1");
            let row = sqlx::query(&q).bind(key).fetch_optional(tx).await.map_err(map)?;
            row.as_ref().map(credential_record).transpose()
        })).await
    }
    async fn find_by_principal(&self, principal: Id) -> Result<Option<CredentialRecord>, Failure> {
        self.database.transaction(move |tx| Box::pin(async move {
            let q = format!("SELECT {PRINCIPAL_COLUMNS}, {CREDENTIAL_COLUMNS}, {AUTH_COLUMNS} {IDENTITY_JOIN} WHERE c.principal_id=$1::uuid");
            let row = sqlx::query(&q).bind(principal.to_string()).fetch_optional(tx).await.map_err(map)?;
            row.as_ref().map(credential_record).transpose()
        })).await
    }
}
impl ChallengeReader for Store {
    async fn find_by_digest(
        &self,
        purpose: TokenPurpose,
        digest: &TokenDigest,
    ) -> Result<Option<ChallengeRecord>, Failure> {
        let bytes = digest.bytes().to_vec();
        self.database.transaction(move |tx| Box::pin(async move {
            let q = format!("SELECT {CHALLENGE_COLUMNS}, {PRINCIPAL_COLUMNS}, {CREDENTIAL_COLUMNS}, {AUTH_COLUMNS} FROM public.n2f_identity_challenges ch JOIN public.n2f_identity_credentials c ON c.principal_id=ch.principal_id JOIN public.n2f_identity_principals p ON p.id=ch.principal_id JOIN public.n2f_identity_auth_states a ON a.principal_id=ch.principal_id WHERE ch.purpose=$1 AND ch.token_digest=$2");
            let row = sqlx::query(&q).bind(purpose.as_str()).bind(bytes).fetch_optional(tx).await.map_err(map)?;
            row.as_ref().map(|r| Ok::<_, Failure>(ChallengeRecord { challenge: challenge_out(r)?, credential: credential_out(r)?, principal_status: principal_out(r)?.status, auth_epoch: epoch_out(r)? })).transpose()
        })).await
    }
}

// ---- Mutations ---------------------------------------------------------------
impl RegisterStore for Store {
    async fn register(&self, record: RegisterRecord) -> Result<RegisterOutcome, Failure> {
        let result = self.database.transaction(move |tx| Box::pin(async move {
            let p = &record.principal;
            sqlx::query("INSERT INTO public.n2f_identity_principals(id,kind,display_name,status,created_at_ms,updated_at_ms,version) VALUES($1::uuid,$2,$3,$4,$5,$6,$7)")
                .bind(p.id.to_string()).bind(kind_text(p.kind)).bind(&p.display_name).bind(status_text(p.status)).bind(p.created_at_ms).bind(p.updated_at_ms).bind(version_in(p.version))
                .execute(&mut *tx).await.map_err(map)?;
            sqlx::query("INSERT INTO public.n2f_identity_auth_states(principal_id,auth_epoch) VALUES($1::uuid,$2)")
                .bind(p.id.to_string()).bind(version_in(record.auth_epoch))
                .execute(&mut *tx).await.map_err(map)?;
            let c = &record.credential;
            let inserted = sqlx::query("INSERT INTO public.n2f_identity_credentials(principal_id,canonical_email,password_hash,verified_at_ms,password_version,created_at_ms,changed_at_ms) VALUES($1::uuid,$2,$3,$4,$5,$6,$7)")
                .bind(c.principal_id.to_string()).bind(c.email.reveal()).bind(c.password_hash.reveal()).bind(c.verified_at_ms).bind(version_in(c.password_version)).bind(c.created_at_ms).bind(c.changed_at_ms)
                .execute(&mut *tx).await;
            match inserted {
                Ok(_) => {}
                Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") && e.constraint() == Some("n2f_identity_credentials_canonical_email_key") => return Err(marker(DUPLICATE)),
                Err(e) => return Err(map(e)),
            }
            insert_challenge(tx, &record.challenge).await?;
            enqueue(tx, &record.event).await
        })).await;
        match result {
            Ok(()) => Ok(RegisterOutcome::Created),
            Err(e) if is_marker(&e, DUPLICATE) => Ok(RegisterOutcome::DuplicateEmail),
            Err(e) => Err(e),
        }
    }
}
async fn insert_challenge(tx: &mut PgConnection, ch: &ChallengeSnapshot) -> Result<(), Failure> {
    sqlx::query("INSERT INTO public.n2f_identity_challenges(id,principal_id,purpose,token_digest,password_version,issued_at_ms,expires_at_ms,consumed_at_ms,invalidated_at_ms) VALUES($1::uuid,$2::uuid,$3,$4,$5,$6,$7,$8,$9)")
        .bind(ch.id.to_string()).bind(ch.principal_id.to_string()).bind(ch.token_digest.purpose().as_str()).bind(ch.token_digest.bytes().to_vec()).bind(version_in(ch.password_version)).bind(ch.issued_at_ms).bind(ch.expires_at_ms).bind(ch.consumed_at_ms).bind(ch.invalidated_at_ms)
        .execute(&mut *tx).await.map_err(map)?;
    Ok(())
}
impl LoginStore for Store {
    async fn commit_login(&self, record: LoginRecord) -> Result<Commit, Failure> {
        commit_of(self.database.transaction(move |tx| Box::pin(async move {
            let s = &record.session;
            let Some((epoch, principal, credential)) = lock_identity(tx, s.principal_id).await? else { return Err(marker(STALE)) };
            if principal.status != Status::Active || credential.verified_at_ms.is_none() || credential.password_version != record.expected_password_version || epoch != record.expected_auth_epoch || s.auth_epoch != epoch {
                return Err(marker(STALE));
            }
            sqlx::query("INSERT INTO public.n2f_identity_sessions(id,principal_id,token_digest,auth_epoch,issued_at_ms,last_seen_at_ms,absolute_expires_at_ms,idle_expires_at_ms,revoked_at_ms) VALUES($1::uuid,$2::uuid,$3,$4,$5,$6,$7,$8,$9)")
                .bind(s.id.to_string()).bind(s.principal_id.to_string()).bind(s.token_digest.bytes().to_vec()).bind(version_in(s.auth_epoch)).bind(s.issued_at_ms).bind(s.last_seen_at_ms).bind(s.absolute_expires_at_ms).bind(s.idle_expires_at_ms).bind(s.revoked_at_ms)
                .execute(&mut *tx).await.map_err(map)?;
            enqueue(tx, &record.event).await
        })).await)
    }
}
impl UpgradeTicketStore for Store {
    async fn issue_upgrade_ticket(&self, record: UpgradeTicketRecord) -> Result<Commit, Failure> {
        commit_of(self.database.transaction(move |tx| Box::pin(async move {
            let owner: Option<String> = sqlx::query_scalar("SELECT principal_id::text FROM public.n2f_identity_sessions WHERE id=$1::uuid")
                .bind(record.ticket.session_id.to_string()).fetch_optional(&mut *tx).await.map_err(map)?;
            let Some(owner) = owner else { return Err(marker(STALE)); };
            let principal = Id::parse(&owner).map_err(corrupt)?;
            let Some((epoch, principal_row, credential)) = lock_identity(tx, principal).await? else { return Err(marker(STALE)); };
            if epoch != record.expected_auth_epoch || principal_row.status != Status::Active || credential.verified_at_ms.is_none() { return Err(marker(STALE)); }
            let Some(session) = lock_session(tx, record.ticket.session_id, principal).await? else { return Err(marker(STALE)); };
            if session.auth_epoch != epoch || Session::restore(session.clone())?.check(record.ticket.issued_at_ms).is_err() { return Err(marker(STALE)); }
            UpgradeTicket::restore(record.ticket.clone()).map_err(corrupt)?;
            sqlx::query("INSERT INTO public.n2f_identity_upgrade_tickets(id,session_id,token_digest,issued_at_ms,expires_at_ms,consumed_at_ms) VALUES($1::uuid,$2::uuid,$3,$4,$5,$6)")
                .bind(record.ticket.id.to_string()).bind(record.ticket.session_id.to_string()).bind(record.ticket.token_digest.bytes().to_vec()).bind(record.ticket.issued_at_ms).bind(record.ticket.expires_at_ms).bind(record.ticket.consumed_at_ms)
                .execute(&mut *tx).await.map_err(map)?;
            Ok(())
        })).await)
    }

    async fn consume_upgrade_ticket(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
    ) -> Result<Option<UpgradeTicketAdmission>, Failure> {
        let bytes = digest.bytes().to_vec();
        self.database.transaction(move |tx| Box::pin(async move {
            let owner: Option<String> = sqlx::query_scalar("SELECT s.principal_id::text FROM public.n2f_identity_upgrade_tickets ut JOIN public.n2f_identity_sessions s ON s.id=ut.session_id WHERE ut.token_digest=$1")
                .bind(&bytes).fetch_optional(&mut *tx).await.map_err(map)?;
            let Some(owner) = owner else { return Ok(None); };
            let principal = Id::parse(&owner).map_err(corrupt)?;
            let Some((epoch, principal_row, credential)) = lock_identity(tx, principal).await? else { return Ok(None); };
            if principal_row.status != Status::Active || credential.verified_at_ms.is_none() { return Ok(None); }
            let Some(session) = lock_session_by_digest(tx, &bytes, principal).await? else { return Ok(None); };
            if session.auth_epoch != epoch || Session::restore(session.clone())?.check(now_ms).is_err() { return Ok(None); }
            let q = format!("SELECT {UPGRADE_TICKET_COLUMNS} FROM public.n2f_identity_upgrade_tickets ut WHERE ut.token_digest=$1 FOR UPDATE");
            let Some(row) = sqlx::query(&q).bind(&bytes).fetch_optional(&mut *tx).await.map_err(map)? else { return Ok(None); };
            let snapshot = upgrade_ticket_out(&row)?;
            let used = match UpgradeTicket::restore(snapshot.clone())?.consume(now_ms) { Ok(v) => v, Err(e) if e.classification().and_then(|c| c.error_type) == Some("identity.upgrade_ticket_rejected") => return Ok(None), Err(e) => return Err(e) };
            guarded(tx, sqlx::query("UPDATE public.n2f_identity_upgrade_tickets SET consumed_at_ms=$2 WHERE id=$1::uuid AND consumed_at_ms IS NULL").bind(snapshot.id.to_string()).bind(used.snapshot().consumed_at_ms)).await?;
            Ok(Some(UpgradeTicketAdmission { principal_id: principal, session_id: session.id, auth_epoch: session.auth_epoch }))
        })).await
    }
}
impl SessionResolver for Store {
    async fn resolve(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
        idle_ttl_ms: i64,
    ) -> Result<Resolution, Failure> {
        let bytes = digest.bytes().to_vec();
        self.database.transaction(move |tx| Box::pin(async move {
            let owner: Option<String> = sqlx::query_scalar("SELECT principal_id::text FROM public.n2f_identity_sessions WHERE token_digest=$1").bind(&bytes).fetch_optional(&mut *tx).await.map_err(map)?;
            let Some(owner) = owner else { return Ok(Resolution::Absent) };
            let principal = Id::parse(&owner).map_err(corrupt)?;
            let Some((epoch, principal_row, credential)) = lock_identity(tx, principal).await? else { return Err(corrupt(Failure::new(Kind::Invalid, "session without identity").with_type("identity.session_invalid"))) };
            let q = format!("SELECT {SESSION_COLUMNS} FROM public.n2f_identity_sessions s WHERE s.token_digest=$1 FOR UPDATE");
            let row = sqlx::query(&q).bind(&bytes).fetch_optional(&mut *tx).await.map_err(map)?;
            let Some(row) = row else { return Ok(Resolution::Absent) };
            let snapshot = session_out(&row)?;
            let session = Session::restore(snapshot.clone())?;
            if now_ms < snapshot.last_seen_at_ms {
                return Ok(Resolution::Rejected);
            }
            if let Err(e) = session.check(now_ms) {
                if e.classification().and_then(|c| c.error_type) != Some("identity.session_rejected") {
                    return Err(e);
                }
                if snapshot.revoked_at_ms.is_none() {
                    guarded(tx, sqlx::query("UPDATE public.n2f_identity_sessions SET revoked_at_ms=$2 WHERE id=$1::uuid AND revoked_at_ms IS NULL").bind(snapshot.id.to_string()).bind(now_ms)).await?;
                }
                return Ok(Resolution::Rejected);
            }
            if principal_row.status != Status::Active || credential.verified_at_ms.is_none() || snapshot.auth_epoch != epoch {
                return Ok(Resolution::Rejected);
            }
            let touched = session.touch(now_ms, idle_ttl_ms)?.snapshot();
            guarded(tx, sqlx::query("UPDATE public.n2f_identity_sessions SET last_seen_at_ms=$2, idle_expires_at_ms=$3 WHERE id=$1::uuid AND last_seen_at_ms=$4 AND revoked_at_ms IS NULL")
                .bind(snapshot.id.to_string()).bind(touched.last_seen_at_ms).bind(touched.idle_expires_at_ms).bind(snapshot.last_seen_at_ms)).await?;
            Ok(Resolution::Admitted(AuthenticatedPrincipal { principal_id: principal, session_id: snapshot.id, auth_epoch: epoch }))
        })).await
    }
}
impl SessionRevoker for Store {
    async fn revoke_session(
        &self,
        session: Id,
        principal: Id,
        now_ms: i64,
        event: Envelope,
    ) -> Result<RevokeOutcome, Failure> {
        self.database.transaction(move |tx| Box::pin(async move {
            let Some(_) = lock_epoch(tx, principal).await? else { return Ok(RevokeOutcome::Absent) };
            let Some(snapshot) = lock_session(tx, session, principal).await? else { return Ok(RevokeOutcome::Absent) };
            if snapshot.revoked_at_ms.is_some() {
                return Ok(RevokeOutcome::AlreadyInactive);
            }
            let revoked = Session::restore(snapshot)?.revoke(now_ms)?.snapshot();
            guarded(tx, sqlx::query("UPDATE public.n2f_identity_sessions SET revoked_at_ms=$2 WHERE id=$1::uuid AND revoked_at_ms IS NULL").bind(session.to_string()).bind(revoked.revoked_at_ms)).await?;
            enqueue(tx, &event).await?;
            Ok(RevokeOutcome::Revoked)
        })).await
    }
}
impl EpochStore for Store {
    async fn revoke_all(
        &self,
        principal: Id,
        expected_epoch: u32,
        _now_ms: i64,
        event: Envelope,
    ) -> Result<Commit, Failure> {
        commit_of(self.database.transaction(move |tx| Box::pin(async move {
            let Some(epoch) = lock_epoch(tx, principal).await? else { return Err(marker(STALE)) };
            if epoch != expected_epoch {
                return Err(marker(STALE));
            }
            let next = AuthState::new(principal, epoch)?.invalidate(expected_epoch)?.epoch();
            guarded(tx, sqlx::query("UPDATE public.n2f_identity_auth_states SET auth_epoch=$2 WHERE principal_id=$1::uuid AND auth_epoch=$3").bind(principal.to_string()).bind(version_in(next)).bind(version_in(epoch))).await?;
            enqueue(tx, &event).await
        })).await)
    }
}
impl ChallengeStore for Store {
    async fn issue_challenge(&self, record: IssueRecord) -> Result<Commit, Failure> {
        commit_of(
            self.database
                .transaction(move |tx| {
                    Box::pin(async move {
                        let ch = &record.challenge;
                        let Some((_, _, credential)) = lock_identity(tx, ch.principal_id).await?
                        else {
                            return Err(marker(STALE));
                        };
                        if credential.password_version != record.expected_password_version {
                            return Err(marker(STALE));
                        }
                        invalidate_outstanding(
                            tx,
                            ch.principal_id,
                            Some(ch.token_digest.purpose()),
                            ch.issued_at_ms,
                        )
                        .await?;
                        insert_challenge(tx, ch).await
                    })
                })
                .await,
        )
    }
}
impl VerifyStore for Store {
    async fn verify_email(&self, record: VerifyRecord) -> Result<Commit, Failure> {
        commit_of(self.database.transaction(move |tx| Box::pin(async move {
            let Some((_, _, credential)) = lock_identity(tx, record.principal_id).await? else { return Err(marker(STALE)) };
            if credential.password_version != record.expected_password_version {
                return Err(marker(STALE));
            }
            consume_challenge(tx, record.challenge_id, record.principal_id, TokenPurpose::EmailVerification, record.consumed_at_ms).await?;
            guarded(tx, sqlx::query("UPDATE public.n2f_identity_credentials SET verified_at_ms=COALESCE(verified_at_ms,$2) WHERE principal_id=$1::uuid").bind(record.principal_id.to_string()).bind(record.consumed_at_ms)).await?;
            enqueue(tx, &record.event).await
        })).await)
    }
}
impl PasswordStore for Store {
    async fn change_password(&self, record: ChangeRecord) -> Result<Commit, Failure> {
        commit_of(
            self.database
                .transaction(move |tx| {
                    Box::pin(async move {
                        let Some((epoch, _, credential)) =
                            lock_identity(tx, record.principal_id).await?
                        else {
                            return Err(marker(STALE));
                        };
                        if credential.password_version != record.expected_password_version
                            || epoch != record.expected_auth_epoch
                        {
                            return Err(marker(STALE));
                        }
                        replace_password(
                            tx,
                            record.principal_id,
                            epoch,
                            &credential,
                            &record.new_hash,
                            record.changed_at_ms,
                            false,
                        )
                        .await?;
                        enqueue_all(tx, &record.events).await
                    })
                })
                .await,
        )
    }
    async fn reset_password(&self, record: ResetRecord) -> Result<Commit, Failure> {
        commit_of(
            self.database
                .transaction(move |tx| {
                    Box::pin(async move {
                        let Some((epoch, _, credential)) =
                            lock_identity(tx, record.principal_id).await?
                        else {
                            return Err(marker(STALE));
                        };
                        if credential.password_version != record.expected_password_version
                            || epoch != record.expected_auth_epoch
                        {
                            return Err(marker(STALE));
                        }
                        consume_challenge(
                            tx,
                            record.challenge_id,
                            record.principal_id,
                            TokenPurpose::PasswordReset,
                            record.consumed_at_ms,
                        )
                        .await?;
                        replace_password(
                            tx,
                            record.principal_id,
                            epoch,
                            &credential,
                            &record.new_hash,
                            record.consumed_at_ms,
                            record.set_verified,
                        )
                        .await?;
                        enqueue_all(tx, &record.events).await
                    })
                })
                .await,
        )
    }
}
