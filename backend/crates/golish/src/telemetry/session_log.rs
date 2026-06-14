//! Per-run tracing log layer (design 2026-06-14-per-run-debug-log).
//!
//! The global `~/.golish/backend.log` interleaves every session, which makes
//! debugging one run painful. This `tracing` layer routes each event to a
//! per-run `run.log` co-located with that run's `transcript.json`
//! (`{transcripts_base}/{session}/run.log`), so a single run's full trace —
//! main agent + every sub-agent + tool calls + harness/gate decisions — lands
//! in one AI-readable file.
//!
//! How routing works: the agentic loop tags its `chat_message` / `agent` spans
//! with `langfuse.session.id`, and sub-agent spans are children of that tree, so
//! walking an event's span scope yields the owning session for main and
//! sub-agent events alike. Events with no session in scope (startup, etc.) are
//! skipped here — they still reach `backend.log`.
//!
//! Best-effort + non-fatal: a file/IO error drops the line, never the event.

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::sync::Mutex;

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Span fields that carry the chat session id (first match wins).
const SESSION_FIELDS: [&str; 2] = ["langfuse.session.id", "session_id"];

/// Cap on simultaneously-open per-session log files (FD hygiene).
const MAX_OPEN_FILES: usize = 64;

/// Stored in a span's extensions once we learn its session id.
#[derive(Clone)]
struct SpanSessionId(String);

/// Captures a session id from span attributes.
#[derive(Default)]
struct SessionVisitor(Option<String>);

impl SessionVisitor {
    fn consider(&mut self, field: &Field, value: &str) {
        if self.0.is_none() && SESSION_FIELDS.contains(&field.name()) {
            let trimmed = value.trim().trim_matches('"');
            if !trimmed.is_empty() {
                self.0 = Some(trimmed.to_string());
            }
        }
    }
}

impl Visit for SessionVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.consider(field, value);
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if self.0.is_none() && SESSION_FIELDS.contains(&field.name()) {
            self.consider(field, &format!("{value:?}"));
        }
    }
}

/// Renders an event's `message` + remaining fields into one line fragment.
#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: String,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={}", field.name(), value);
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }
}

/// A `tracing` layer that appends each session-scoped event to that run's
/// `run.log`. Add it to the subscriber after the global `EnvFilter` so it only
/// sees the same events as `backend.log`.
pub struct SessionLogLayer {
    writers: Mutex<HashMap<String, File>>,
}

impl SessionLogLayer {
    pub fn new() -> Self {
        Self {
            writers: Mutex::new(HashMap::new()),
        }
    }

    fn append(&self, session: &str, line: &str) {
        let Ok(mut writers) = self.writers.lock() else {
            return;
        };
        if !writers.contains_key(session) {
            // FD hygiene: evict an arbitrary entry once the cache is full.
            if writers.len() >= MAX_OPEN_FILES {
                if let Some(key) = writers.keys().next().cloned() {
                    writers.remove(&key);
                }
            }
            let dir = golish_events::op_trace::active_transcript_base_or_home().join(session);
            if std::fs::create_dir_all(&dir).is_err() {
                return;
            }
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("run.log"))
            {
                Ok(file) => {
                    writers.insert(session.to_string(), file);
                }
                Err(_) => return,
            }
        }
        if let Some(file) = writers.get_mut(session) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

impl Default for SessionLogLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for SessionLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let mut visitor = SessionVisitor::default();
        attrs.record(&mut visitor);
        if let Some(session) = visitor.0 {
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(SpanSessionId(session));
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Resolve the owning session by walking the event's span scope.
        let mut session: Option<String> = None;
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                if let Some(SpanSessionId(s)) = span.extensions().get::<SpanSessionId>() {
                    session = Some(s.clone());
                    break;
                }
            }
        }
        let Some(session) = session else {
            return;
        };

        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        // Span path (root → leaf) for correlation, e.g. chat_message:agent:sub_agent.
        let mut span_path = String::new();
        if let Some(scope) = ctx.event_scope(event) {
            let names: Vec<&str> = scope.from_root().map(|s| s.name()).collect();
            span_path = names.join(":");
        }

        let meta = event.metadata();
        let span_segment = if span_path.is_empty() {
            String::new()
        } else {
            format!(" [{span_path}]")
        };
        let line = format!(
            "{ts} {level:<5} {target}{span_segment} {message}{fields}\n",
            ts = chrono::Utc::now().to_rfc3339(),
            level = meta.level().as_str(),
            target = meta.target(),
            message = visitor.message,
            fields = visitor.fields,
        );
        self.append(&session, &line);
    }
}
