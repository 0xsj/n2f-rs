//! Identity-owned Argon2id password hashing with bounded admission. See CONTRACT.md.
//!
//! Records are parsed against an exact allowlist before any Argon2 memory is
//! allocated; the argon2 crate computes tags only. CPU work runs on the blocking
//! pool behind a semaphore; once admitted it completes regardless of deadlines.
#![doc = include_str!("CONTRACT.md")]

use crate::{
    domains::identity::domain::password::{NewPassword, PasswordInput},
    shared::{
        entropy::Entropy,
        errors::{Failure, Kind},
        secret::SecretString,
    },
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

const MEMORY_KIB: u32 = 19456;
const PASSES: u32 = 2;
const PARALLELISM: u32 = 1;
const SALT_LEN: usize = 16;
const TAG_LEN: usize = 32;
const MAX_RECORD_BYTES: usize = 512;
const PREFIX: &str = "$argon2id$v=19$m=19456,t=2,p=1$";
/// Fixed record for absent-credential verification (H05); equals the fixture's dummy_record.
const DUMMY_RECORD: &str = "$argon2id$v=19$m=19456,t=2,p=1$gIGCg4SFhoeIiYqLjI2Ojw$hCpNSDRlggIY1L1+l6kN4tcjdR+eIOW4qSmc8vLLUXg";
/// The record verify_absent works against; tests pin it to the fixture.
pub fn dummy_record() -> &'static str {
    DUMMY_RECORD
}

/// Admission limits (H06). Refusals never consume a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_concurrent: usize,
    pub max_queued: usize,
    pub queue_wait: Duration,
}
/// Verification is a value; corruption and admission failures are errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Match,
    Mismatch,
}
/// Blocking tag computation: (normalized password bytes, salt) -> tag.
/// Production uses Argon2id; tests may inject a controllable fake.
pub type Work = Arc<dyn Fn(&[u8], &[u8; SALT_LEN]) -> [u8; TAG_LEN] + Send + Sync>;

pub struct Hasher<E> {
    entropy: Mutex<E>,
    limits: Limits,
    slots: Arc<Semaphore>,
    queued: AtomicUsize,
    work: Work,
}

fn corrupt() -> Failure {
    Failure::new(Kind::Internal, "stored password record is corrupt")
        .with_type("identity.credential_corrupt")
}
fn configuration() -> Failure {
    Failure::new(Kind::Invalid, "invalid password hasher configuration")
        .with_type("identity.hasher_configuration")
}
fn argon2id_work() -> Work {
    Arc::new(|pwd: &[u8], salt: &[u8; SALT_LEN]| {
        let params = Params::new(MEMORY_KIB, PASSES, PARALLELISM, Some(TAG_LEN))
            .expect("write format parameters are valid");
        let mut out = [0u8; TAG_LEN];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(pwd, salt, &mut out)
            .expect("fixed-size salt and output");
        out
    })
}
/// Strict PHC B64: no padding, standard alphabet, canonical re-encoding.
fn b64_strict(field: &str, len: usize) -> Result<Vec<u8>, Failure> {
    let bytes = STANDARD_NO_PAD.decode(field).map_err(|_| corrupt())?;
    if bytes.len() != len || STANDARD_NO_PAD.encode(&bytes) != field {
        return Err(corrupt());
    }
    Ok(bytes)
}
/// H03: the complete read allowlist, evaluated before any allocation.
fn parse(record: &str) -> Result<([u8; SALT_LEN], [u8; TAG_LEN]), Failure> {
    if record.is_empty() || record.len() > MAX_RECORD_BYTES {
        return Err(corrupt());
    }
    let rest = record.strip_prefix('$').ok_or_else(corrupt)?;
    let fields: Vec<&str> = rest.split('$').collect();
    if fields.len() != 5
        || fields[0] != "argon2id"
        || fields[1] != "v=19"
        || fields[2] != "m=19456,t=2,p=1"
    {
        return Err(corrupt());
    }
    let salt: [u8; SALT_LEN] = b64_strict(fields[3], SALT_LEN)?
        .try_into()
        .map_err(|_| corrupt())?;
    let tag: [u8; TAG_LEN] = b64_strict(fields[4], TAG_LEN)?
        .try_into()
        .map_err(|_| corrupt())?;
    Ok((salt, tag))
}
fn encode(salt: &[u8; SALT_LEN], tag: &[u8; TAG_LEN]) -> String {
    format!(
        "{PREFIX}{}${}",
        STANDARD_NO_PAD.encode(salt),
        STANDARD_NO_PAD.encode(tag)
    )
}
/// Decrements the queued counter when a waiter leaves for any reason,
/// including an abandoned future.
struct Queued<'a>(&'a AtomicUsize);
impl Drop for Queued<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<E: Entropy + Send> Hasher<E> {
    pub fn new(entropy: E, limits: Limits) -> Result<Self, Failure> {
        Self::with_work(entropy, limits, argon2id_work())
    }
    /// Test seam for admission scenarios; production callers use `new`.
    #[doc(hidden)]
    pub fn with_work(entropy: E, limits: Limits, work: Work) -> Result<Self, Failure> {
        if limits.max_concurrent == 0 || limits.queue_wait.is_zero() {
            return Err(configuration());
        }
        Ok(Self {
            entropy: Mutex::new(entropy),
            limits,
            slots: Arc::new(Semaphore::new(limits.max_concurrent)),
            queued: AtomicUsize::new(0),
            work,
        })
    }
    /// H06: immediate slot, else a bounded queue with a wait limit.
    async fn admit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Failure> {
        if let Ok(permit) = self.slots.clone().try_acquire_owned() {
            return Ok(permit);
        }
        let position = self.queued.fetch_add(1, Ordering::SeqCst);
        let waiting = Queued(&self.queued);
        if position >= self.limits.max_queued {
            drop(waiting);
            return Err(
                Failure::new(Kind::Unavailable, "password hashing saturated")
                    .with_type("identity.hash_saturated"),
            );
        }
        let acquired =
            tokio::time::timeout(self.limits.queue_wait, self.slots.clone().acquire_owned()).await;
        drop(waiting);
        match acquired {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(Failure::new(Kind::Internal, "password hashing unavailable")
                .with_type("identity.hash_failed")),
            Err(_) => Err(
                Failure::new(Kind::Timeout, "password hashing queue wait exceeded")
                    .with_type("identity.hash_queue_timeout"),
            ),
        }
    }
    /// H07: admitted work completes on the blocking pool; the permit is released
    /// when the task finishes, including by panic.
    async fn run(&self, password: Vec<u8>, salt: [u8; SALT_LEN]) -> Result<[u8; TAG_LEN], Failure> {
        let permit = self.admit().await?;
        let work = self.work.clone();
        tokio::task::spawn_blocking(move || {
            let _held = permit;
            work(&password, &salt)
        })
        .await
        .map_err(|_| {
            Failure::new(Kind::Internal, "password hashing failed")
                .with_type("identity.hash_failed")
        })
    }
    fn salt(&self) -> Result<[u8; SALT_LEN], Failure> {
        let mut salt = [0u8; SALT_LEN];
        let mut entropy = self.entropy.lock().map_err(|_| {
            Failure::new(Kind::Internal, "entropy lock poisoned").with_type("identity.hash_failed")
        })?;
        entropy.fill(&mut salt).map_err(|source| {
            Failure::new(Kind::Unavailable, "entropy unavailable")
                .with_type("identity.entropy_unavailable")
                .with_boxed_source(source)
        })?;
        Ok(salt)
    }
    pub async fn hash(&self, password: &NewPassword) -> Result<SecretString, Failure> {
        let salt = self.salt()?;
        let tag = self
            .run(password.secret().reveal().as_bytes().to_vec(), salt)
            .await?;
        Ok(SecretString::new(encode(&salt, &tag)))
    }
    pub async fn verify(
        &self,
        input: &PasswordInput,
        record: &SecretString,
    ) -> Result<Outcome, Failure> {
        let (salt, stored) = parse(record.reveal())?;
        let computed = self
            .run(input.secret().reveal().as_bytes().to_vec(), salt)
            .await?;
        Ok(if bool::from(computed.ct_eq(&stored)) {
            Outcome::Match
        } else {
            Outcome::Mismatch
        })
    }
    /// H05: comparable work for an unknown identifier; the comparison is discarded.
    pub async fn verify_absent(&self, input: &PasswordInput) -> Result<Outcome, Failure> {
        let (salt, stored) = parse(DUMMY_RECORD)?;
        let computed = self
            .run(input.secret().reveal().as_bytes().to_vec(), salt)
            .await?;
        let _discarded = bool::from(computed.ct_eq(&stored));
        Ok(Outcome::Mismatch)
    }
    pub fn needs_rehash(&self, record: &SecretString) -> Result<bool, Failure> {
        parse(record.reveal()).map(|_| false)
    }
}
