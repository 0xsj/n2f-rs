//! Concrete client observation; SDK context stays at this adapter boundary.
use super::{Client as Native, Request, Response};
use crate::shared::{
    errors::{Classified, Failure},
    telemetry::{TraceRef, otel::Runtime},
};
use opentelemetry::{
    Context, KeyValue,
    metrics::{Histogram, MeterProvider},
    trace::{FutureExt, SpanKind, Status, TraceContextExt, Tracer, TracerProvider},
};
use std::{sync::Arc, time::Instant};
pub struct Client {
    native: Arc<Native>,
    runtime: Arc<Runtime>,
    duration: Option<Histogram<f64>>,
}
impl Client {
    pub fn new(native: Arc<Native>, runtime: Arc<Runtime>) -> Self {
        let duration = runtime.metrics.as_ref().map(|p| {
            p.meter("n2f.httpclient")
                .f64_histogram("http.client.request.duration")
                .with_unit("s")
                .build()
        });
        Self {
            native,
            runtime,
            duration,
        }
    }
    pub async fn execute(&self, mut input: Request) -> Result<Response, Failure> {
        let Some(provider) = &self.runtime.traces else {
            return self.native.execute(input).await;
        };
        let tracer = provider.tracer("n2f.httpclient");
        let span = tracer.build_with_context(
            tracer
                .span_builder("HTTP outbound")
                .with_kind(SpanKind::Client),
            &Context::current(),
        );
        let child = Context::current().with_span(span);
        let sc = child.span().span_context().clone();
        input.trace = TraceRef::parse(
            &sc.trace_id().to_string(),
            &sc.span_id().to_string(),
            sc.is_sampled(),
        )
        .ok();
        let method = if matches!(
            input.method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
        ) {
            input.method.clone()
        } else {
            "_OTHER".into()
        };
        let started = Instant::now();
        let result = self.native.execute(input).with_context(child.clone()).await;
        let mut attrs = vec![KeyValue::new("http.request.method", method)];
        let error_type = match &result {
            Err(e) => Some(
                e.classification()
                    .map(|c| c.kind.as_str())
                    .unwrap_or("internal")
                    .to_owned(),
            ),
            Ok(r) => {
                attrs.push(KeyValue::new(
                    "http.response.status_code",
                    i64::from(r.status),
                ));
                if r.status >= 400 {
                    Some(r.status.to_string())
                } else {
                    None
                }
            }
        };
        if let Some(e) = error_type {
            child.span().set_status(Status::error(""));
            attrs.push(KeyValue::new("error.type", e));
        }
        child.span().set_attributes(attrs.clone());
        {
            let _active = child.clone().attach();
            if let Some(h) = &self.duration {
                h.record(started.elapsed().as_secs_f64(), &attrs);
            }
        }
        child.span().end();
        result
    }
}
