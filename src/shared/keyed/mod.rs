//! Keyed digest: HMAC-SHA256 under a root-supplied key, bound to a purpose.
//!
//! Two consumers own the policy: CSRF token signing in the identity transport and
//! rate-limit subject digests in the attempt limiter. This module only signs and
//! verifies bytes; it never logs and never reveals its key.
#![doc = include_str!("CONTRACT.md")]
use crate::shared::{
    errors::{Failure, Kind},
    secret::SecretString,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fmt;
use subtle::ConstantTimeEq;

const MIN_KEY_BYTES: usize = 32;
const TAG_BYTES: usize = 32;
const MAX_PURPOSE: usize = 64;

/// A keyed digest whose key is never shown. Not Clone: share it by reference.
pub struct Digest {
    key: Vec<u8>,
}
fn configuration() -> Failure {
    Failure::new(Kind::Invalid, "invalid keyed digest configuration")
        .with_type("keyed.configuration")
}
fn purpose_valid(purpose: &str) -> Result<(), Failure> {
    if purpose.is_empty()
        || purpose.len() > MAX_PURPOSE
        || !purpose.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return Err(Failure::new(Kind::Invalid, "invalid digest purpose")
            .with_type("keyed.purpose_invalid"));
    }
    Ok(())
}
impl Digest {
    /// The key text's UTF-8 bytes are the key; fewer than 32 bytes is refused (K01).
    pub fn new(key: SecretString) -> Result<Self, Failure> {
        let key = key.reveal().as_bytes();
        if key.len() < MIN_KEY_BYTES {
            return Err(configuration());
        }
        Ok(Self { key: key.to_vec() })
    }
    /// HMAC-SHA256 over purpose, one zero byte and the message (K02).
    pub fn sign(&self, purpose: &str, message: &[u8]) -> Result<[u8; TAG_BYTES], Failure> {
        purpose_valid(purpose)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| configuration())?;
        mac.update(purpose.as_bytes());
        mac.update(&[0]);
        mac.update(message);
        Ok(mac.finalize().into_bytes().into())
    }
    /// Constant-time comparison; a wrong-length tag is false without comparing (K03).
    pub fn verify(&self, purpose: &str, message: &[u8], tag: &[u8]) -> Result<bool, Failure> {
        let expected = self.sign(purpose, message)?;
        if tag.len() != TAG_BYTES {
            return Ok(false);
        }
        Ok(bool::from(expected.ct_eq(tag)))
    }
}
impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Digest([REDACTED])")
    }
}
