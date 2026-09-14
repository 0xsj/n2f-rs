//! H15–H19: feature routes with methods, bodies, cookies, response values and admission.
use axum::{Router, body::Body, http::Request as HttpRequest};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use n2f_rs::shared::{
    clock::SystemClock,
    errors::{Classified, Failure, Kind},
    http::axum::{
        Admission, Config, Cookie, Noop, Peer, Refusal, Request, RequestContext, RequestFailure,
        Response, Route, SameSite, Server, allow_set, parse_cookies,
    },
    id::V7,
    logger::{Config as LogConfig, Resource, Runtime},
    provenance as p,
};
use std::{
    io::Write,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
fn kind_type(e: &Failure) -> (String, String) {
    let c = e.classification().unwrap();
    (c.kind.as_str().into(), c.error_type.unwrap_or("").into())
}
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
fn logger(sink: Sink) -> Runtime {
    let mut c = LogConfig::new(
        Resource {
            name: "test".into(),
            ..Default::default()
        },
        Arc::new(SystemTime::now),
        Box::new(sink),
    );
    c.format = "json".into();
    Runtime::new(c).unwrap()
}
#[test]
fn h15_method_matching_and_allow_set() {
    assert_eq!(allow_set(&["POST".into(), "GET".into()]), "GET, HEAD, POST");
    assert_eq!(allow_set(&["DELETE".into()]), "DELETE");
    let ok = |method: &str| Route {
        path: "/x".into(),
        method: method.into(),
        operation: "op".into(),
        admission: None,
        handler: Arc::new(|_| Box::pin(async { Ok(Response::json(serde_json::json!({}))) })),
    };
    let config = |routes| Config {
        routes,
        observer: Arc::new(Noop),
        log: logger(Sink::default()).log.clone(),
        open: Arc::new(|_, _, _| Err(Failure::new(Kind::Internal, "unused"))),
        source: Arc::new(|_, _| String::new()),
        timeout: Duration::from_secs(1),
        max_body: 1024,
    };
    assert!(Server::new(config(vec![ok("GET"), ok("POST")])).is_ok());
    assert!(Server::new(config(vec![ok("GET"), ok("GET")])).is_err());
    assert!(Server::new(config(vec![ok("HEAD")])).is_err());
    assert!(Server::new(config(vec![ok("FETCH")])).is_err());
}
#[test]
fn h15_cookie_parsing_keeps_duplicates_and_refuses_malformed() {
    let c = parse_cookies(&["a=1; b=2", "a=3"]).unwrap();
    assert_eq!(c["a"], vec!["1", "3"]);
    assert_eq!(c["b"], vec!["2"]);
    assert!(parse_cookies(&["a=1;   b=2 "]).is_ok());
    assert!(parse_cookies(&[]).unwrap().is_empty());
    for bad in [
        "novalue",
        "=1",
        "a=1; ;",
        "a=\"quoted\"",
        "a=x y",
        "a\x01=1",
        "a=1,2",
    ] {
        let e = parse_cookies(&[bad]).unwrap_err();
        assert_eq!(
            kind_type(&e),
            ("invalid".into(), "http.invalid_cookie".into()),
            "{bad}"
        );
    }
}
#[test]
fn h16_response_allowlist_and_host_cookie_refusals() {
    let ok = Response::json(serde_json::json!({"a":1}))
        .with_header("Cache-Control", "no-store")
        .with_header("Retry-After", "3");
    assert!(ok.validate().is_ok());
    for (name, value) in [
        ("X-Powered-By", "n2f"),
        ("Content-Type", "text/plain"),
        ("Set-Cookie", "a=b"),
    ] {
        let e = Response::json(serde_json::json!({}))
            .with_header(name, value)
            .validate()
            .unwrap_err();
        assert_eq!(kind_type(&e).1, "http.invalid_response", "{name}");
    }
    assert!(Response::empty(204).validate().is_ok());
    assert!(
        Response::json(serde_json::json!({}))
            .with_status(204)
            .validate()
            .is_err()
    );
    for status in [100, 301, 400, 500] {
        assert!(Response::empty(status).validate().is_err(), "{status}");
    }
    let host = Cookie {
        name: "__Host-n2f_session".into(),
        value: "abc".into(),
        path: Some("/".into()),
        max_age: Some(3600),
        expires: None,
        secure: true,
        http_only: true,
        same_site: Some(SameSite::Lax),
    };
    assert_eq!(
        host.serialize().unwrap(),
        "__Host-n2f_session=abc; Path=/; Max-Age=3600; Secure; HttpOnly; SameSite=Lax"
    );
    let insecure = Cookie {
        secure: false,
        ..host.clone()
    };
    let rooted = Cookie {
        path: Some("/api".into()),
        ..host.clone()
    };
    let control = Cookie {
        value: "a\rb".into(),
        ..host.clone()
    };
    let bad_name = Cookie {
        name: "se ssion".into(),
        ..host.clone()
    };
    for (c, why) in [
        (insecure, "insecure"),
        (rooted, "path"),
        (control, "control"),
        (bad_name, "name"),
    ] {
        assert_eq!(
            kind_type(&c.serialize().unwrap_err()).1,
            "http.invalid_cookie",
            "{why}"
        );
    }
    let dev = Cookie {
        name: "n2f_session_dev".into(),
        secure: false,
        path: None,
        ..host
    };
    assert!(dev.serialize().is_ok());
}
struct Harness {
    address: SocketAddr,
    sink: Sink,
    runtime: Runtime,
    order: Arc<Mutex<Vec<String>>>,
}
async fn start(routes: Vec<Route>, order: Arc<Mutex<Vec<String>>>) -> Harness {
    let sink = Sink::default();
    let runtime = logger(sink.clone());
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
    let seen = order.clone();
    let open = Arc::new(
        move |name: &str, incoming: &p::IncomingResult, attribution: Option<p::Attribution>| {
            seen.lock().unwrap().push(format!("open:{name}"));
            factory.lock().unwrap().enter(
                p::RootSpec {
                    work_id: None,
                    origin: p::Origin::Request,
                    operation: p::Operation::new(name.into())?,
                    attribution: attribution.unwrap_or_else(|| anonymous.clone()),
                    executor: executor.clone(),
                },
                incoming,
            )
        },
    );
    let server = Arc::new(
        Server::new(Config {
            routes,
            observer: Arc::new(Noop),
            log: runtime.log.clone(),
            open,
            source: Arc::new(|peer: Option<SocketAddr>, _| {
                peer.map(|a| a.ip().to_string()).unwrap_or_default()
            }),
            timeout: Duration::from_secs(2),
            max_body: 64,
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
        runtime,
        order,
    }
}
fn route(
    method: &str,
    path: &str,
    admission: Option<n2f_rs::shared::http::axum::Admit>,
    handler: impl Fn(RequestContext) -> n2f_rs::shared::http::axum::Work + Send + Sync + 'static,
) -> Route {
    Route {
        path: path.into(),
        method: method.into(),
        operation: format!("op.{}", path.trim_start_matches('/')),
        admission,
        handler: Arc::new(handler),
    }
}
fn echo(context: RequestContext) -> n2f_rs::shared::http::axum::Work {
    Box::pin(async move {
        let r: &Request = &context.request;
        let initiator = context
            .scope
            .snapshot()
            .work
            .attribution
            .snapshot()
            .initiator
            .map(|a| format!("{:?}:{}", a.kind(), a.identity()));
        Ok(Response::json(serde_json::json!({
            "method": r.method, "template": r.template, "body": String::from_utf8_lossy(&r.body),
            "origin": r.headers.origin, "csrf": r.headers.csrf_token, "cookies": r.cookies, "source": r.source,
            "admitted": r.admitted.as_ref().and_then(|a| a.downcast_ref::<String>().cloned()), "initiator": initiator,
        })))
    })
}
#[tokio::test(flavor = "multi_thread")]
async fn h15_h16_post_body_head_and_set_cookie_over_the_wire() {
    let h = start(
        vec![
            route("POST", "/things", None, echo),
            route("GET", "/things", None, echo),
            route("POST", "/login", None, |_| {
                Box::pin(async {
                    Ok(Response::empty(204)
                        .with_header("Cache-Control", "no-store")
                        .with_cookie(Cookie {
                            name: "n2f_session_dev".into(),
                            value: "tok".into(),
                            path: Some("/".into()),
                            max_age: Some(60),
                            expires: None,
                            secure: false,
                            http_only: true,
                            same_site: Some(SameSite::Lax),
                        }))
                })
            }),
            route("POST", "/broken", None, |_| {
                Box::pin(async {
                    Ok(Response::json(serde_json::json!({})).with_header("X-Leak", "1"))
                })
            }),
        ],
        Arc::new(Mutex::new(Vec::new())),
    )
    .await;
    let c = reqwest::Client::new();
    let base = format!("http://{}", h.address);
    let r = c
        .post(format!("{base}/things"))
        .header("content-type", "application/json")
        .header("origin", "https://app.example")
        .header("cookie", "a=1; b=2")
        .header("cookie", "a=3")
        .header("x-csrf-token", "t")
        .body(r#"{"n":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
    assert_eq!(v["method"], "POST");
    assert_eq!(v["template"], "/things");
    assert_eq!(v["body"], r#"{"n":1}"#);
    assert_eq!(v["origin"][0], "https://app.example");
    assert_eq!(v["cookies"]["a"], serde_json::json!(["1", "3"]));
    assert_eq!(v["source"], "127.0.0.1");
    assert_eq!(v["initiator"], "Anonymous:");
    let r = c.head(format!("{base}/things")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "");
    let r = c.delete(format!("{base}/things")).send().await.unwrap();
    assert_eq!(r.status(), 405);
    assert_eq!(r.headers().get("allow").unwrap(), "GET, HEAD, POST");
    let r = c
        .post(format!("{base}/things"))
        .header("content-type", "application/json")
        .body("x".repeat(65))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
    let r = c
        .post(format!("{base}/things"))
        .header("content-type", "text/plain")
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 415);
    let r = c.post(format!("{base}/login")).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(
        r.headers().get("set-cookie").unwrap(),
        "n2f_session_dev=tok; Path=/; Max-Age=60; HttpOnly; SameSite=Lax"
    );
    assert_eq!(r.headers().get("cache-control").unwrap(), "no-store");
    assert!(r.headers().get("x-request-id").is_some());
    let r = c.post(format!("{base}/broken")).send().await.unwrap();
    assert_eq!(r.status(), 500);
    assert!(r.headers().get("x-leak").is_none());
    let r = c.get(format!("{base}/missing")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    h.runtime.close(Duration::from_secs(1)).unwrap();
    let logs = String::from_utf8(h.sink.0.lock().unwrap().clone()).unwrap();
    assert!(
        !logs.contains("tok") && !logs.contains("a=1"),
        "cookie values leaked"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn h17_h18_admission_before_open_once_with_attribution_and_refusal() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let admit: n2f_rs::shared::http::axum::Admit = Arc::new(move |request: Arc<Request>| {
        let seen = seen.clone();
        Box::pin(async move {
            seen.lock().unwrap().push("admit".into());
            match request
                .cookies
                .get("session")
                .and_then(|v| v.first())
                .map(String::as_str)
            {
                Some("good") => Admission::Authenticated {
                    initiator: p::Actor::new(p::ActorKind::User, "user-1".into()).unwrap(),
                    tenant: None,
                    admitted: Arc::new("principal-1".to_string()),
                },
                Some("limited") => Admission::Refused {
                    failure: Failure::new(Kind::RateLimited, "slow down")
                        .with_type("identity.auth_rate_limited"),
                    headers: vec![("Retry-After".into(), "7".into())],
                    cookies: vec![],
                },
                Some(_) => Admission::Refused {
                    failure: Failure::new(Kind::Unauthenticated, "no")
                        .with_type("identity.session_rejected"),
                    headers: vec![],
                    cookies: vec![],
                },
                None => Admission::Anonymous,
            }
        })
    });
    let h = start(
        vec![
            route("GET", "/me", Some(admit), echo),
            route("GET", "/open", None, echo),
        ],
        calls.clone(),
    )
    .await;
    let c = reqwest::Client::new();
    let base = format!("http://{}", h.address);
    let (a, b) = tokio::join!(
        c.get(format!("{base}/me"))
            .header("cookie", "session=good")
            .send(),
        c.get(format!("{base}/me")).send()
    );
    let a: serde_json::Value = serde_json::from_str(&a.unwrap().text().await.unwrap()).unwrap();
    let b: serde_json::Value = serde_json::from_str(&b.unwrap().text().await.unwrap()).unwrap();
    assert_eq!(a["initiator"], "User:user-1");
    assert_eq!(a["admitted"], "principal-1");
    assert_eq!(b["initiator"], "Anonymous:");
    assert!(b["admitted"].is_null());
    let r = c
        .get(format!("{base}/me"))
        .header("cookie", "session=bad")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = c
        .get(format!("{base}/me"))
        .header("cookie", "session=limited")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers().get("retry-after").unwrap(), "7");
    let r = c
        .post(format!("{base}/me"))
        .header("cookie", "session=good")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
    let r = c
        .get(format!("{base}/nowhere"))
        .header("cookie", "session=good")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let order = h.order.lock().unwrap().clone();
    assert_eq!(
        order.iter().filter(|s| *s == "admit").count(),
        4,
        "admission once per matched request, never for 404/405: {order:?}"
    );
    assert_eq!(
        order.iter().filter(|s| *s == "open:http.admission").count(),
        2
    );
    let first_open = order.iter().position(|s| s.starts_with("open:")).unwrap();
    assert!(
        order[..first_open].contains(&"admit".to_string()),
        "{order:?}"
    );
    h.runtime.close(Duration::from_secs(1)).unwrap();
    let logs = String::from_utf8(h.sink.0.lock().unwrap().clone()).unwrap();
    let refused = logs
        .lines()
        .filter(|l| l.contains("http.request.completed") && l.contains("\"refused\""))
        .count();
    assert!(refused >= 4, "refusals observed: {refused}");
    assert!(logs.contains("\"/me\""));
    let _ = RequestFailure::Unknown;
}
#[tokio::test(flavor = "multi_thread")]
async fn h17_admission_runs_before_open() {
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = order.clone();
    let admit: n2f_rs::shared::http::axum::Admit = Arc::new(move |_| {
        let seen = seen.clone();
        Box::pin(async move {
            seen.lock().unwrap().push("admit".into());
            Admission::Anonymous
        })
    });
    let h = start(vec![route("GET", "/a", Some(admit), echo)], order.clone()).await;
    let r = reqwest::get(format!("http://{}/a", h.address))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(order.lock().unwrap().clone(), vec!["admit", "open:op.a"]);
}

#[test]
fn h16_refusal_value_keeps_classification_and_enforces_the_allowlist() {
    let failure =
        || Failure::new(Kind::Unauthenticated, "no").with_type("identity.session_rejected");
    let cleared = Cookie {
        name: "n2f_session_dev".into(),
        value: String::new(),
        path: Some("/".into()),
        max_age: Some(0),
        expires: None,
        secure: false,
        http_only: true,
        same_site: Some(SameSite::Lax),
    };
    let ok = Refusal::new(failure())
        .with_header("Cache-Control", "no-store")
        .with_header("Retry-After", "1")
        .with_cookie(cleared.clone());
    assert!(ok.validate().is_ok());
    assert_eq!(
        cleared.serialize().unwrap(),
        "n2f_session_dev=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
    );
    assert_eq!(
        kind_type(&ok.failure),
        (
            "unauthenticated".to_string(),
            "identity.session_rejected".to_string()
        )
    );
    let unlisted = Refusal::new(failure()).with_header("X-Powered-By", "n2f");
    assert_eq!(
        kind_type(&unlisted.validate().unwrap_err()),
        ("internal".to_string(), "http.invalid_response".to_string())
    );
    let insecure_host = Refusal::new(failure()).with_cookie(Cookie {
        name: "__Host-n2f".into(),
        secure: false,
        ..cleared.clone()
    });
    assert!(insecure_host.validate().is_err());
    let _ = RequestFailure::Refused(ok);
}
#[tokio::test(flavor = "multi_thread")]
async fn h16_h17_refusals_carry_headers_and_cookies_over_the_wire() {
    let cleared = || Cookie {
        name: "n2f_session_dev".into(),
        value: String::new(),
        path: Some("/".into()),
        max_age: Some(0),
        expires: None,
        secure: false,
        http_only: true,
        same_site: Some(SameSite::Lax),
    };
    let admit: n2f_rs::shared::http::axum::Admit = Arc::new(move |_| {
        Box::pin(async move {
            Admission::Refused {
                failure: Failure::new(Kind::Unauthenticated, "no")
                    .with_type("identity.session_rejected"),
                headers: vec![("Cache-Control".into(), "no-store".into())],
                cookies: vec![cleared()],
            }
        })
    });
    let h = start(
        vec![
            route("GET", "/admitted", Some(admit), echo),
            route("GET", "/expired", None, move |_| {
                Box::pin(async move {
                    Err(RequestFailure::Refused(
                        Refusal::new(
                            Failure::new(Kind::Unauthenticated, "expired")
                                .with_type("identity.session_rejected"),
                        )
                        .with_header("Cache-Control", "no-store")
                        .with_cookie(cleared()),
                    ))
                })
            }),
            route("GET", "/limited", None, |_| {
                Box::pin(async {
                    Err(RequestFailure::Refused(
                        Refusal::new(
                            Failure::new(Kind::RateLimited, "slow down")
                                .with_type("identity.auth_rate_limited"),
                        )
                        .with_header("Retry-After", "30"),
                    ))
                })
            }),
            route("GET", "/unlisted", None, |_| {
                Box::pin(async {
                    Err(RequestFailure::Refused(
                        Refusal::new(Failure::new(Kind::Forbidden, "no").with_type("x.no"))
                            .with_header("X-Powered-By", "n2f"),
                    ))
                })
            }),
        ],
        Arc::new(Mutex::new(Vec::new())),
    )
    .await;
    let base = format!("http://{}", h.address);
    for path in ["/admitted", "/expired"] {
        let r = reqwest::get(format!("{base}{path}")).await.unwrap();
        assert_eq!(r.status(), 401, "{path}");
        assert_eq!(
            r.headers().get("cache-control").unwrap(),
            "no-store",
            "{path}"
        );
        let set: Vec<String> = r
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            set,
            vec!["n2f_session_dev=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax".to_string()],
            "{path}"
        );
        assert_eq!(
            r.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        let doc: serde_json::Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
        assert_eq!(doc["status"], 401);
        assert_eq!(doc["code"], "identity.session_rejected");
        assert_eq!(doc["kind"], "unauthenticated");
    }
    let r = reqwest::get(format!("{base}/limited")).await.unwrap();
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers().get("retry-after").unwrap(), "30");
    assert!(r.headers().get("set-cookie").is_none());
    let doc: serde_json::Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
    assert_eq!(doc["code"], "identity.auth_rate_limited");
    // An unlisted header on a refusal is a handler error, never a silent drop.
    let r = reqwest::get(format!("{base}/unlisted")).await.unwrap();
    assert_eq!(r.status(), 500);
    assert!(r.headers().get("x-powered-by").is_none());
    let doc: serde_json::Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
    assert_eq!(doc["kind"], "internal");
}
