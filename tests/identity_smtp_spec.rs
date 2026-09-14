//! M10–M16: identity SMTP mail delivery against an in-test fake SMTP server.
use base64::Engine;
use n2f_rs::{
    domains::identity::{
        app::command::MailDelivery,
        domain::email::Email,
        infra::smtp::{Config as MailConfig, Mailer, Security},
    },
    shared::{
        errors::{Classified, Failure},
        logger::{Config as LogConfig, Resource, Runtime},
        secret::SecretString,
    },
};
use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

const TOKEN: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
const EXPIRES: i64 = 1789000000123;
const EXPIRES_ISO: &str = "2026-09-10T00:26:40.123Z";

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
fn log_text(runtime: Runtime, sink: &Sink) -> String {
    runtime.close(Duration::from_secs(2)).unwrap();
    String::from_utf8(sink.0.lock().unwrap().clone()).unwrap()
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Accept,
    RejectData,
    Stall,
}
#[derive(Clone, Default, Debug)]
struct Captured {
    mail_from: String,
    rcpt: Vec<String>,
    data: String,
}
type Store = Arc<Mutex<Vec<Captured>>>;
async fn fake_server(mode: Mode) -> (u16, Store) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let store: Store = Arc::default();
    let shared = store.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let store = shared.clone();
            tokio::spawn(session(socket, mode, store));
        }
    });
    (port, store)
}
async fn session(socket: TcpStream, mode: Mode, store: Store) {
    if mode == Mode::Stall {
        tokio::time::sleep(Duration::from_secs(30)).await;
        return;
    }
    let (read, mut write) = socket.into_split();
    let mut reader = BufReader::new(read);
    let _ = write.write_all(b"220 fake ESMTP\r\n").await;
    let mut captured = Captured::default();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let upper = line.to_ascii_uppercase();
        let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            b"250 fake\r\n"
        } else if upper.starts_with("MAIL FROM") {
            captured.mail_from = line.trim_end().into();
            b"250 ok\r\n"
        } else if upper.starts_with("RCPT TO") {
            captured.rcpt.push(line.trim_end().into());
            b"250 ok\r\n"
        } else if upper.starts_with("DATA") {
            let _ = write.write_all(b"354 go ahead\r\n").await;
            let mut data = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                if line == ".\r\n" {
                    break;
                }
                data.push_str(line.strip_prefix('.').unwrap_or(&line));
            }
            captured.data = data;
            store.lock().unwrap().push(captured.clone());
            if mode == Mode::RejectData {
                b"550 message rejected\r\n"
            } else {
                b"250 queued\r\n"
            }
        } else if upper.starts_with("QUIT") {
            let _ = write.write_all(b"221 bye\r\n").await;
            return;
        } else if upper.starts_with("RSET") || upper.starts_with("NOOP") {
            b"250 ok\r\n"
        } else {
            b"502 unsupported\r\n"
        };
        let _ = write.write_all(reply).await;
    }
}
async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}
fn quoted_printable(body: &str) -> String {
    let joined = body.replace("=\r\n", "");
    let bytes = joined.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&joined[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap()
}
/// Split headers from the decoded plain-text body, whatever transfer encoding was chosen.
fn decode(data: &str) -> (String, String) {
    let (head, body) = data.split_once("\r\n\r\n").expect("header/body separator");
    let lower = head.to_ascii_lowercase();
    let text = if lower.contains("content-transfer-encoding: base64") {
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(compact)
                .unwrap(),
        )
        .unwrap()
    } else if lower.contains("content-transfer-encoding: quoted-printable") {
        quoted_printable(body)
    } else {
        body.to_string()
    };
    (head.to_string(), text)
}
fn config(port: u16) -> MailConfig {
    MailConfig {
        host: "127.0.0.1".into(),
        port,
        security: Security::None,
        username: None,
        password: None,
        from: "No-Reply@n2f.local".into(),
        from_name: "n2f".into(),
        link_origin: "http://127.0.0.1:3000".into(),
        verify_path: "/verify-email".into(),
        reset_path: "/reset-password".into(),
        timeout_ms: 2000,
    }
}
fn recipient() -> Email {
    Email::parse("Ada@Example.com").unwrap()
}
fn token() -> SecretString {
    SecretString::new(TOKEN.into())
}
fn error_type(e: &Failure) -> String {
    e.classification()
        .and_then(|c| c.error_type)
        .unwrap_or("")
        .to_string()
}
fn assert_safe_log(log: &str) {
    assert!(log.contains("identity.mail.attempted"), "{log}");
    assert!(!log.contains(TOKEN), "token leaked: {log}");
    assert!(!log.contains("ada@example.com"), "recipient leaked: {log}");
    assert!(!log.contains("no-reply@n2f.local"), "from leaked: {log}");
    assert!(!log.contains("#token="), "link leaked: {log}");
    assert_eq!(log.matches("identity.mail.attempted").count(), 1, "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn m11_m12_m13_verification_message_is_delivered() {
    let (port, store) = fake_server(Mode::Accept).await;
    let sink = Sink::default();
    let runtime = logger(sink.clone());
    let mailer = Mailer::new(config(port), runtime.log.clone()).unwrap();
    assert!(
        mailer
            .send_verification(&recipient(), &token(), EXPIRES)
            .await
    );
    let captured = store.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    let message = &captured[0];
    assert_eq!(message.rcpt.len(), 1, "{message:?}");
    assert!(message.rcpt[0].contains("<ada@example.com>"), "{message:?}");
    assert!(
        message.mail_from.contains("<no-reply@n2f.local>"),
        "{message:?}"
    );
    let (head, body) = decode(&message.data);
    let head_lower = head.to_ascii_lowercase();
    assert!(
        head.contains("Subject: Verify your email address"),
        "{head}"
    );
    assert!(
        head.lines().any(|l| l.starts_with("From:")
            && l.contains("n2f")
            && l.contains("no-reply@n2f.local")),
        "{head}"
    );
    assert!(
        head.lines()
            .any(|l| l.starts_with("To:") && l.contains("ada@example.com")),
        "{head}"
    );
    assert!(head_lower.contains("content-type: text/plain"), "{head}");
    assert!(
        !head_lower.contains("text/html") && !head_lower.contains("multipart"),
        "{head}"
    );
    let link = format!("http://127.0.0.1:3000/verify-email#token={TOKEN}");
    assert!(body.contains(&link), "{body}");
    assert!(
        !body.contains("?token") && !body.contains(&format!("/{TOKEN}")),
        "{body}"
    );
    assert_eq!(body.matches(TOKEN).count(), 1, "{body}");
    assert!(body.contains(EXPIRES_ISO), "{body}");
    let log = log_text(runtime, &sink);
    assert_safe_log(&log);
    assert!(
        log.contains("\"verification\"") && log.contains("\"delivered\""),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn m11_reset_message_uses_the_reset_link_and_subject() {
    let (port, store) = fake_server(Mode::Accept).await;
    let sink = Sink::default();
    let runtime = logger(sink.clone());
    let mailer = Mailer::new(config(port), runtime.log.clone()).unwrap();
    assert!(mailer.send_reset(&recipient(), &token(), EXPIRES).await);
    let captured = store.lock().unwrap().clone();
    let (head, body) = decode(&captured[0].data);
    assert!(head.contains("Subject: Reset your password"), "{head}");
    assert!(
        body.contains(&format!(
            "http://127.0.0.1:3000/reset-password#token={TOKEN}"
        )),
        "{body}"
    );
    assert!(
        !body.contains("/verify-email") && body.contains(EXPIRES_ISO),
        "{body}"
    );
    let log = log_text(runtime, &sink);
    assert_safe_log(&log);
    assert!(
        log.contains("\"reset\"") && log.contains("\"delivered\""),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn m12_rejection_is_not_delivered() {
    let (port, store) = fake_server(Mode::RejectData).await;
    let sink = Sink::default();
    let runtime = logger(sink.clone());
    let mailer = Mailer::new(config(port), runtime.log.clone()).unwrap();
    assert!(
        !mailer
            .send_verification(&recipient(), &token(), EXPIRES)
            .await
    );
    assert_eq!(store.lock().unwrap().len(), 1);
    let log = log_text(runtime, &sink);
    assert_safe_log(&log);
    assert!(log.contains("\"refused\""), "{log}");
    assert!(
        !log.contains("message rejected"),
        "SMTP reply leaked: {log}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn m12_refused_connection_is_not_delivered() {
    let port = closed_port().await;
    let sink = Sink::default();
    let runtime = logger(sink.clone());
    let mailer = Mailer::new(config(port), runtime.log.clone()).unwrap();
    assert!(!mailer.send_reset(&recipient(), &token(), EXPIRES).await);
    let log = log_text(runtime, &sink);
    assert_safe_log(&log);
    assert!(log.contains("\"failed\""), "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn m12_stall_is_bounded_by_the_timeout() {
    let (port, _) = fake_server(Mode::Stall).await;
    let sink = Sink::default();
    let runtime = logger(sink.clone());
    let mut c = config(port);
    c.timeout_ms = 300;
    let mailer = Mailer::new(c, runtime.log.clone()).unwrap();
    let started = Instant::now();
    assert!(
        !mailer
            .send_verification(&recipient(), &token(), EXPIRES)
            .await
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let log = log_text(runtime, &sink);
    assert_safe_log(&log);
    assert!(log.contains("\"timeout\""), "{log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn m10_configuration_refusals() {
    let sink = Sink::default();
    let runtime = logger(sink);
    let refused = |c: MailConfig, why: &str| match Mailer::new(c, runtime.log.clone()) {
        Ok(_) => panic!("accepted: {why}"),
        Err(e) => assert_eq!(error_type(&e), "identity.mail_configuration", "{why}"),
    };
    let remote = || MailConfig {
        host: "smtp.example.com".into(),
        port: 587,
        security: Security::StartTls,
        username: Some("mailer".into()),
        password: Some(SecretString::new("smtp-password".into())),
        from: "no-reply@example.com".into(),
        from_name: "n2f".into(),
        link_origin: "https://app.example.com".into(),
        verify_path: "/verify-email".into(),
        reset_path: "/reset-password".into(),
        timeout_ms: 5000,
    };
    assert!(
        Mailer::new(remote(), runtime.log.clone()).is_ok(),
        "valid remote starttls"
    );
    let mut tls = remote();
    tls.security = Security::Tls;
    tls.port = 465;
    assert!(
        Mailer::new(tls, runtime.log.clone()).is_ok(),
        "valid remote tls"
    );
    let mut c = remote();
    c.security = Security::None;
    c.username = None;
    c.password = None;
    refused(c, "none on a remote host");
    let mut c = remote();
    c.security = Security::None;
    refused(c, "credentials without TLS on a remote host");
    let mut c = remote();
    c.link_origin = "http://app.example.com".into();
    refused(c, "non-https remote origin");
    let mut c = remote();
    c.link_origin = "https://app.example.com/app".into();
    refused(c, "origin with a path");
    let mut c = remote();
    c.link_origin = "https://app.example.com/".into();
    refused(c, "origin with a trailing slash path");
    let mut c = remote();
    c.link_origin = "https://app.example.com?x=1".into();
    refused(c, "origin with a query");
    let mut c = remote();
    c.verify_path = "/verify#x".into();
    refused(c, "path containing #");
    let mut c = remote();
    c.reset_path = "reset".into();
    refused(c, "path without a leading slash");
    let mut c = remote();
    c.reset_path = "/reset?x".into();
    refused(c, "path containing ?");
    let mut c = remote();
    c.password = None;
    refused(c, "username without password");
    let mut c = remote();
    c.username = None;
    refused(c, "password without username");
    let mut c = remote();
    c.from = "not an email".into();
    refused(c, "invalid from");
    let mut c = remote();
    c.from_name = "n2f <admin>".into();
    refused(c, "from name with angle brackets");
    let mut c = remote();
    c.from_name = "x".repeat(65);
    refused(c, "from name too long");
    let mut c = remote();
    c.host = String::new();
    refused(c, "empty host");
    let mut c = remote();
    c.port = 0;
    refused(c, "port zero");
    let mut c = remote();
    c.timeout_ms = 0;
    refused(c, "timeout zero");
    let mut c = remote();
    c.timeout_ms = 30_001;
    refused(c, "timeout above ceiling");
}
