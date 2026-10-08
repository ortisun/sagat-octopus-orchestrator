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

/// Starts capturing warnings and errors; keep the guard alive for the duration of the test.
pub fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    (tracing::subscriber::set_default(subscriber), captured)
}
