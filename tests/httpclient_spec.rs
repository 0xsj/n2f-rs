use n2f_rs::shared::{
    errors::Classified,
    httpclient::{Client, Config, Request},
    telemetry::TraceRef,
};
use std::time::Duration;
#[tokio::test]
async fn real_bounded_attempts() {
    let app = axum::Router::new().route(
        "/{*path}",
        axum::routing::any(
            |request: axum::http::Request<axum::body::Body>| async move {
                match request.uri().path() {
                    "/status" => axum::http::Response::builder()
                        .status(409)
                        .body(axum::body::Body::from("{\"ok\":false}"))
                        .unwrap(),
                    "/redirect" => axum::http::Response::builder()
                        .status(302)
                        .header("location", "/status")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                    "/large" => axum::http::Response::new(axum::body::Body::from("x".repeat(100))),
                    "/slow" => {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        axum::http::Response::new(axum::body::Body::empty())
                    }
                    "/trace" => axum::http::Response::new(axum::body::Body::from(
                        request
                            .headers()
                            .get("traceparent")
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_owned(),
                    )),
                    _ => axum::http::Response::new(axum::body::Body::empty()),
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let c = Client::new(Config {
        origin: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_millis(50),
        max_request: 64,
        max_response: 64,
        bearer: None,
    })
    .unwrap();
    let request = |path: &str| Request {
        method: "GET".into(),
        path: path.into(),
        body: vec![],
        trace: None,
    };
    for (path, status) in [("/status", 409), ("/redirect", 302)] {
        assert_eq!(c.execute(request(path)).await.unwrap().status, status);
    }
    for (path, code) in [
        ("/large", "httpclient.response_too_large"),
        ("/slow", "httpclient.timeout"),
        ("//evil/x", "httpclient.request"),
        ("/\\evil", "httpclient.request"),
    ] {
        assert_eq!(
            c.execute(request(path))
                .await
                .unwrap_err()
                .classification()
                .unwrap()
                .error_type,
            Some(code)
        );
    }
    let mut r = request("/trace");
    r.trace = Some(TraceRef::parse(&"1".repeat(32), &"2".repeat(16), true).unwrap());
    assert_eq!(
        String::from_utf8(c.execute(r).await.unwrap().body).unwrap(),
        format!("00-{}-{}-01", "1".repeat(32), "2".repeat(16))
    );
    // Caller cancellation drops the owned future; it cannot produce a result after drop.
    assert!(
        tokio::time::timeout(Duration::from_millis(1), c.execute(request("/slow")))
            .await
            .is_err()
    );
    c.close();
    assert_eq!(
        c.execute(request("/status"))
            .await
            .unwrap_err()
            .classification()
            .unwrap()
            .error_type,
        Some("httpclient.closed")
    );
    server.abort();
    let _ = server.await;
}
