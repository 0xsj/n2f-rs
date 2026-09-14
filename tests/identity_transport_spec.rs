//! Executable specification for the identity HTTP transport (transport/http/CONTRACT.md):
//! the real axum boundary, the stage 4 operations on an in-memory store, the real
//! hasher (cheap work) and the real codec (fixed entropy).
use axum::{Router, body::Body, http::Request as HttpRequest};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use n2f_rs::{
    domains::identity::{
        app::{
            self, AuthenticatedPrincipal,
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
            Snapshot, Status,
            challenge::ChallengeSnapshot,
            credential::CredentialSnapshot,
            email::Email,
            password::{NewPassword, PasswordInput},
            session::{Session, SessionSnapshot},
            token::{TokenDigest, TokenPurpose},
            upgrade_ticket::UpgradeTicket,
        },
        password_hash::{Hasher, Limits, Outcome},
        token_codec::Codec,
        transport::http::{Config, Deps, Transport},
    },
    shared::{
        clock::SystemClock,
        entropy::EntropyError,
        errors::{Classified, Failure},
        http::axum::{Config as ServerConfig, Noop, Peer, Server},
        id::{Id, V7},
        keyed::Digest,
        logger::{Config as LogConfig, Resource, Runtime},
        provenance as p,
        secret::SecretString,
    },
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    io::Write,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const NOW: i64 = 1_700_000_000_000;
const ORIGIN: &str = "http://127.0.0.1:3000";
const PASSWORD: &str = "correct horse battery SENTINEL";
const EMAIL: &str = "Ada@Example.com";

// ---- In-memory fake ports ----------------------------------------------------------

struct Rec {
    principal: Snapshot,
    credential: CredentialSnapshot,
    epoch: u32,
}
#[derive(Default)]
struct Mem {
    creds: HashMap<String, Rec>,
    sessions: HashMap<[u8; 32], SessionSnapshot>,
    challenges: HashMap<(String, [u8; 32]), ChallengeSnapshot>,
    tickets:
        HashMap<[u8; 32], n2f_rs::domains::identity::domain::upgrade_ticket::UpgradeTicketSnapshot>,
    mail: Vec<(String, String, String)>,
    limiter_refuse: bool,
    resolves: usize,
    next_id: u32,
    now_ms: i64,
}
type FakeEntropy = Box<dyn FnMut(&mut [u8]) -> Result<(), EntropyError> + Send>;
#[derive(Clone)]
struct Fake {
    mem: Arc<Mutex<Mem>>,
    hasher: Arc<Hasher<FakeEntropy>>,
    codec: Arc<Codec<FakeEntropy>>,
}
fn counter_entropy(seed: u8) -> FakeEntropy {
    let mut n: u64 = u64::from(seed);
    Box::new(move |bytes: &mut [u8]| {
        n += 1;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (n as u8).wrapping_mul(31).wrapping_add(i as u8);
        }
        Ok(())
    })
}
fn cheap_work(password: &[u8], salt: &[u8; 16]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(password);
    h.update(salt);
    h.finalize().into()
}
impl Fake {
    fn new() -> Self {
        let hasher = Hasher::with_work(
            counter_entropy(1),
            Limits {
                max_concurrent: 2,
                max_queued: 8,
                queue_wait: Duration::from_secs(1),
            },
            Arc::new(cheap_work),
        )
        .unwrap();
        Self {
            mem: Arc::new(Mutex::new(Mem {
                next_id: 1,
                now_ms: NOW,
                ..Default::default()
            })),
            hasher: Arc::new(hasher),
            codec: Arc::new(Codec::new(counter_entropy(7)).unwrap()),
        }
    }
    fn with<T>(&self, f: impl FnOnce(&mut Mem) -> T) -> T {
        f(&mut self.mem.lock().unwrap())
    }
    fn set_now(&self, ms: i64) {
        self.with(|m| m.now_ms = ms);
    }
    fn last_mail(&self, kind: &str) -> Option<String> {
        self.with(|m| {
            m.mail
                .iter()
                .rev()
                .find(|(k, _, _)| k == kind)
                .map(|(_, _, t)| t.clone())
        })
    }
    fn resolves(&self) -> usize {
        self.with(|m| m.resolves)
    }
}
impl Clock for Fake {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.with(|m| m.now_ms) as u64)
    }
}
impl IdSource for Fake {
    fn new_id(&self) -> Result<Id, Failure> {
        let n = self.with(|m| {
            m.next_id += 1;
            m.next_id
        });
        Ok(Id::parse(&format!("01900000-0000-7000-8000-{n:012x}")).unwrap())
    }
}
impl PasswordHasher for Fake {
    fn hash(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<SecretString, Failure>> + Send {
        let h = self.hasher.clone();
        let p = NewPassword::parse(SecretString::new(password.secret().reveal().into())).unwrap();
        async move { h.hash(&p).await }
    }
    fn verify(
        &self,
        input: &PasswordInput,
        record: &SecretString,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        let h = self.hasher.clone();
        let i = PasswordInput::parse(SecretString::new(input.secret().reveal().into())).unwrap();
        let r = SecretString::new(record.reveal().into());
        async move { h.verify(&i, &r).await.map(|o| o == Outcome::Match) }
    }
    fn verify_absent(
        &self,
        input: &PasswordInput,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        let h = self.hasher.clone();
        let i = PasswordInput::parse(SecretString::new(input.secret().reveal().into())).unwrap();
        async move { h.verify_absent(&i).await.map(|_| false) }
    }
}
impl TokenCodec for Fake {
    fn issue(&self, purpose: TokenPurpose) -> Result<(SecretString, TokenDigest), Failure> {
        self.codec.issue(purpose).map(|i| (i.secret, i.digest))
    }
    fn digest(&self, purpose: TokenPurpose, secret: &SecretString) -> Result<TokenDigest, Failure> {
        self.codec.digest(purpose, secret)
    }
}
impl EnrollmentPolicy for Fake {
    fn check_blocklist(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        let allowed = password.secret().reveal() != "password123456789";
        async move { Ok(allowed) }
    }
}
impl AttemptLimiter for Fake {
    fn admit(
        &self,
        _: &str,
        _: &SecretString,
        _: &str,
    ) -> impl Future<Output = Result<Admission, Failure>> + Send {
        let refuse = self.with(|m| m.limiter_refuse);
        async move {
            Ok(if refuse {
                Admission::Refused {
                    retry_after_ms: 30_000,
                }
            } else {
                Admission::Permitted
            })
        }
    }
}
impl MailDelivery for Fake {
    fn send_verification(
        &self,
        email: &Email,
        token: &SecretString,
        _: i64,
    ) -> impl Future<Output = bool> + Send {
        self.with(|m| {
            m.mail.push((
                "verification".into(),
                email.reveal().into(),
                token.reveal().into(),
            ))
        });
        async { true }
    }
    fn send_reset(
        &self,
        email: &Email,
        token: &SecretString,
        _: i64,
    ) -> impl Future<Output = bool> + Send {
        self.with(|m| {
            m.mail
                .push(("reset".into(), email.reveal().into(), token.reveal().into()))
        });
        async { true }
    }
}
fn key(d: &TokenDigest) -> (String, [u8; 32]) {
    (d.purpose().as_str().to_string(), d.bytes())
}
impl RegisterStore for Fake {
    fn register(
        &self,
        r: RegisterRecord,
    ) -> impl Future<Output = Result<RegisterOutcome, Failure>> + Send {
        let out = self.with(|m| {
            let email = r.credential.email.reveal().to_string();
            if m.creds.contains_key(&email) {
                return RegisterOutcome::DuplicateEmail;
            }
            m.challenges
                .insert(key(&r.challenge.token_digest), r.challenge);
            m.creds.insert(
                email,
                Rec {
                    principal: r.principal,
                    credential: r.credential,
                    epoch: r.auth_epoch,
                },
            );
            RegisterOutcome::Created
        });
        async move { Ok(out) }
    }
}
fn record(r: &Rec) -> CredentialRecord {
    CredentialRecord {
        principal: r.principal.clone(),
        credential: CredentialSnapshot {
            principal_id: r.credential.principal_id,
            email: Email::parse(r.credential.email.reveal()).unwrap(),
            password_hash: SecretString::new(r.credential.password_hash.reveal().into()),
            verified_at_ms: r.credential.verified_at_ms,
            password_version: r.credential.password_version,
            created_at_ms: r.credential.created_at_ms,
            changed_at_ms: r.credential.changed_at_ms,
        },
        auth_epoch: r.epoch,
    }
}
impl CredentialReader for Fake {
    fn find_by_email(
        &self,
        email: &Email,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send {
        let found = self.with(|m| m.creds.get(email.reveal()).map(record));
        async move { Ok(found) }
    }
    fn find_by_principal(
        &self,
        principal: Id,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send {
        let found = self.with(|m| {
            m.creds
                .values()
                .find(|r| r.principal.id == principal)
                .map(record)
        });
        async move { Ok(found) }
    }
}
impl LoginStore for Fake {
    fn commit_login(&self, r: LoginRecord) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let Some(rec) = m
                .creds
                .values()
                .find(|c| c.principal.id == r.session.principal_id)
            else {
                return Commit::Stale;
            };
            if rec.principal.status != Status::Active
                || rec.credential.verified_at_ms.is_none()
                || rec.credential.password_version != r.expected_password_version
                || rec.epoch != r.expected_auth_epoch
            {
                return Commit::Stale;
            }
            m.sessions.insert(r.session.token_digest.bytes(), r.session);
            Commit::Committed
        });
        async move { Ok(out) }
    }
}
impl SessionResolver for Fake {
    fn resolve(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
        idle_ttl_ms: i64,
    ) -> impl Future<Output = Result<Resolution, Failure>> + Send {
        let out = self.with(|m| {
            m.resolves += 1;
            let Some(s) = m.sessions.get(&digest.bytes()).cloned() else {
                return Resolution::Absent;
            };
            let session = Session::restore(s.clone()).unwrap();
            if session.check(now_ms).is_err() {
                return Resolution::Rejected;
            }
            let Some(rec) = m.creds.values().find(|c| c.principal.id == s.principal_id) else {
                return Resolution::Rejected;
            };
            if rec.principal.status != Status::Active
                || rec.credential.verified_at_ms.is_none()
                || rec.epoch != s.auth_epoch
            {
                return Resolution::Rejected;
            }
            let touched = session.touch(now_ms, idle_ttl_ms).unwrap().snapshot();
            m.sessions.insert(digest.bytes(), touched);
            Resolution::Admitted(AuthenticatedPrincipal {
                principal_id: s.principal_id,
                session_id: s.id,
                auth_epoch: s.auth_epoch,
            })
        });
        async move { Ok(out) }
    }
}
impl SessionRevoker for Fake {
    fn revoke_session(
        &self,
        session: Id,
        principal: Id,
        now_ms: i64,
        _: n2f_rs::shared::events::Envelope,
    ) -> impl Future<Output = Result<RevokeOutcome, Failure>> + Send {
        let out = self.with(|m| {
            let Some(s) = m
                .sessions
                .values_mut()
                .find(|s| s.id == session && s.principal_id == principal)
            else {
                return RevokeOutcome::Absent;
            };
            if s.revoked_at_ms.is_some() {
                return RevokeOutcome::AlreadyInactive;
            }
            s.revoked_at_ms = Some(now_ms);
            RevokeOutcome::Revoked
        });
        async move { Ok(out) }
    }
}
impl EpochStore for Fake {
    fn revoke_all(
        &self,
        principal: Id,
        expected_epoch: u32,
        _: i64,
        _: n2f_rs::shared::events::Envelope,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let Some(rec) = m.creds.values_mut().find(|c| c.principal.id == principal) else {
                return Commit::Stale;
            };
            if rec.epoch != expected_epoch {
                return Commit::Stale;
            }
            rec.epoch += 1;
            Commit::Committed
        });
        async move { Ok(out) }
    }
}
impl ChallengeReader for Fake {
    fn find_by_digest(
        &self,
        purpose: TokenPurpose,
        digest: &TokenDigest,
    ) -> impl Future<Output = Result<Option<ChallengeRecord>, Failure>> + Send {
        let out = self.with(|m| {
            let c = m
                .challenges
                .get(&(purpose.as_str().to_string(), digest.bytes()))?
                .clone();
            let rec = m
                .creds
                .values()
                .find(|r| r.principal.id == c.principal_id)?;
            let r = record(rec);
            Some(ChallengeRecord {
                challenge: c,
                credential: r.credential,
                principal_status: r.principal.status,
                auth_epoch: r.auth_epoch,
            })
        });
        async move { Ok(out) }
    }
}
impl ChallengeStore for Fake {
    fn issue_challenge(
        &self,
        r: IssueRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let Some(rec) = m
                .creds
                .values()
                .find(|c| c.principal.id == r.challenge.principal_id)
            else {
                return Commit::Stale;
            };
            if rec.credential.password_version != r.expected_password_version {
                return Commit::Stale;
            }
            let purpose = r.challenge.token_digest.purpose();
            for (k, c) in m.challenges.iter_mut() {
                if c.principal_id == r.challenge.principal_id
                    && k.0 == purpose.as_str()
                    && c.consumed_at_ms.is_none()
                    && c.invalidated_at_ms.is_none()
                {
                    c.invalidated_at_ms = Some(r.challenge.issued_at_ms);
                }
            }
            m.challenges
                .insert(key(&r.challenge.token_digest), r.challenge);
            Commit::Committed
        });
        async move { Ok(out) }
    }
}
impl VerifyStore for Fake {
    fn verify_email(
        &self,
        r: VerifyRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let Some(c) = m.challenges.values_mut().find(|c| c.id == r.challenge_id) else {
                return Commit::Stale;
            };
            if c.consumed_at_ms.is_some() || c.invalidated_at_ms.is_some() {
                return Commit::Stale;
            }
            c.consumed_at_ms = Some(r.consumed_at_ms);
            let rec = m
                .creds
                .values_mut()
                .find(|x| x.principal.id == r.principal_id)
                .unwrap();
            if rec.credential.password_version != r.expected_password_version {
                return Commit::Stale;
            }
            rec.credential
                .verified_at_ms
                .get_or_insert(r.consumed_at_ms);
            Commit::Committed
        });
        async move { Ok(out) }
    }
}
impl PasswordStore for Fake {
    fn change_password(
        &self,
        r: ChangeRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let Some(rec) = m
                .creds
                .values_mut()
                .find(|x| x.principal.id == r.principal_id)
            else {
                return Commit::Stale;
            };
            if rec.credential.password_version != r.expected_password_version
                || rec.epoch != r.expected_auth_epoch
            {
                return Commit::Stale;
            }
            rec.credential.password_hash = r.new_hash;
            rec.credential.password_version += 1;
            rec.credential.changed_at_ms = r.changed_at_ms;
            rec.epoch += 1;
            Commit::Committed
        });
        async move { Ok(out) }
    }
    fn reset_password(
        &self,
        r: ResetRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let Some(c) = m.challenges.values_mut().find(|c| c.id == r.challenge_id) else {
                return Commit::Stale;
            };
            if c.consumed_at_ms.is_some() || c.invalidated_at_ms.is_some() {
                return Commit::Stale;
            }
            c.consumed_at_ms = Some(r.consumed_at_ms);
            let rec = m
                .creds
                .values_mut()
                .find(|x| x.principal.id == r.principal_id)
                .unwrap();
            if rec.credential.password_version != r.expected_password_version
                || rec.epoch != r.expected_auth_epoch
            {
                return Commit::Stale;
            }
            rec.credential.password_hash = r.new_hash;
            rec.credential.password_version += 1;
            rec.credential.changed_at_ms = r.consumed_at_ms;
            if r.set_verified {
                rec.credential
                    .verified_at_ms
                    .get_or_insert(r.consumed_at_ms);
            }
            rec.epoch += 1;
            Commit::Committed
        });
        async move { Ok(out) }
    }
}
impl UpgradeTicketStore for Fake {
    fn issue_upgrade_ticket(
        &self,
        r: UpgradeTicketRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        let out = self.with(|m| {
            let session = m
                .sessions
                .values()
                .find(|s| s.id == r.ticket.session_id)
                .cloned();
            let Some(session) = session else {
                return Commit::Stale;
            };
            let Some(rec) = m
                .creds
                .values()
                .find(|x| x.principal.id == session.principal_id)
            else {
                return Commit::Stale;
            };
            let usable = Session::restore(session.clone())
                .ok()
                .is_some_and(|s| s.check(m.now_ms).is_ok());
            if !usable
                || session.auth_epoch != r.expected_auth_epoch
                || rec.epoch != r.expected_auth_epoch
                || rec.principal.status != Status::Active
                || rec.credential.verified_at_ms.is_none()
            {
                return Commit::Stale;
            }
            m.tickets.insert(r.ticket.token_digest.bytes(), r.ticket);
            Commit::Committed
        });
        async move { Ok(out) }
    }
    fn consume_upgrade_ticket(
        &self,
        digest: &TokenDigest,
        now_ms: i64,
    ) -> impl Future<Output = Result<Option<UpgradeTicketAdmission>, Failure>> + Send {
        let key = digest.bytes();
        let out = self.with(|m| {
            let ticket = m.tickets.get(&key).cloned()?;
            let session = m
                .sessions
                .values()
                .find(|s| s.id == ticket.session_id)
                .cloned()?;
            let rec = m
                .creds
                .values()
                .find(|x| x.principal.id == session.principal_id)?;
            if rec.principal.status != Status::Active
                || rec.credential.verified_at_ms.is_none()
                || session.auth_epoch != rec.epoch
                || Session::restore(session.clone())
                    .ok()
                    .is_none_or(|s| s.check(now_ms).is_err())
            {
                return None;
            }
            let Ok(used) = UpgradeTicket::restore(ticket).and_then(|t| t.consume(now_ms)) else {
                return None;
            };
            m.tickets.insert(key, used.snapshot());
            Some(UpgradeTicketAdmission {
                principal_id: session.principal_id,
                session_id: session.id,
                auth_epoch: session.auth_epoch,
            })
        });
        async move { Ok(out) }
    }
}

// ---- Server harness ------------------------------------------------------------------

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Harness {
    address: SocketAddr,
    sink: Sink,
    _runtime: Runtime,
    fake: Fake,
    opened: Arc<Mutex<Vec<(String, String)>>>,
}
fn config(secure: bool) -> Config {
    Config {
        session_cookie: if secure {
            "__Host-n2f_session".into()
        } else {
            "n2f_session".into()
        },
        csrf_cookie: if secure {
            "__Host-n2f_csrf".into()
        } else {
            "n2f_csrf".into()
        },
        secure,
        allowed_origins: vec![ORIGIN.into()],
        csrf_ttl_ms: 3_600_000,
        session_absolute_ms: 43_200_000,
    }
}
async fn start(fake: Fake, secure: bool) -> Harness {
    let sink = Sink::default();
    let mut lc = LogConfig::new(
        Resource {
            name: "test".into(),
            ..Default::default()
        },
        Arc::new(SystemTime::now),
        Box::new(sink.clone()),
    );
    lc.format = "json".into();
    let runtime = Runtime::new(lc).unwrap();
    let transport = Transport::new(
        config(secure),
        Deps {
            ports: fake.clone(),
            app: app::Config::defaults(),
            keyed: Arc::new(
                Digest::new(SecretString::new("0123456789abcdef0123456789abcdef".into())).unwrap(),
            ),
            entropy: counter_entropy(3),
        },
    )
    .unwrap();
    let clock = SystemClock::new();
    let ids = Arc::new(Mutex::new(V7::new(move || clock.now())));
    let factory = Arc::new(Mutex::new(p::Factory::new(SystemTime::now, move || {
        ids.lock().unwrap().new_id()
    })));
    let executor = p::Actor::new(p::ActorKind::Service, "test".into()).unwrap();
    let anonymous = p::Attribution::new(p::AttributionSpec {
        initiator: Some(p::Actor::anonymous()),
        ..Default::default()
    })
    .unwrap();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    let open = Arc::new(
        move |name: &str, incoming: &p::IncomingResult, attribution: Option<p::Attribution>| {
            let a = attribution.unwrap_or_else(|| anonymous.clone());
            let initiator = a
                .snapshot()
                .initiator
                .map(|i| {
                    format!(
                        "{}:{}",
                        format!("{:?}", i.kind()).to_lowercase(),
                        i.identity()
                    )
                })
                .unwrap_or_default();
            seen.lock().unwrap().push((name.to_string(), initiator));
            factory.lock().unwrap().enter(
                p::RootSpec {
                    work_id: None,
                    origin: p::Origin::Request,
                    operation: p::Operation::new(name.into())?,
                    attribution: a,
                    executor: executor.clone(),
                },
                incoming,
            )
        },
    );
    let server = Arc::new(
        Server::new(ServerConfig {
            routes: transport.routes(),
            observer: Arc::new(Noop),
            log: runtime.log.clone(),
            open,
            source: Arc::new(|peer: Option<SocketAddr>, _| {
                peer.map(|a| a.ip().to_string()).unwrap_or_default()
            }),
            timeout: Duration::from_secs(5),
            max_body: 4096,
        })
        .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                break;
            };
            let server = server.clone();
            let app = Router::new().fallback(move |mut req: HttpRequest<Body>| {
                let server = server.clone();
                async move {
                    req.extensions_mut().insert(Peer(peer));
                    server.handle(req).await
                }
            });
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), TowerToHyperService::new(app))
                    .await;
            });
        }
    });
    Harness {
        address,
        sink,
        _runtime: runtime,
        fake,
        opened,
    }
}

// ---- A cookie-carrying client ----------------------------------------------------------

struct Reply {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: serde_json::Value,
}
impl Reply {
    fn code(&self) -> String {
        self.body["code"].as_str().unwrap_or("").to_string()
    }
    fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect()
    }
}
struct Client {
    base: String,
    cookies: Vec<(String, String)>,
    http: reqwest::Client,
}
impl Client {
    fn new(h: &Harness) -> Self {
        Self {
            base: format!("http://{}", h.address),
            cookies: vec![],
            http: reqwest::Client::new(),
        }
    }
    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }
    async fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&str>,
        origin: Option<&str>,
        csrf: Option<&str>,
        extra_cookie: Option<(&str, &str)>,
    ) -> Reply {
        let mut r = self
            .http
            .request(method.parse().unwrap(), format!("{}{}", self.base, path));
        if let Some(o) = origin {
            r = r.header("origin", o);
        }
        if let Some(t) = csrf {
            r = r.header("x-csrf-token", t);
        }
        let mut jar: Vec<String> = self
            .cookies
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        if let Some((n, v)) = extra_cookie {
            jar.push(format!("{n}={v}"));
        }
        if !jar.is_empty() {
            r = r.header("cookie", jar.join("; "));
        }
        if let Some(b) = body {
            r = r
                .header("content-type", "application/json")
                .body(b.to_string());
        }
        let response = r.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        for v in headers.get_all("set-cookie") {
            let text = v.to_str().unwrap();
            let (name, rest) = text.split_once('=').unwrap();
            let value = rest.split(';').next().unwrap().to_string();
            self.cookies.retain(|(n, _)| n != name);
            if !text.to_ascii_lowercase().contains("max-age=0") {
                self.cookies.push((name.to_string(), value));
            }
        }
        let text = response.text().await.unwrap();
        let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        Reply {
            status,
            headers,
            body,
        }
    }
    async fn get(&mut self, path: &str) -> Reply {
        self.send("GET", path, None, None, None, None).await
    }
    async fn post(&mut self, path: &str, body: Option<&str>, csrf: &str) -> Reply {
        self.send("POST", path, body, Some(ORIGIN), Some(csrf), None)
            .await
    }
    async fn csrf(&mut self) -> String {
        let r = self.get("/v1/auth/csrf").await;
        assert_eq!(r.status, 200);
        r.body["csrf_token"].as_str().unwrap().to_string()
    }
}
fn login_body(email: &str, password: &str) -> String {
    serde_json::json!({"email": email, "password": password}).to_string()
}
/// Register, verify through the recorded mail, and log in; returns the client and its CSRF token.
async fn logged_in(h: &Harness) -> (Client, String) {
    logged_in_as(h, EMAIL).await
}
async fn logged_in_as(h: &Harness, email: &str) -> (Client, String) {
    let mut c = Client::new(h);
    let token = c.csrf().await;
    let r = c
        .post(
            "/v1/auth/register",
            Some(&login_body(email, PASSWORD)),
            &token,
        )
        .await;
    assert_eq!(
        (r.status, r.body.clone()),
        (202, serde_json::json!({"accepted": true}))
    );
    let mail = h.fake.last_mail("verification").unwrap();
    let r = c
        .post(
            "/v1/auth/email/verify",
            Some(&serde_json::json!({"token": mail, "password": PASSWORD}).to_string()),
            &token,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    let r = c
        .post("/v1/auth/login", Some(&login_body(email, PASSWORD)), &token)
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let rotated = r.body["csrf_token"].as_str().unwrap().to_string();
    (c, rotated)
}

// ---- Scenarios --------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn r02_r04_csrf_issuance_and_no_store() {
    let h = start(Fake::new(), true).await;
    let mut c = Client::new(&h);
    let r = c.get("/v1/auth/csrf").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.headers.get("cache-control").unwrap(), "no-store");
    let token = r.body["csrf_token"].as_str().unwrap().to_string();
    assert_eq!(token.split('.').count(), 3);
    let cookies = r.set_cookies();
    assert_eq!(cookies.len(), 1);
    let cookie = &cookies[0];
    assert!(
        cookie.starts_with(&format!("__Host-n2f_csrf={token}; Path=/; Max-Age=3600")),
        "{cookie}"
    );
    for attribute in ["; Secure", "; HttpOnly", "; SameSite=Lax"] {
        assert!(cookie.contains(attribute), "{cookie}");
    }
    // Each issuance is a fresh nonce.
    let again = c.csrf().await;
    assert_ne!(again, token);
}

#[tokio::test(flavor = "multi_thread")]
async fn r05_origin_admission() {
    let h = start(Fake::new(), false).await;
    let mut c = Client::new(&h);
    let token = c.csrf().await;
    let body = login_body(EMAIL, PASSWORD);
    for origin in [None, Some("http://evil.example")] {
        let r = c
            .send(
                "POST",
                "/v1/auth/register",
                Some(&body),
                origin,
                Some(&token),
                None,
            )
            .await;
        assert_eq!(
            (r.status, r.code()),
            (403, "identity.origin_rejected".into())
        );
    }
    let r = c
        .http
        .post(format!("{}/v1/auth/register", c.base))
        .header("origin", ORIGIN)
        .header("origin", ORIGIN)
        .header("x-csrf-token", &token)
        .header("cookie", format!("n2f_csrf={token}"))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);
    // GET routes do not check Origin.
    assert_eq!(c.get("/v1/auth/csrf").await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn r06_csrf_admission() {
    let h = start(Fake::new(), false).await;
    let mut c = Client::new(&h);
    let token = c.csrf().await;
    let body = login_body(EMAIL, PASSWORD);
    // Header differs from the cookie.
    let r = c
        .post(
            "/v1/auth/register",
            Some(&body),
            &format!("{}x", &token[..token.len() - 1]),
        )
        .await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    // Missing header.
    let r = c
        .send(
            "POST",
            "/v1/auth/register",
            Some(&body),
            Some(ORIGIN),
            None,
            None,
        )
        .await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    // Missing cookie.
    let saved = std::mem::take(&mut c.cookies);
    let r = c.post("/v1/auth/register", Some(&body), &token).await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    c.cookies = saved;
    // Duplicate CSRF cookie.
    let r = c
        .send(
            "POST",
            "/v1/auth/register",
            Some(&body),
            Some(ORIGIN),
            Some(&token),
            Some(("n2f_csrf", &token)),
        )
        .await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    // Forged tag with a valid shape.
    let mut parts: Vec<&str> = token.split('.').collect();
    let forged_tag = "A".repeat(43);
    parts[2] = &forged_tag;
    let forged = parts.join(".");
    c.cookies = vec![("n2f_csrf".into(), forged.clone())];
    let r = c.post("/v1/auth/register", Some(&body), &forged).await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    // Expired context: one hour and one millisecond later.
    c.cookies = vec![("n2f_csrf".into(), token.clone())];
    h.fake.set_now(NOW + 3_600_001);
    let r = c.post("/v1/auth/register", Some(&body), &token).await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    h.fake.set_now(NOW + 3_600_000);
    let r = c.post("/v1/auth/register", Some(&body), &token).await;
    assert_eq!(r.status, 202, "{}", r.body);
}

#[tokio::test(flavor = "multi_thread")]
async fn r06_csrf_binding_between_anonymous_and_session_contexts() {
    let h = start(Fake::new(), false).await;
    let (mut c, session_token) = logged_in(&h).await;
    // A session-bound token is refused on the anonymous register route once the
    // session is gone (the binding no longer matches), and an anonymous token is
    // refused on a session-bound route.
    let anonymous = {
        let mut a = Client::new(&h);
        a.csrf().await
    };
    let r = c
        .send(
            "POST",
            "/v1/auth/logout-all",
            None,
            Some(ORIGIN),
            Some(&anonymous),
            None,
        )
        .await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    let session_cookie = c.cookie("n2f_session").unwrap();
    let mut other = Client::new(&h);
    other.cookies = vec![("n2f_csrf".into(), session_token.clone())];
    let r = other
        .post(
            "/v1/auth/register",
            Some(&login_body("x@example.com", PASSWORD)),
            &session_token,
        )
        .await;
    assert_eq!((r.status, r.code()), (403, "identity.csrf_rejected".into()));
    // With the session cookie the bound token still works on a session route.
    other.cookies.push(("n2f_session".into(), session_cookie));
    let r = other
        .send(
            "POST",
            "/v1/auth/logout-all",
            None,
            Some(ORIGIN),
            Some(&session_token),
            None,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
}

#[tokio::test(flavor = "multi_thread")]
async fn r02_r09_bodies_and_accepted_responses() {
    let h = start(Fake::new(), false).await;
    let mut c = Client::new(&h);
    let token = c.csrf().await;
    for raw in [
        r#"{"email": "a@example.com", "password": "correct horse battery", "extra": 1}"#,
        r#"{"email": "a@example.com", "password": 5}"#,
        r#"{"email": "a@example.com", "email": "b@example.com", "password": "correct horse battery"}"#,
        r#"["a@example.com"]"#,
        "",
        "not json",
    ] {
        let r = c.post("/v1/auth/register", Some(raw), &token).await;
        assert_eq!(
            (r.status, r.code()),
            (400, "http.invalid_body".into()),
            "{raw}"
        );
    }
    let r = c
        .post(
            "/v1/auth/register",
            Some(&login_body(EMAIL, "password123456789")),
            &token,
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (400, "identity.password_invalid".into())
    );
    let r = c
        .post(
            "/v1/auth/register",
            Some(&login_body(EMAIL, "short")),
            &token,
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (400, "identity.password_invalid".into())
    );
    let first = c
        .post(
            "/v1/auth/register",
            Some(&login_body(EMAIL, PASSWORD)),
            &token,
        )
        .await;
    let duplicate = c
        .post(
            "/v1/auth/register",
            Some(&login_body(EMAIL, PASSWORD)),
            &token,
        )
        .await;
    assert_eq!(
        (first.status, first.body.clone()),
        (202, serde_json::json!({"accepted": true}))
    );
    assert_eq!(
        (duplicate.status, duplicate.body.clone()),
        (202, serde_json::json!({"accepted": true}))
    );
    assert_eq!(first.headers.get("cache-control").unwrap(), "no-store");
    for path in [
        "/v1/auth/email/verification-requests",
        "/v1/auth/password/reset-requests",
    ] {
        let known = c
            .post(
                path,
                Some(&serde_json::json!({"email": EMAIL}).to_string()),
                &token,
            )
            .await;
        let unknown = c
            .post(
                path,
                Some(&serde_json::json!({"email": "nobody@example.com"}).to_string()),
                &token,
            )
            .await;
        assert_eq!(
            (known.status, known.body),
            (202, serde_json::json!({"accepted": true})),
            "{path}"
        );
        assert_eq!(
            (unknown.status, unknown.body),
            (202, serde_json::json!({"accepted": true})),
            "{path}"
        );
    }
    // A JSON body on a bodiless route is refused; a non-JSON body is 415 at the boundary.
    let r = c.post("/v1/auth/logout", Some("{}"), &token).await;
    assert_eq!((r.status, r.code()), (400, "http.invalid_body".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn r10_login_refusals_are_one_refusal_and_success_sets_cookies() {
    let h = start(Fake::new(), false).await;
    let mut c = Client::new(&h);
    let token = c.csrf().await;
    let r = c
        .post(
            "/v1/auth/register",
            Some(&login_body(EMAIL, PASSWORD)),
            &token,
        )
        .await;
    assert_eq!(r.status, 202);
    let unverified = c
        .post("/v1/auth/login", Some(&login_body(EMAIL, PASSWORD)), &token)
        .await;
    let wrong = c
        .post(
            "/v1/auth/login",
            Some(&login_body(EMAIL, "not the password at all")),
            &token,
        )
        .await;
    let unknown = c
        .post(
            "/v1/auth/login",
            Some(&login_body("nobody@example.com", PASSWORD)),
            &token,
        )
        .await;
    let strip = |v: &serde_json::Value| {
        let mut o = v.as_object().unwrap().clone();
        o.remove("request_id");
        o.remove("correlation_id");
        o
    };
    for r in [&unverified, &wrong, &unknown] {
        assert_eq!(
            (r.status, r.code()),
            (401, "identity.credentials_rejected".into())
        );
        assert!(r.body.get("fields").is_none(), "{}", r.body);
        assert_eq!(strip(&r.body), strip(&unverified.body));
        assert!(r.set_cookies().is_empty());
    }
    let mail = h.fake.last_mail("verification").unwrap();
    let r = c
        .post(
            "/v1/auth/email/verify",
            Some(
                &serde_json::json!({"token": mail, "password": "wrong password here"}).to_string(),
            ),
            &token,
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (401, "identity.challenge_rejected".into())
    );
    let r = c
        .post(
            "/v1/auth/email/verify",
            Some(&serde_json::json!({"token": mail, "password": PASSWORD}).to_string()),
            &token,
        )
        .await;
    assert_eq!(r.status, 204);
    let r = c
        .post("/v1/auth/login", Some(&login_body(EMAIL, PASSWORD)), &token)
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let keys: Vec<&String> = r.body.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "absolute_expires_at_ms",
            "csrf_token",
            "idle_expires_at_ms",
            "principal_id"
        ]
    );
    assert_eq!(r.body["absolute_expires_at_ms"], NOW + 43_200_000);
    assert_eq!(r.body["idle_expires_at_ms"], NOW + 1_800_000);
    let text = r.body.to_string();
    assert!(!text.contains(PASSWORD));
    let session = c.cookie("n2f_session").unwrap();
    assert_eq!(session.len(), 43);
    assert!(!text.contains(&session), "token in body");
    let cookies = r.set_cookies();
    let session_cookie = cookies
        .iter()
        .find(|c| c.starts_with("n2f_session="))
        .unwrap();
    assert!(
        session_cookie.contains("; Path=/; Max-Age=43200")
            && session_cookie.contains("; HttpOnly")
            && session_cookie.contains("; SameSite=Lax")
            && !session_cookie.contains("Secure"),
        "{session_cookie}"
    );
    let rotated = r.body["csrf_token"].as_str().unwrap();
    assert_ne!(rotated, token);
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with(&format!("n2f_csrf={rotated}")))
    );
    assert_eq!(r.headers.get("cache-control").unwrap(), "no-store");
}

#[tokio::test(flavor = "multi_thread")]
async fn r07_session_admission_and_duplicate_cookie() {
    let h = start(Fake::new(), false).await;
    let (mut c, token) = logged_in(&h).await;
    let r = c.get("/v1/auth/session").await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["auth_epoch"], 1);
    assert!(r.body["principal_id"].as_str().unwrap().len() == 36);
    let anonymous_get = Client::new(&h).get("/v1/auth/session").await;
    assert_eq!(
        (anonymous_get.status, anonymous_get.code()),
        (401, "identity.session_rejected".into())
    );
    // Duplicate session cookie: refused before any lookup.
    let before = h.fake.resolves();
    let value = c.cookie("n2f_session").unwrap();
    let r = c
        .send(
            "GET",
            "/v1/auth/session",
            None,
            None,
            None,
            Some(("n2f_session", &value)),
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (401, "identity.session_rejected".into())
    );
    assert_eq!(h.fake.resolves(), before);
    // Malformed and stale cookies.
    let mut stale = Client::new(&h);
    stale.cookies = vec![("n2f_session".into(), "not-a-token".into())];
    let r = stale.get("/v1/auth/session").await;
    assert_eq!(
        (r.status, r.code()),
        (401, "identity.session_rejected".into())
    );
    // An expired session is rejected by the resolver; the 401 clears the session
    // cookie and is no-store (R07). The jar honours Max-Age=0, so restore the
    // secret afterwards to keep exercising the same session below.
    h.fake.set_now(NOW + 43_200_000);
    let r = c.get("/v1/auth/session").await;
    assert_eq!(
        (r.status, r.code()),
        (401, "identity.session_rejected".into())
    );
    assert_eq!(r.headers.get("cache-control").unwrap(), "no-store");
    assert!(
        r.set_cookies()
            .iter()
            .any(|v| v.starts_with("n2f_session=;") && v.to_ascii_lowercase().contains("max-age=0")),
        "401 must clear the session cookie: {:?}",
        r.set_cookies()
    );
    assert!(c.cookie("n2f_session").is_none());
    c.cookies.push(("n2f_session".into(), value.clone()));
    h.fake.set_now(NOW);
    // Session-required POST needs origin and CSRF too.
    let r = c
        .send(
            "POST",
            "/v1/auth/logout-all",
            None,
            None,
            Some(&token),
            None,
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (403, "identity.origin_rejected".into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn r11_logout_is_idempotent_and_clears() {
    let h = start(Fake::new(), false).await;
    let (mut c, token) = logged_in(&h).await;
    let r = c
        .send(
            "POST",
            "/v1/auth/logout",
            None,
            Some(ORIGIN),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    let cookies = r.set_cookies();
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("n2f_session=; Path=/; Max-Age=0")),
        "{cookies:?}"
    );
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("n2f_csrf=") && !c.contains("Max-Age=0")),
        "{cookies:?}"
    );
    assert!(c.cookie("n2f_session").is_none());
    let r = c.get("/v1/auth/session").await;
    assert_eq!(r.status, 401);
    // Without a session: still 204, with a fresh anonymous CSRF context.
    let fresh = c.csrf().await;
    let r = c
        .send(
            "POST",
            "/v1/auth/logout",
            None,
            Some(ORIGIN),
            Some(&fresh),
            None,
        )
        .await;
    assert_eq!(r.status, 204);
    // Logout-all clears both cookies and later admission fails.
    let (mut d, token) = logged_in_as(&h, "Bob@Example.com").await;
    let r = d
        .send(
            "POST",
            "/v1/auth/logout-all",
            None,
            Some(ORIGIN),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    assert!(d.cookie("n2f_session").is_none() && d.cookie("n2f_csrf").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn r02_password_change_and_reset() {
    let h = start(Fake::new(), false).await;
    let (mut c, token) = logged_in(&h).await;
    let next = "an entirely new passphrase";
    let r = c.post("/v1/auth/password/change", Some(&serde_json::json!({"current_password": "wrong one entirely", "new_password": next}).to_string()), &token).await;
    assert_eq!(
        (r.status, r.code()),
        (401, "identity.credentials_rejected".into())
    );
    let r = c
        .post(
            "/v1/auth/password/change",
            Some(
                &serde_json::json!({"current_password": PASSWORD, "new_password": next})
                    .to_string(),
            ),
            &token,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    assert!(c.cookie("n2f_session").is_none());
    let mut d = Client::new(&h);
    let t = d.csrf().await;
    assert_eq!(
        d.post("/v1/auth/login", Some(&login_body(EMAIL, PASSWORD)), &t)
            .await
            .status,
        401
    );
    assert_eq!(
        d.post("/v1/auth/login", Some(&login_body(EMAIL, next)), &t)
            .await
            .status,
        200
    );
    // Reset through the recorded mail.
    let mut e = Client::new(&h);
    let t = e.csrf().await;
    assert_eq!(
        e.post(
            "/v1/auth/password/reset-requests",
            Some(&serde_json::json!({"email": EMAIL}).to_string()),
            &t
        )
        .await
        .status,
        202
    );
    let mail = h.fake.last_mail("reset").unwrap();
    let last = "the final passphrase here";
    let r = e
        .post(
            "/v1/auth/password/reset",
            Some(&serde_json::json!({"token": mail, "password": last}).to_string()),
            &t,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    assert!(e.cookie("n2f_session").is_none());
    let r = e
        .post(
            "/v1/auth/password/reset",
            Some(&serde_json::json!({"token": mail, "password": last}).to_string()),
            &t,
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (401, "identity.challenge_rejected".into())
    );
    assert_eq!(
        e.post("/v1/auth/login", Some(&login_body(EMAIL, last)), &t)
            .await
            .status,
        200
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn r08_rate_limit_projection() {
    let h = start(Fake::new(), false).await;
    let mut c = Client::new(&h);
    let token = c.csrf().await;
    h.fake.with(|m| m.limiter_refuse = true);
    let r = c
        .post(
            "/v1/auth/register",
            Some(&login_body(EMAIL, PASSWORD)),
            &token,
        )
        .await;
    assert_eq!(
        (r.status, r.code()),
        (429, "identity.auth_rate_limited".into())
    );
    // R08: Retry-After in whole seconds (ceiling, minimum 1) from the failure's
    // retry-after value; the refusal is no-store like every other.
    assert_eq!(r.headers.get("retry-after").unwrap(), "30");
    assert_eq!(r.headers.get("cache-control").unwrap(), "no-store");
    assert!(r.body.get("retry_after_ms").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn r13_provenance_and_no_private_values_in_logs() {
    let h = start(Fake::new(), false).await;
    let (mut c, token) = logged_in(&h).await;
    let session = c.cookie("n2f_session").unwrap();
    let r = c.get("/v1/auth/session").await;
    assert_eq!(r.status, 200);
    let principal = r.body["principal_id"].as_str().unwrap().to_string();
    let opened = h.opened.lock().unwrap().clone();
    let by = |name: &str| {
        opened
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, i)| i.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        by("identity.http.session"),
        vec![format!("user:{principal}")]
    );
    assert_eq!(by("identity.http.register"), vec!["anonymous:".to_string()]);
    assert!(by("identity.http.login").iter().all(|i| i == "anonymous:"));
    let r = c
        .send(
            "POST",
            "/v1/auth/logout",
            None,
            Some(ORIGIN),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(r.status, 204);
    let logs = String::from_utf8(h.sink.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("http.request.completed"));
    for private in [
        PASSWORD,
        "SENTINEL",
        &session,
        &token,
        "ada@example.com",
        "Ada@Example.com",
    ] {
        assert!(!logs.contains(private), "log leaked {private}");
    }
    assert!(logs.contains("/v1/auth/session"));
}

#[test]
fn r01_configuration_refusals() {
    let bad = |f: &dyn Fn(&mut Config)| {
        let mut c = config(true);
        f(&mut c);
        c
    };
    for c in [
        bad(&|c| c.secure = false),
        bad(&|c| c.allowed_origins = vec![]),
        bad(&|c| c.allowed_origins = vec!["*".into()]),
        bad(&|c| c.allowed_origins = vec!["http://a.example/path".into()]),
        bad(&|c| c.csrf_ttl_ms = 3_600_001),
        bad(&|c| c.csrf_ttl_ms = 0),
        bad(&|c| c.session_cookie = "bad name".into()),
        bad(&|c| c.session_cookie = c.csrf_cookie.clone()),
    ] {
        let fake = Fake::new();
        let e = Transport::new(
            c,
            Deps {
                ports: fake,
                app: app::Config::defaults(),
                keyed: Arc::new(
                    Digest::new(SecretString::new("0123456789abcdef0123456789abcdef".into()))
                        .unwrap(),
                ),
                entropy: counter_entropy(0),
            },
        )
        .err()
        .expect("refused");
        assert_eq!(
            e.classification().unwrap().error_type,
            Some("identity.transport_configuration")
        );
    }
}
