use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use super::{Classification, Classified, Kind};

/// Owned shared error data with an optional task-safe diagnostic source.
#[derive(Debug)]
pub struct Failure {
    kind: Kind,
    message: String,
    error_type: Option<String>,
    fields: BTreeMap<String, String>,
    details: BTreeMap<String, String>,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}
impl Failure {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            error_type: None,
            fields: BTreeMap::new(),
            details: BTreeMap::new(),
            source: None,
        }
    }
    pub fn with_type(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.error_type = if value.is_empty() { None } else { Some(value) };
        self
    }
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }
    pub fn with_fields(mut self, values: BTreeMap<String, String>) -> Self {
        self.fields.extend(values);
        self
    }
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }
    pub fn with_details(mut self, values: BTreeMap<String, String>) -> Self {
        self.details.extend(values);
        self
    }
    /// Replace the source without requiring it to implement Clone.
    pub fn with_source(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
    /// Replace the source with an already boxed error, preserving direct downcasts.
    pub fn with_boxed_source(mut self, source: Box<dyn StdError + Send + Sync + 'static>) -> Self {
        self.source = Some(source);
        self
    }
    pub fn details(&self) -> &BTreeMap<String, String> {
        &self.details
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.message.is_empty() {
            self.kind.as_str()
        } else {
            &self.message
        })
    }
}
impl StdError for Failure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}
impl Classified for Failure {
    fn classification(&self) -> Option<Classification<'_>> {
        Some(Classification {
            kind: self.kind,
            error_type: self.error_type.as_deref(),
            message: &self.message,
            fields: Some(&self.fields),
        })
    }
}
