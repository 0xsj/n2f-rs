//! Injected cryptographic entropy with an explicit failure path.
//!
//! One shared owner of the OS randomness dependency, consumed by ID generation
//! and identity token/salt issuance. Implementations fill the entire buffer or
//! return an error; there is no fallback source, and a failure is the caller's
//! refusal (decision 0004). Never use test entropy in production.

use std::error::Error;

/// Diagnostic source owned at the entropy boundary; no library-specific type leaks.
pub type EntropyError = Box<dyn Error + Send + Sync + 'static>;
/// Implementations fill the entire buffer or return an error. A closure can
/// implement this capability directly.
pub trait Entropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), EntropyError>;
}
impl<F> Entropy for F
where
    F: FnMut(&mut [u8]) -> Result<(), EntropyError>,
{
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), EntropyError> {
        self(bytes)
    }
}
/// The only owner of the OS randomness dependency.
pub struct OsEntropy;
impl Entropy for OsEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), EntropyError> {
        getrandom::fill(bytes).map_err(|e| Box::new(e) as EntropyError)
    }
}
