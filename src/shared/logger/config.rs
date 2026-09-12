use crate::shared::errors::{Failure, Kind};
use std::{io::Write, sync::Arc, time::SystemTime};
#[derive(Clone, Debug, Default)]
pub struct Resource {
    pub name: String,
    pub namespace: String,
    pub version: String,
    pub instance_id: String,
    pub environment: String,
}
pub struct Config {
    pub format: String,
    pub level: String,
    pub color: String,
    pub terminal: bool,
    pub no_color: bool,
    pub resource: Resource,
    pub clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    pub output: Box<dyn Write + Send>,
    pub capacity: usize,
    pub max_record_bytes: usize,
}
impl Config {
    pub fn new(
        resource: Resource,
        clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
        output: Box<dyn Write + Send>,
    ) -> Self {
        Self {
            format: "console".into(),
            level: "info".into(),
            color: "auto".into(),
            terminal: false,
            no_color: false,
            resource,
            clock,
            output,
            capacity: 256,
            max_record_bytes: 65536,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}
impl Level {
    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}
pub(super) fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid logger configuration")
        .with_type("logger.invalid_configuration")
}
pub fn parse_level(value: &str) -> Result<Level, Failure> {
    match value {
        "debug" => Ok(Level::Debug),
        "info" => Ok(Level::Info),
        "warn" => Ok(Level::Warn),
        "error" => Ok(Level::Error),
        _ => Err(invalid()),
    }
}
pub fn color_enabled(mode: &str, terminal: bool, no_color: bool) -> Result<bool, Failure> {
    match mode {
        "auto" => Ok(terminal && !no_color),
        "always" => Ok(true),
        "never" => Ok(false),
        _ => Err(invalid()),
    }
}
