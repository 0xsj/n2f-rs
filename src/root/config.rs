use crate::shared::{
    env::{Reader, Var},
    errors::{Classified, Failure, Kind},
    logger::Resource,
    provenance::{Actor, ActorKind},
    secret::SecretString,
};
use std::time::Duration;
pub struct Config {
    pub resource: Resource,
    pub format: String,
    pub level: String,
    pub color: String,
    pub capacity: usize,
    pub max_bytes: usize,
    pub flush: Duration,
    pub verbose: bool,
    pub no_color: bool,
    pub token: SecretString,
    pub manifest: Vec<Var>,
}
pub fn load_config(lookup: impl Fn(&str) -> Option<String>) -> Result<Config, Failure> {
    let mut r = Reader::new(lookup);
    let resource = Resource {
        name: r.string("SERVICE_NAME", "n2f-foundations"),
        namespace: r.string("SERVICE_NAMESPACE", "n2f"),
        version: r.string("SERVICE_VERSION", "dev"),
        environment: r.enumeration(
            "APP_ENV",
            "development",
            &["development", "test", "production"],
        ),
        instance_id: String::new(),
    };
    let format = r.enumeration("LOG_FORMAT", "console", &["console", "json", "none"]);
    let level = r.enumeration("LOG_LEVEL", "info", &["debug", "info", "warn", "error"]);
    let color = r.enumeration("LOG_COLOR", "auto", &["auto", "always", "never"]);
    let capacity = r.int("LOG_CAPACITY", 256, 1, 65536) as usize;
    let max_bytes = r.int("LOG_MAX_BYTES", 65536, 256, 1048576) as usize;
    let flush = Duration::from_millis(r.int("LOG_FLUSH_MS", 1000, 1, 10000) as u64);
    let verbose = r.boolean("DEMO_VERBOSE", false);
    let no_color = !r.string("NO_COLOR", "").is_empty();
    let token = r.secret("DEMO_TOKEN");
    let mut problems = r
        .check()
        .err()
        .and_then(|e| e.classification().and_then(|v| v.fields.cloned()))
        .unwrap_or_default();
    if Actor::new(ActorKind::Service, resource.name.clone()).is_err() {
        problems.insert("SERVICE_NAME".into(), "invalid_identity".into());
    }
    if !problems.is_empty() {
        return Err(Failure::new(Kind::Invalid, "invalid configuration")
            .with_type("env.invalid")
            .with_fields(problems));
    }
    Ok(Config {
        resource,
        format,
        level,
        color,
        capacity,
        max_bytes,
        flush,
        verbose,
        no_color,
        token,
        manifest: r.manifest()?,
    })
}
