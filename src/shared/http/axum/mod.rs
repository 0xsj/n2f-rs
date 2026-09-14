//! Bounded JSON transport. The body owns finalization after handoff to Hyper.
use super::{Active, Completion, CompletionFacts, Termination, normalize_method, problem_of};
use crate::shared::{
    errors::{Classified, Failure, Kind, public_info},
    logger::{Fields, Logger},
    provenance::{
        Actor, Attribution, AttributionSpec, IncomingHints, IncomingResult, Scope, inspect_incoming,
    },
    telemetry::TraceRef,
};
use ::axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request as HttpRequest, Response as HttpResponse},
};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    any::Any,
    collections::BTreeMap,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};
pub enum RequestFailure {
    Known(Failure),
    /// A classified refusal that also carries allowlisted headers and cookies
    /// (H16/H17): the problem projection is unchanged, the headers are written.
    Refused(Refusal),
    Unknown,
}
/// A refusal value: the failure that selects status and problem body, plus the
/// headers and Set-Cookie values the owner needs on that refusal (a cleared
/// session cookie on 401, Retry-After on 429, Cache-Control on any of them).
/// Headers follow the same allowlist as Response; violations are handler errors.
#[derive(Debug)]
pub struct Refusal {
    pub failure: Failure,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<Cookie>,
}
impl Refusal {
    pub fn new(failure: Failure) -> Self {
        Self {
            failure,
            headers: Vec::new(),
            cookies: Vec::new(),
        }
    }
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
    pub fn with_cookie(mut self, cookie: Cookie) -> Self {
        self.cookies.push(cookie);
        self
    }
    /// The Response allowlist applies to refusals too; the status is the failure's.
    pub fn validate(&self) -> Result<(), Failure> {
        Response {
            status: 200,
            body: None,
            headers: self.headers.clone(),
            cookies: self.cookies.clone(),
        }
        .validate()
    }
}
impl RequestFailure {
    fn classification(&self) -> Option<crate::shared::errors::Classification<'_>> {
        match self {
            RequestFailure::Known(e) | RequestFailure::Refused(Refusal { failure: e, .. }) => {
                e.classification()
            }
            RequestFailure::Unknown => None,
        }
    }
}
pub type Work = Pin<Box<dyn Future<Output = Result<Response, RequestFailure>> + Send>>;
#[derive(Clone)]
pub struct RequestContext {
    pub scope: Scope,
    pub log: Logger,
    pub request: Arc<Request>,
}
/// Peer address, inserted as a request extension by the serving loop (H15).
#[derive(Clone, Copy, Debug)]
pub struct Peer(pub SocketAddr);
/// Selected request headers, each as the list of values received (H15).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers {
    pub origin: Vec<String>,
    pub content_type: Vec<String>,
    pub csrf_token: Vec<String>,
    pub subprotocol: Vec<String>,
}
/// The immutable request value a feature handler receives (H15). Never the
/// framework request; bodies are already bounded and media-type checked.
#[derive(Clone)]
pub struct Request {
    pub method: String,
    pub template: String,
    pub query: String,
    pub body: Vec<u8>,
    pub headers: Headers,
    pub cookies: BTreeMap<String, Vec<String>>,
    pub source: String,
    pub admitted: Option<Arc<dyn Any + Send + Sync>>,
}
fn invalid_cookie() -> Failure {
    Failure::new(Kind::Invalid, "invalid cookie header").with_type("http.invalid_cookie")
}
fn token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
fn cookie_octet(b: u8) -> bool {
    (0x21..=0x7e).contains(&b) && !b"\",;\\".contains(&b)
}
/// Parse every Cookie header value into name → values, keeping duplicates so
/// the route owner can refuse them; a malformed pair refuses the request.
pub fn parse_cookies(values: &[&str]) -> Result<BTreeMap<String, Vec<String>>, Failure> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for header in values {
        for pair in header.split(';') {
            let pair = pair.trim_matches(' ');
            let Some((name, value)) = pair.split_once('=') else {
                return Err(invalid_cookie());
            };
            if name.is_empty() || !name.bytes().all(token_char) || !value.bytes().all(cookie_octet)
            {
                return Err(invalid_cookie());
            }
            out.entry(name.to_owned())
                .or_default()
                .push(value.to_owned());
        }
    }
    Ok(out)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}
/// A Set-Cookie value the boundary serializes (H16). The owner encodes the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub path: Option<String>,
    pub max_age: Option<u64>,
    pub expires: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<SameSite>,
}
impl Cookie {
    pub fn serialize(&self) -> Result<String, Failure> {
        let text_ok = |s: &str| s.bytes().all(|b| (0x20..=0x7e).contains(&b));
        if self.name.is_empty()
            || !self.name.bytes().all(token_char)
            || !self.value.bytes().all(cookie_octet)
            || !self
                .path
                .as_deref()
                .is_none_or(|p| text_ok(p) && !p.contains(';'))
            || !self
                .expires
                .as_deref()
                .is_none_or(|e| text_ok(e) && !e.contains(';'))
        {
            return Err(invalid_cookie());
        }
        if self.name.starts_with("__Host-") && (!self.secure || self.path.as_deref() != Some("/")) {
            return Err(invalid_cookie());
        }
        let mut out = format!("{}={}", self.name, self.value);
        if let Some(p) = &self.path {
            out.push_str("; Path=");
            out.push_str(p);
        }
        if let Some(a) = self.max_age {
            out.push_str(&format!("; Max-Age={a}"));
        }
        if let Some(e) = &self.expires {
            out.push_str("; Expires=");
            out.push_str(e);
        }
        if self.secure {
            out.push_str("; Secure");
        }
        if self.http_only {
            out.push_str("; HttpOnly");
        }
        if let Some(s) = self.same_site {
            out.push_str(match s {
                SameSite::Strict => "; SameSite=Strict",
                SameSite::Lax => "; SameSite=Lax",
                SameSite::None => "; SameSite=None",
            });
        }
        Ok(out)
    }
}
const ALLOWED_STATUS: [u16; 5] = [200, 201, 202, 204, 303];
const ALLOWED_HEADERS: [&str; 5] = [
    "cache-control",
    "allow",
    "www-authenticate",
    "retry-after",
    "location",
];
fn invalid_response() -> Failure {
    Failure::new(Kind::Internal, "invalid response value").with_type("http.invalid_response")
}
/// The response value a feature handler returns (H16).
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub body: Option<serde_json::Value>,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<Cookie>,
}
impl Response {
    pub fn json(body: serde_json::Value) -> Self {
        Self {
            status: 200,
            body: Some(body),
            headers: Vec::new(),
            cookies: Vec::new(),
        }
    }
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            body: None,
            headers: Vec::new(),
            cookies: Vec::new(),
        }
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
    pub fn with_cookie(mut self, cookie: Cookie) -> Self {
        self.cookies.push(cookie);
        self
    }
    /// Status from the allowed set, no body on 204, allowlisted headers only,
    /// serializable cookies. Violations are handler errors, never silent drops.
    pub fn validate(&self) -> Result<(), Failure> {
        if !ALLOWED_STATUS.contains(&self.status) || (self.status == 204 && self.body.is_some()) {
            return Err(invalid_response());
        }
        for (name, value) in &self.headers {
            let lower = name.to_ascii_lowercase();
            if !ALLOWED_HEADERS.contains(&lower.as_str())
                || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
            {
                return Err(invalid_response());
            }
        }
        for c in &self.cookies {
            c.serialize()?;
        }
        Ok(())
    }
}
/// Admission outcome (H17): the boundary opens the scope with the returned
/// initiator, or projects the refusal with the owner's allowlisted headers.
pub enum Admission {
    Anonymous,
    Authenticated {
        initiator: Actor,
        tenant: Option<String>,
        admitted: Arc<dyn Any + Send + Sync>,
    },
    Refused {
        failure: Failure,
        headers: Vec<(String, String)>,
        cookies: Vec<Cookie>,
    },
}
pub type Admit =
    Arc<dyn Fn(Arc<Request>) -> Pin<Box<dyn Future<Output = Admission> + Send>> + Send + Sync>;
const KNOWN_METHODS: [&str; 8] = [
    "GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "CONNECT", "TRACE",
];
/// Wrap a JSON-only handler future as feature work: Ok(value) becomes a 200 JSON response.
pub fn json_work(
    f: impl Future<Output = Result<serde_json::Value, RequestFailure>> + Send + 'static,
) -> Work {
    Box::pin(async move { f.await.map(Response::json) })
}
/// The Allow header for a template: its registered methods plus HEAD for GET.
pub fn allow_set(registered: &[String]) -> String {
    let mut set: Vec<String> = registered.to_vec();
    if set.iter().any(|m| m == "GET") {
        set.push("HEAD".into());
    }
    set.sort();
    set.dedup();
    set.join(", ")
}
tokio::task_local! {static CURRENT:RequestContext;}
pub fn current() -> Option<RequestContext> {
    CURRENT.try_with(Clone::clone).ok()
}
pub trait Observation: Send + Sync {
    fn trace(&self) -> Option<&TraceRef>;
    fn instrument(&self, work: Work) -> Work;
    fn finish(
        &self,
        c: &Completion,
        route: Option<&str>,
        scope: Option<&Scope>,
        failure: Option<&RequestFailure>,
    );
}
pub trait Observer: Send + Sync {
    fn start(&self, method: &str, scheme: &str, headers: &HeaderMap) -> Arc<dyn Observation>;
}
pub struct Noop;
impl Observation for Noop {
    fn trace(&self) -> Option<&TraceRef> {
        None
    }
    fn instrument(&self, work: Work) -> Work {
        work
    }
    fn finish(
        &self,
        _: &Completion,
        _: Option<&str>,
        _: Option<&Scope>,
        _: Option<&RequestFailure>,
    ) {
    }
}
impl Observer for Noop {
    fn start(&self, _: &str, _: &str, _: &HeaderMap) -> Arc<dyn Observation> {
        Arc::new(Noop)
    }
}
pub struct Route {
    pub path: String,
    pub method: String,
    pub operation: String,
    pub admission: Option<Admit>,
    pub handler: Arc<dyn Fn(RequestContext) -> Work + Send + Sync>,
}
/// Open the request scope; `None` attribution means the root's anonymous default.
pub type Open =
    Arc<dyn Fn(&str, &IncomingResult, Option<Attribution>) -> Result<Scope, Failure> + Send + Sync>;
/// Derive the trusted source key from the peer address and headers under the
/// root's trusted-proxy policy; the boundary itself trusts no forwarding header.
pub type Source = Arc<dyn Fn(Option<SocketAddr>, &HeaderMap) -> String + Send + Sync>;
pub struct Config {
    pub routes: Vec<Route>,
    pub observer: Arc<dyn Observer>,
    pub log: Logger,
    pub open: Open,
    pub source: Source,
    pub timeout: Duration,
    pub max_body: usize,
}
pub struct Server {
    config: Config,
}
struct Info {
    route: Option<String>,
    scope: Option<Scope>,
    log: Logger,
    failure: Option<RequestFailure>,
}
struct Guard {
    active: Active,
    started: Instant,
    info: Arc<Mutex<Info>>,
    status: Option<u16>,
    termination: Termination,
}
impl Guard {
    fn finish(&mut self) {
        let info = self.info.lock().unwrap_or_else(|e| e.into_inner());
        let kind = info
            .failure
            .as_ref()
            .map(|e| e.classification().map_or(Kind::Internal, |c| c.kind));
        drop(info);
        let _ = self.active.finish(
            CompletionFacts {
                status: self.status,
                failure_kind: kind,
                termination: self.termination,
            },
            self.started.elapsed(),
        );
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.finish();
    }
}
struct ObservedBody {
    status: u16,
    inner: Body,
    guard: Guard,
}
impl HttpBody for ObservedBody {
    type Data = bytes::Bytes;
    type Error = ::axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.guard.status = Some(self.status);
        let result = Pin::new(&mut self.inner).poll_frame(cx);
        match &result {
            Poll::Ready(Some(Err(_))) => {
                self.guard.termination = Termination::WriteError;
                self.guard.finish();
            }
            Poll::Ready(None) => self.guard.finish(),
            Poll::Ready(Some(Ok(_))) if self.inner.is_end_stream() => self.guard.finish(),
            _ => {}
        }
        result
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
impl Drop for ObservedBody {
    fn drop(&mut self) {
        if !self.inner.is_end_stream() {
            self.guard.termination = Termination::PeerClosed;
        }
        self.guard.finish();
    }
}
impl Server {
    pub fn new(config: Config) -> Result<Self, Failure> {
        if config.timeout.is_zero() || config.max_body == 0 {
            return Err(Failure::new(Kind::Invalid, "invalid HTTP configuration"));
        }
        let mut keys = std::collections::BTreeSet::new();
        for r in &config.routes {
            if r.path.is_empty()
                || r.operation.is_empty()
                || !KNOWN_METHODS.contains(&r.method.as_str())
                || !keys.insert((&r.path, &r.method))
            {
                return Err(Failure::new(Kind::Invalid, "invalid route"));
            }
        }
        Ok(Self { config })
    }
    fn registered(&self, path: &str) -> Vec<String> {
        self.config
            .routes
            .iter()
            .filter(|r| r.path == path)
            .map(|r| r.method.clone())
            .collect()
    }
    pub async fn handle(self: Arc<Self>, request: HttpRequest<Body>) -> HttpResponse<Body> {
        let started = Instant::now();
        let method = normalize_method(request.method().as_str()).to_owned();
        let observation = self
            .config
            .observer
            .start(&method, "http", request.headers());
        let path = request.uri().path().to_owned();
        let query = request.uri().query().unwrap_or("").to_owned();
        let head = request.method() == "HEAD";
        let wanted = if head {
            "GET".to_owned()
        } else {
            request.method().as_str().to_owned()
        };
        let registered = self.registered(&path);
        let route = self
            .config
            .routes
            .iter()
            .find(|r| r.path == path && r.method == wanted);
        let wanted_method = wanted.clone();
        let template_route = self.config.routes.iter().find(|r| r.path == path);
        let info = Arc::new(Mutex::new(Info {
            route: template_route.map(|r| r.path.clone()),
            scope: None,
            log: self.config.log.clone(),
            failure: None,
        }));
        let observed = observation.clone();
        let i = info.clone();
        let j = info.clone();
        let active = Active::begin(
            Duration::ZERO,
            vec![
                Box::new(move |c| {
                    let info = i.lock().unwrap_or_else(|e| e.into_inner());
                    observed.finish(
                        c,
                        info.route.as_deref(),
                        info.scope.as_ref(),
                        info.failure.as_ref(),
                    );
                }),
                Box::new(move |c| {
                    let info = j.lock().unwrap_or_else(|e| e.into_inner());
                    let mut log = info.log.clone();
                    if let Some(e) = &info.failure {
                        log = match e {
                            RequestFailure::Known(e) => log.with_failure(e),
                            RequestFailure::Refused(r) => log.with_failure(&r.failure),
                            RequestFailure::Unknown => {
                                log.with_unknown(&std::io::Error::other("unknown"))
                            }
                        };
                    }
                    let mut fields = Fields::new();
                    fields.insert("method".into(), method.clone().into());
                    fields.insert("outcome".into(), c.classification.outcome.as_str().into());
                    fields.insert("termination".into(), c.facts.termination.as_str().into());
                    fields.insert(
                        "elapsed_ms".into(),
                        crate::shared::logger::Value::Float(c.elapsed.as_secs_f64() * 1000.),
                    );
                    if let Some(route) = &info.route {
                        fields.insert("route".into(), route.clone().into());
                    }
                    if let Some(status) = c.facts.status {
                        fields.insert("status".into(), i64::from(status).into());
                    }
                    match c.classification.outcome.as_str() {
                        "failed" => log.error("http.request.completed", fields),
                        "canceled" | "timed_out" => log.warn("http.request.completed", fields),
                        _ => log.info("http.request.completed", fields),
                    }
                }),
            ],
        );
        let mut guard = Guard {
            active,
            started,
            info: info.clone(),
            status: None,
            termination: Termination::PeerClosed,
        };
        let values: Vec<_> = request
            .headers()
            .get_all("x-correlation-id")
            .iter()
            .collect();
        let incoming = inspect_incoming(IncomingHints {
            correlation: if values.is_empty() {
                None
            } else if values.len() == 1 {
                Some(values[0].to_str().unwrap_or(""))
            } else {
                Some("")
            },
            causation: None,
        });
        let set_failure = |f: Failure| {
            info.lock().unwrap().failure = Some(RequestFailure::Known(f));
        };
        let failed = || info.lock().unwrap().failure.is_some();
        let mut protocol = None;
        if template_route.is_none() {
            set_failure(
                Failure::new(Kind::NotFound, "route not found").with_type("http.not_found"),
            );
        } else if route.is_none() {
            protocol = Some(405);
            set_failure(
                Failure::new(Kind::Invalid, "method not allowed")
                    .with_type("http.method_not_allowed"),
            );
        }
        let (headers, cookie_headers, source) = {
            let header_list = |name: &str| -> Vec<String> {
                request
                    .headers()
                    .get_all(name)
                    .iter()
                    .filter_map(|v| v.to_str().ok().map(str::to_owned))
                    .collect()
            };
            let headers = Headers {
                origin: header_list("origin"),
                content_type: header_list("content-type"),
                csrf_token: header_list("x-csrf-token"),
                subprotocol: header_list("sec-websocket-protocol"),
            };
            let peer = request.extensions().get::<Peer>().map(|p| p.0);
            let source = (self.config.source)(peer, request.headers());
            (headers, header_list("cookie"), source)
        };
        let mut body_bytes = Vec::new();
        if !failed() && !request.body().is_end_stream() {
            let media = headers
                .content_type
                .first()
                .map(String::as_str)
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            if media != "application/json" {
                protocol = Some(415);
                set_failure(
                    Failure::new(Kind::Invalid, "unsupported media type")
                        .with_type("http.unsupported_media_type"),
                );
            } else {
                match tokio::time::timeout(
                    Duration::from_secs(1),
                    to_bytes(request.into_body(), self.config.max_body),
                )
                .await
                {
                    Ok(Ok(bytes)) => body_bytes = bytes.to_vec(),
                    Ok(Err(_)) => {
                        protocol = Some(413);
                        set_failure(
                            Failure::new(Kind::Invalid, "request body too large")
                                .with_type("http.body_too_large"),
                        );
                    }
                    Err(_) => {
                        protocol = Some(408);
                        guard.termination = Termination::Deadline;
                        set_failure(
                            Failure::new(Kind::Invalid, "request body timed out")
                                .with_type("http.request_timeout"),
                        );
                    }
                }
            }
        }
        let mut cookies = BTreeMap::new();
        if !failed() {
            let borrowed: Vec<&str> = cookie_headers.iter().map(String::as_str).collect();
            match parse_cookies(&borrowed) {
                Ok(c) => cookies = c,
                Err(e) => set_failure(e),
            }
        }
        let mut feature = Arc::new(Request {
            method: wanted_method,
            template: path.clone(),
            query,
            body: body_bytes,
            headers,
            cookies,
            source,
            admitted: None,
        });
        // H17: admission runs once, after routing and body, before the scope opens.
        let mut attribution = None;
        let mut operation = template_route.map_or("http.unmatched", |r| r.operation.as_str());
        if !failed() {
            if let Some(admit) = route.and_then(|r| r.admission.clone()) {
                match tokio::time::timeout(self.config.timeout, admit(feature.clone())).await {
                    Ok(Admission::Anonymous) => {}
                    Ok(Admission::Authenticated {
                        initiator,
                        tenant,
                        admitted,
                    }) => match Attribution::new(AttributionSpec {
                        initiator: Some(initiator),
                        on_behalf_of: None,
                        tenant,
                    }) {
                        Ok(a) => {
                            attribution = Some(a);
                            feature = Arc::new(Request {
                                admitted: Some(admitted),
                                ..(*feature).clone()
                            });
                        }
                        Err(e) => set_failure(e),
                    },
                    Ok(Admission::Refused {
                        failure,
                        headers,
                        cookies,
                    }) => {
                        operation = "http.admission";
                        let refusal = Refusal {
                            failure,
                            headers,
                            cookies,
                        };
                        match refusal.validate() {
                            Ok(()) => {
                                info.lock().unwrap().failure =
                                    Some(RequestFailure::Refused(refusal))
                            }
                            Err(e) => set_failure(e),
                        }
                    }
                    Err(_) => {
                        guard.termination = Termination::Deadline;
                        set_failure(
                            Failure::new(Kind::Timeout, "admission timed out")
                                .with_type("http.timeout"),
                        );
                    }
                }
            }
        }
        let opened = (self.config.open)(operation, &incoming, attribution);
        let mut scope = None;
        match opened {
            Ok(s) => {
                let mut log = self.config.log.with_scope(&s);
                if let Some(trace) = observation.trace() {
                    log = log.with_trace(trace)
                };
                let mut i = info.lock().unwrap();
                i.log = log;
                i.scope = Some(s.clone());
                scope = Some(s);
            }
            Err(e) => {
                if !failed() {
                    set_failure(e)
                }
            }
        }
        let mut reply = Response::empty(200);
        if !failed() {
            let context = {
                let i = info.lock().unwrap();
                RequestContext {
                    scope: scope.clone().unwrap(),
                    log: i.log.clone(),
                    request: feature.clone(),
                }
            };
            let handler = route.unwrap().handler.clone();
            let work = observation.instrument(Box::pin(
                CURRENT.scope(context.clone(), async move { handler(context).await }),
            ));
            // Abort the owned task on timeout; Rust drops its future. That is not a rollback.
            use futures_util::FutureExt;
            let work = std::panic::AssertUnwindSafe(work).catch_unwind();
            match tokio::time::timeout(self.config.timeout, work).await {
                Ok(Ok(Ok(v))) => match v.validate() {
                    Ok(()) => reply = v,
                    Err(e) => set_failure(e),
                },
                Ok(Ok(Err(e))) => match &e {
                    RequestFailure::Refused(r) if r.validate().is_err() => {
                        set_failure(invalid_response())
                    }
                    _ => info.lock().unwrap().failure = Some(e),
                },
                Ok(Err(_)) => info.lock().unwrap().failure = Some(RequestFailure::Unknown),
                Err(_) => {
                    guard.termination = Termination::Deadline;
                    set_failure(
                        Failure::new(Kind::Timeout, "request timed out").with_type("http.timeout"),
                    );
                }
            }
        }
        let mut status = reply.status;
        let mut content_type = "application/json";
        let mut value = reply.body.take();
        {
            let info = info.lock().unwrap();
            if let Some(e) = &info.failure {
                let public = public_info(e.classification());
                let mut problem = problem_of(Some(&public)).unwrap();
                if let Some(code) = protocol {
                    problem.status = code;
                    problem.title = match code {
                        405 => "Method Not Allowed",
                        408 => "Request Timeout",
                        413 => "Payload Too Large",
                        _ => "Unsupported Media Type",
                    }
                    .into();
                }
                status = problem.status;
                content_type = "application/problem+json";
                let mut doc = problem.json();
                if let Some(scope) = &scope {
                    let s = scope.snapshot();
                    doc["request_id"] = s.scope_id.to_string().into();
                    doc["correlation_id"] = s.work.correlation_id.to_string().into();
                }
                value = Some(doc);
                reply.headers.clear();
                reply.cookies.clear();
                if let RequestFailure::Refused(r) = e {
                    reply.headers = r.headers.clone();
                    reply.cookies = r.cookies.clone();
                }
            }
        }
        guard.status = if head || value.is_none() {
            Some(status)
        } else {
            None
        };
        if guard.termination != Termination::Deadline {
            guard.termination = Termination::ResponseCompleted;
        }
        let body = match (&value, head) {
            (Some(v), false) => Body::from(v.to_string()),
            _ => Body::empty(),
        };
        let mut response = HttpResponse::new(Body::new(ObservedBody {
            status,
            inner: body,
            guard,
        }));
        *response.status_mut() = ::axum::http::StatusCode::from_u16(status).unwrap();
        if value.is_some() {
            response
                .headers_mut()
                .insert("content-type", HeaderValue::from_static(content_type));
        }
        if protocol == Some(405) {
            if let Ok(v) = HeaderValue::from_str(&allow_set(&registered)) {
                response.headers_mut().insert("allow", v);
            }
        }
        for (name, v) in reply.headers.iter() {
            let lower = name.to_ascii_lowercase();
            if ALLOWED_HEADERS.contains(&lower.as_str()) {
                if let (Ok(n), Ok(v)) = (
                    ::axum::http::HeaderName::from_bytes(lower.as_bytes()),
                    HeaderValue::from_str(v),
                ) {
                    response.headers_mut().append(n, v);
                }
            }
        }
        for c in &reply.cookies {
            if let Ok(v) = c
                .serialize()
                .and_then(|s| HeaderValue::from_str(&s).map_err(|_| invalid_cookie()))
            {
                response.headers_mut().append("set-cookie", v);
            }
        }
        if let Some(scope) = scope {
            let s = scope.snapshot();
            response
                .headers_mut()
                .insert("x-request-id", s.scope_id.to_string().parse().unwrap());
            response.headers_mut().insert(
                "x-correlation-id",
                s.work.correlation_id.to_string().parse().unwrap(),
            );
        }
        response
    }
}
