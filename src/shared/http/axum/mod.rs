//! Bounded JSON transport. The body owns finalization after handoff to Hyper.
use super::{Active, Completion, CompletionFacts, Termination, normalize_method, problem_of};
use crate::shared::{
    errors::{Classified, Failure, Kind, public_info},
    logger::{Fields, Logger},
    provenance::{IncomingHints, IncomingResult, Scope, inspect_incoming},
    telemetry::TraceRef,
};
use ::axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request, Response},
};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};
pub enum RequestFailure {
    Known(Failure),
    Unknown,
}
pub type Work = Pin<Box<dyn Future<Output = Result<serde_json::Value, RequestFailure>> + Send>>;
#[derive(Clone)]
pub struct RequestContext {
    pub scope: Scope,
    pub log: Logger,
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
    pub operation: String,
    pub handler: Arc<dyn Fn(RequestContext) -> Work + Send + Sync>,
}
pub type Open = Arc<dyn Fn(&str, &IncomingResult) -> Result<Scope, Failure> + Send + Sync>;
pub struct Config {
    pub routes: Vec<Route>,
    pub observer: Arc<dyn Observer>,
    pub log: Logger,
    pub open: Open,
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
        let kind = info.failure.as_ref().map(|e| match e {
            RequestFailure::Known(e) => e.classification().map_or(Kind::Internal, |c| c.kind),
            RequestFailure::Unknown => Kind::Internal,
        });
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
        let mut paths = std::collections::BTreeSet::new();
        for r in &config.routes {
            if r.path.is_empty() || r.operation.is_empty() || !paths.insert(&r.path) {
                return Err(Failure::new(Kind::Invalid, "invalid route"));
            }
        }
        Ok(Self { config })
    }
    pub async fn handle(self: Arc<Self>, request: Request<Body>) -> Response<Body> {
        let started = Instant::now();
        let method = normalize_method(request.method().as_str()).to_owned();
        let observation = self
            .config
            .observer
            .start(&method, "http", request.headers());
        let route = self
            .config
            .routes
            .iter()
            .find(|r| r.path == request.uri().path());
        let info = Arc::new(Mutex::new(Info {
            route: route.map(|r| r.path.clone()),
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
        let opened =
            (self.config.open)(route.map_or("http.unmatched", |r| &r.operation), &incoming);
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
            Err(e) => info.lock().unwrap().failure = Some(RequestFailure::Known(e)),
        }
        let head = request.method() == "HEAD";
        let allowed = request.method() == "GET" || head;
        let mut protocol = None;
        if info.lock().unwrap().failure.is_none() {
            if route.is_none() {
                info.lock().unwrap().failure = Some(RequestFailure::Known(
                    Failure::new(Kind::NotFound, "route not found").with_type("http.not_found"),
                ));
            } else if !allowed {
                protocol = Some(405);
                info.lock().unwrap().failure = Some(RequestFailure::Known(
                    Failure::new(Kind::Invalid, "method not allowed")
                        .with_type("http.method_not_allowed"),
                ));
            }
        }
        if info.lock().unwrap().failure.is_none() && !request.body().is_end_stream() {
            let media = request
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            if media != "application/json" {
                protocol = Some(415);
                info.lock().unwrap().failure = Some(RequestFailure::Known(
                    Failure::new(Kind::Invalid, "unsupported media type")
                        .with_type("http.unsupported_media_type"),
                ));
            } else {
                match tokio::time::timeout(
                    Duration::from_secs(1),
                    to_bytes(request.into_body(), self.config.max_body),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => {
                        protocol = Some(413);
                        info.lock().unwrap().failure = Some(RequestFailure::Known(
                            Failure::new(Kind::Invalid, "request body too large")
                                .with_type("http.body_too_large"),
                        ));
                    }
                    Err(_) => {
                        protocol = Some(408);
                        guard.termination = Termination::Deadline;
                        info.lock().unwrap().failure = Some(RequestFailure::Known(
                            Failure::new(Kind::Invalid, "request body timed out")
                                .with_type("http.request_timeout"),
                        ));
                    }
                }
            }
        }
        let mut value = serde_json::json!({"ok":true});
        if info.lock().unwrap().failure.is_none() {
            let context = {
                let i = info.lock().unwrap();
                RequestContext {
                    scope: scope.clone().unwrap(),
                    log: i.log.clone(),
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
                Ok(Ok(Ok(v))) => value = v,
                Ok(Ok(Err(e))) => info.lock().unwrap().failure = Some(e),
                Ok(Err(_)) => info.lock().unwrap().failure = Some(RequestFailure::Unknown),
                Err(_) => {
                    guard.termination = Termination::Deadline;
                    info.lock().unwrap().failure = Some(RequestFailure::Known(
                        Failure::new(Kind::Timeout, "request timed out").with_type("http.timeout"),
                    ));
                }
            }
        }
        let mut status = 200;
        let mut content_type = "application/json";
        {
            let info = info.lock().unwrap();
            if let Some(e) = &info.failure {
                let public = public_info(match e {
                    RequestFailure::Known(e) => e.classification(),
                    RequestFailure::Unknown => None,
                });
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
                value = problem.json();
                if let Some(scope) = &scope {
                    let s = scope.snapshot();
                    value["request_id"] = s.scope_id.to_string().into();
                    value["correlation_id"] = s.work.correlation_id.to_string().into();
                }
            }
        }
        guard.status = if head { Some(status) } else { None };
        if guard.termination != Termination::Deadline {
            guard.termination = Termination::ResponseCompleted;
        }
        let body = if head {
            Body::empty()
        } else {
            Body::from(value.to_string())
        };
        let mut response = Response::new(Body::new(ObservedBody {
            status,
            inner: body,
            guard,
        }));
        *response.status_mut() = ::axum::http::StatusCode::from_u16(status).unwrap();
        response
            .headers_mut()
            .insert("content-type", HeaderValue::from_static(content_type));
        if protocol == Some(405) {
            response
                .headers_mut()
                .insert("allow", HeaderValue::from_static("GET, HEAD"));
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
