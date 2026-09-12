//! Native OTel request observation. Context is activated per future poll.
use crate::shared::{
    http::Completion,
    telemetry::{TraceRef, otel::Runtime},
};
use opentelemetry::{
    Context, KeyValue,
    logs::{LogRecord, Logger, LoggerProvider, Severity},
    metrics::{Histogram, MeterProvider},
    propagation::{Extractor, TextMapPropagator},
    trace::{SpanKind, Status, TraceContextExt, Tracer, TracerProvider},
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::sync::Arc;
pub struct Observer {
    runtime: Arc<Runtime>,
    duration: Option<Histogram<f64>>,
}
pub struct Observation {
    pub context: Context,
    pub trace: Option<TraceRef>,
    runtime: Arc<Runtime>,
    duration: Option<Histogram<f64>>,
    method: String,
    scheme: String,
}
struct Headers<'a>(&'a axum::http::HeaderMap);
impl Extractor for Headers<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        let mut values = self.0.get_all(key).iter();
        let first = values.next()?;
        if values.next().is_some() {
            return None;
        }
        first.to_str().ok()
    }
    fn keys(&self) -> Vec<&str> {
        vec!["traceparent", "tracestate"]
    }
}
impl Observer {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let duration = runtime.metrics.as_ref().map(|m| {
            m.meter("n2f.http")
                .f64_histogram("http.server.request.duration")
                .with_unit("s")
                .with_boundaries(vec![
                    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1., 2.5, 5., 7.5, 10.,
                ])
                .build()
        });
        Self { runtime, duration }
    }
    pub fn start(
        &self,
        method: &str,
        scheme: &str,
        headers: &axum::http::HeaderMap,
    ) -> Observation {
        let mut context = Context::new();
        let mut trace = None;
        if let Some(provider) = &self.runtime.traces {
            context =
                TraceContextPropagator::new().extract_with_context(&context, &Headers(headers));
            let tracer = provider.tracer("n2f.http");
            let span = tracer.build_with_context(
                tracer
                    .span_builder(if method == "_OTHER" { "HTTP" } else { method }.to_owned())
                    .with_kind(SpanKind::Server),
                &context,
            );
            context = context.with_span(span);
            let sc = context.span().span_context().clone();
            trace = TraceRef::parse(
                &sc.trace_id().to_string(),
                &sc.span_id().to_string(),
                sc.is_sampled(),
            )
            .ok();
        }
        Observation {
            context,
            trace,
            runtime: self.runtime.clone(),
            duration: self.duration.clone(),
            method: method.into(),
            scheme: scheme.into(),
        }
    }
}
impl super::axum::Observation for Observation {
    fn trace(&self) -> Option<&TraceRef> {
        self.trace.as_ref()
    }
    fn instrument(&self, work: super::axum::Work) -> super::axum::Work {
        use opentelemetry::trace::FutureExt;
        Box::pin(work.with_context(self.context.clone()))
    }

    fn finish(
        &self,
        c: &Completion,
        route: Option<&str>,
        scope: Option<&crate::shared::provenance::Scope>,
        failure: Option<&super::axum::RequestFailure>,
    ) {
        let _active = self.context.clone().attach();
        let mut attrs = vec![
            KeyValue::new("http.request.method", self.method.clone()),
            KeyValue::new("url.scheme", self.scheme.clone()),
            KeyValue::new("n2f.outcome", c.classification.outcome.as_str()),
        ];
        let span = self.context.span();
        let method = if self.method == "_OTHER" {
            "HTTP"
        } else {
            &self.method
        };
        if let Some(route) = route {
            attrs.push(KeyValue::new("http.route", route.to_owned()));
            span.update_name(format!("{method} {route}"));
        }
        if let Some(status) = c.facts.status {
            attrs.push(KeyValue::new(
                "http.response.status_code",
                i64::from(status),
            ));
        }
        if let Some(reason) = &c.classification.error_type {
            attrs.push(KeyValue::new("error.type", reason.clone()));
        }
        span.set_attributes(attrs.clone());
        if c.classification.span_error {
            span.set_status(Status::error(""));
        }
        if let Some(duration) = &self.duration {
            duration.record(c.elapsed.as_secs_f64(), &attrs);
        }
        if let Some(provider) = &self.runtime.logs {
            let logger = provider.logger("n2f.http");
            let mut record = logger.create_log_record();
            record.set_timestamp(std::time::SystemTime::now());
            record.set_body("http.request.completed".into());
            record.set_severity_number(match c.classification.outcome.as_str() {
                "failed" => Severity::Error,
                "canceled" | "timed_out" => Severity::Warn,
                _ => Severity::Info,
            });
            let sc = span.span_context();
            record.set_trace_context(sc.trace_id(), sc.span_id(), Some(sc.trace_flags()));
            for attr in attrs {
                match attr.value {
                    opentelemetry::Value::I64(v) => record.add_attribute(attr.key, v),
                    opentelemetry::Value::String(v) => {
                        record.add_attribute(attr.key, v.to_string())
                    }
                    _ => {}
                }
            }
            record.add_attribute("termination", c.facts.termination.as_str());
            record.add_attribute("elapsed_ms", c.elapsed.as_secs_f64() * 1000.);
            if let Some(scope) = scope {
                let s = scope.snapshot();
                record.add_attribute("n2f.scope_id", s.scope_id.to_string());
                record.add_attribute("n2f.work_id", s.work.work_id.to_string());
                record.add_attribute("n2f.correlation_id", s.work.correlation_id.to_string());
                record.add_attribute("n2f.operation", s.work.operation.as_str().to_owned());
            }
            if let Some(failure) = failure {
                use crate::shared::errors::{Classified, public_info};
                let p = public_info(match failure {
                    super::axum::RequestFailure::Known(e) => e.classification(),
                    super::axum::RequestFailure::Unknown => None,
                });
                record.add_attribute("n2f.error.kind", p.kind.as_str());
                record.add_attribute("n2f.error.message", p.message);
                if let Some(code) = p.error_type {
                    record.add_attribute("n2f.error.code", code);
                }
            }
            logger.emit(record);
        }
        span.end();
    }
}

impl super::axum::Observer for Observer {
    fn start(
        &self,
        method: &str,
        scheme: &str,
        headers: &axum::http::HeaderMap,
    ) -> Arc<dyn super::axum::Observation> {
        Arc::new(Observer::start(self, method, scheme, headers))
    }
}
