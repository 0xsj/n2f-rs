//! Redis-backed attempt limiter. The wire client is private; the application
//! sees only its AttemptLimiter port.
use crate::{
    domains::identity::app::command::{Admission, AttemptLimiter},
    shared::{
        errors::{Failure, Kind},
        keyed::Digest,
        secret::SecretString,
    },
};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct Config {
    pub url: SecretString,
    pub timeout: Duration,
    pub max_keys: usize,
}
pub type WallClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

fn configuration() -> Failure {
    Failure::new(Kind::Invalid, "invalid Redis limiter configuration")
        .with_type("identity.limiter_configuration")
}
fn dependency() -> Failure {
    Failure::new(Kind::Unavailable, "attempt limiter unavailable")
        .with_type("identity.auth_dependency_failed")
}

#[derive(Clone)]
struct Endpoint {
    address: String,
    username: Option<String>,
    password: Option<String>,
    db: u8,
}

fn endpoint(raw: &str) -> Result<Endpoint, Failure> {
    let Some(rest) = raw.strip_prefix("redis://") else {
        return Err(configuration());
    };
    if rest.contains('?') || rest.contains('#') {
        return Err(configuration());
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() || path.contains('/') {
        return Err(configuration());
    }
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .map_or((None, authority), |(u, h)| (Some(u), h));
    let (username, password) = if let Some(info) = userinfo {
        let Some((user, pass)) = info.split_once(':') else {
            return Err(configuration());
        };
        if user.is_empty() {
            (None, Some(pass.to_owned()))
        } else {
            (Some(user.to_owned()), Some(pass.to_owned()))
        }
    } else {
        (None, None)
    };
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return Err(configuration());
        };
        let host = &rest[..end];
        let suffix = &rest[end + 1..];
        let port = if suffix.is_empty() {
            "6379"
        } else {
            suffix.strip_prefix(':').ok_or_else(configuration)?
        };
        (host.to_owned(), port.to_owned())
    } else {
        if hostport.matches(':').count() > 1 {
            return Err(configuration());
        }
        let (host, port) = hostport
            .rsplit_once(':')
            .map_or((hostport, "6379"), |(h, p)| (h, p));
        (host.to_owned(), port.to_owned())
    };
    if host.is_empty() {
        return Err(configuration());
    }
    let port: u16 = port.parse().map_err(|_| configuration())?;
    if port == 0 {
        return Err(configuration());
    }
    let db: u8 = if path.is_empty() {
        0
    } else {
        path.parse().map_err(|_| configuration())?
    };
    if db > 15 {
        return Err(configuration());
    }
    let address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    Ok(Endpoint {
        address,
        username,
        password,
        db,
    })
}

const SCRIPT: &str = r##"
local now = tonumber(ARGV[1])
local max_keys = tonumber(ARGV[2])
local n = tonumber(ARGV[3])
local expired = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', now)
for _, field in ipairs(expired) do
  redis.call('ZREM', KEYS[1], field)
  redis.call('HDEL', KEYS[2], field)
end
local new = 0
for i = 0, n - 1 do
  local field = ARGV[4 + i * 3]
  if not redis.call('ZSCORE', KEYS[1], field) then
    local duplicate = 0
    for j = 0, i - 1 do
      if ARGV[4 + j * 3] == field then duplicate = 1 break end
    end
    if duplicate == 0 then new = new + 1 end
  end
end
if redis.call('ZCARD', KEYS[1]) + new > max_keys then
  return redis.error_reply('LIMITER_FULL')
end
local denied = 0
local retry = 0
for i = 0, n - 1 do
  local base = 4 + i * 3
  local field = ARGV[base]
  local attempts = tonumber(ARGV[base + 1])
  local window = tonumber(ARGV[base + 2])
  local end_ms = redis.call('ZSCORE', KEYS[1], field)
  if not end_ms then
    end_ms = now + window
    redis.call('ZADD', KEYS[1], end_ms, field)
    redis.call('HSET', KEYS[2], field, 0)
  end
  local count = redis.call('HINCRBY', KEYS[2], field, 1)
  if count > attempts then
    denied = 1
    local remaining = end_ms - now
    if remaining > retry then retry = remaining end
  end
end
return {denied == 0 and 1 or 0, retry}
"##;

#[derive(Clone)]
struct State {
    endpoint: Endpoint,
    keyed: Arc<Digest>,
    clock: WallClock,
    timeout: Duration,
    max_keys: usize,
}

#[derive(Clone)]
pub struct RedisLimiter {
    state: Arc<State>,
}

impl RedisLimiter {
    pub fn new(config: Config, keyed: Arc<Digest>, clock: WallClock) -> Result<Self, Failure> {
        if config.timeout.is_zero() || config.max_keys == 0 {
            return Err(configuration());
        }
        let endpoint = endpoint(config.url.reveal())?;
        Ok(Self {
            state: Arc::new(State {
                endpoint,
                keyed,
                clock,
                timeout: config.timeout,
                max_keys: config.max_keys,
            }),
        })
    }

    fn field(&self, operation: &str, value: &str) -> Result<String, Failure> {
        let mut message = Vec::with_capacity(operation.len() + 1 + value.len());
        message.extend_from_slice(operation.as_bytes());
        message.push(0);
        message.extend_from_slice(value.as_bytes());
        let tag = self.state.keyed.sign("rate_limit", &message)?;
        Ok(tag.iter().map(|b| format!("{b:02x}")).collect())
    }

    fn call(&self, now: i64, buckets: &[(String, u32, u64)]) -> Result<Admission, Failure> {
        let address = self
            .state
            .endpoint
            .address
            .to_socket_addrs()
            .map_err(|_| dependency())?
            .next()
            .ok_or_else(dependency)?;
        let mut stream =
            TcpStream::connect_timeout(&address, self.state.timeout).map_err(|_| dependency())?;
        stream
            .set_read_timeout(Some(self.state.timeout))
            .and_then(|_| stream.set_write_timeout(Some(self.state.timeout)))
            .map_err(|_| dependency())?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|_| dependency())?);
        if let Some(password) = &self.state.endpoint.password {
            let mut args = vec!["AUTH".to_owned()];
            if let Some(user) = &self.state.endpoint.username {
                args.push(user.clone());
            }
            args.push(password.clone());
            write_command(&mut stream, &args)?;
            if !matches!(read_reply(&mut reader), Ok(Reply::Simple)) {
                return Err(dependency());
            }
        }
        if self.state.endpoint.db != 0 {
            write_command(
                &mut stream,
                &["SELECT".to_owned(), self.state.endpoint.db.to_string()],
            )?;
            if !matches!(read_reply(&mut reader), Ok(Reply::Simple)) {
                return Err(dependency());
            }
        }
        let mut args = vec![
            "EVAL".to_owned(),
            SCRIPT.to_owned(),
            "2".to_owned(),
            "{n2f-limiter}:index".to_owned(),
            "{n2f-limiter}:counts".to_owned(),
            now.to_string(),
            self.state.max_keys.to_string(),
            buckets.len().to_string(),
        ];
        for (field, attempts, window) in buckets {
            args.extend([field.clone(), attempts.to_string(), window.to_string()]);
        }
        write_command(&mut stream, &args)?;
        let reply = read_reply(&mut reader).map_err(|_| dependency())?;
        match reply {
            Reply::Error => Err(dependency()),
            Reply::Array(values) if values.len() == 2 => match (&values[0], &values[1]) {
                (Reply::Integer(1), Reply::Integer(_)) => Ok(Admission::Permitted),
                (Reply::Integer(0), Reply::Integer(retry)) if *retry >= 0 => {
                    Ok(Admission::Refused {
                        retry_after_ms: *retry,
                    })
                }
                _ => Err(dependency()),
            },
            _ => Err(dependency()),
        }
    }
}

impl AttemptLimiter for RedisLimiter {
    fn admit(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> impl std::future::Future<Output = Result<Admission, Failure>> + Send {
        let operation = operation.to_owned();
        let subject = subject.reveal().to_owned();
        let source = source.to_owned();
        let this = self.clone();
        async move {
            let now = (this.state.clock)()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_millis()).ok())
                .filter(|v| *v >= 0)
                .ok_or_else(dependency)?;
            let subject = this.field(&operation, &subject)?;
            let mut buckets = vec![(subject, subject_attempts(&operation)?, subject_window())];
            if let Some((attempts, window)) = source_limits(&operation) {
                if !source.is_empty() {
                    buckets.push((this.field(&operation, &source)?, attempts, window));
                }
            }
            tokio::time::timeout(
                this.state.timeout,
                tokio::task::spawn_blocking(move || this.call(now, &buckets)),
            )
            .await
            .map_err(|_| dependency())?
            .map_err(|_| dependency())?
        }
    }
}

fn subject_attempts(operation: &str) -> Result<u32, Failure> {
    match operation {
        "register" => Ok(5),
        "login" => Ok(10),
        "verification_request" | "reset_request" => Ok(3),
        "verify" | "reset" => Ok(10),
        "password_change" => Ok(5),
        _ => Err(dependency()),
    }
}
fn subject_window() -> u64 {
    15 * 60 * 1000
}
fn source_limits(operation: &str) -> Option<(u32, u64)> {
    match operation {
        "register" => Some((30, subject_window())),
        "login" => Some((100, subject_window())),
        "verification_request" | "reset_request" => Some((30, subject_window())),
        _ => None,
    }
}

enum Reply {
    Simple,
    Error,
    Integer(i64),
    Array(Vec<Reply>),
}
fn line(reader: &mut BufReader<TcpStream>) -> io::Result<Vec<u8>> {
    let mut value = Vec::new();
    reader.read_until(b'\n', &mut value)?;
    if value.len() < 2 || value[value.len() - 2..] != *b"\r\n" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "redis line"));
    }
    value.truncate(value.len() - 2);
    Ok(value)
}
fn read_reply(reader: &mut BufReader<TcpStream>) -> io::Result<Reply> {
    let mut kind = [0; 1];
    reader.read_exact(&mut kind)?;
    let value = line(reader)?;
    match kind[0] {
        b'+' => Ok(Reply::Simple),
        b'-' => Ok(Reply::Error),
        b':' => Ok(Reply::Integer(
            String::from_utf8_lossy(&value)
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "redis integer"))?,
        )),
        b'*' => {
            let n: usize = String::from_utf8_lossy(&value)
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "redis array"))?;
            if n > 16 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "redis array"));
            }
            let mut values = Vec::with_capacity(n);
            for _ in 0..n {
                values.push(read_reply(reader)?);
            }
            Ok(Reply::Array(values))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "redis reply")),
    }
}
fn write_command(stream: &mut TcpStream, args: &[String]) -> Result<(), Failure> {
    write!(stream, "*{}\r\n", args.len()).map_err(|_| dependency())?;
    for arg in args {
        write!(stream, "${}\r\n{}\r\n", arg.len(), arg).map_err(|_| dependency())?;
    }
    Ok(())
}
