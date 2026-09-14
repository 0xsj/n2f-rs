//! Authentication composition (AUTH_BUILD.md): settings, adapters wrapped into the
//! application ports, the identity transport routes and the safe startup manifest.
use crate::{
    domains::identity::{
        app::{
            self,
            command::{
                Admission, AttemptLimiter, ChallengeReader, ChallengeRecord, ChallengeStore,
                ChangeRecord, Clock, Commit, CredentialReader, CredentialRecord, EnrollmentPolicy,
                EpochStore, IdSource, IssueRecord, LoginRecord, LoginStore, MailDelivery,
                PasswordHasher, PasswordStore, RegisterOutcome, RegisterRecord, RegisterStore,
                ResetRecord, RevokeOutcome, SessionRevoker, TokenCodec, UpgradeTicketAdmission,
                UpgradeTicketRecord, UpgradeTicketStore, VerifyRecord, VerifyStore,
            },
            query::{Resolution, SessionResolver},
        },
        domain::{
            Status,
            email::Email,
            password::{NewPassword, PasswordInput},
            token::{TokenDigest, TokenPurpose},
        },
        infra::{
            local::{Blocklist, Limiter, LimiterConfig, UndeliveredMail},
            postgres::{Store, migration as identity_migration, upgrade_ticket_migration},
            redis::{Config as RedisLimiterConfig, RedisLimiter},
            smtp::{Config as MailConfig, Mailer, Security},
        },
        password_hash::{Hasher, Limits, Outcome},
        token_codec::Codec,
        transport::http::{
            Config as TransportConfig, Deps, Transport, WebSocketAdmission, principal_id,
        },
    },
    domains::org::infra::postgres::migration as org_migration,
    domains::org::{
        app::{
            self as org_app, Clock as OrgClock, Eligibility as OrgEligibility,
            IdSource as OrgIdSource, InvitationStore as OrgInvitationStore,
            OrganizationReader as OrgReader, OrganizationStore as OrgStorePort,
            PrincipalEligibility as OrgPrincipalEligibility,
        },
        infra::postgres::{Store as OrganizationStore, invitation_migration},
        transport::http::{
            Config as OrgTransportConfig, PrincipalId as OrgPrincipalId, Transport as OrgTransport,
        },
    },
    shared::{
        clock::SystemClock,
        entropy::OsEntropy,
        env::{Reader, Var},
        errors::{Failure, Kind},
        events::{Envelope, postgres::migration as events_migration},
        http::axum::{Admit, Route},
        id::{Id, V7},
        keyed::Digest,
        logger::{Fields, Logger, Value},
        postgres::{Database, Migration},
        secret::SecretString,
        socket::axum::{
            Admission as SocketAdmission, Authorize as SocketAuthorize,
            Revalidate as SocketRevalidate,
        },
    },
};
use std::{
    future::Future,
    net::IpAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

/// AUTH_* settings (AUTH_BUILD.md). Present only when AUTH_ENABLED=true.
pub struct AuthConfig {
    pub cookie_secure: bool,
    pub session_cookie: String,
    pub csrf_cookie: String,
    pub allowed_origins: Vec<String>,
    pub csrf_key: SecretString,
    pub csrf_ttl_ms: i64,
    pub session_absolute_ms: i64,
    pub session_idle_ms: i64,
    pub verification_ttl_ms: i64,
    pub reset_ttl_ms: i64,
    pub hash_max_concurrent: usize,
    pub hash_max_queued: usize,
    pub hash_queue_wait: Duration,
    pub blocklist_path: String,
    pub limiter_mode: String,
    pub redis_limiter: Option<RedisLimiterConfig>,
    /// SMTP delivery settings; `None` keeps mail disabled (AUTH_SMTP_HOST empty).
    pub mail: Option<MailConfig>,
}
fn invalid(field: &str, why: &str) -> Failure {
    Failure::new(Kind::Invalid, "invalid configuration")
        .with_type("env.invalid")
        .with_field(field, why)
}
/// Read the AUTH_* settings through the shared reader; `None` when disabled.
pub fn load_auth<L: Fn(&str) -> Option<String>>(
    r: &mut Reader<L>,
    host: IpAddr,
    smtp_password_present: bool,
) -> Result<Option<AuthConfig>, Failure> {
    if !r.boolean("AUTH_ENABLED", false) {
        return Ok(None);
    }
    let cookie_secure = r.boolean("AUTH_COOKIE_SECURE", true);
    let dev_insecure = r.boolean("AUTH_DEV_INSECURE_COOKIES", false);
    let (session_default, csrf_default) = if cookie_secure {
        ("__Host-n2f_session", "__Host-n2f_csrf")
    } else {
        ("n2f_session", "n2f_csrf")
    };
    let config = AuthConfig {
        cookie_secure,
        session_cookie: r.string("AUTH_SESSION_COOKIE", session_default),
        csrf_cookie: r.string("AUTH_CSRF_COOKIE", csrf_default),
        allowed_origins: r
            .required("AUTH_ALLOWED_ORIGINS")
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(String::from)
            .collect(),
        csrf_key: r.secret("AUTH_CSRF_KEY"),
        csrf_ttl_ms: r.int("AUTH_CSRF_TTL_MS", 3_600_000, 1, 3_600_000),
        session_absolute_ms: r.int(
            "AUTH_SESSION_ABSOLUTE_MS",
            43_200_000,
            1,
            253_402_300_799_999,
        ),
        session_idle_ms: r.int("AUTH_SESSION_IDLE_MS", 1_800_000, 1, 253_402_300_799_999),
        verification_ttl_ms: r.int(
            "AUTH_VERIFICATION_TTL_MS",
            86_400_000,
            1,
            253_402_300_799_999,
        ),
        reset_ttl_ms: r.int("AUTH_RESET_TTL_MS", 900_000, 1, 253_402_300_799_999),
        hash_max_concurrent: r.int("AUTH_HASH_MAX_CONCURRENT", 2, 1, 64) as usize,
        hash_max_queued: r.int("AUTH_HASH_MAX_QUEUED", 16, 0, 4096) as usize,
        hash_queue_wait: Duration::from_millis(
            r.int("AUTH_HASH_QUEUE_WAIT_MS", 2000, 1, 60_000) as u64
        ),
        blocklist_path: r.required("AUTH_BLOCKLIST_PATH"),
        limiter_mode: r.enumeration("AUTH_LIMITER", "local", &["local", "redis"]),
        redis_limiter: None,
        mail: load_mail(r, smtp_password_present)?,
    };
    let mut config = config;
    if config.limiter_mode == "redis" {
        config.redis_limiter = Some(RedisLimiterConfig {
            url: r.secret("AUTH_LIMITER_REDIS_URL"),
            timeout: Duration::from_millis(r.int("AUTH_LIMITER_TIMEOUT_MS", 500, 1, 10_000) as u64),
            max_keys: r.int("AUTH_LIMITER_MAX_KEYS", 100_000, 1, 1_000_000) as usize,
        });
    }
    if !(cookie_secure || dev_insecure && host.is_loopback()) {
        return Err(invalid(
            "AUTH_COOKIE_SECURE",
            "insecure cookies need AUTH_DEV_INSECURE_COOKIES=true and a loopback HTTP_HOST",
        ));
    }
    Ok(Some(config))
}

/// AUTH_SMTP_* and mail link settings (AUTH_BUILD.md). The password is read only
/// when a username is configured, so it is never recorded without its pair; a
/// password present without a username refuses startup. Nothing but the host is
/// read when the host is empty.
fn load_mail<L: Fn(&str) -> Option<String>>(
    r: &mut Reader<L>,
    password_present: bool,
) -> Result<Option<MailConfig>, Failure> {
    let host = r.string("AUTH_SMTP_HOST", "");
    if host.is_empty() {
        return Ok(None);
    }
    let port = r.int("AUTH_SMTP_PORT", 587, 1, 65_535) as u16;
    let security = match r
        .enumeration(
            "AUTH_SMTP_SECURITY",
            "starttls",
            &["none", "starttls", "tls"],
        )
        .as_str()
    {
        "none" => Security::None,
        "tls" => Security::Tls,
        _ => Security::StartTls,
    };
    let username = r.string("AUTH_SMTP_USERNAME", "");
    let (username, password) = if username.is_empty() {
        if password_present {
            return Err(invalid("AUTH_SMTP_PASSWORD", "requires AUTH_SMTP_USERNAME"));
        }
        (None, None)
    } else {
        (Some(username), Some(r.secret("AUTH_SMTP_PASSWORD")))
    };
    Ok(Some(MailConfig {
        host,
        port,
        security,
        username,
        password,
        from: r.required("AUTH_MAIL_FROM"),
        from_name: r.string("AUTH_MAIL_FROM_NAME", "n2f"),
        link_origin: r.required("AUTH_LINK_ORIGIN"),
        verify_path: r.string("AUTH_VERIFY_PATH", "/verify-email"),
        reset_path: r.string("AUTH_RESET_PATH", "/reset-password"),
        timeout_ms: r.int("AUTH_SMTP_TIMEOUT_MS", 5000, 1, 30_000),
    }))
}

/// Settings whose values identify infrastructure or people; the manifest shows
/// their names only (AUTH_BUILD.md).
const NAME_ONLY: [&str; 5] = [
    "AUTH_LIMITER_REDIS_URL",
    "AUTH_SMTP_HOST",
    "AUTH_SMTP_USERNAME",
    "AUTH_MAIL_FROM",
    "AUTH_LINK_ORIGIN",
];

/// The configured mail adapter behind one port implementation.
pub enum Mail {
    Undelivered(UndeliveredMail),
    Smtp(Mailer),
}
impl MailDelivery for Mail {
    async fn send_verification(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> bool {
        match self {
            Mail::Undelivered(m) => m.send_verification(email, token, expires_at_ms).await,
            Mail::Smtp(m) => m.send_verification(email, token, expires_at_ms).await,
        }
    }
    async fn send_reset(&self, email: &Email, token: &SecretString, expires_at_ms: i64) -> bool {
        match self {
            Mail::Undelivered(m) => m.send_reset(email, token, expires_at_ms).await,
            Mail::Smtp(m) => m.send_reset(email, token, expires_at_ms).await,
        }
    }
}

type WallSource = Box<dyn Fn() -> SystemTime + Send + Sync>;
/// One adapter set implementing every application port (AUTH_BUILD.md step 3).
#[derive(Clone)]
pub struct AuthPorts {
    clock: Arc<SystemClock>,
    ids: Arc<Mutex<V7<WallSource>>>,
    hasher: Arc<Hasher<OsEntropy>>,
    codec: Arc<Codec<OsEntropy>>,
    blocklist: Arc<Blocklist>,
    limiter: Arc<LimiterAdapter>,
    mail: Arc<Mail>,
    store: Arc<Store>,
    org_store: Arc<OrganizationStore>,
}
#[derive(Clone)]
enum LimiterAdapter {
    Local(Arc<Limiter>),
    Redis(Arc<RedisLimiter>),
}
impl AttemptLimiter for LimiterAdapter {
    fn admit(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> impl Future<Output = Result<Admission, Failure>> + Send {
        let operation = operation.to_owned();
        let subject = subject.reveal().to_owned();
        let source = source.to_owned();
        let this = match self {
            Self::Local(limiter) => Self::Local(limiter.clone()),
            Self::Redis(limiter) => Self::Redis(limiter.clone()),
        };
        async move {
            let subject = SecretString::new(subject);
            match this {
                Self::Local(limiter) => limiter.admit(&operation, &subject, &source).await,
                Self::Redis(limiter) => limiter.admit(&operation, &subject, &source).await,
            }
        }
    }
}
impl Clock for AuthPorts {
    fn now(&self) -> SystemTime {
        self.clock.now()
    }
}
impl IdSource for AuthPorts {
    fn new_id(&self) -> Result<Id, Failure> {
        self.ids.lock().unwrap_or_else(|e| e.into_inner()).new_id()
    }
}
impl PasswordHasher for AuthPorts {
    fn hash(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<SecretString, Failure>> + Send {
        self.hasher.hash(password)
    }
    async fn verify(&self, input: &PasswordInput, record: &SecretString) -> Result<bool, Failure> {
        self.hasher
            .verify(input, record)
            .await
            .map(|o| o == Outcome::Match)
    }
    async fn verify_absent(&self, input: &PasswordInput) -> Result<bool, Failure> {
        self.hasher.verify_absent(input).await.map(|_| false)
    }
}
impl TokenCodec for AuthPorts {
    fn issue(&self, purpose: TokenPurpose) -> Result<(SecretString, TokenDigest), Failure> {
        self.codec.issue(purpose).map(|i| (i.secret, i.digest))
    }
    fn digest(&self, purpose: TokenPurpose, secret: &SecretString) -> Result<TokenDigest, Failure> {
        self.codec.digest(purpose, secret)
    }
}
impl EnrollmentPolicy for AuthPorts {
    fn check_blocklist(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        self.blocklist.check_blocklist(password)
    }
}
impl AttemptLimiter for AuthPorts {
    fn admit(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> impl Future<Output = Result<Admission, Failure>> + Send {
        self.limiter.admit(operation, subject, source)
    }
}
impl MailDelivery for AuthPorts {
    fn send_verification(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send {
        self.mail.send_verification(email, token, expires_at_ms)
    }
    fn send_reset(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send {
        self.mail.send_reset(email, token, expires_at_ms)
    }
}
impl RegisterStore for AuthPorts {
    fn register(
        &self,
        record: RegisterRecord,
    ) -> impl Future<Output = Result<RegisterOutcome, Failure>> + Send {
        self.store.register(record)
    }
}
impl CredentialReader for AuthPorts {
    fn find_by_email(
        &self,
        email: &Email,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send {
        self.store.find_by_email(email)
    }
    fn find_by_principal(
        &self,
        principal: Id,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send {
        self.store.find_by_principal(principal)
    }
}
impl LoginStore for AuthPorts {
    fn commit_login(
        &self,
        record: LoginRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store.commit_login(record)
    }
}
impl SessionResolver for AuthPorts {
    fn resolve(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
        idle_ttl_ms: i64,
    ) -> impl Future<Output = Result<Resolution, Failure>> + Send {
        self.store.resolve(digest, now_ms, idle_ttl_ms)
    }
}
impl SessionRevoker for AuthPorts {
    fn revoke_session(
        &self,
        session: Id,
        principal: Id,
        now_ms: i64,
        event: Envelope,
    ) -> impl Future<Output = Result<RevokeOutcome, Failure>> + Send {
        self.store.revoke_session(session, principal, now_ms, event)
    }
}
impl EpochStore for AuthPorts {
    fn revoke_all(
        &self,
        principal: Id,
        expected_epoch: u32,
        now_ms: i64,
        event: Envelope,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store
            .revoke_all(principal, expected_epoch, now_ms, event)
    }
}
impl ChallengeReader for AuthPorts {
    fn find_by_digest(
        &self,
        purpose: TokenPurpose,
        digest: &TokenDigest,
    ) -> impl Future<Output = Result<Option<ChallengeRecord>, Failure>> + Send {
        self.store.find_by_digest(purpose, digest)
    }
}
impl ChallengeStore for AuthPorts {
    fn issue_challenge(
        &self,
        record: IssueRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store.issue_challenge(record)
    }
}
impl VerifyStore for AuthPorts {
    fn verify_email(
        &self,
        record: VerifyRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store.verify_email(record)
    }
}
impl PasswordStore for AuthPorts {
    fn change_password(
        &self,
        record: ChangeRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store.change_password(record)
    }
    fn reset_password(
        &self,
        record: ResetRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store.reset_password(record)
    }
}
impl UpgradeTicketStore for AuthPorts {
    fn issue_upgrade_ticket(
        &self,
        record: UpgradeTicketRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.store.issue_upgrade_ticket(record)
    }
    fn consume_upgrade_ticket(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
    ) -> impl Future<Output = Result<Option<UpgradeTicketAdmission>, Failure>> + Send {
        self.store.consume_upgrade_ticket(digest, now_ms)
    }
}

impl OrgClock for AuthPorts {
    fn now(&self) -> SystemTime {
        self.clock.now()
    }
}
impl OrgIdSource for AuthPorts {
    fn new_id(&self) -> Result<Id, Failure> {
        self.ids.lock().unwrap_or_else(|e| e.into_inner()).new_id()
    }
}
impl OrgPrincipalEligibility for AuthPorts {
    fn check_active(
        &self,
        principal_id: Id,
    ) -> impl Future<Output = Result<OrgEligibility, Failure>> + Send {
        let store = self.store.clone();
        async move {
            let record = store.find_by_principal(principal_id).await?;
            Ok(OrgEligibility {
                eligible: record.is_some_and(|record| record.principal.status == Status::Active),
            })
        }
    }
}
impl OrgStorePort for AuthPorts {
    fn create_organization(
        &self,
        record: org_app::CreateOrganizationRecord,
    ) -> impl Future<Output = Result<org_app::CreateOutcome, Failure>> + Send {
        let store = self.org_store.clone();
        async move { store.create_organization(record).await }
    }
}
impl OrgReader for AuthPorts {
    fn list_for_principal(
        &self,
        principal_id: Id,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<org_app::OrganizationMembership>, Failure>> + Send {
        let store = self.org_store.clone();
        async move { store.list_for_principal(principal_id, limit).await }
    }
}
impl OrgInvitationStore for AuthPorts {
    fn create_invitation(
        &self,
        record: org_app::InvitationRecord,
    ) -> impl Future<Output = Result<org_app::InvitationCreateOutcome, Failure>> + Send {
        let store = self.org_store.clone();
        async move { store.create_invitation(record).await }
    }
    fn accept_invitation(
        &self,
        input: org_app::AcceptInvitationInput,
        now_ms: i64,
        membership_id: Id,
    ) -> impl Future<
        Output = Result<
            (
                org_app::InvitationAcceptOutcome,
                Option<org_app::InvitationRecord>,
                Option<crate::domains::org::domain::MembershipSnapshot>,
            ),
            Failure,
        >,
    > + Send {
        let store = self.org_store.clone();
        async move { store.accept_invitation(input, now_ms, membership_id).await }
    }
    fn change_membership_role(
        &self,
        input: org_app::ChangeMembershipRoleInput,
    ) -> impl Future<
        Output = Result<
            (
                org_app::RoleChangeOutcome,
                Option<crate::domains::org::domain::MembershipSnapshot>,
            ),
            Failure,
        >,
    > + Send {
        let store = self.org_store.clone();
        async move { store.change_membership_role(input).await }
    }
}

/// Apply the ledger and build the identity routes plus the safe manifest fields.
pub async fn compose(
    config: &AuthConfig,
    database: Arc<Database>,
    clock: Arc<SystemClock>,
    manifest: &[Var],
    log: Logger,
    audit_schema: Option<Migration>,
) -> Result<(Vec<Route>, Fields, Admit, SocketAuthorize, SocketRevalidate), Failure> {
    let mut migrations = vec![
        events_migration(1),
        identity_migration(2),
        crate::shared::events::postgres::receipts_migration(3),
    ];
    if let Some(audit) = audit_schema {
        migrations.push(audit);
    }
    migrations.push(upgrade_ticket_migration(5));
    migrations.push(org_migration(6));
    migrations.push(invitation_migration(7));
    database.migrate(migrations).await?;
    let id_clock = clock.clone();
    let ids: V7<WallSource> = V7::new(Box::new(move || id_clock.now()));
    let keyed = Arc::new(Digest::new(SecretString::new(
        config.csrf_key.reveal().to_string(),
    ))?);
    let limiter_clock = clock.clone();
    let local_limiter_clock = clock.clone();
    let blocklist = Blocklist::load(Path::new(&config.blocklist_path))?;
    let entries = blocklist.len();
    let (mail, mail_mode) = match &config.mail {
        None => (Mail::Undelivered(UndeliveredMail::default()), "disabled"),
        Some(settings) => (
            Mail::Smtp(Mailer::new(
                MailConfig {
                    host: settings.host.clone(),
                    port: settings.port,
                    security: settings.security,
                    username: settings.username.clone(),
                    password: settings
                        .password
                        .as_ref()
                        .map(|p| SecretString::new(p.reveal().to_string())),
                    from: settings.from.clone(),
                    from_name: settings.from_name.clone(),
                    link_origin: settings.link_origin.clone(),
                    verify_path: settings.verify_path.clone(),
                    reset_path: settings.reset_path.clone(),
                    timeout_ms: settings.timeout_ms,
                },
                log,
            )?),
            "smtp",
        ),
    };
    let limiter = if let Some(redis_config) = &config.redis_limiter {
        LimiterAdapter::Redis(Arc::new(RedisLimiter::new(
            RedisLimiterConfig {
                url: SecretString::new(redis_config.url.reveal().to_owned()),
                timeout: redis_config.timeout,
                max_keys: redis_config.max_keys,
            },
            keyed.clone(),
            Arc::new(move || limiter_clock.now()),
        )?))
    } else {
        LimiterAdapter::Local(Arc::new(Limiter::new(
            keyed.clone(),
            Arc::new(move || local_limiter_clock.now()),
            LimiterConfig::defaults(),
        )?))
    };
    let limiter_mode = match &limiter {
        LimiterAdapter::Local(_) => "process_local",
        LimiterAdapter::Redis(_) => "redis",
    };
    let ports = AuthPorts {
        clock: clock.clone(),
        ids: Arc::new(Mutex::new(ids)),
        hasher: Arc::new(Hasher::new(
            OsEntropy,
            Limits {
                max_concurrent: config.hash_max_concurrent,
                max_queued: config.hash_max_queued,
                queue_wait: config.hash_queue_wait,
            },
        )?),
        codec: Arc::new(Codec::new(OsEntropy)?),
        blocklist: Arc::new(blocklist),
        limiter: Arc::new(limiter),
        mail: Arc::new(mail),
        store: Arc::new(Store::new(database.clone())),
        org_store: Arc::new(OrganizationStore::new(database)),
    };
    let org_ports = ports.clone();
    let transport = Transport::new(
        TransportConfig {
            session_cookie: config.session_cookie.clone(),
            csrf_cookie: config.csrf_cookie.clone(),
            secure: config.cookie_secure,
            allowed_origins: config.allowed_origins.clone(),
            csrf_ttl_ms: config.csrf_ttl_ms,
            session_absolute_ms: config.session_absolute_ms,
        },
        Deps {
            ports,
            app: app::Config::new(
                config.session_absolute_ms,
                config.session_idle_ms,
                config.verification_ttl_ms,
                config.reset_ttl_ms,
            )?,
            keyed,
            entropy: OsEntropy,
        },
    )?;
    let socket_transport = transport.clone();
    let socket_authorize: SocketAuthorize = Arc::new(move |headers| {
        let transport = socket_transport.clone();
        Box::pin(async move {
            transport
                .authorize_websocket(headers)
                .await
                .map(|admission| Arc::new(admission) as SocketAdmission)
        })
    });
    let revalidate_transport = transport.clone();
    let socket_revalidate: SocketRevalidate = Arc::new(move |value| {
        let transport = revalidate_transport.clone();
        Box::pin(async move {
            let admission = Arc::downcast::<WebSocketAdmission>(value).map_err(|_| {
                Failure::new(Kind::Internal, "invalid socket admission")
                    .with_type("socket.admission_corrupt")
            })?;
            transport.revalidate_websocket(&admission).await
        })
    });
    let settings = Value::Array(
        manifest
            .iter()
            .filter(|v| v.key.starts_with("AUTH_"))
            .map(|v| {
                Value::Object(Fields::from([
                    ("key".into(), v.key.clone().into()),
                    (
                        "value".into(),
                        if NAME_ONLY.contains(&v.key.as_str()) && !v.value.is_empty() {
                            "[REDACTED]".into()
                        } else {
                            v.value.clone().into()
                        },
                    ),
                    ("source".into(), v.source.clone().into()),
                    ("secret".into(), v.secret.into()),
                ]))
            })
            .collect(),
    );
    let fields = Fields::from([
        ("config".into(), settings),
        ("auth.blocklist_entries".into(), (entries as u64).into()),
        ("auth.mail".into(), mail_mode.into()),
        ("auth.limiter".into(), limiter_mode.into()),
    ]);
    let org_list = Arc::new(org_app::ListOrganizations::new(org_ports.clone())?);
    let org_create = Arc::new(org_app::CreateOrganization::new(org_ports.clone())?);
    let org_invite = Arc::new(org_app::InviteMember::new(org_ports.clone())?);
    let org_accept = Arc::new(org_app::AcceptInvitation::new(org_ports.clone())?);
    let org_role = Arc::new(org_app::ChangeMembershipRole::new(org_ports)?);
    let org_transport = Arc::new(OrgTransport::new(
        OrgTransportConfig {
            admission: transport.require_session_post(),
            read_admission: transport.require_session(),
            principal_id: Arc::new(principal_id) as OrgPrincipalId,
        },
        org_create,
        org_list,
        org_invite,
        org_accept,
        org_role,
    )?);
    let mut routes = transport.routes();
    routes.extend(org_transport.routes());
    Ok((
        routes,
        fields,
        transport.require_session(),
        socket_authorize,
        socket_revalidate,
    ))
}

/// AUTH_BUILD.md: SMTP credentials are both or neither; a stray password refuses
/// startup and is never recorded. Nothing but the host is read without a host.
#[cfg(test)]
mod mail_settings_tests {
    use super::load_auth;
    use crate::shared::env::Reader;
    use std::{collections::BTreeMap, net::IpAddr};

    fn reader(extra: &[(&str, &str)]) -> Reader<impl Fn(&str) -> Option<String>> {
        let mut values: BTreeMap<String, String> = [
            ("AUTH_ENABLED", "true"),
            ("AUTH_ALLOWED_ORIGINS", "http://127.0.0.1:7200"),
            ("AUTH_CSRF_KEY", "0123456789abcdef0123456789abcdef"),
            ("AUTH_BLOCKLIST_PATH", "config/password-blocklist.txt"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        for (k, v) in extra {
            values.insert((*k).to_owned(), (*v).to_owned());
        }
        Reader::new(move |k: &str| values.get(k).cloned())
    }
    fn loopback() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }
    const MAIL: [(&str, &str); 4] = [
        ("AUTH_SMTP_HOST", "127.0.0.1"),
        ("AUTH_SMTP_SECURITY", "none"),
        ("AUTH_MAIL_FROM", "no-reply@n2f.local"),
        ("AUTH_LINK_ORIGIN", "http://127.0.0.1:7200"),
    ];

    #[test]
    fn stray_password_without_username_refuses_startup() {
        let mut r = reader(&MAIL);
        assert!(load_auth(&mut r, loopback(), true).is_err());
    }

    #[test]
    fn mail_without_credentials_is_accepted() {
        let mut r = reader(&MAIL);
        assert!(matches!(load_auth(&mut r, loopback(), false), Ok(Some(_))));
        assert!(r.check().is_ok());
    }

    #[test]
    fn username_with_password_reads_the_secret_without_recording_it() {
        let mut extra = MAIL.to_vec();
        extra.push(("AUTH_SMTP_USERNAME", "mailer"));
        extra.push(("AUTH_SMTP_PASSWORD", "smtp-SENTINEL"));
        let mut r = reader(&extra);
        assert!(matches!(load_auth(&mut r, loopback(), true), Ok(Some(_))));
        let manifest = format!("{:?}", r.manifest().unwrap());
        assert!(!manifest.contains("SENTINEL"));
    }

    #[test]
    fn no_host_reads_no_other_mail_setting() {
        let mut r = reader(&[("AUTH_SMTP_PASSWORD", "smtp-SENTINEL")]);
        assert!(matches!(load_auth(&mut r, loopback(), true), Ok(Some(_))));
        let manifest = format!("{:?}", r.manifest().unwrap());
        assert!(!manifest.contains("AUTH_SMTP_PASSWORD") && !manifest.contains("SENTINEL"));
    }
}
