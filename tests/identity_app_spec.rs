//! Executable specification for the identity application operations (app/CONTRACT.md).
//! Every port is a recording fake; clock and IDs are fixed.
use n2f_rs::{
    domains::identity::{
        app::{
            AuthenticatedPrincipal, Config,
            command::{
                Admission, AttemptLimiter, ChallengeReader, ChallengeRecord, ChallengeStore,
                ChangePassword, ChangePasswordInput, ChangeRecord, Clock, Commit, CredentialReader,
                CredentialRecord, EnrollmentPolicy, EpochStore, IdSource, IssueRecord, Login,
                LoginInput, LoginRecord, LoginStore, Logout, LogoutAll, MailDelivery,
                PasswordHasher, PasswordStore, Register, RegisterInput, RegisterOutcome,
                RegisterRecord, RegisterStore, RequestInput, RequestReset, RequestVerification,
                ResetPassword, ResetPasswordInput, ResetRecord, RevokeOutcome, SessionRevoker,
                TokenCodec, VerifyEmail, VerifyEmailInput, VerifyRecord, VerifyStore,
            },
            query::{Authenticate, Resolution, SessionResolver},
        },
        domain::{
            self, Kind as PrincipalKind, Snapshot, Status,
            challenge::ChallengeSnapshot,
            credential::CredentialSnapshot,
            email::Email,
            password::NewPassword,
            password::PasswordInput,
            token::{TokenDigest, TokenPurpose},
        },
    },
    shared::{
        errors::{Classified, Failure, Kind},
        events::Envelope,
        id::Id,
        provenance::{
            Actor, ActorKind, Attribution, Factory, Operation, Origin, RootSpec, WorkContext,
        },
        secret::SecretString,
    },
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const NOW: i64 = 1_700_000_000_000;
const EMAIL: &str = "Ada.Lovelace@Example.com";
const CANON: &str = "ada.lovelace@example.com";
const PASSWORD: &str = "correct horse battery staple";
const TOKEN: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

fn id(n: u32) -> Id {
    Id::parse(&format!("01900000-0000-7000-8000-{n:012x}")).unwrap()
}
fn work() -> WorkContext {
    let mut f = Factory::new(|| UNIX_EPOCH + Duration::from_millis(1000), || Ok(id(900)));
    f.open(RootSpec {
        work_id: None,
        origin: Origin::Request,
        operation: Operation::new("identity.test".into()).unwrap(),
        attribution: Attribution::default(),
        executor: Actor::new(ActorKind::Service, "api".into()).unwrap(),
    })
    .unwrap()
    .work_context()
    .clone()
}
fn failure(kind: Kind, t: &str) -> Failure {
    Failure::new(kind, "scripted").with_type(t)
}
fn kind_of(e: &Failure) -> Kind {
    e.classification().unwrap().kind
}
fn type_of(e: &Failure) -> String {
    e.classification()
        .unwrap()
        .error_type
        .unwrap_or("")
        .to_string()
}
fn expect_refusal<T>(r: Result<T, Failure>, kind: Kind, t: &str) {
    match r {
        Ok(_) => panic!("expected {t}, got success"),
        Err(e) => assert!(
            kind_of(&e) == kind && type_of(&e) == t,
            "expected {kind:?}/{t}, got {:?}/{}",
            kind_of(&e),
            type_of(&e)
        ),
    }
}
fn digest(p: TokenPurpose) -> TokenDigest {
    TokenDigest::parse(p, &[7u8; 32]).unwrap()
}
fn secret(s: &str) -> SecretString {
    SecretString::new(s.into())
}

/// Scripted credential facts.
#[derive(Clone, Copy)]
struct Cred {
    verified: bool,
    active: bool,
    epoch: u32,
    version: u32,
}
impl Default for Cred {
    fn default() -> Self {
        Self {
            verified: true,
            active: true,
            epoch: 3,
            version: 5,
        }
    }
}
#[derive(Clone, Copy)]
struct Chal {
    consumed: bool,
    expired: bool,
    purpose: TokenPurpose,
    version: u32,
    cred: Cred,
}
type Scripted<T> = Result<T, (Kind, &'static str)>;
type Script = Box<dyn Fn(&mut State)>;
struct State {
    calls: Vec<String>,
    admit: Scripted<Admission>,
    blocklist_allowed: Scripted<bool>,
    hash: Scripted<&'static str>,
    verify: Scripted<bool>,
    find_email: Scripted<Option<Cred>>,
    find_principal: Scripted<Option<Cred>>,
    find_digest: Scripted<Option<Chal>>,
    register: Scripted<RegisterOutcome>,
    commit_login: Scripted<Commit>,
    resolve: Scripted<Resolution>,
    revoke_session: Scripted<RevokeOutcome>,
    revoke_all: Scripted<Commit>,
    issue_challenge: Scripted<Commit>,
    verify_email: Scripted<Commit>,
    change_password: Scripted<Commit>,
    reset_password: Scripted<Commit>,
    mail_delivered: bool,
    now_ms: i64,
    next_id: u32,
    sent: Vec<(String, String, i64)>,
    events: Vec<Envelope>,
    register_record: Option<RegisterRecord>,
    login_record: Option<LoginRecord>,
    issue_record: Option<IssueRecord>,
    verify_record: Option<VerifyRecord>,
    change_record: Option<ChangeRecord>,
    reset_record: Option<ResetRecord>,
    revoke_args: Option<(Id, Id, i64)>,
    revoke_all_args: Option<(Id, u32, i64)>,
    resolve_args: Option<(TokenDigest, i64, i64)>,
}
#[derive(Clone)]
struct Fake(Arc<Mutex<State>>);
impl Fake {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            calls: vec![],
            admit: Ok(Admission::Permitted),
            blocklist_allowed: Ok(true),
            hash: Ok("$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$aGFzaA"),
            verify: Ok(true),
            find_email: Ok(Some(Cred::default())),
            find_principal: Ok(Some(Cred::default())),
            find_digest: Ok(Some(Chal {
                consumed: false,
                expired: false,
                purpose: TokenPurpose::EmailVerification,
                version: 5,
                cred: Cred::default(),
            })),
            register: Ok(RegisterOutcome::Created),
            commit_login: Ok(Commit::Committed),
            resolve: Ok(Resolution::Admitted(AuthenticatedPrincipal {
                principal_id: id(1),
                session_id: id(2),
                auth_epoch: 3,
            })),
            revoke_session: Ok(RevokeOutcome::Revoked),
            revoke_all: Ok(Commit::Committed),
            issue_challenge: Ok(Commit::Committed),
            verify_email: Ok(Commit::Committed),
            change_password: Ok(Commit::Committed),
            reset_password: Ok(Commit::Committed),
            mail_delivered: true,
            now_ms: NOW,
            next_id: 100,
            sent: vec![],
            events: vec![],
            register_record: None,
            login_record: None,
            issue_record: None,
            verify_record: None,
            change_record: None,
            reset_record: None,
            revoke_args: None,
            revoke_all_args: None,
            resolve_args: None,
        })))
    }
    fn with(&self, f: impl FnOnce(&mut State)) -> &Self {
        f(&mut self.0.lock().unwrap());
        self
    }
    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().calls.clone()
    }
    fn record(&self, name: &str) {
        self.0.lock().unwrap().calls.push(name.into());
    }
    fn events(&self) -> Vec<Envelope> {
        self.0.lock().unwrap().events.clone()
    }
    fn sent(&self) -> Vec<(String, String, i64)> {
        self.0.lock().unwrap().sent.clone()
    }
}
fn scripted<T: Clone>(s: &Scripted<T>) -> Result<T, Failure> {
    s.clone().map_err(|(k, t)| failure(k, t))
}
fn credential(c: Cred) -> CredentialRecord {
    CredentialRecord {
        principal: Snapshot {
            id: id(1),
            kind: PrincipalKind::Human,
            display_name: "ada".into(),
            status: if c.active {
                Status::Active
            } else {
                Status::Suspended
            },
            created_at_ms: 0,
            updated_at_ms: 0,
            version: 1,
        },
        credential: CredentialSnapshot {
            principal_id: id(1),
            email: Email::parse(CANON).unwrap(),
            password_hash: secret("stored-hash-sentinel"),
            verified_at_ms: if c.verified { Some(10) } else { None },
            password_version: c.version,
            created_at_ms: 0,
            changed_at_ms: 0,
        },
        auth_epoch: c.epoch,
    }
}
fn challenge(c: Chal) -> ChallengeRecord {
    let cred = credential(c.cred);
    ChallengeRecord {
        challenge: ChallengeSnapshot {
            id: id(50),
            principal_id: id(1),
            token_digest: digest(c.purpose),
            password_version: c.version,
            issued_at_ms: NOW - 1000,
            expires_at_ms: if c.expired { NOW } else { NOW + 1000 },
            consumed_at_ms: if c.consumed { Some(NOW - 500) } else { None },
            invalidated_at_ms: None,
        },
        credential: cred.credential,
        principal_status: cred.principal.status,
        auth_epoch: cred.auth_epoch,
    }
}
impl Clock for Fake {
    fn now(&self) -> SystemTime {
        self.record("now");
        let ms = self.0.lock().unwrap().now_ms;
        UNIX_EPOCH + Duration::from_millis(ms as u64)
    }
}
impl IdSource for Fake {
    fn new_id(&self) -> Result<Id, Failure> {
        self.record("new_id");
        let mut s = self.0.lock().unwrap();
        s.next_id += 1;
        Ok(id(s.next_id))
    }
}
impl PasswordHasher for Fake {
    fn hash(&self, _: &NewPassword) -> impl Future<Output = Result<SecretString, Failure>> + Send {
        self.record("hash");
        let r = scripted(&self.0.lock().unwrap().hash).map(secret);
        async move { r }
    }
    fn verify(
        &self,
        _: &PasswordInput,
        record: &SecretString,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        self.record("verify");
        assert_eq!(record.reveal(), "stored-hash-sentinel");
        let r = scripted(&self.0.lock().unwrap().verify);
        async move { r }
    }
    fn verify_absent(
        &self,
        _: &PasswordInput,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        self.record("verify_absent");
        async move { Ok(false) }
    }
}
impl TokenCodec for Fake {
    fn issue(&self, purpose: TokenPurpose) -> Result<(SecretString, TokenDigest), Failure> {
        self.record("issue");
        Ok((
            secret(&format!("issued-{}", purpose.as_str())),
            digest(purpose),
        ))
    }
    fn digest(&self, purpose: TokenPurpose, s: &SecretString) -> Result<TokenDigest, Failure> {
        self.record("digest");
        if s.reveal() != TOKEN {
            return Err(failure(Kind::Invalid, "identity.token_invalid"));
        }
        Ok(digest(purpose))
    }
}
impl EnrollmentPolicy for Fake {
    fn check_blocklist(
        &self,
        _: &NewPassword,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        self.record("check_blocklist");
        let r = scripted(&self.0.lock().unwrap().blocklist_allowed);
        async move { r }
    }
}
impl AttemptLimiter for Fake {
    fn admit(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> impl Future<Output = Result<Admission, Failure>> + Send {
        self.record(&format!("admit:{operation}:{}:{source}", subject.reveal()));
        let r = scripted(&self.0.lock().unwrap().admit);
        async move { r }
    }
}
impl MailDelivery for Fake {
    fn send_verification(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send {
        self.record("send_verification");
        let mut s = self.0.lock().unwrap();
        s.sent
            .push((email.reveal().into(), token.reveal().into(), expires_at_ms));
        let d = s.mail_delivered;
        async move { d }
    }
    fn send_reset(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send {
        self.record("send_reset");
        let mut s = self.0.lock().unwrap();
        s.sent
            .push((email.reveal().into(), token.reveal().into(), expires_at_ms));
        let d = s.mail_delivered;
        async move { d }
    }
}
impl RegisterStore for Fake {
    fn register(
        &self,
        record: RegisterRecord,
    ) -> impl Future<Output = Result<RegisterOutcome, Failure>> + Send {
        self.record("register");
        let mut s = self.0.lock().unwrap();
        s.events.push(record.event.clone());
        let r = scripted(&s.register);
        s.register_record = Some(record);
        async move { r }
    }
}
impl CredentialReader for Fake {
    fn find_by_email(
        &self,
        email: &Email,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send {
        self.record("find_by_email");
        assert_eq!(email.reveal(), CANON);
        let r = scripted(&self.0.lock().unwrap().find_email).map(|c| c.map(credential));
        async move { r }
    }
    fn find_by_principal(
        &self,
        principal: Id,
    ) -> impl Future<Output = Result<Option<CredentialRecord>, Failure>> + Send {
        self.record("find_by_principal");
        assert_eq!(principal, id(1));
        let r = scripted(&self.0.lock().unwrap().find_principal).map(|c| c.map(credential));
        async move { r }
    }
}
impl LoginStore for Fake {
    fn commit_login(
        &self,
        record: LoginRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.record("commit_login");
        let mut s = self.0.lock().unwrap();
        s.events.push(record.event.clone());
        let r = scripted(&s.commit_login);
        s.login_record = Some(record);
        async move { r }
    }
}
impl SessionResolver for Fake {
    fn resolve(
        &self,
        d: &TokenDigest,
        now: i64,
        idle_ttl_ms: i64,
    ) -> impl Future<Output = Result<Resolution, Failure>> + Send {
        self.record("resolve");
        let mut s = self.0.lock().unwrap();
        s.resolve_args = Some((*d, now, idle_ttl_ms));
        let r = scripted(&s.resolve);
        async move { r }
    }
}
impl SessionRevoker for Fake {
    fn revoke_session(
        &self,
        session: Id,
        principal: Id,
        now: i64,
        event: Envelope,
    ) -> impl Future<Output = Result<RevokeOutcome, Failure>> + Send {
        self.record("revoke_session");
        let mut s = self.0.lock().unwrap();
        s.events.push(event);
        s.revoke_args = Some((session, principal, now));
        let r = scripted(&s.revoke_session);
        async move { r }
    }
}
impl EpochStore for Fake {
    fn revoke_all(
        &self,
        principal: Id,
        expected_epoch: u32,
        now: i64,
        event: Envelope,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.record("revoke_all");
        let mut s = self.0.lock().unwrap();
        s.events.push(event);
        s.revoke_all_args = Some((principal, expected_epoch, now));
        let r = scripted(&s.revoke_all);
        async move { r }
    }
}
impl ChallengeReader for Fake {
    fn find_by_digest(
        &self,
        purpose: TokenPurpose,
        d: &TokenDigest,
    ) -> impl Future<Output = Result<Option<ChallengeRecord>, Failure>> + Send {
        self.record(&format!("find_by_digest:{}", purpose.as_str()));
        assert_eq!(d.purpose(), purpose);
        let r = scripted(&self.0.lock().unwrap().find_digest).map(|c| c.map(challenge));
        async move { r }
    }
}
impl ChallengeStore for Fake {
    fn issue_challenge(
        &self,
        record: IssueRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.record("issue_challenge");
        let mut s = self.0.lock().unwrap();
        let r = scripted(&s.issue_challenge);
        s.issue_record = Some(record);
        async move { r }
    }
}
impl VerifyStore for Fake {
    fn verify_email(
        &self,
        record: VerifyRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.record("verify_email");
        let mut s = self.0.lock().unwrap();
        s.events.push(record.event.clone());
        let r = scripted(&s.verify_email);
        s.verify_record = Some(record);
        async move { r }
    }
}
impl PasswordStore for Fake {
    fn change_password(
        &self,
        record: ChangeRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.record("change_password");
        let mut s = self.0.lock().unwrap();
        s.events.extend(record.events.iter().cloned());
        let r = scripted(&s.change_password);
        s.change_record = Some(record);
        async move { r }
    }
    fn reset_password(
        &self,
        record: ResetRecord,
    ) -> impl Future<Output = Result<Commit, Failure>> + Send {
        self.record("reset_password");
        let mut s = self.0.lock().unwrap();
        s.events.extend(record.events.iter().cloned());
        let r = scripted(&s.reset_password);
        s.reset_record = Some(record);
        async move { r }
    }
}

fn caller() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: id(1),
        session_id: id(2),
        auth_epoch: 3,
    }
}
fn register_input() -> RegisterInput {
    RegisterInput {
        email: EMAIL.into(),
        password: secret(PASSWORD),
        source: "10.0.0.1".into(),
    }
}
fn login_input() -> LoginInput {
    LoginInput {
        email: EMAIL.into(),
        password: secret(PASSWORD),
        source: "10.0.0.1".into(),
    }
}
fn request_input() -> RequestInput {
    RequestInput {
        email: EMAIL.into(),
        source: "10.0.0.1".into(),
    }
}
fn assert_no_private_data(events: &[Envelope]) {
    for e in events {
        let text = String::from_utf8(e.bytes().to_vec()).unwrap();
        for private in [
            "ada",
            "example.com",
            "issued-",
            "stored-hash",
            "argon2",
            TOKEN,
            "10.0.0.1",
        ] {
            assert!(!text.contains(private), "event leaked {private}: {text}");
        }
    }
}
fn payload(e: &Envelope) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(e.bytes()).unwrap()
}
fn event_type(e: &Envelope) -> String {
    payload(e)["type"].as_str().unwrap().to_string()
}

#[test]
fn u02_config_validates_once() {
    assert!(Config::defaults().session_absolute_ms == 12 * 3600 * 1000);
    for (a, i, v, r) in [
        (0, 1, 1, 1),
        (10, 20, 1, 1),
        (10, 5, 0, 1),
        (10, 5, 1, -1),
        (domain::MAX_TIME_MS + 1, 5, 1, 1),
    ] {
        expect_refusal(
            Config::new(a, i, v, r),
            Kind::Invalid,
            "identity.auth_configuration",
        );
    }
    assert!(Config::new(10, 5, 1, 1).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn u04_register_hashes_before_store_and_mails_after_commit() {
    let f = Fake::new();
    let op = Register::new(f.clone(), Config::defaults()).unwrap();
    let out = op.register(&work(), register_input()).await.unwrap();
    assert!(out.created && out.mail_delivered);
    let calls = f.calls();
    let pos = |n: &str| {
        calls
            .iter()
            .position(|c| c.starts_with(n))
            .unwrap_or_else(|| panic!("missing {n}: {calls:?}"))
    };
    assert_eq!(calls[0], format!("admit:register:{CANON}:10.0.0.1"));
    assert!(
        pos("check_blocklist") < pos("hash")
            && pos("hash") < pos("register")
            && pos("register") < pos("send_verification")
    );
    {
        let s = f.0.lock().unwrap();
        let r = s.register_record.as_ref().unwrap();
        assert_eq!(r.principal.kind, PrincipalKind::Human);
        assert_eq!(r.principal.display_name, "ada.lovelace");
        assert_eq!(r.principal.status, Status::Active);
        assert_eq!(r.auth_epoch, 1);
        assert_eq!(r.credential.password_version, 1);
        assert!(r.credential.verified_at_ms.is_none());
        assert_eq!(r.credential.email.reveal(), CANON);
        assert_eq!(
            r.credential.password_hash.reveal(),
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$aGFzaA"
        );
        assert_eq!(r.challenge.principal_id, r.principal.id);
        assert_eq!(
            r.challenge.token_digest.purpose(),
            TokenPurpose::EmailVerification
        );
        assert_eq!(r.challenge.expires_at_ms, NOW + 24 * 3600 * 1000);
        assert_eq!(r.challenge.password_version, 1);
        assert_eq!(event_type(&r.event), "identity.principal.registered.v1");
        let p = payload(&r.event)["payload"].clone();
        assert_eq!(p["principal_id"], r.principal.id.to_string());
        assert_eq!(p["kind"], "human");
        assert_eq!(p["origin"], "self_registration");
        assert_eq!(
            s.sent,
            vec![(
                CANON.to_string(),
                "issued-email_verification".to_string(),
                NOW + 24 * 3600 * 1000
            )]
        );
    }
    assert_no_private_data(&f.events());
}

#[tokio::test(flavor = "multi_thread")]
async fn u04_duplicate_is_accepted_without_replacement_or_mail() {
    let f = Fake::new();
    f.with(|s| s.register = Ok(RegisterOutcome::DuplicateEmail));
    let op = Register::new(f.clone(), Config::defaults()).unwrap();
    let out = op.register(&work(), register_input()).await.unwrap();
    assert!(!out.created && !out.mail_delivered);
    assert!(f.sent().is_empty());
    assert!(!f.calls().iter().any(|c| c.starts_with("send_")));
}

#[tokio::test(flavor = "multi_thread")]
async fn u04_register_refusals_and_failures_keep_their_meaning() {
    let cases: Vec<(Script, Kind, &str)> = vec![
        (
            Box::new(|s| {
                s.admit = Ok(Admission::Refused {
                    retry_after_ms: 30_000,
                })
            }),
            Kind::RateLimited,
            "identity.auth_rate_limited",
        ),
        (
            Box::new(|s| s.admit = Err((Kind::Unavailable, "limiter.down"))),
            Kind::Unavailable,
            "limiter.down",
        ),
        (
            Box::new(|s| s.blocklist_allowed = Ok(false)),
            Kind::Invalid,
            "identity.password_invalid",
        ),
        (
            Box::new(|s| s.blocklist_allowed = Err((Kind::Unavailable, "blocklist.down"))),
            Kind::Unavailable,
            "blocklist.down",
        ),
        (
            Box::new(|s| s.hash = Err((Kind::Unavailable, "identity.hash_saturated"))),
            Kind::Unavailable,
            "identity.hash_saturated",
        ),
        (
            Box::new(|s| s.register = Err((Kind::Unavailable, "database.commit_uncertain"))),
            Kind::Unavailable,
            "database.commit_uncertain",
        ),
        (
            Box::new(|s| s.register = Err((Kind::Timeout, "database.timeout"))),
            Kind::Timeout,
            "database.timeout",
        ),
    ];
    for (script, kind, t) in cases {
        let f = Fake::new();
        f.with(|s| script(s));
        let op = Register::new(f.clone(), Config::defaults()).unwrap();
        expect_refusal(op.register(&work(), register_input()).await, kind, t);
        assert!(f.sent().is_empty(), "no mail after {t}");
    }
    let f = Fake::new();
    let op = Register::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.register(
            &work(),
            RegisterInput {
                email: "not an email".into(),
                ..register_input()
            },
        )
        .await,
        Kind::Invalid,
        "identity.email_invalid",
    );
    expect_refusal(
        op.register(
            &work(),
            RegisterInput {
                password: secret("short"),
                ..register_input()
            },
        )
        .await,
        Kind::Invalid,
        "identity.password_invalid",
    );
    assert!(!f.calls().iter().any(|c| c == "hash" || c == "register"));
    let f = Fake::new();
    f.with(|s| s.now_ms = domain::MAX_TIME_MS + 1);
    let op = Register::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.register(&work(), register_input()).await,
        Kind::Unavailable,
        "identity.auth_dependency_failed",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u05_request_verification_outcomes() {
    // absent: accepted, no write, no mail
    let f = Fake::new();
    f.with(|s| s.find_email = Ok(None));
    let op = RequestVerification::new(f.clone(), Config::defaults()).unwrap();
    let out = op.request(request_input()).await.unwrap();
    assert!(!out.issued && !out.mail_delivered);
    assert!(
        !f.calls()
            .iter()
            .any(|c| c == "issue_challenge" || c.starts_with("send_"))
    );
    // already verified: accepted, no write
    let f = Fake::new();
    let op = RequestVerification::new(f.clone(), Config::defaults()).unwrap();
    let out = op.request(request_input()).await.unwrap();
    assert!(!out.issued && !f.calls().iter().any(|c| c == "issue_challenge"));
    // unverified: replacement challenge, then mail
    let f = Fake::new();
    f.with(|s| {
        s.find_email = Ok(Some(Cred {
            verified: false,
            ..Cred::default()
        }))
    });
    let op = RequestVerification::new(f.clone(), Config::defaults()).unwrap();
    let out = op.request(request_input()).await.unwrap();
    assert!(out.issued && out.mail_delivered);
    let calls = f.calls();
    assert_eq!(
        calls[0],
        format!("admit:verification_request:{CANON}:10.0.0.1")
    );
    assert!(
        calls.iter().position(|c| c == "issue_challenge")
            < calls.iter().position(|c| c == "send_verification")
    );
    {
        let s = f.0.lock().unwrap();
        let r = s.issue_record.as_ref().unwrap();
        assert_eq!(r.expected_password_version, 5);
        assert_eq!(r.challenge.password_version, 5);
        assert_eq!(
            r.challenge.token_digest.purpose(),
            TokenPurpose::EmailVerification
        );
        assert_eq!(r.challenge.expires_at_ms, NOW + 24 * 3600 * 1000);
        assert_eq!(s.sent[0].1, "issued-email_verification");
    }
    // stale: accepted, no mail
    let f = Fake::new();
    f.with(|s| {
        s.find_email = Ok(Some(Cred {
            verified: false,
            ..Cred::default()
        }));
        s.issue_challenge = Ok(Commit::Stale);
    });
    let op = RequestVerification::new(f.clone(), Config::defaults()).unwrap();
    let out = op.request(request_input()).await.unwrap();
    assert!(!out.issued && f.sent().is_empty());
    // uncertain: pass-through, no mail
    let f = Fake::new();
    f.with(|s| {
        s.find_email = Ok(Some(Cred {
            verified: false,
            ..Cred::default()
        }));
        s.issue_challenge = Err((Kind::Unavailable, "database.commit_uncertain"));
    });
    let op = RequestVerification::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.request(request_input()).await,
        Kind::Unavailable,
        "database.commit_uncertain",
    );
    assert!(f.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn u06_verify_email_requires_password_proof_and_consumes_once() {
    let ok = || VerifyEmailInput {
        token: secret(TOKEN),
        password: secret(PASSWORD),
    };
    // happy path
    let f = Fake::new();
    let op = VerifyEmail::new(f.clone(), Config::defaults()).unwrap();
    op.verify(&work(), ok()).await.unwrap();
    let calls = f.calls();
    assert_eq!(calls[0], "digest");
    assert!(calls[1].starts_with("admit:verify:"));
    assert!(
        calls.iter().position(|c| c == "verify") < calls.iter().position(|c| c == "verify_email")
    );
    {
        let s = f.0.lock().unwrap();
        let r = s.verify_record.as_ref().unwrap();
        assert_eq!(
            (
                r.challenge_id,
                r.principal_id,
                r.consumed_at_ms,
                r.expected_password_version
            ),
            (id(50), id(1), NOW, 5)
        );
        assert_eq!(event_type(&r.event), "identity.email.verified.v1");
        assert_eq!(
            payload(&r.event)["payload"]["challenge_id"],
            id(50).to_string()
        );
    }
    assert_no_private_data(&f.events());
    // refusals
    let cases: Vec<(Script, &str)> = vec![
        (Box::new(|s| s.find_digest = Ok(None)), "absent"),
        (Box::new(|s| s.verify = Ok(false)), "mismatch"),
        (
            Box::new(|s| {
                s.find_digest = Ok(Some(Chal {
                    expired: true,
                    ..s.find_digest.unwrap().unwrap()
                }))
            }),
            "expired",
        ),
        (
            Box::new(|s| {
                s.find_digest = Ok(Some(Chal {
                    consumed: true,
                    ..s.find_digest.unwrap().unwrap()
                }))
            }),
            "consumed",
        ),
        (
            Box::new(|s| {
                s.find_digest = Ok(Some(Chal {
                    version: 4,
                    ..s.find_digest.unwrap().unwrap()
                }))
            }),
            "stale_version",
        ),
        (
            Box::new(|s| {
                s.find_digest = Ok(Some(Chal {
                    purpose: TokenPurpose::PasswordReset,
                    ..s.find_digest.unwrap().unwrap()
                }))
            }),
            "wrong_purpose",
        ),
        (
            Box::new(|s| s.verify_email = Ok(Commit::Stale)),
            "stale_commit",
        ),
        (
            Box::new(|s| {
                let mut c = s.find_digest.unwrap().unwrap();
                c.cred.active = false;
                s.find_digest = Ok(Some(c))
            }),
            "suspended",
        ),
    ];
    for (script, name) in cases {
        let f = Fake::new();
        f.with(|s| script(s));
        let op = VerifyEmail::new(f.clone(), Config::defaults()).unwrap();
        let r = op.verify(&work(), ok()).await;
        assert!(r.is_err(), "{name} should refuse");
        expect_refusal(r, Kind::Unauthenticated, "identity.challenge_rejected");
        if name == "mismatch" || name == "absent" {
            assert!(
                !f.calls().iter().any(|c| c == "verify_email"),
                "{name} must not commit"
            );
        }
    }
    let f = Fake::new();
    let op = VerifyEmail::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.verify(
            &work(),
            VerifyEmailInput {
                token: secret("malformed"),
                ..ok()
            },
        )
        .await,
        Kind::Unauthenticated,
        "identity.challenge_rejected",
    );
    assert!(!f.calls().iter().any(|c| c.starts_with("find_by_digest")));
    let f = Fake::new();
    f.with(|s| s.verify = Err((Kind::Internal, "identity.credential_corrupt")));
    let op = VerifyEmail::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.verify(&work(), ok()).await,
        Kind::Internal,
        "identity.credential_corrupt",
    );
    // already-verified credential still consumes idempotently
    let f = Fake::new();
    let op = VerifyEmail::new(f.clone(), Config::defaults()).unwrap();
    assert!(op.verify(&work(), ok()).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn u07_login_issues_a_fresh_session_only_for_verified_active_matches() {
    let f = Fake::new();
    let op = Login::new(f.clone(), Config::defaults()).unwrap();
    let out = op.login(&work(), login_input()).await.unwrap();
    assert_eq!(out.secret.reveal(), "issued-session");
    assert_eq!(out.absolute_expires_at_ms, NOW + 12 * 3600 * 1000);
    assert_eq!(out.idle_expires_at_ms, NOW + 30 * 60 * 1000);
    assert_eq!(out.principal.id, id(1));
    assert_eq!(out.principal.kind, PrincipalKind::Human);
    let calls = f.calls();
    assert_eq!(calls[0], format!("admit:login:{CANON}:10.0.0.1"));
    assert!(
        calls.iter().position(|c| c == "verify") < calls.iter().position(|c| c == "commit_login")
    );
    assert!(!calls.iter().any(|c| c == "verify_absent"));
    {
        let s = f.0.lock().unwrap();
        let r = s.login_record.as_ref().unwrap();
        assert_eq!(r.session.id, out.session_id);
        assert_eq!(r.session.principal_id, id(1));
        assert_eq!(r.session.auth_epoch, 3);
        assert_eq!((r.expected_password_version, r.expected_auth_epoch), (5, 3));
        assert_eq!(r.session.token_digest.purpose(), TokenPurpose::Session);
        assert_eq!(
            (r.session.issued_at_ms, r.session.last_seen_at_ms),
            (NOW, NOW)
        );
        assert_eq!(event_type(&r.event), "identity.session.created.v1");
        assert_eq!(payload(&r.event)["payload"]["auth_epoch"], 3);
    }
    assert_no_private_data(&f.events());
    // unknown email: dummy work then the same refusal
    let f = Fake::new();
    f.with(|s| s.find_email = Ok(None));
    let op = Login::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.login(&work(), login_input()).await,
        Kind::Unauthenticated,
        "identity.credentials_rejected",
    );
    let calls = f.calls();
    assert!(
        calls.iter().any(|c| c == "verify_absent")
            && !calls.iter().any(|c| c == "commit_login" || c == "issue")
    );
    for (script, name) in [
        (
            Box::new(|s: &mut State| s.verify = Ok(false)) as Box<dyn Fn(&mut State)>,
            "mismatch",
        ),
        (
            Box::new(|s: &mut State| {
                s.find_email = Ok(Some(Cred {
                    verified: false,
                    ..Cred::default()
                }))
            }),
            "unverified",
        ),
        (
            Box::new(|s: &mut State| {
                s.find_email = Ok(Some(Cred {
                    active: false,
                    ..Cred::default()
                }))
            }),
            "suspended",
        ),
        (
            Box::new(|s: &mut State| s.commit_login = Ok(Commit::Stale)),
            "stale",
        ),
    ] {
        let f = Fake::new();
        f.with(|s| script(s));
        let op = Login::new(f.clone(), Config::defaults()).unwrap();
        expect_refusal(
            op.login(&work(), login_input()).await,
            Kind::Unauthenticated,
            "identity.credentials_rejected",
        );
        let calls = f.calls();
        assert!(
            calls.iter().any(|c| c == "verify"),
            "{name}: verification work still runs"
        );
        if name != "stale" {
            assert!(
                !calls.iter().any(|c| c == "commit_login"),
                "{name}: no session"
            );
        }
    }
    let f = Fake::new();
    f.with(|s| s.commit_login = Err((Kind::Unavailable, "database.commit_uncertain")));
    let op = Login::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.login(&work(), login_input()).await,
        Kind::Unavailable,
        "database.commit_uncertain",
    );
    let f = Fake::new();
    let op = Login::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.login(
            &work(),
            LoginInput {
                password: secret(""),
                ..login_input()
            },
        )
        .await,
        Kind::Invalid,
        "identity.password_invalid",
    );
    expect_refusal(
        op.login(
            &work(),
            LoginInput {
                email: "nope".into(),
                ..login_input()
            },
        )
        .await,
        Kind::Invalid,
        "identity.email_invalid",
    );
    let f = Fake::new();
    f.with(|s| {
        s.admit = Ok(Admission::Refused {
            retry_after_ms: 1000,
        })
    });
    let op = Login::new(f.clone(), Config::defaults()).unwrap();
    let e = op.login(&work(), login_input()).await.unwrap_err();
    assert_eq!(type_of(&e), "identity.auth_rate_limited");
    assert_eq!(
        e.details().get("retry_after_ms").map(String::as_str),
        Some("1000")
    );
    assert!(
        !f.calls()
            .iter()
            .any(|c| c == "verify" || c == "find_by_email")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u08_authenticate_resolves_through_storage() {
    let f = Fake::new();
    let op = Authenticate::new(f.clone(), Config::defaults()).unwrap();
    let p = op.authenticate(&secret(TOKEN)).await.unwrap();
    assert_eq!(p, caller());
    {
        let s = f.0.lock().unwrap();
        let (d, now, idle) = s.resolve_args.unwrap();
        assert_eq!(
            (d.purpose(), now, idle),
            (TokenPurpose::Session, NOW, 30 * 60 * 1000)
        );
    }
    for (script, name) in [
        (
            Box::new(|s: &mut State| s.resolve = Ok(Resolution::Absent)) as Box<dyn Fn(&mut State)>,
            "absent",
        ),
        (
            Box::new(|s: &mut State| s.resolve = Ok(Resolution::Rejected)),
            "rejected",
        ),
    ] {
        let f = Fake::new();
        f.with(|s| script(s));
        let op = Authenticate::new(f.clone(), Config::defaults()).unwrap();
        let r = op.authenticate(&secret(TOKEN)).await;
        assert!(r.is_err(), "{name}");
        expect_refusal(r, Kind::Unauthenticated, "identity.session_rejected");
    }
    let f = Fake::new();
    let op = Authenticate::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.authenticate(&secret("malformed cookie")).await,
        Kind::Unauthenticated,
        "identity.session_rejected",
    );
    assert!(!f.calls().iter().any(|c| c == "resolve"));
    let f = Fake::new();
    f.with(|s| s.resolve = Err((Kind::Timeout, "database.timeout")));
    let op = Authenticate::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.authenticate(&secret(TOKEN)).await,
        Kind::Timeout,
        "database.timeout",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u09_logout_is_idempotent() {
    for (outcome, revoked) in [
        (RevokeOutcome::Revoked, true),
        (RevokeOutcome::AlreadyInactive, false),
        (RevokeOutcome::Absent, false),
    ] {
        let f = Fake::new();
        f.with(|s| s.revoke_session = Ok(outcome));
        let op = Logout::new(f.clone()).unwrap();
        let out = op.logout(&work(), &caller()).await.unwrap();
        assert_eq!(out.revoked, revoked);
        let s = f.0.lock().unwrap();
        assert_eq!(s.revoke_args, Some((id(2), id(1), NOW)));
        let e = &s.events[0];
        assert_eq!(event_type(e), "identity.session.revoked.v1");
        let p = payload(e)["payload"].clone();
        assert_eq!(
            (
                p["session_id"].as_str().unwrap(),
                p["reason"].as_str().unwrap()
            ),
            (id(2).to_string().as_str(), "logout")
        );
    }
    let f = Fake::new();
    f.with(|s| s.revoke_session = Err((Kind::Unavailable, "database.commit_uncertain")));
    let op = Logout::new(f.clone()).unwrap();
    expect_refusal(
        op.logout(&work(), &caller()).await,
        Kind::Unavailable,
        "database.commit_uncertain",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u10_logout_all_increments_the_epoch_with_a_guard() {
    let f = Fake::new();
    let op = LogoutAll::new(f.clone()).unwrap();
    op.logout_all(&work(), &caller()).await.unwrap();
    {
        let s = f.0.lock().unwrap();
        assert_eq!(s.revoke_all_args, Some((id(1), 3, NOW)));
        let p = payload(&s.events[0]);
        assert_eq!(p["type"], "identity.sessions.revoked.v1");
        assert_eq!(p["payload"]["auth_epoch"], 4);
        assert_eq!(p["payload"]["reason"], "logout_all");
    }
    let f = Fake::new();
    f.with(|s| s.revoke_all = Ok(Commit::Stale));
    let op = LogoutAll::new(f.clone()).unwrap();
    expect_refusal(
        op.logout_all(&work(), &caller()).await,
        Kind::Conflict,
        "identity.version_conflict",
    );
    let f = Fake::new();
    let op = LogoutAll::new(f.clone()).unwrap();
    let exhausted = AuthenticatedPrincipal {
        auth_epoch: domain::MAX_VERSION,
        ..caller()
    };
    expect_refusal(
        op.logout_all(&work(), &exhausted).await,
        Kind::Conflict,
        "identity.version_exhausted",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u11_change_password_verifies_proof_then_replaces_with_guards() {
    let input = || ChangePasswordInput {
        current: secret(PASSWORD),
        new: secret("a brand new passphrase 42"),
        source: "10.0.0.1".into(),
    };
    let f = Fake::new();
    let op = ChangePassword::new(f.clone()).unwrap();
    let out = op.change(&work(), &caller(), input()).await.unwrap();
    assert!(out.sessions_invalidated);
    let calls = f.calls();
    assert_eq!(
        calls[0],
        format!("admit:password_change:{}:10.0.0.1", id(1))
    );
    let pos = |n: &str| calls.iter().position(|c| c == n).unwrap();
    assert!(
        pos("find_by_principal") < pos("verify")
            && pos("verify") < pos("check_blocklist")
            && pos("check_blocklist") < pos("hash")
            && pos("hash") < pos("change_password")
    );
    {
        let s = f.0.lock().unwrap();
        let r = s.change_record.as_ref().unwrap();
        assert_eq!(
            (
                r.principal_id,
                r.expected_password_version,
                r.expected_auth_epoch,
                r.changed_at_ms
            ),
            (id(1), 5, 3, NOW)
        );
        assert_eq!(
            r.new_hash.reveal(),
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$aGFzaA"
        );
        let types: Vec<String> = r.events.iter().map(event_type).collect();
        assert_eq!(
            types,
            vec![
                "identity.password.changed.v1",
                "identity.sessions.revoked.v1"
            ]
        );
        assert_eq!(payload(&r.events[0])["payload"]["password_version"], 6);
        assert_eq!(payload(&r.events[0])["payload"]["reason"], "change");
        assert_eq!(payload(&r.events[1])["payload"]["auth_epoch"], 4);
        assert_eq!(
            payload(&r.events[1])["payload"]["reason"],
            "password_change"
        );
    }
    assert_no_private_data(&f.events());
    let cases: Vec<(Script, Kind, &str)> = vec![
        (
            Box::new(|s| s.find_principal = Ok(None)),
            Kind::Unauthenticated,
            "identity.session_rejected",
        ),
        (
            Box::new(|s| {
                s.find_principal = Ok(Some(Cred {
                    epoch: 2,
                    ..Cred::default()
                }))
            }),
            Kind::Unauthenticated,
            "identity.session_rejected",
        ),
        (
            Box::new(|s| s.verify = Ok(false)),
            Kind::Unauthenticated,
            "identity.credentials_rejected",
        ),
        (
            Box::new(|s| s.blocklist_allowed = Ok(false)),
            Kind::Invalid,
            "identity.password_invalid",
        ),
        (
            Box::new(|s| s.change_password = Ok(Commit::Stale)),
            Kind::Conflict,
            "identity.version_conflict",
        ),
        (
            Box::new(|s| s.change_password = Err((Kind::Unavailable, "database.commit_uncertain"))),
            Kind::Unavailable,
            "database.commit_uncertain",
        ),
        (
            Box::new(|s| {
                s.find_principal = Ok(Some(Cred {
                    version: domain::MAX_VERSION,
                    ..Cred::default()
                }))
            }),
            Kind::Conflict,
            "identity.version_exhausted",
        ),
    ];
    for (script, kind, t) in cases {
        let f = Fake::new();
        f.with(|s| script(s));
        let op = ChangePassword::new(f.clone()).unwrap();
        expect_refusal(op.change(&work(), &caller(), input()).await, kind, t);
        if t != "identity.version_conflict" && t != "database.commit_uncertain" {
            assert!(
                !f.calls().iter().any(|c| c == "change_password"),
                "{t}: no write"
            );
        }
    }
    let f = Fake::new();
    let op = ChangePassword::new(f.clone()).unwrap();
    expect_refusal(
        op.change(
            &work(),
            &caller(),
            ChangePasswordInput {
                new: secret("short"),
                ..input()
            },
        )
        .await,
        Kind::Invalid,
        "identity.password_invalid",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u12_request_reset_mails_even_unverified_credentials() {
    let f = Fake::new();
    f.with(|s| {
        s.find_email = Ok(Some(Cred {
            verified: false,
            ..Cred::default()
        }))
    });
    let op = RequestReset::new(f.clone(), Config::defaults()).unwrap();
    let out = op.request(request_input()).await.unwrap();
    assert!(out.issued && out.mail_delivered);
    {
        let s = f.0.lock().unwrap();
        let r = s.issue_record.as_ref().unwrap();
        assert_eq!(
            r.challenge.token_digest.purpose(),
            TokenPurpose::PasswordReset
        );
        assert_eq!(r.challenge.expires_at_ms, NOW + 15 * 60 * 1000);
        assert_eq!(s.sent[0].1, "issued-password_reset");
        assert_eq!(s.calls[0], format!("admit:reset_request:{CANON}:10.0.0.1"));
    }
    let f = Fake::new();
    f.with(|s| s.find_email = Ok(None));
    let op = RequestReset::new(f.clone(), Config::defaults()).unwrap();
    let out = op.request(request_input()).await.unwrap();
    assert!(!out.issued && f.sent().is_empty());
    let f = Fake::new();
    f.with(|s| s.issue_challenge = Err((Kind::Unavailable, "database.commit_uncertain")));
    let op = RequestReset::new(f.clone(), Config::defaults()).unwrap();
    expect_refusal(
        op.request(request_input()).await,
        Kind::Unavailable,
        "database.commit_uncertain",
    );
    assert!(f.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn u13_reset_password_consumes_and_replaces_without_a_session() {
    let input = || ResetPasswordInput {
        token: secret(TOKEN),
        new: secret("a brand new passphrase 42"),
        source: "10.0.0.1".into(),
    };
    let reset = |s: &mut State| {
        s.find_digest = Ok(Some(Chal {
            purpose: TokenPurpose::PasswordReset,
            cred: Cred {
                verified: false,
                ..Cred::default()
            },
            ..s.find_digest.unwrap().unwrap()
        }))
    };
    let f = Fake::new();
    f.with(reset);
    let op = ResetPassword::new(f.clone()).unwrap();
    op.reset(&work(), input()).await.unwrap();
    let calls = f.calls();
    assert_eq!(calls[0], "digest");
    assert!(calls[1].starts_with("admit:reset:"));
    assert!(calls.iter().any(|c| c == "find_by_digest:password_reset"));
    assert!(
        !calls
            .iter()
            .any(|c| c == "verify" || c == "issue" || c == "commit_login")
    );
    {
        let s = f.0.lock().unwrap();
        let r = s.reset_record.as_ref().unwrap();
        assert_eq!(
            (
                r.challenge_id,
                r.principal_id,
                r.expected_password_version,
                r.expected_auth_epoch,
                r.consumed_at_ms,
                r.set_verified
            ),
            (id(50), id(1), 5, 3, NOW, true)
        );
        let types: Vec<String> = r.events.iter().map(event_type).collect();
        assert_eq!(
            types,
            vec![
                "identity.password.changed.v1",
                "identity.sessions.revoked.v1"
            ]
        );
        assert_eq!(payload(&r.events[0])["payload"]["reason"], "reset");
        assert_eq!(payload(&r.events[1])["payload"]["reason"], "password_reset");
    }
    assert_no_private_data(&f.events());
    let f = Fake::new();
    f.with(|s| {
        reset(s);
        let mut c = s.find_digest.unwrap().unwrap();
        c.cred.verified = true;
        s.find_digest = Ok(Some(c))
    });
    let op = ResetPassword::new(f.clone()).unwrap();
    op.reset(&work(), input()).await.unwrap();
    assert!(
        !f.0.lock()
            .unwrap()
            .reset_record
            .as_ref()
            .unwrap()
            .set_verified
    );
    let cases: Vec<(Script, Kind, &str)> = vec![
        (
            Box::new(|s| s.find_digest = Ok(None)),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        ),
        (
            Box::new(move |s| {
                reset(s);
                let mut c = s.find_digest.unwrap().unwrap();
                c.consumed = true;
                s.find_digest = Ok(Some(c))
            }),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        ),
        (
            Box::new(move |s| {
                reset(s);
                let mut c = s.find_digest.unwrap().unwrap();
                c.expired = true;
                s.find_digest = Ok(Some(c))
            }),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        ),
        (
            Box::new(|_| {}),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        ), // default digest purpose is verification
        (
            Box::new(move |s| {
                reset(s);
                s.reset_password = Ok(Commit::Stale)
            }),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        ),
        (
            Box::new(move |s| {
                reset(s);
                s.blocklist_allowed = Ok(false)
            }),
            Kind::Invalid,
            "identity.password_invalid",
        ),
        (
            Box::new(move |s| {
                reset(s);
                s.reset_password = Err((Kind::Unavailable, "database.commit_uncertain"))
            }),
            Kind::Unavailable,
            "database.commit_uncertain",
        ),
        (
            Box::new(move |s| {
                reset(s);
                let mut c = s.find_digest.unwrap().unwrap();
                c.cred.active = false;
                s.find_digest = Ok(Some(c))
            }),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        ),
    ];
    for (script, kind, t) in cases {
        let f = Fake::new();
        f.with(|s| script(s));
        let op = ResetPassword::new(f.clone()).unwrap();
        expect_refusal(op.reset(&work(), input()).await, kind, t);
    }
    let f = Fake::new();
    let op = ResetPassword::new(f.clone()).unwrap();
    expect_refusal(
        op.reset(
            &work(),
            ResetPasswordInput {
                token: secret("malformed"),
                ..input()
            },
        )
        .await,
        Kind::Unauthenticated,
        "identity.challenge_rejected",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn u05_u12_inactive_principal_receives_no_challenge_or_mail() {
    for purpose in [TokenPurpose::EmailVerification, TokenPurpose::PasswordReset] {
        let f = Fake::new();
        f.with(|s| {
            s.find_email = Ok(Some(Cred {
                verified: false,
                active: false,
                ..Cred::default()
            }))
        });
        let out = if purpose == TokenPurpose::PasswordReset {
            RequestReset::new(f.clone(), Config::defaults())
                .unwrap()
                .request(request_input())
                .await
                .unwrap()
        } else {
            RequestVerification::new(f.clone(), Config::defaults())
                .unwrap()
                .request(request_input())
                .await
                .unwrap()
        };
        assert!(!out.issued && !out.mail_delivered, "{purpose:?}");
        assert!(
            !f.calls()
                .iter()
                .any(|c| c == "issue_challenge" || c.starts_with("send_")),
            "{purpose:?}: {:?}",
            f.calls()
        );
        assert!(f.sent().is_empty());
    }
}
