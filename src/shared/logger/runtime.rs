use super::{
    config::{Config, Level, color_enabled, invalid, parse_level},
    delivery::{Delivery, Stats, Writer},
    formatter::Formatter,
    projection,
    value::{Fields, Value, json},
};
use crate::shared::{
    errors::{Classified, Failure},
    provenance::Scope,
};
use std::{
    error::Error,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
struct Core {
    delivery: Arc<Delivery>,
    clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    level: Level,
    resource: serde_json::Value,
    dispatch: tracing::Dispatch,
}
#[derive(Clone)]
pub struct Logger {
    core: Arc<Core>,
    fields: Fields,
    scope: Option<serde_json::Value>,
    trace: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}
impl Logger {
    pub fn with_trace(&self, trace: &crate::shared::telemetry::TraceRef) -> Self {
        let mut out = self.clone();
        let s = trace.snapshot();
        out.trace = Some(
            serde_json::json!({"trace_id":s.trace_id,"span_id":s.span_id,"sampled":s.sampled}),
        );
        out
    }
    pub fn with(&self, fields: Fields) -> Self {
        let mut out = self.clone();
        out.fields.extend(fields);
        out
    }
    pub fn with_scope(&self, scope: &Scope) -> Self {
        let mut out = self.clone();
        out.scope = Some(projection::scope(scope));
        out
    }
    pub fn with_failure<E: Error + Classified>(&self, error: &E) -> Self {
        let mut out = self.clone();
        out.error = Some(projection::error(error, error.classification()));
        out
    }
    pub fn with_unknown(&self, error: &dyn Error) -> Self {
        let mut out = self.clone();
        out.error = Some(projection::error(error, None));
        out
    }
    pub fn debug(&self, message: &str, fields: Fields) {
        self.emit(Level::Debug, message, fields)
    }
    pub fn info(&self, message: &str, fields: Fields) {
        self.emit(Level::Info, message, fields)
    }
    pub fn warn(&self, message: &str, fields: Fields) {
        self.emit(Level::Warn, message, fields)
    }
    pub fn error(&self, message: &str, fields: Fields) {
        self.emit(Level::Error, message, fields)
    }
    fn emit(&self, level: Level, message: &str, fields: Fields) {
        let c = &self.core;
        if c.delivery.noop || level < c.level {
            return;
        }
        if c.delivery.closed() {
            c.delivery.drop_record();
            return;
        }
        let Ok(duration) = (c.clock)().duration_since(UNIX_EPOCH) else {
            c.delivery.fail();
            return;
        };
        let ms = duration.as_millis();
        if ms > 281474976710655 {
            c.delivery.fail();
            return;
        }
        let mut merged = self.fields.clone();
        merged.extend(fields);
        let Ok(fields) = json(&Value::Object(merged)) else {
            c.delivery.fail();
            return;
        };
        let mut record = serde_json::json!({"timestamp_ms":ms as u64,"level":level.text(),"message":message,"service":c.resource,"fields":fields});
        if let Some(t) = &self.trace {
            record["trace"] = t.clone();
        }
        if let Some(s) = &self.scope {
            record["scope"] = s.clone();
        }
        if let Some(e) = &self.error {
            record["error"] = e.clone();
        }
        let Ok(encoded) = serde_json::to_string(&record) else {
            c.delivery.fail();
            return;
        };
        tracing::dispatcher::with_default(&c.dispatch, || match level {
            Level::Debug => tracing::debug!(n2f_record = encoded.as_str()),
            Level::Info => tracing::info!(n2f_record = encoded.as_str()),
            Level::Warn => tracing::warn!(n2f_record = encoded.as_str()),
            Level::Error => tracing::error!(n2f_record = encoded.as_str()),
        });
    }
}
pub struct Runtime {
    pub log: Logger,
}
impl Runtime {
    pub fn new(config: Config) -> Result<Self, Failure> {
        let level = parse_level(&config.level)?;
        let color = color_enabled(&config.color, config.terminal, config.no_color)?;
        if !["console", "json", "none"].contains(&config.format.as_str())
            || config.capacity == 0
            || config.max_record_bytes == 0
            || config.resource.name.is_empty()
        {
            return Err(invalid());
        }
        let resource = config.resource;
        let mut r = serde_json::json!({"name":resource.name});
        for (k, v) in [
            ("namespace", resource.namespace),
            ("version", resource.version),
            ("instance_id", resource.instance_id),
            ("environment", resource.environment),
        ] {
            if !v.is_empty() {
                r[k] = v.into();
            }
        }
        let delivery = Delivery::start(
            config.output,
            config.capacity,
            config.max_record_bytes,
            config.format == "none",
        );
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(Writer(delivery.clone()))
            .event_format(Formatter {
                console: config.format == "console",
                color,
            })
            .finish();
        let core = Arc::new(Core {
            delivery,
            clock: config.clock,
            level,
            resource: r,
            dispatch: tracing::Dispatch::new(subscriber),
        });
        Ok(Self {
            log: Logger {
                core,
                fields: Fields::new(),
                scope: None,
                trace: None,
                error: None,
            },
        })
    }
    pub fn stats(&self) -> Stats {
        self.log.core.delivery.stats()
    }
    pub fn close(&self, timeout: Duration) -> Result<(), Failure> {
        self.log.core.delivery.close(timeout)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.log.core.delivery.stop();
    }
}
