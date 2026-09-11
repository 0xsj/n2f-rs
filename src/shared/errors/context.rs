use std::error::Error as StdError;
use std::fmt;

use super::{Classification, Classified};

/// Context retaining the typed inner error and delegating its public meaning.
#[derive(Debug)]
pub struct Context<E> {
    operation: String,
    inner: E,
}
impl<E> Context<E> {
    pub fn new(operation: impl Into<String>, inner: E) -> Self {
        Self {
            operation: operation.into(),
            inner,
        }
    }
    pub fn inner(&self) -> &E {
        &self.inner
    }
    pub fn into_inner(self) -> E {
        self.inner
    }
}
impl<E> fmt::Display for Context<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.operation)
    }
}
impl<E: StdError + 'static> StdError for Context<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.inner)
    }
}
impl<E: Classified + 'static> Classified for Context<E> {
    fn classification(&self) -> Option<Classification<'_>> {
        self.inner.classification()
    }
}
