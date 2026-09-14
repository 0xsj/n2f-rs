//! Identity local adapters: process-local limiter, file-backed blocklist and
//! undelivered mail. Development stand-ins for stage 7, not production protection.
#![doc = include_str!("CONTRACT.md")]
use crate::{
    domains::identity::{
        app::command::{Admission, AttemptLimiter, EnrollmentPolicy, MailDelivery},
        domain::{email::Email, password::NewPassword},
    },
    shared::{
        errors::{Failure, Kind},
        keyed::Digest,
        secret::SecretString,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use unicode_normalization::UnicodeNormalization;

/// Attempts per fixed window (L02).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bucket {
    pub attempts: u32,
    pub window: Duration,
}
/// Per-operation limits: (subject bucket, source bucket); `None` means unbounded.
pub type OperationLimits = (Option<Bucket>, Option<Bucket>);
#[derive(Clone, Debug)]
pub struct LimiterConfig {
    pub operations: BTreeMap<String, OperationLimits>,
    pub max_keys: usize,
}
impl LimiterConfig {
    /// The L02 defaults.
    pub fn defaults() -> Self {
        let m = Duration::from_secs(15 * 60);
        let b = |attempts: u32| {
            Some(Bucket {
                attempts,
                window: m,
            })
        };
        Self {
            operations: BTreeMap::from([
                ("register".into(), (b(5), b(30))),
                ("login".into(), (b(10), b(100))),
                ("verification_request".into(), (b(3), b(30))),
                ("reset_request".into(), (b(3), b(30))),
                ("verify".into(), (b(10), None)),
                ("reset".into(), (b(10), None)),
                ("password_change".into(), (b(5), None)),
            ]),
            max_keys: 100_000,
        }
    }
}
fn limiter_configuration() -> Failure {
    Failure::new(Kind::Invalid, "invalid limiter configuration")
        .with_type("identity.limiter_configuration")
}
fn dependency_failed(message: &'static str) -> Failure {
    Failure::new(Kind::Unavailable, message).with_type("identity.auth_dependency_failed")
}
#[derive(Clone, Copy)]
struct Window {
    end_ms: u64,
    count: u32,
}
pub type WallClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;
/// Process-local fixed-window limiter (L01–L03). State holds keyed digests only.
pub struct Limiter {
    keyed: Arc<Digest>,
    clock: WallClock,
    config: LimiterConfig,
    windows: Mutex<HashMap<[u8; 32], Window>>,
}
impl Limiter {
    pub fn new(
        keyed: Arc<Digest>,
        clock: WallClock,
        config: LimiterConfig,
    ) -> Result<Self, Failure> {
        if config.max_keys == 0 || config.operations.is_empty() {
            return Err(limiter_configuration());
        }
        for (name, (subject, source)) in &config.operations {
            if name.is_empty() {
                return Err(limiter_configuration());
            }
            for b in [subject, source].into_iter().flatten() {
                if b.attempts == 0
                    || b.window.is_zero()
                    || b.window.as_millis() > u128::from(u64::MAX / 4)
                {
                    return Err(limiter_configuration());
                }
            }
        }
        Ok(Self {
            keyed,
            clock,
            config,
            windows: Mutex::new(HashMap::new()),
        })
    }
    fn now_ms(&self) -> Result<u64, Failure> {
        (self.clock)()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .ok_or_else(|| dependency_failed("clock reading out of range"))
    }
    fn key(&self, operation: &str, value: &str) -> Result<[u8; 32], Failure> {
        let mut message = Vec::with_capacity(operation.len() + 1 + value.len());
        message.extend_from_slice(operation.as_bytes());
        message.push(0);
        message.extend_from_slice(value.as_bytes());
        self.keyed
            .sign("rate_limit", &message)
            .map_err(|e| dependency_failed("limiter digest failed").with_source(e))
    }
    /// Count one attempt in a bucket; returns the retry-after when over the limit.
    fn count(
        windows: &mut HashMap<[u8; 32], Window>,
        max_keys: usize,
        key: [u8; 32],
        bucket: Bucket,
        now: u64,
    ) -> Result<Option<i64>, Failure> {
        let window_ms = u64::try_from(bucket.window.as_millis()).unwrap_or(u64::MAX);
        let fresh = |now: u64| Window {
            end_ms: now.saturating_add(window_ms),
            count: 0,
        };
        let entry = match windows.get(&key) {
            Some(w) if w.end_ms > now => *w,
            _ => {
                if !windows.contains_key(&key) && windows.len() >= max_keys {
                    windows.retain(|_, w| w.end_ms > now);
                    if windows.len() >= max_keys {
                        return Err(dependency_failed("limiter store full"));
                    }
                }
                fresh(now)
            }
        };
        let next = Window {
            count: entry.count.saturating_add(1),
            ..entry
        };
        windows.insert(key, next);
        Ok(if next.count > bucket.attempts {
            Some(i64::try_from(next.end_ms.saturating_sub(now)).unwrap_or(i64::MAX))
        } else {
            None
        })
    }
    pub async fn admit_now(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> Result<Admission, Failure> {
        let Some((subject_bucket, source_bucket)) = self.config.operations.get(operation).copied()
        else {
            return Err(dependency_failed(
                "limiter has no policy for this operation",
            ));
        };
        let now = self.now_ms()?;
        let subject_key = subject_bucket
            .map(|b| self.key(operation, subject.reveal()).map(|k| (k, b)))
            .transpose()?;
        let source_key = source_bucket
            .map(|b| self.key(operation, source).map(|k| (k, b)))
            .transpose()?;
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let mut retry = None;
        for (key, bucket) in [subject_key, source_key].into_iter().flatten() {
            if let Some(after) = Self::count(&mut windows, self.config.max_keys, key, bucket, now)?
            {
                retry = Some(retry.map_or(after, |r: i64| r.max(after)));
            }
        }
        Ok(match retry {
            Some(retry_after_ms) => Admission::Refused { retry_after_ms },
            None => Admission::Permitted,
        })
    }
    /// Test visibility: the stored keys, which are digests only (V01).
    pub fn debug_keys(&self) -> Vec<Vec<u8>> {
        self.windows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .map(|k| k.to_vec())
            .collect()
    }
}
/// Debug shows only the number of tracked windows; keys are digests and stay private.
impl std::fmt::Debug for Limiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.windows.lock().map(|w| w.len()).unwrap_or(0);
        write!(f, "Limiter(windows={n}, max_keys={})", self.config.max_keys)
    }
}
impl AttemptLimiter for Limiter {
    fn admit(
        &self,
        operation: &str,
        subject: &SecretString,
        source: &str,
    ) -> impl Future<Output = Result<Admission, Failure>> + Send {
        self.admit_now(operation, subject, source)
    }
}

fn fold(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}
fn blocklist_configuration() -> Failure {
    Failure::new(Kind::Invalid, "invalid password blocklist configuration")
        .with_type("identity.blocklist_configuration")
}
/// File-backed enrollment policy (B01–B02): exact, NFC- and case-folded matches.
pub struct Blocklist {
    entries: std::collections::HashSet<String>,
}
impl Blocklist {
    pub fn load(path: &Path) -> Result<Self, Failure> {
        let text =
            std::fs::read_to_string(path).map_err(|e| blocklist_configuration().with_source(e))?;
        let entries: std::collections::HashSet<String> = text
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(fold)
            .collect();
        if entries.is_empty() {
            return Err(blocklist_configuration());
        }
        Ok(Self { entries })
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn allows(&self, password: &NewPassword) -> bool {
        !self.entries.contains(&fold(password.secret().reveal()))
    }
}
/// Debug shows the entry count only (B01): contents never leave the adapter.
impl std::fmt::Debug for Blocklist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blocklist(entries={})", self.entries.len())
    }
}
impl EnrollmentPolicy for Blocklist {
    fn check_blocklist(
        &self,
        password: &NewPassword,
    ) -> impl Future<Output = Result<bool, Failure>> + Send {
        let allowed = self.allows(password);
        async move { Ok(allowed) }
    }
}

/// Mail port that delivers nothing (M01); counts only.
#[derive(Default)]
pub struct UndeliveredMail {
    verification: AtomicU64,
    reset: AtomicU64,
}
impl UndeliveredMail {
    pub fn counts(&self) -> (u64, u64) {
        (
            self.verification.load(Ordering::Relaxed),
            self.reset.load(Ordering::Relaxed),
        )
    }
}
impl std::fmt::Debug for UndeliveredMail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (v, r) = self.counts();
        write!(f, "UndeliveredMail(verification={v}, reset={r})")
    }
}
impl MailDelivery for UndeliveredMail {
    fn send_verification(
        &self,
        _: &Email,
        _: &SecretString,
        _: i64,
    ) -> impl Future<Output = bool> + Send {
        self.verification.fetch_add(1, Ordering::Relaxed);
        async { false }
    }
    fn send_reset(&self, _: &Email, _: &SecretString, _: i64) -> impl Future<Output = bool> + Send {
        self.reset.fetch_add(1, Ordering::Relaxed);
        async { false }
    }
}
