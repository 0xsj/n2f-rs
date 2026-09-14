//! Identity SMTP mail delivery (CONTRACT.md M10–M16).
//!
//! Implements the stage 4 MailDelivery port for verification and reset messages.
//! It builds a plain-text message whose link carries the token only in the
//! fragment, submits it over one bounded connection, and reports delivery as a
//! value. It decides nothing about who receives mail and never retries.
use crate::{
    domains::identity::{app::command::MailDelivery, domain::email::Email},
    shared::{
        errors::{Failure, Kind},
        logger::{Fields, Logger},
        secret::SecretString,
    },
};
use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::{
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
};
use std::{
    future::Future,
    net::IpAddr,
    time::{Duration, Instant},
};
use url::{Host, Url};

/// Connection security (M10). `None` is accepted only for a loopback host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    None,
    StartTls,
    Tls,
}

/// Root-supplied settings (M10); validated once by [`Mailer::new`].
pub struct Config {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub from: String,
    pub from_name: String,
    pub link_origin: String,
    pub verify_path: String,
    pub reset_path: String,
    pub timeout_ms: i64,
}

pub struct Mailer {
    host: String,
    port: u16,
    security: Security,
    credentials: Option<(String, SecretString)>,
    from: Mailbox,
    link_origin: String,
    verify_path: String,
    reset_path: String,
    timeout: Duration,
    log: Logger,
}
impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mailer([REDACTED])")
    }
}

fn refused() -> Failure {
    Failure::new(Kind::Invalid, "invalid mail configuration")
        .with_type("identity.mail_configuration")
}
fn plain(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic())
}
fn loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
fn valid_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains(['?', '#']) && plain(path)
}
fn valid_origin(origin: &str) -> bool {
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    let loopback = match url.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => return false,
    };
    let scheme_ok = url.scheme() == "https" || url.scheme() == "http" && loopback;
    // An exact origin: re-serializing must give the input back, which refuses a
    // path (including a bare trailing slash), a query, a fragment and userinfo.
    scheme_ok
        && url.username().is_empty()
        && url.password().is_none()
        && url.origin().ascii_serialization() == origin
}
/// ISO 8601 UTC with milliseconds, from Unix milliseconds (civil-from-days).
fn iso8601(ms: i64) -> String {
    let (secs, millis) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (hour, minute, second) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

impl Mailer {
    pub fn new(config: Config, log: Logger) -> Result<Self, Failure> {
        let host_ok = !config.host.is_empty() && config.host.len() <= 253 && plain(&config.host);
        if !host_ok || config.port == 0 || !(1..=30_000).contains(&config.timeout_ms) {
            return Err(refused());
        }
        if config.security == Security::None && !loopback_host(&config.host) {
            return Err(refused());
        }
        let credentials = match (config.username, config.password) {
            (None, None) => None,
            (Some(user), Some(password)) if plain(&user) && !password.reveal().is_empty() => {
                Some((user, password))
            }
            _ => return Err(refused()),
        };
        let from = Email::parse(&config.from).map_err(|_| refused())?;
        let address: Address = from.reveal().parse().map_err(|_| refused())?;
        let name = &config.from_name;
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|c| (' '..='~').contains(&c) && !matches!(c, '"' | '<' | '>' | '\\'))
        {
            return Err(refused());
        }
        if !valid_origin(&config.link_origin)
            || !valid_path(&config.verify_path)
            || !valid_path(&config.reset_path)
        {
            return Err(refused());
        }
        if config.security != Security::None {
            TlsParameters::new(config.host.clone()).map_err(|_| refused())?;
        }
        Ok(Self {
            host: config.host,
            port: config.port,
            security: config.security,
            credentials,
            from: Mailbox::new(Some(config.from_name), address),
            link_origin: config.link_origin,
            verify_path: config.verify_path,
            reset_path: config.reset_path,
            timeout: Duration::from_millis(config.timeout_ms as u64),
            log,
        })
    }

    async fn deliver(
        &self,
        kind: &'static str,
        to: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> bool {
        let started = Instant::now();
        let outcome =
            match tokio::time::timeout(self.timeout, self.attempt(kind, to, token, expires_at_ms))
                .await
            {
                Ok(Ok(())) => "delivered",
                Ok(Err(outcome)) => outcome,
                Err(_) => "timeout",
            };
        self.log.info(
            "identity.mail.attempted",
            Fields::from([
                ("kind".into(), kind.into()),
                ("outcome".into(), outcome.into()),
                (
                    "elapsed_ms".into(),
                    (started.elapsed().as_millis() as u64).into(),
                ),
            ]),
        );
        outcome == "delivered"
    }

    /// One connection, one message. The token and password exist only for the
    /// duration of this call (M14); errors are reduced to a fixed outcome word.
    async fn attempt(
        &self,
        kind: &'static str,
        to: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> Result<(), &'static str> {
        let (subject, path, purpose, ignore) = if kind == "reset" {
            (
                "Reset your password",
                &self.reset_path,
                "Reset the password for your n2f account by opening this link:",
                "If you did not ask to reset your password, ignore this message.",
            )
        } else {
            (
                "Verify your email address",
                &self.verify_path,
                "Confirm the email address for your n2f account by opening this link:",
                "If you did not create an account, ignore this message.",
            )
        };
        let link = format!("{}{}#token={}", self.link_origin, path, token.reveal());
        let body = format!(
            "{purpose}\n\n{link}\n\nThe link expires at {}. {ignore}\n",
            iso8601(expires_at_ms)
        );
        let recipient: Address = to.reveal().parse().map_err(|_| "failed")?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(None, recipient))
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .map_err(|_| "failed")?;
        let tls = match self.security {
            Security::None => Tls::None,
            Security::StartTls => {
                Tls::Required(TlsParameters::new(self.host.clone()).map_err(|_| "failed")?)
            }
            Security::Tls => {
                Tls::Wrapper(TlsParameters::new(self.host.clone()).map_err(|_| "failed")?)
            }
        };
        let mut builder =
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(self.host.as_str())
                .port(self.port)
                .tls(tls)
                .timeout(Some(self.timeout));
        if let Some((user, password)) = &self.credentials {
            builder = builder.credentials(Credentials::new(
                user.clone(),
                password.reveal().to_string(),
            ));
        }
        match builder.build().send(message).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_timeout() => Err("timeout"),
            Err(e) if e.is_permanent() || e.is_transient() => Err("refused"),
            Err(_) => Err("failed"),
        }
    }
}

impl MailDelivery for Mailer {
    fn send_verification(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send {
        self.deliver("verification", email, token, expires_at_ms)
    }
    fn send_reset(
        &self,
        email: &Email,
        token: &SecretString,
        expires_at_ms: i64,
    ) -> impl Future<Output = bool> + Send {
        self.deliver("reset", email, token, expires_at_ms)
    }
}
