//! Optional OpenTelemetry adapters can consume these bounded events; no IDs, URLs or payloads.
use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Started,
    Succeeded,
    Failed,
    Retry,
    Registered,
    Received,
    Heartbeat,
    Completed,
    Stale,
    Reconnect,
    ActivityDuration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdkTransport {
    Rest,
    Grpc,
}
#[derive(Clone, Copy, Debug)]
pub struct SdkEvent {
    pub kind: EventKind,
    pub operation: &'static str,
    pub transport: SdkTransport,
    pub duration: Option<Duration>,
    pub code: Option<&'static str>,
}
type TraceContext = dyn Fn() -> Option<String> + Send + Sync;
type Observer = dyn Fn(SdkEvent) + Send + Sync;
#[derive(Clone, Default)]
pub struct Telemetry {
    pub trace_context: Option<Arc<TraceContext>>,
    pub on_event: Option<Arc<Observer>>,
}
impl std::fmt::Debug for Telemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Telemetry")
            .field("trace_context", &self.trace_context.is_some())
            .field("observer", &self.on_event.is_some())
            .finish()
    }
}
impl Telemetry {
    pub(crate) fn trace(&self, explicit: Option<String>) -> Option<String> {
        if explicit.is_some() {
            return explicit;
        }
        self.trace_context
            .as_ref()
            .and_then(|hook| {
                std::panic::catch_unwind(AssertUnwindSafe(|| hook()))
                    .ok()
                    .flatten()
            })
            .filter(|value| crate::client::valid_traceparent(value))
    }
    pub(crate) fn emit(
        &self,
        kind: EventKind,
        operation: &'static str,
        transport: SdkTransport,
        duration: Option<Duration>,
        code: Option<&'static str>,
    ) {
        if let Some(hook) = &self.on_event {
            let event = SdkEvent {
                kind,
                operation,
                transport,
                duration,
                code,
            };
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| hook(event)));
        }
    }
}
