//! Identity HTTP transport: the authentication routes on the shared feature-route
//! boundary. Owns cookie policy, origin and CSRF admission, the session admission
//! that establishes provenance attribution, request decoding and response encoding.
//! No business rule, no SQL, no crypto beyond the shared keyed digest.
#![doc = include_str!("CONTRACT.md")]
use crate::{
    domains::identity::app::{
        self, AuthenticatedPrincipal,
        command::{
            AttemptLimiter, ChallengeReader, ChallengeStore, ChangePassword, ChangePasswordInput,
            Clock, CredentialReader, EnrollmentPolicy, EpochStore, IdSource, Login, LoginInput,
            LoginStore, Logout, LogoutAll, MailDelivery, PasswordHasher, PasswordStore, Register,
            RegisterInput, RegisterStore, RequestInput, RequestReset, RequestVerification,
            ResetPassword, ResetPasswordInput, SessionRevoker, TokenCodec, UpgradeTicketStore,
            VerifyEmail, VerifyEmailInput, VerifyStore, WebSocketTicket,
        },
        query::{Authenticate, SessionResolver},
    },
    domains::identity::domain::token::{TokenDigest, TokenPurpose},
    shared::{
        entropy::Entropy,
        errors::{Classified, Failure, Kind},
        http::axum::{
            Admission, Cookie, Refusal, Request, RequestContext, RequestFailure, Response, Route,
            SameSite, Work, parse_cookies,
        },
        id::Id,
        keyed::Digest,
        provenance::{Actor, ActorKind},
        secret::SecretString,
    },
};
use ::axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::de::{DeserializeOwned, Deserializer, MapAccess, Visitor};
use std::{
    any::Any,
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

/// Every port the operations behind the routes consume (R14 native shape).
pub trait Ports:
    Clock
    + IdSource
    + PasswordHasher
    + TokenCodec
    + EnrollmentPolicy
    + AttemptLimiter
    + MailDelivery
    + RegisterStore
    + CredentialReader
    + LoginStore
    + SessionRevoker
    + EpochStore
    + ChallengeReader
    + ChallengeStore
    + VerifyStore
    + PasswordStore
    + UpgradeTicketStore
    + SessionResolver
    + Clone
    + Send
    + Sync
    + 'static
{
}
impl<T> Ports for T where
    T: Clock
        + IdSource
        + PasswordHasher
        + TokenCodec
        + EnrollmentPolicy
        + AttemptLimiter
        + MailDelivery
        + RegisterStore
        + CredentialReader
        + LoginStore
        + SessionRevoker
        + EpochStore
        + ChallengeReader
        + ChallengeStore
        + VerifyStore
        + PasswordStore
        + UpgradeTicketStore
        + SessionResolver
        + Clone
        + Send
        + Sync
        + 'static
{
}

/// Root-supplied transport policy (R01).
#[derive(Clone, Debug)]
pub struct Config {
    pub session_cookie: String,
    pub csrf_cookie: String,
    pub secure: bool,
    pub allowed_origins: Vec<String>,
    pub csrf_ttl_ms: i64,
    pub session_absolute_ms: i64,
}
pub struct Deps<P, E> {
    pub ports: P,
    pub app: app::Config,
    pub keyed: Arc<Digest>,
    pub entropy: E,
}
/// The admitted value a session-bound handler receives (R07).
#[derive(Clone)]
pub struct Admitted {
    pub principal: AuthenticatedPrincipal,
    pub session_digest_hex: String,
}

/// Extracts only the safe principal reference from a session admission. Root
/// uses this to translate identity admission into another module's port while
/// the session credential remains private to identity.
pub fn principal_id(value: Option<&(dyn Any + Send + Sync)>) -> Result<Id, Failure> {
    value
        .and_then(|value| value.downcast_ref::<Admitted>())
        .map(|admitted| admitted.principal.principal_id)
        .ok_or_else(|| {
            Failure::new(Kind::Internal, "invalid session admission")
                .with_type("identity.admission_corrupt")
        })
}

/// Opaque socket admission: root may read the safe principal, while only this
/// identity transport can use the session credential for revalidation.
pub struct WebSocketAdmission {
    pub principal: AuthenticatedPrincipal,
    session: SecretString,
}

const MAX_CSRF_TTL_MS: i64 = 3_600_000;
const ANONYMOUS: &str = "anonymous";
const NO_STORE: (&str, &str) = ("Cache-Control", "no-store");

fn configuration() -> Failure {
    Failure::new(Kind::Invalid, "invalid identity transport configuration")
        .with_type("identity.transport_configuration")
}
fn cookie_name_valid(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
fn origin_valid(origin: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    (scheme == "http" || scheme == "https")
        && !rest.is_empty()
        && !rest.contains(['/', '*', ' ', '?', '#'])
        && rest.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn refused(kind: Kind, code: &'static str, message: &'static str) -> Failure {
    Failure::new(kind, message).with_type(code)
}
fn origin_rejected() -> Failure {
    refused(
        Kind::Forbidden,
        "identity.origin_rejected",
        "origin not allowed",
    )
}
fn csrf_rejected() -> Failure {
    refused(
        Kind::Forbidden,
        "identity.csrf_rejected",
        "csrf check failed",
    )
}
fn session_rejected() -> Failure {
    refused(
        Kind::Unauthenticated,
        "identity.session_rejected",
        "authentication refused",
    )
}
fn invalid_body() -> Failure {
    Failure::new(Kind::Invalid, "invalid request body").with_type("http.invalid_body")
}

/// Strict JSON object decoding: unknown members are refused by the target type,
/// duplicate keys and non-object documents here (R02).
struct DuplicateFreeMap;
impl<'de> Visitor<'de> for DuplicateFreeMap {
    type Value = BTreeMap<String, serde_json::Value>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON object without duplicate members")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value::<serde_json::Value>()?;
            if out.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate member"));
            }
        }
        Ok(out)
    }
}
/// Every refusal this transport emits is a Refusal value carrying no-store (R12).
fn bare(e: Failure) -> RequestFailure {
    RequestFailure::Refused(Refusal::new(e).with_header(NO_STORE.0, NO_STORE.1))
}
fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, RequestFailure> {
    let mut de = serde_json::Deserializer::from_slice(body);
    let map = de
        .deserialize_map(DuplicateFreeMap)
        .map_err(|_| bare(invalid_body()))?;
    de.end().map_err(|_| bare(invalid_body()))?;
    serde_json::from_value(serde_json::Value::Object(map.into_iter().collect()))
        .map_err(|_| bare(invalid_body()))
}
fn no_body(request: &Request) -> Result<(), RequestFailure> {
    if request.body.is_empty() {
        Ok(())
    } else {
        Err(bare(invalid_body()))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailPassword {
    email: String,
    password: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailOnly {
    email: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenPassword {
    token: String,
    password: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeBody {
    current_password: String,
    new_password: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// GET /csrf: a valid session binds the context; anything else is anonymous.
    CsrfIssue,
    /// Anonymous POST: origin and CSRF; a stale session cookie is ignored.
    AnonymousPost,
    /// POST /logout: origin and CSRF; a present session must be valid.
    SessionOptionalPost,
    SessionRequiredGet,
    SessionRequiredPost,
}
struct Inner<P> {
    config: Config,
    keyed: Arc<Digest>,
    entropy: Mutex<Box<dyn Entropy + Send>>,
    ports: P,
    register: Register<P>,
    request_verification: RequestVerification<P>,
    verify_email: VerifyEmail<P>,
    login: Login<P>,
    authenticate: Authenticate<P>,
    logout: Logout<P>,
    logout_all: LogoutAll<P>,
    change: ChangePassword<P>,
    request_reset: RequestReset<P>,
    reset: ResetPassword<P>,
    websocket_ticket: WebSocketTicket<P>,
}
#[derive(Clone)]
pub struct Transport<P> {
    inner: Arc<Inner<P>>,
}
type Handler<P> = Arc<dyn Fn(Arc<Inner<P>>, RequestContext) -> Work + Send + Sync>;

impl<P: Ports> Transport<P> {
    pub fn new<E: Entropy + Send + 'static>(
        config: Config,
        deps: Deps<P, E>,
    ) -> Result<Self, Failure> {
        let c = &config;
        if !cookie_name_valid(&c.session_cookie)
            || !cookie_name_valid(&c.csrf_cookie)
            || c.session_cookie == c.csrf_cookie
            || (!c.secure
                && (c.session_cookie.starts_with("__Host-")
                    || c.csrf_cookie.starts_with("__Host-")))
            || c.allowed_origins.is_empty()
            || !c.allowed_origins.iter().all(|o| origin_valid(o))
            || !(1..=MAX_CSRF_TTL_MS).contains(&c.csrf_ttl_ms)
            || c.session_absolute_ms < 1
        {
            return Err(configuration());
        }
        let p = deps.ports;
        let a = deps.app;
        let inner = Inner {
            config,
            keyed: deps.keyed,
            entropy: Mutex::new(Box::new(deps.entropy)),
            ports: p.clone(),
            register: Register::new(p.clone(), a)?,
            request_verification: RequestVerification::new(p.clone(), a)?,
            verify_email: VerifyEmail::new(p.clone(), a)?,
            login: Login::new(p.clone(), a)?,
            authenticate: Authenticate::new(p.clone(), a)?,
            logout: Logout::new(p.clone())?,
            logout_all: LogoutAll::new(p.clone())?,
            change: ChangePassword::new(p.clone())?,
            request_reset: RequestReset::new(p.clone(), a)?,
            reset: ResetPassword::new(p.clone())?,
            websocket_ticket: WebSocketTicket::new(p)?,
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }
    /// Reuses the identity session boundary for a root-owned protected route.
    pub fn require_session(&self) -> crate::shared::http::axum::Admit {
        let inner = self.inner.clone();
        Arc::new(move |request| {
            let inner = inner.clone();
            Box::pin(async move { admit(inner, request, Mode::SessionRequiredGet).await })
        })
    }
    /// The root-owned POST boundary applies the same session check plus
    /// origin and CSRF enforcement as identity's mutating routes.
    pub fn require_session_post(&self) -> crate::shared::http::axum::Admit {
        let inner = self.inner.clone();
        Arc::new(move |request| {
            let inner = inner.clone();
            Box::pin(async move { admit(inner, request, Mode::SessionRequiredPost).await })
        })
    }
    /// The root-owned WebSocket boundary uses the same session resolver as HTTP,
    /// then consumes a one-use ticket bound to that session and auth epoch.
    pub async fn authorize_websocket(
        &self,
        headers: HeaderMap,
    ) -> Result<WebSocketAdmission, Failure> {
        let origins: Vec<_> = headers.get_all("origin").iter().collect();
        if origins.len() != 1
            || origins[0]
                .to_str()
                .ok()
                .is_none_or(|v| !self.inner.config.allowed_origins.iter().any(|o| o == v))
        {
            return Err(origin_rejected());
        }
        let cookie_headers: Vec<&str> = headers
            .get_all("cookie")
            .iter()
            .map(|v| v.to_str().map_err(|_| session_rejected()))
            .collect::<Result<_, _>>()?;
        let cookies = parse_cookies(&cookie_headers).map_err(|_| session_rejected())?;
        let session = match cookies
            .get(&self.inner.config.session_cookie)
            .map(Vec::as_slice)
        {
            Some([value]) => value.clone(),
            _ => return Err(session_rejected()),
        };
        let protocols: Vec<&str> = headers
            .get_all("sec-websocket-protocol")
            .iter()
            .map(|v| v.to_str().map_err(|_| session_rejected()))
            .collect::<Result<_, _>>()?;
        let mut version = false;
        let mut ticket = None;
        for header in protocols {
            for part in header.split(',').map(str::trim) {
                if part == "n2f.v1" {
                    if version {
                        return Err(session_rejected());
                    }
                    version = true;
                } else if let Some(value) = part.strip_prefix("n2f.ticket.") {
                    if ticket.is_some() || value.is_empty() {
                        return Err(session_rejected());
                    }
                    ticket = Some(value.to_owned());
                } else {
                    return Err(session_rejected());
                }
            }
        }
        if !version {
            return Err(session_rejected());
        }
        let ticket = ticket.ok_or_else(session_rejected)?;
        let session = SecretString::new(session);
        let principal = self.inner.authenticate.authenticate(&session).await?;
        let digest = self
            .inner
            .ports
            .digest(TokenPurpose::WebSocketUpgrade, &SecretString::new(ticket))
            .map_err(|_| session_rejected())?;
        let now = self.inner.now_ms()?;
        let admitted = self
            .inner
            .ports
            .consume_upgrade_ticket(&digest, now)
            .await?;
        if admitted.is_none_or(|value| {
            value.principal_id != principal.principal_id
                || value.session_id != principal.session_id
                || value.auth_epoch != principal.auth_epoch
        }) {
            return Err(session_rejected());
        }
        Ok(WebSocketAdmission { principal, session })
    }
    /// Repeats the session resolver's transactional status, expiry, idle and
    /// auth-epoch checks for an existing connection.
    pub async fn revalidate_websocket(
        &self,
        admission: &WebSocketAdmission,
    ) -> Result<(), Failure> {
        let principal = self
            .inner
            .authenticate
            .authenticate(&admission.session)
            .await?;
        if principal != admission.principal {
            return Err(session_rejected());
        }
        Ok(())
    }
    /// The registered routes (R02), each with its fixed provenance operation (R13).
    pub fn routes(&self) -> Vec<Route> {
        let table: Vec<(&str, &str, &str, Mode, Handler<P>)> = vec![
            (
                "GET",
                "/v1/auth/csrf",
                "identity.http.csrf",
                Mode::CsrfIssue,
                Arc::new(|i, c| Box::pin(csrf(i, c))),
            ),
            (
                "POST",
                "/v1/auth/register",
                "identity.http.register",
                Mode::AnonymousPost,
                Arc::new(|i, c| Box::pin(register(i, c))),
            ),
            (
                "POST",
                "/v1/auth/email/verification-requests",
                "identity.http.verification_request",
                Mode::AnonymousPost,
                Arc::new(|i, c| Box::pin(verification_request(i, c))),
            ),
            (
                "POST",
                "/v1/auth/email/verify",
                "identity.http.verify",
                Mode::AnonymousPost,
                Arc::new(|i, c| Box::pin(verify(i, c))),
            ),
            (
                "POST",
                "/v1/auth/login",
                "identity.http.login",
                Mode::AnonymousPost,
                Arc::new(|i, c| Box::pin(login(i, c))),
            ),
            (
                "GET",
                "/v1/auth/session",
                "identity.http.session",
                Mode::SessionRequiredGet,
                Arc::new(|i, c| Box::pin(session(i, c))),
            ),
            (
                "POST",
                "/v1/auth/websocket-ticket",
                "identity.http.websocket_ticket",
                Mode::SessionRequiredPost,
                Arc::new(|i, c| Box::pin(websocket_ticket(i, c))),
            ),
            (
                "POST",
                "/v1/auth/logout",
                "identity.http.logout",
                Mode::SessionOptionalPost,
                Arc::new(|i, c| Box::pin(logout(i, c))),
            ),
            (
                "POST",
                "/v1/auth/logout-all",
                "identity.http.logout_all",
                Mode::SessionRequiredPost,
                Arc::new(|i, c| Box::pin(logout_all(i, c))),
            ),
            (
                "POST",
                "/v1/auth/password/change",
                "identity.http.password_change",
                Mode::SessionRequiredPost,
                Arc::new(|i, c| Box::pin(change(i, c))),
            ),
            (
                "POST",
                "/v1/auth/password/reset-requests",
                "identity.http.reset_request",
                Mode::AnonymousPost,
                Arc::new(|i, c| Box::pin(reset_request(i, c))),
            ),
            (
                "POST",
                "/v1/auth/password/reset",
                "identity.http.reset",
                Mode::AnonymousPost,
                Arc::new(|i, c| Box::pin(reset(i, c))),
            ),
        ];
        table
            .into_iter()
            .map(|(method, path, operation, mode, handler)| {
                let admit_inner = self.inner.clone();
                let handle_inner = self.inner.clone();
                Route {
                    path: path.into(),
                    method: method.into(),
                    operation: operation.into(),
                    admission: Some(Arc::new(move |request: Arc<Request>| {
                        let inner = admit_inner.clone();
                        Box::pin(async move { admit(inner, request, mode).await })
                    })),
                    handler: Arc::new(move |context| handler(handle_inner.clone(), context)),
                }
            })
            .collect()
    }
}

// ---- Cookies and CSRF ---------------------------------------------------------------

impl<P: Ports> Inner<P> {
    fn now_ms(&self) -> Result<i64, Failure> {
        self.ports
            .now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .ok_or_else(|| {
                Failure::new(Kind::Unavailable, "clock reading out of range")
                    .with_type("identity.auth_dependency_failed")
            })
    }
    fn cookie(&self, name: &str, value: String, max_age_ms: i64) -> Cookie {
        Cookie {
            name: name.into(),
            value,
            path: Some("/".into()),
            max_age: Some(u64::try_from(max_age_ms / 1000).unwrap_or(0)),
            expires: None,
            secure: self.config.secure,
            http_only: true,
            same_site: Some(SameSite::Lax),
        }
    }
    fn session_cookie(&self, secret: &SecretString) -> Cookie {
        self.cookie(
            &self.config.session_cookie,
            secret.reveal().into(),
            self.config.session_absolute_ms,
        )
    }
    fn clear(&self, name: &str) -> Cookie {
        self.cookie(name, String::new(), 0)
    }
    fn csrf_message(nonce: &[u8], issued_at_ms: i64, binding: &str) -> Vec<u8> {
        let mut m = Vec::with_capacity(nonce.len() + 24 + binding.len());
        m.extend_from_slice(nonce);
        m.push(0);
        m.extend_from_slice(issued_at_ms.to_string().as_bytes());
        m.push(0);
        m.extend_from_slice(binding.as_bytes());
        m
    }
    /// A fresh CSRF context (R04): `nonce.issued.tag`, bound to the session digest or anonymous.
    fn issue_csrf(&self, binding: &str) -> Result<(String, Cookie), Failure> {
        let mut nonce = [0u8; 16];
        self.entropy
            .lock()
            .map_err(|_| {
                Failure::new(Kind::Internal, "entropy lock poisoned")
                    .with_type("identity.csrf_failed")
            })?
            .fill(&mut nonce)
            .map_err(|e| {
                Failure::new(Kind::Unavailable, "entropy unavailable")
                    .with_type("identity.entropy_unavailable")
                    .with_boxed_source(e)
            })?;
        let issued = self.now_ms()?;
        let tag = self
            .keyed
            .sign("csrf", &Self::csrf_message(&nonce, issued, binding))?;
        let token = format!(
            "{}.{issued}.{}",
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(tag)
        );
        let cookie = self.cookie(
            &self.config.csrf_cookie,
            token.clone(),
            self.config.csrf_ttl_ms,
        );
        Ok((token, cookie))
    }
    /// R06: exactly one cookie and one header, byte-equal, verified under the binding, within the lifetime.
    fn check_csrf(&self, request: &Request, binding: &str) -> Result<(), Failure> {
        let cookies = request
            .cookies
            .get(&self.config.csrf_cookie)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let headers = &request.headers.csrf_token;
        if cookies.len() != 1 || headers.len() != 1 || cookies[0] != headers[0] {
            return Err(csrf_rejected());
        }
        let token = &cookies[0];
        let mut parts = token.split('.');
        let (Some(nonce), Some(issued), Some(tag), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(csrf_rejected());
        };
        let nonce = URL_SAFE_NO_PAD.decode(nonce).map_err(|_| csrf_rejected())?;
        let tag = URL_SAFE_NO_PAD.decode(tag).map_err(|_| csrf_rejected())?;
        let issued: i64 = issued.parse().map_err(|_| csrf_rejected())?;
        let now = self.now_ms()?;
        if nonce.len() != 16 || issued > now || now - issued > self.config.csrf_ttl_ms {
            return Err(csrf_rejected());
        }
        if !self
            .keyed
            .verify("csrf", &Self::csrf_message(&nonce, issued, binding), &tag)?
        {
            return Err(csrf_rejected());
        }
        Ok(())
    }
    fn check_origin(&self, request: &Request) -> Result<(), Failure> {
        match request.headers.origin.as_slice() {
            [one] if self.config.allowed_origins.iter().any(|o| o == one) => Ok(()),
            _ => Err(origin_rejected()),
        }
    }
    /// The presented session, when any: `Err` for a duplicate cookie (R03) or a
    /// failed resolution; `Ok(None)` when no cookie was presented.
    async fn presented_session(&self, request: &Request) -> Result<Option<Admitted>, Failure> {
        let values = request
            .cookies
            .get(&self.config.session_cookie)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        match values {
            [] => Ok(None),
            [one] => {
                let secret = SecretString::new(one.clone());
                let principal = self.authenticate.authenticate(&secret).await?;
                let digest = self
                    .ports
                    .digest(TokenPurpose::Session, &secret)
                    .map_err(|_| session_rejected())?;
                Ok(Some(Admitted {
                    principal,
                    session_digest_hex: hex_digest(&digest),
                }))
            }
            _ => Err(session_rejected()),
        }
    }
}
fn hex_digest(d: &TokenDigest) -> String {
    d.bytes().iter().map(|b| format!("{b:02x}")).collect()
}
fn is_unauthenticated(e: &Failure) -> bool {
    e.classification()
        .is_some_and(|c| c.kind == Kind::Unauthenticated)
}
async fn admit<P: Ports>(inner: Arc<Inner<P>>, request: Arc<Request>, mode: Mode) -> Admission {
    let refuse = |failure: Failure| {
        let cookies = if is_unauthenticated(&failure) {
            vec![inner.clear(&inner.config.session_cookie)]
        } else {
            Vec::new()
        };
        Admission::Refused {
            failure,
            headers: vec![(NO_STORE.0.into(), NO_STORE.1.into())],
            cookies,
        }
    };
    let session = match inner.presented_session(&request).await {
        Ok(s) => s,
        Err(e) => {
            let anonymous_route = matches!(mode, Mode::CsrfIssue | Mode::AnonymousPost);
            let duplicate = request
                .cookies
                .get(&inner.config.session_cookie)
                .is_some_and(|v| v.len() > 1);
            if anonymous_route && is_unauthenticated(&e) && !duplicate {
                None
            } else {
                return refuse(e);
            }
        }
    };
    if matches!(mode, Mode::SessionRequiredGet | Mode::SessionRequiredPost) && session.is_none() {
        return refuse(session_rejected());
    }
    if matches!(
        mode,
        Mode::AnonymousPost | Mode::SessionOptionalPost | Mode::SessionRequiredPost
    ) {
        if let Err(e) = inner.check_origin(&request) {
            return refuse(e);
        }
        let binding = session
            .as_ref()
            .map_or(ANONYMOUS, |s| s.session_digest_hex.as_str());
        if let Err(e) = inner.check_csrf(&request, binding) {
            return refuse(e);
        }
    }
    match session {
        Some(admitted) => {
            match Actor::new(ActorKind::User, admitted.principal.principal_id.to_string()) {
                Ok(initiator) => Admission::Authenticated {
                    initiator,
                    tenant: None,
                    admitted: Arc::new(admitted),
                },
                Err(e) => refuse(e),
            }
        }
        None => Admission::Anonymous,
    }
}

// ---- Handlers -----------------------------------------------------------------------

fn admitted(context: &RequestContext) -> Option<Admitted> {
    context
        .request
        .admitted
        .as_ref()
        .and_then(|a| a.downcast_ref::<Admitted>().cloned())
}
/// R12: identity refusals keep their kind and type and nothing else; the transport
/// never adds a field that says which credential failed.
fn project<P: Ports>(i: &Inner<P>, e: Failure) -> RequestFailure {
    let sanitized = match e.classification() {
        Some(c) if c.error_type == Some("identity.credentials_rejected") => Some(
            Failure::new(Kind::Unauthenticated, "authentication refused")
                .with_type("identity.credentials_rejected"),
        ),
        _ => None,
    };
    known(i, sanitized.unwrap_or(e))
}
/// R07: a session refusal clears the session cookie; R08: a rate-limit refusal
/// carries Retry-After in whole seconds (ceiling, minimum 1) from the failure's
/// retry-after detail; every refusal is no-store.
fn known<P: Ports>(i: &Inner<P>, e: Failure) -> RequestFailure {
    let (kind, error_type) = e
        .classification()
        .map(|c| (Some(c.kind), c.error_type.map(str::to_owned)))
        .unwrap_or((None, None));
    let mut refusal = Refusal::new(e).with_header(NO_STORE.0, NO_STORE.1);
    if error_type.as_deref() == Some("identity.session_rejected") {
        refusal = refusal.with_cookie(i.clear(&i.config.session_cookie));
    }
    if kind == Some(Kind::RateLimited) {
        let ms: i64 = refusal
            .failure
            .details()
            .get("retry_after_ms")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let seconds = ((ms.max(0) + 999) / 1000).max(1);
        refusal = refusal.with_header("Retry-After", &seconds.to_string());
    }
    RequestFailure::Refused(refusal)
}
fn accepted() -> Response {
    Response::json(serde_json::json!({"accepted": true}))
        .with_status(202)
        .with_header(NO_STORE.0, NO_STORE.1)
}
fn no_content() -> Response {
    Response::empty(204).with_header(NO_STORE.0, NO_STORE.1)
}
async fn csrf<P: Ports>(i: Arc<Inner<P>>, c: RequestContext) -> Result<Response, RequestFailure> {
    let binding = admitted(&c).map_or(ANONYMOUS.to_string(), |a| a.session_digest_hex);
    let (token, cookie) = i.issue_csrf(&binding).map_err(|e| known(&i, e))?;
    Ok(Response::json(serde_json::json!({"csrf_token": token}))
        .with_header(NO_STORE.0, NO_STORE.1)
        .with_cookie(cookie))
}
async fn register<P: Ports>(
    i: Arc<Inner<P>>,
    c: RequestContext,
) -> Result<Response, RequestFailure> {
    let body: EmailPassword = decode(&c.request.body)?;
    i.register
        .register(
            c.scope.work_context(),
            RegisterInput {
                email: body.email,
                password: SecretString::new(body.password),
                source: c.request.source.clone(),
            },
        )
        .await
        .map_err(|e| project(&i, e))?;
    Ok(accepted())
}
async fn verification_request<P: Ports>(
    i: Arc<Inner<P>>,
    c: RequestContext,
) -> Result<Response, RequestFailure> {
    let body: EmailOnly = decode(&c.request.body)?;
    i.request_verification
        .request(RequestInput {
            email: body.email,
            source: c.request.source.clone(),
        })
        .await
        .map_err(|e| project(&i, e))?;
    Ok(accepted())
}
async fn verify<P: Ports>(i: Arc<Inner<P>>, c: RequestContext) -> Result<Response, RequestFailure> {
    let body: TokenPassword = decode(&c.request.body)?;
    i.verify_email
        .verify(
            c.scope.work_context(),
            VerifyEmailInput {
                token: SecretString::new(body.token),
                password: SecretString::new(body.password),
            },
        )
        .await
        .map_err(|e| project(&i, e))?;
    Ok(no_content())
}
async fn login<P: Ports>(i: Arc<Inner<P>>, c: RequestContext) -> Result<Response, RequestFailure> {
    let body: EmailPassword = decode(&c.request.body)?;
    let issued = i
        .login
        .login(
            c.scope.work_context(),
            LoginInput {
                email: body.email,
                password: SecretString::new(body.password),
                source: c.request.source.clone(),
            },
        )
        .await
        .map_err(|e| project(&i, e))?;
    let digest = i
        .ports
        .digest(TokenPurpose::Session, &issued.secret)
        .map_err(|e| known(&i, e))?;
    let (token, csrf_cookie) = i
        .issue_csrf(&hex_digest(&digest))
        .map_err(|e| known(&i, e))?;
    Ok(Response::json(serde_json::json!({
        "principal_id": issued.principal.id.to_string(),
        "absolute_expires_at_ms": issued.absolute_expires_at_ms,
        "idle_expires_at_ms": issued.idle_expires_at_ms,
        "csrf_token": token,
    }))
    .with_header(NO_STORE.0, NO_STORE.1)
    .with_cookie(i.session_cookie(&issued.secret))
    .with_cookie(csrf_cookie))
}
async fn session<P: Ports>(
    i: Arc<Inner<P>>,
    c: RequestContext,
) -> Result<Response, RequestFailure> {
    let a = admitted(&c).ok_or_else(|| known(&i, session_rejected()))?;
    Ok(Response::json(serde_json::json!({
        "principal_id": a.principal.principal_id.to_string(),
        "auth_epoch": a.principal.auth_epoch,
    }))
    .with_header(NO_STORE.0, NO_STORE.1))
}
async fn websocket_ticket<P: Ports>(
    i: Arc<Inner<P>>,
    c: RequestContext,
) -> Result<Response, RequestFailure> {
    no_body(&c.request)?;
    let a = admitted(&c).ok_or_else(|| known(&i, session_rejected()))?;
    let issued = i
        .websocket_ticket
        .issue(a.principal)
        .await
        .map_err(|e| project(&i, e))?;
    Ok(Response::json(serde_json::json!({"ticket": issued.secret.reveal(), "expires_at_ms": issued.expires_at_ms}))
        .with_header(NO_STORE.0, NO_STORE.1))
}
async fn logout<P: Ports>(i: Arc<Inner<P>>, c: RequestContext) -> Result<Response, RequestFailure> {
    no_body(&c.request)?;
    if let Some(a) = admitted(&c) {
        i.logout
            .logout(c.scope.work_context(), &a.principal)
            .await
            .map_err(|e| project(&i, e))?;
    }
    let (_, csrf_cookie) = i.issue_csrf(ANONYMOUS).map_err(|e| known(&i, e))?;
    Ok(no_content()
        .with_cookie(i.clear(&i.config.session_cookie))
        .with_cookie(csrf_cookie))
}
async fn logout_all<P: Ports>(
    i: Arc<Inner<P>>,
    c: RequestContext,
) -> Result<Response, RequestFailure> {
    no_body(&c.request)?;
    let a = admitted(&c).ok_or_else(|| known(&i, session_rejected()))?;
    i.logout_all
        .logout_all(c.scope.work_context(), &a.principal)
        .await
        .map_err(|e| project(&i, e))?;
    Ok(no_content()
        .with_cookie(i.clear(&i.config.session_cookie))
        .with_cookie(i.clear(&i.config.csrf_cookie)))
}
async fn change<P: Ports>(i: Arc<Inner<P>>, c: RequestContext) -> Result<Response, RequestFailure> {
    let body: ChangeBody = decode(&c.request.body)?;
    let a = admitted(&c).ok_or_else(|| known(&i, session_rejected()))?;
    i.change
        .change(
            c.scope.work_context(),
            &a.principal,
            ChangePasswordInput {
                current: SecretString::new(body.current_password),
                new: SecretString::new(body.new_password),
                source: c.request.source.clone(),
            },
        )
        .await
        .map_err(|e| project(&i, e))?;
    Ok(no_content()
        .with_cookie(i.clear(&i.config.session_cookie))
        .with_cookie(i.clear(&i.config.csrf_cookie)))
}
async fn reset_request<P: Ports>(
    i: Arc<Inner<P>>,
    c: RequestContext,
) -> Result<Response, RequestFailure> {
    let body: EmailOnly = decode(&c.request.body)?;
    i.request_reset
        .request(RequestInput {
            email: body.email,
            source: c.request.source.clone(),
        })
        .await
        .map_err(|e| project(&i, e))?;
    Ok(accepted())
}
async fn reset<P: Ports>(i: Arc<Inner<P>>, c: RequestContext) -> Result<Response, RequestFailure> {
    let body: TokenPassword = decode(&c.request.body)?;
    i.reset
        .reset(
            c.scope.work_context(),
            ResetPasswordInput {
                token: SecretString::new(body.token),
                new: SecretString::new(body.password),
                source: c.request.source.clone(),
            },
        )
        .await
        .map_err(|e| project(&i, e))?;
    Ok(no_content().with_cookie(i.clear(&i.config.session_cookie)))
}
