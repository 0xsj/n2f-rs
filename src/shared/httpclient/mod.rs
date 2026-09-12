//! One bounded attempt to a configured origin. No implicit redirects or retries.
pub mod otel;
use crate::shared::{
    errors::{Failure, Kind},
    secret::SecretString,
    telemetry::TraceRef,
};
use futures_util::StreamExt;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
pub struct Config {
    pub origin: String,
    pub timeout: Duration,
    pub max_request: usize,
    pub max_response: usize,
    pub bearer: Option<SecretString>,
}
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub trace: Option<TraceRef>,
}
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
}
pub struct Client {
    config: Config,
    base: url::Url,
    native: reqwest::Client,
    closed: AtomicBool,
    slots: tokio::sync::Semaphore,
    stop: tokio::sync::watch::Sender<bool>,
}
fn fail(kind: Kind, code: &str) -> Failure {
    Failure::new(kind, "outbound HTTP operation failed").with_type(format!("httpclient.{code}"))
}
impl Client {
    pub fn new(c: Config) -> Result<Self, Failure> {
        let u = url::Url::parse(&c.origin).map_err(|_| fail(Kind::Invalid, "config"))?;
        if !matches!(u.scheme(), "http" | "https")
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || !matches!(u.path(), "" | "/")
            || c.timeout < Duration::from_millis(1)
            || c.timeout > Duration::from_secs(30)
            || !(1..=1048576).contains(&c.max_request)
            || !(1..=1048576).contains(&c.max_response)
            || c.bearer
                .as_ref()
                .is_some_and(|s| s.reveal().contains(['\r', '\n']))
        {
            return Err(fail(Kind::Invalid, "config"));
        }
        let native = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(c.timeout)
            .pool_max_idle_per_host(0)
            .http1_only()
            .build()
            .map_err(|_| fail(Kind::Invalid, "config"))?;
        Ok(Self {
            config: c,
            base: u,
            native,
            closed: AtomicBool::new(false),
            slots: tokio::sync::Semaphore::new(64),
            stop: tokio::sync::watch::channel(false).0,
        })
    }
    pub async fn execute(&self, input: Request) -> Result<Response, Failure> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(fail(Kind::Unavailable, "closed"));
        }
        let _permit = self
            .slots
            .try_acquire()
            .map_err(|_| fail(Kind::RateLimited, "busy"))?;
        if !matches!(
            input.method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
        ) || !input.path.starts_with('/')
            || input.path.starts_with("//")
            || input.path.contains(['\\', '\r', '\n', '#'])
            || input.body.len() > self.config.max_request
        {
            return Err(fail(Kind::Invalid, "request"));
        }
        let target = self
            .base
            .join(&input.path)
            .map_err(|_| fail(Kind::Invalid, "request"))?;
        if target.origin() != self.base.origin() {
            return Err(fail(Kind::Invalid, "request"));
        }
        let method = reqwest::Method::from_bytes(input.method.as_bytes())
            .map_err(|_| fail(Kind::Invalid, "request"))?;
        let mut request = self
            .native
            .request(method, target)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(input.body);
        if let Some(bearer) = &self.config.bearer {
            if !bearer.reveal().is_empty() {
                request = request.bearer_auth(bearer.reveal());
            }
        }
        if let Some(t) = input.trace {
            let t = t.snapshot();
            request = request.header(
                "traceparent",
                format!(
                    "00-{}-{}-{}",
                    t.trace_id,
                    t.span_id,
                    if t.sampled { "01" } else { "00" }
                ),
            );
        }
        let mut stop = self.stop.subscribe();
        let attempt = async {
            let response = request.send().await.map_err(network)?;
            let status = response.status().as_u16();
            if !(200..=599).contains(&status) {
                return Err(fail(Kind::Unavailable, "protocol"));
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(network)?;
                if body.len() + chunk.len() > self.config.max_response {
                    return Err(fail(Kind::Unavailable, "response_too_large"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Response {
                status,
                body,
                content_type,
            })
        };
        if self.closed.load(Ordering::SeqCst) {
            return Err(fail(Kind::Unavailable, "closed"));
        }
        tokio::select! {result=tokio::time::timeout(self.config.timeout,attempt)=>result.map_err(|_|fail(Kind::Timeout,"timeout"))?,_=stop.changed()=>Err(fail(Kind::Canceled,"canceled"))}
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop.send_replace(true);
    }
}
fn network(e: reqwest::Error) -> Failure {
    if e.is_timeout() {
        fail(Kind::Timeout, "timeout")
    } else {
        fail(Kind::Unavailable, "network")
    }
}
