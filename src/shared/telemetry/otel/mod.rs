//! Concrete SDK providers; root constructs and closes these outside Tokio tasks.
use crate::shared::errors::{Failure, Kind};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{LogExporter, MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    logs::{BatchConfigBuilder as LogBatch, BatchLogProcessor, SdkLoggerProvider},
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::{BatchConfigBuilder as TraceBatch, BatchSpanProcessor, Sampler, SdkTracerProvider},
};
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};
pub struct Config {
    pub mode: String,
    pub endpoint: String,
    pub sampling: String,
    pub resource: BTreeMap<String, String>,
    pub queue: usize,
    pub batch: usize,
    pub timeout: Duration,
    pub interval: Duration,
}
impl Config {
    pub fn validate(&self) -> Result<(), Failure> {
        let u = url::Url::parse(&self.endpoint).map_err(|_| invalid())?;
        if !matches!(self.mode.as_str(), "none" | "otlp")
            || !matches!(self.sampling.as_str(), "all" | "none")
            || !matches!(u.scheme(), "http" | "https")
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || self
                .resource
                .get("service.name")
                .is_none_or(String::is_empty)
            || self
                .resource
                .get("service.instance.id")
                .is_none_or(String::is_empty)
            || self.queue < 1
            || self.queue > 65536
            || self.batch < 1
            || self.batch > self.queue
            || self.timeout < Duration::from_millis(1)
            || self.timeout > Duration::from_secs(10)
            || self.interval < Duration::from_millis(100)
            || self.interval > Duration::from_secs(60)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid telemetry configuration").with_type("env.invalid")
}
fn unavailable() -> Failure {
    Failure::new(Kind::Unavailable, "telemetry export unavailable").with_type("telemetry.export")
}
pub struct Runtime {
    pub traces: Option<SdkTracerProvider>,
    pub metrics: Option<SdkMeterProvider>,
    pub logs: Option<SdkLoggerProvider>,
    closed: Mutex<Option<Result<(), Kind>>>,
}
impl Runtime {
    pub fn new(c: Config) -> Result<Self, Failure> {
        c.validate()?;
        let mut out = Self {
            traces: None,
            metrics: None,
            logs: None,
            closed: Mutex::new(None),
        };
        if c.mode == "none" {
            return Ok(out);
        }
        let resource = Resource::builder_empty()
            .with_attributes(c.resource.into_iter().map(|(k, v)| KeyValue::new(k, v)))
            .build();
        let base = c.endpoint.trim_end_matches('/');
        let exporter = SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!("{base}/v1/traces"))
            .with_timeout(c.timeout)
            .build()
            .map_err(|_| unavailable())?;
        let processor = BatchSpanProcessor::builder(exporter)
            .with_batch_config(
                TraceBatch::default()
                    .with_max_queue_size(c.queue)
                    .with_max_export_batch_size(c.batch)
                    .with_scheduled_delay(c.interval)
                    .build(),
            )
            .build();
        out.traces = Some(
            SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_sampler(if c.sampling == "all" {
                    Sampler::AlwaysOn
                } else {
                    Sampler::AlwaysOff
                })
                .with_span_processor(processor)
                .build(),
        );
        let exporter = MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!("{base}/v1/metrics"))
            .with_timeout(c.timeout)
            .build()
            .map_err(|_| unavailable())?;
        out.metrics = Some(
            SdkMeterProvider::builder()
                .with_resource(resource.clone())
                .with_reader(
                    PeriodicReader::builder(exporter)
                        .with_interval(c.interval)
                        .build(),
                )
                .build(),
        );
        let exporter = LogExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!("{base}/v1/logs"))
            .with_timeout(c.timeout)
            .build()
            .map_err(|_| unavailable())?;
        let processor = BatchLogProcessor::builder(exporter)
            .with_batch_config(
                LogBatch::default()
                    .with_max_queue_size(c.queue)
                    .with_max_export_batch_size(c.batch)
                    .with_scheduled_delay(c.interval)
                    .build(),
            )
            .build();
        out.logs = Some(
            SdkLoggerProvider::builder()
                .with_resource(resource)
                .with_log_processor(processor)
                .build(),
        );
        Ok(out)
    }
    pub fn close(&self, budget: Duration) -> Result<(), Failure> {
        let mut closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(result) = &*closed {
            return (*result).map_err(close_failure);
        }
        let start = Instant::now();
        let remaining = || budget.saturating_sub(start.elapsed());
        let mut result = Ok(());
        if let Some(p) = &self.traces {
            if p.shutdown_with_timeout(remaining()).is_err() {
                result = Err(Kind::Unavailable);
            }
        }
        if let Some(p) = &self.metrics {
            if p.shutdown_with_timeout(remaining()).is_err() {
                result = Err(Kind::Unavailable);
            }
        }
        if let Some(p) = &self.logs {
            if p.shutdown_with_timeout(remaining()).is_err() {
                result = Err(Kind::Unavailable);
            }
        }
        if start.elapsed() >= budget {
            result = Err(Kind::Timeout);
        }
        *closed = Some(result);
        result.map_err(close_failure)
    }
}

fn close_failure(kind: Kind) -> Failure {
    if kind == Kind::Timeout {
        Failure::new(kind, "telemetry flush timed out").with_type("telemetry.flush_timeout")
    } else {
        unavailable()
    }
}
