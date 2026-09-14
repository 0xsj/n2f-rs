//! Identity-owned opaque token issuance and digesting. See CONTRACT.md.
//!
//! Secrets are 32 injected entropy bytes as canonical unpadded base64url;
//! digests are SHA-256 over purpose, a zero byte and the raw bytes.
#![doc = include_str!("CONTRACT.md")]

use crate::{
    domains::identity::domain::token::{TokenDigest, TokenPurpose},
    shared::{
        entropy::Entropy,
        errors::{Failure, Kind},
        secret::SecretString,
    },
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

const TOKEN_BYTES: usize = 32;
const SECRET_LEN: usize = 43;

/// An issued secret with its digest. Deliberately no Debug: the secret is
/// disclosed only to its transport or mail consumer.
pub struct Issued {
    pub secret: SecretString,
    pub digest: TokenDigest,
}
pub struct Codec<E> {
    entropy: Mutex<E>,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid token secret").with_type("identity.token_invalid")
}
fn digest_of(purpose: TokenPurpose, raw: &[u8; TOKEN_BYTES]) -> TokenDigest {
    let mut hasher = Sha256::new();
    hasher.update(purpose.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(raw);
    TokenDigest::parse(purpose, &hasher.finalize()).expect("SHA-256 output is 32 bytes")
}
impl<E: Entropy> Codec<E> {
    /// Rust cannot construct a missing capability; the configuration refusal in
    /// the contract is unreachable here and the Result exists for shape parity.
    pub fn new(entropy: E) -> Result<Self, Failure> {
        Ok(Self {
            entropy: Mutex::new(entropy),
        })
    }
    pub fn issue(&self, purpose: TokenPurpose) -> Result<Issued, Failure> {
        let mut raw = [0u8; TOKEN_BYTES];
        let mut entropy = self.entropy.lock().map_err(|_| {
            Failure::new(Kind::Internal, "entropy lock poisoned").with_type("identity.token_failed")
        })?;
        entropy.fill(&mut raw).map_err(|source| {
            Failure::new(Kind::Unavailable, "entropy unavailable")
                .with_type("identity.entropy_unavailable")
                .with_boxed_source(source)
        })?;
        drop(entropy);
        Ok(Issued {
            secret: SecretString::new(URL_SAFE_NO_PAD.encode(raw)),
            digest: digest_of(purpose, &raw),
        })
    }
    /// T02: exactly 43 canonical base64url characters decoding to 32 bytes.
    pub fn digest(
        &self,
        purpose: TokenPurpose,
        secret: &SecretString,
    ) -> Result<TokenDigest, Failure> {
        let text = secret.reveal();
        if text.len() != SECRET_LEN
            || !text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(invalid());
        }
        let decoded = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid())?;
        let raw: [u8; TOKEN_BYTES] = decoded.try_into().map_err(|_| invalid())?;
        if URL_SAFE_NO_PAD.encode(raw) != text {
            return Err(invalid());
        }
        Ok(digest_of(purpose, &raw))
    }
}
