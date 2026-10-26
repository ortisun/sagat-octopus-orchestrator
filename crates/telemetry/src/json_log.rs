//! The platform's log lines: one JSON object per event, redacted on the way out (FR-008-001,
//! FR-008-006).
//!
//! Every field of the event **and of the spans it happens in** is written on the line, so a line
//! logged inside `scheduler.pass{run_id, step_id, attempt}` carries them without the call site
//! repeating them; the innermost span wins, then the event. Values go through [`crate::redact`]
//! before they are written: a value under a sensitive name is replaced, a credential-shaped string
//! is scrubbed, a long one is cut.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Number, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::redact::{REDACTED, is_sensitive_name, scrub};

/// Keys the line itself owns; a field of that name is written as `fields.<name>` so that it cannot
/// forge the level, the time or the service.
const RESERVED: [&str; 5] = ["timestamp", "level", "target", "message", "service"];

/// The fields of one span, kept on it from creation.
struct SpanFields(Map<String, Value>);

/// Collects fields into a JSON map, redacting as it goes.
#[derive(Default)]
struct Collector(Map<String, Value>);

impl Collector {
    fn put(&mut self, field: &Field, value: Value) {
        self.0.insert(field.name().to_owned(), value);
    }

    fn text(&mut self, field: &Field, text: &str) {
        let value = if is_sensitive_name(field.name()) {
            Value::String(REDACTED.to_owned())
        } else {
            Value::String(scrub(text).into_owned())
        };
        self.put(field, value);
    }
}

impl Visit for Collector {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::Number(value.into()));
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        match i64::try_from(value) {
            Ok(small) => self.put(field, Value::Number(small.into())),
            Err(_) => self.text(field, &value.to_string()),
        }
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        match u64::try_from(value) {
            Ok(small) => self.put(field, Value::Number(small.into())),
            Err(_) => self.text(field, &value.to_string()),
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        match Number::from_f64(value) {
            Some(number) => self.put(field, Value::Number(number)),
            None => self.text(field, &value.to_string()),
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.text(field, &format!("{value:?}"));
    }
}

/// How a line is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// One JSON object per line.
    Json,
    /// `time LEVEL target: message key=value ...`, for a developer's terminal.
    Text,
}

/// A layer that writes the events it sees to `make_writer`.
pub struct LogLayer<W> {
    make_writer: W,
    service: String,
    style: Style,
}

impl<W> LogLayer<W> {
    /// A layer that names `service` on every line.
    pub fn new(make_writer: W, service: impl Into<String>, style: Style) -> Self {
        Self {
            make_writer,
            service: service.into(),
            style,
        }
    }
}

fn now_rfc3339() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i128::try_from(d.as_nanos()).unwrap_or(0));
    time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|at| {
            at.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

impl<S, W> Layer<S> for LogLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + 'static,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut collector = Collector::default();
        attrs.record(&mut collector);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(collector.0));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        if let Some(fields) = extensions.get_mut::<SpanFields>() {
            let mut collector = Collector(std::mem::take(&mut fields.0));
            values.record(&mut collector);
            fields.0 = collector.0;
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // The PostgreSQL client/parser log query text at DEBUG and server notices at INFO. Neither
        // target is safe even when an operator enables RUST_LOG=trace. Normalize bridged `log`
        // records before checking the target; their static tracing target may just be `log`.
        use tracing_log::NormalizeEvent as _;
        let normalized = event.normalized_metadata();
        let target = normalized
            .as_ref()
            .unwrap_or_else(|| event.metadata())
            .target();
        if ["tokio_postgres", "sqlparser"].iter().any(|crate_name| {
            target == *crate_name
                || target
                    .strip_prefix(crate_name)
                    .is_some_and(|suffix| suffix.starts_with("::"))
        }) {
            return;
        }
        let mut line = Map::new();
        // Outermost span first, so that the innermost wins.
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(fields) = span.extensions().get::<SpanFields>() {
                    for (key, value) in &fields.0 {
                        line.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        let mut collector = Collector::default();
        event.record(&mut collector);
        let mut message = None;
        for (key, value) in collector.0 {
            if key == "message" {
                message = Some(value);
            } else {
                line.insert(key, value);
            }
        }
        for reserved in RESERVED {
            if let Some(value) = line.remove(reserved) {
                line.insert(format!("fields.{reserved}"), value);
            }
        }
        let meta = event.metadata();
        let mut out = Map::new();
        out.insert("timestamp".to_owned(), Value::String(now_rfc3339()));
        out.insert("level".to_owned(), Value::String(meta.level().to_string()));
        out.insert("service".to_owned(), Value::String(self.service.clone()));
        out.insert("target".to_owned(), Value::String(meta.target().to_owned()));
        if let Some(message) = message {
            out.insert("message".to_owned(), message);
        }
        out.extend(line);
        let mut bytes = match self.style {
            Style::Json => serde_json::to_vec(&Value::Object(out)).unwrap_or_default(),
            Style::Text => text_line(&out).into_bytes(),
        };
        bytes.push(b'\n');
        let mut writer = self.make_writer.make_writer();
        // A log line that cannot be written is not worth failing the work that logged it.
        let _ = writer.write_all(&bytes);
    }
}

/// `text` with the characters that could end or fake a line written as escapes.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

fn text_line(fields: &Map<String, Value>) -> String {
    let text = |key: &str| match fields.get(key) {
        Some(Value::String(s)) => one_line(s),
        _ => String::new(),
    };
    let mut line = format!(
        "{} {:>5} {}: {}",
        text("timestamp"),
        text("level"),
        text("target"),
        text("message")
    );
    for (key, value) in fields {
        if matches!(key.as_str(), "timestamp" | "level" | "target" | "message") {
            continue;
        }
        match value {
            Value::String(s) => line.push_str(&format!(" {}={}", one_line(key), one_line(s))),
            other => line.push_str(&format!(" {}={other}", one_line(key))),
        }
    }
    line
}
