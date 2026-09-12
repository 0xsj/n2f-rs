use std::fmt;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    registry::LookupSpan,
};
pub(super) struct Formatter {
    pub console: bool,
    pub color: bool,
}
#[derive(Default)]
struct Record {
    encoded: Option<String>,
}
impl Visit for Record {
    fn record_str(&mut self, f: &Field, v: &str) {
        if f.name() == "n2f_record" {
            self.encoded = Some(v.into());
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn fmt::Debug) {}
}
fn escape(s: &str) -> String {
    let s = serde_json::to_string(s).expect("string encoding");
    console_controls(&s[1..s.len() - 1])
}
fn console_controls(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if ('\u{7f}'..='\u{9f}').contains(&c) || c == '\u{2028}' || c == '\u{2029}' {
            out.push_str(&format!("\\u{:04x}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}
impl<S, N> FormatEvent<S, N> for Formatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut record = Record::default();
        event.record(&mut record);
        let Some(encoded) = record.encoded else {
            return Ok(());
        };
        if !self.console {
            return writeln!(writer, "{encoded}");
        }
        let mut v: serde_json::Value = serde_json::from_str(&encoded).map_err(|_| fmt::Error)?;
        let ms = v["timestamp_ms"].as_u64().ok_or(fmt::Error)?;
        let level = v["level"].as_str().ok_or(fmt::Error)?;
        let mut severity = level.to_uppercase();
        if self.color {
            severity = format!(
                "\x1b[{}m{severity}\x1b[0m",
                if level == "error" {
                    31
                } else if level == "warn" {
                    33
                } else {
                    36
                }
            );
        }
        let name = escape(v["service"]["name"].as_str().ok_or(fmt::Error)?);
        let message = escape(v["message"].as_str().ok_or(fmt::Error)?);
        let mut details = serde_json::json!({"fields":v["fields"].take()});
        for k in ["scope", "error", "trace"] {
            if v.get(k).is_some() {
                details[k] = v[k].take();
            }
        }
        let details = console_controls(&details.to_string());
        writeln!(
            writer,
            "{:02}:{:02}:{:02}.{:03} {severity} {name} {message} {details}",
            (ms / 3600000) % 24,
            (ms / 60000) % 60,
            (ms / 1000) % 60,
            ms % 1000
        )
    }
}
