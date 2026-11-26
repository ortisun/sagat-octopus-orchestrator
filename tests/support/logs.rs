//! Captures the log output of the current test so it can be asserted on.
//!
//! The subscriber is installed for the current thread only, which covers everything a
//! `#[tokio::test]` runs (its runtime is single-threaded), and is removed when the guard drops.
#![allow(dead_code, clippy::expect_used)]

use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

/// Everything logged at `WARN` or above while the guard from [`capture`] is alive.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log buffer")).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

/// Keeps two subscribers that are interested in nothing registered for the life of the process, so that
/// `tracing-core` never sees a single registered subscriber: with one, it computes the interest of a
/// call site from the default subscriber of whichever thread reaches the call site first (its
/// `has_just_one` shortcut), which on another test's thread is no subscriber at all, and the call site
/// is then silent for a capture that is listening on this thread (D2341). With two or more, every live
/// subscriber is asked, under the lock of the registry. Every subscriber a test installs on its
/// thread calls this first; it is idempotent.
pub fn keep_interest_shared() {
    static KEPT: std::sync::OnceLock<[tracing::Dispatch; 2]> = std::sync::OnceLock::new();
    KEPT.get_or_init(|| [tracing::Dispatch::new(Deaf), tracing::Dispatch::new(Deaf)]);
}

/// A subscriber interested in nothing, at no level: it changes neither the interest of a call site
/// nor the most verbose level the process records.
struct Deaf;

impl tracing::Subscriber for Deaf {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::never()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::OFF)
    }

    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Starts capturing warnings and errors; keep the guard alive for the duration of the test.
///
/// Call it as late as the test allows (after the slow set-up, right before the action whose log is
/// asserted). `tracing` caches, per call site, whether any subscriber is interested, and the cache
/// is computed when a call site is first reached, on whatever thread reaches it. While one scoped
/// subscriber alone is registered in the process, `tracing-core` 0.1 takes a shortcut and asks only
/// the default subscriber *of the thread that reaches the call site*: another test's thread, with
/// none, caches the call site as "nobody is interested", and this capture never hears it again
/// (`the_prefix_and_the_reason_of_a_purged_job_reach_no_log_span_metric_or_audit_record_and_are_gone_afterwards`
/// missed the purge's own line under the whole suite, D2341). [`keep_interest_shared`] closes that
/// shortcut for the life of the process, and the capture rebuilds the cache once it is installed.
pub fn capture() -> (DefaultGuard, Captured) {
    capture_at(tracing::Level::WARN)
}

/// Like [`capture`], for everything at `level` and above (`TRACE` captures every line).
pub fn capture_at(level: tracing::Level) -> (DefaultGuard, Captured) {
    keep_interest_shared();
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(level)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    (guard, captured)
}

/// Like [`capture_at`], and every span is written out with its fields when it is created and when it
/// closes, whether or not an event happens inside it: what a span carries is what a trace exporter
/// would send, so a test can search the spans of a flow for a value that must not be in them.
pub fn capture_with_spans(level: tracing::Level) -> (DefaultGuard, Captured) {
    keep_interest_shared();
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(level)
        .with_span_events(
            tracing_subscriber::fmt::format::FmtSpan::NEW
                | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
        )
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    (guard, captured)
}
