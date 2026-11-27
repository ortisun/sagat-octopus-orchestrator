//! What the harness adds for the step-log scenarios of design 041 (task T041-14; README, "Step logs"): the rule of
//! the pending languages, the search behind `mustNotStore`, the comparison of a line's exception through the
//! scenario's `exceptionTexts`, and the capture of the server's own output while such a scenario plays.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

/// The operations new to design 041 that a runner refuses until its step-log task lands.
pub const STEP_LOG_OPS: [&str; 2] = ["stepLogs", "releaseAnswer"];

/// The languages a scenario may name as pending.
pub const LANGUAGES: [&str; 6] = ["typescript", "go", "python", "dotnet", "java", "rust"];

/// The languages a scenario lists under `pendingLanguages`, or the names that are not languages.
pub fn pending_languages(scenario: &Value) -> Result<Vec<String>, String> {
    let mut pending = Vec::new();
    for value in scenario["pendingLanguages"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(name) = value.as_str().filter(|name| LANGUAGES.contains(name)) else {
            return Err(format!(
                "pendingLanguages names {value}, which is not one of {LANGUAGES:?}"
            ));
        };
        pending.push(name.to_owned());
    }
    Ok(pending)
}

/// The first operation new to design 041 that the scenario plays.
fn first_step_log_op(scenario: &Value) -> Option<&str> {
    scenario["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|step| step["op"].as_str())
        .find(|op| STEP_LOG_OPS.contains(op))
}

/// Whether a pending language refused the scenario's first step-log operation exactly as its runner refuses an
/// unknown operation today (decision D041-162): Go and Rust end the run with their message on the standard error,
/// the four others record the step as failed with the class of a plain error. `played` is what `run.sh run`
/// returned (its standard output and error, or the failure with the standard error), `out` the runner's output when
/// it wrote one.
pub fn pending_refusal(
    lang: &str,
    scenario: &Value,
    played: &Result<(String, String), String>,
    out: Option<&Value>,
) -> Result<(), String> {
    let Some(op) = first_step_log_op(scenario) else {
        return Err(
            "the scenario lists pendingLanguages but plays no step-log operation".to_owned(),
        );
    };
    let stopped_with = |text: &str| match played {
        Err(failure) if failure.contains(text) => Ok(()),
        Err(failure) => Err(format!(
            "listed as pending, its runner failed without refusing {op} as it does today: {}",
            failure.chars().take(400).collect::<String>()
        )),
        Ok(_) => Err(format!(
            "listed as pending, its runner did not refuse {op}: if its step-log task has landed, remove it from \
             pendingLanguages"
        )),
    };
    let recorded = |class: &str, stderr_holds: Option<String>| {
        let (_, stderr) = played.as_ref().map_err(|failure| {
            format!(
                "listed as pending, its runner failed instead of recording {op} as refused: {}",
                failure.chars().take(400).collect::<String>()
            )
        })?;
        let observation = out
            .and_then(|out| out["observations"].as_array())
            .and_then(|all| all.iter().find(|o| o["op"] == op))
            .ok_or_else(|| format!("listed as pending, its runner recorded no {op} step"))?;
        let refused = serde_json::json!({"code": "UNEXPECTED", "message": class});
        if observation["ok"] != false || observation["error"] != refused {
            return Err(format!(
                "listed as pending, its runner did not refuse {op} as it does today ({refused}), it recorded {}: if its \
                 step-log task has landed, remove it from pendingLanguages",
                serde_json::json!({"ok": observation["ok"], "error": observation["error"]})
            ));
        }
        if let Some(text) = stderr_holds
            && !stderr.contains(&text)
        {
            return Err(format!(
                "listed as pending, its runner's standard error does not hold \"{text}\""
            ));
        }
        Ok(())
    };
    match lang {
        "go" => stopped_with(&format!("unknown step op \"{op}\"")),
        "rust" => stopped_with(&format!("scenario operation {op} is not implemented yet")),
        "typescript" => recorded("Error", None),
        "python" => recorded("ValueError", Some(format!("unknown step op {op}"))),
        "dotnet" => recorded("InvalidOperationException", None),
        "java" => recorded("IllegalArgumentException", None),
        other => Err(format!("{other} is not a language of the suite")),
    }
}

/// The texts of a scenario's list `key` (`mustNotStore`, `exceptionTexts`), with `{{unique}}` replaced as the
/// runner replaced it in every string of the scenario.
pub fn texts(scenario: &Value, key: &str, unique: &str) -> Vec<String> {
    scenario[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| text.replace("{{unique}}", unique))
        .collect()
}

/// Where a text that must not be stored appears: one entry per source and text, naming the source and never
/// repeating more of the text than its first characters. `sources` are (name, gathered text).
pub fn stored_appearances(sources: &[(String, String)], markers: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for marker in markers {
        for (source, text) in sources {
            if text.contains(marker.as_str()) {
                found.push(format!(
                    "{source} holds \"{}…\", which the scenario lists under mustNotStore",
                    marker.chars().take(4).collect::<String>()
                ));
            }
        }
    }
    found
}

/// Replaces the `exception` of every line a `stepLogs` step read by the list of the scenario's `exceptionTexts` it
/// holds, in the list's order: the stack a language prints is its own, the texts the scenario raised are not.
pub fn compare_exceptions_by_texts(out: &mut Value, held: &[String]) {
    for observation in out["observations"].as_array_mut().into_iter().flatten() {
        if observation["op"] != "stepLogs" {
            continue;
        }
        for line in observation["result"]["lines"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            if let Some(text) = line["exception"].as_str().map(str::to_owned) {
                line["exception"] = Value::Array(
                    held.iter()
                        .filter(|marker| text.contains(marker.as_str()))
                        .map(|marker| Value::String(marker.clone()))
                        .collect(),
                );
            }
        }
    }
}

/// The most of the server's output a scenario keeps: past it, the scenario fails rather than search a part.
const MAX_CAPTURED_BYTES: usize = 256 * 1024 * 1024;

/// The server's own output, captured in this process while a scenario that lists `mustNotStore` plays.
struct Capture {
    on: AtomicBool,
    bytes: Mutex<Vec<u8>>,
    overflowed: AtomicBool,
}

#[derive(Clone)]
struct Writer(Arc<Capture>);

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = self
            .0
            .bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if bytes.len() + buf.len() > MAX_CAPTURED_BYTES {
            self.0.overflowed.store(true, Ordering::Relaxed);
        } else {
            bytes.extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The capture, installed once as the process's `tracing` subscriber: every event of every target at the `trace`
/// level, written only while a scenario asks for it (a filter evaluated at every event, so the rest of the suite
/// pays no formatting). `Err` when another subscriber was installed first: the scenario then fails, since a leak
/// check that cannot see the server's output proves nothing.
fn capture() -> Result<Arc<Capture>, String> {
    static INSTALLED: OnceLock<Result<Arc<Capture>, String>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            use tracing_subscriber::Layer;
            use tracing_subscriber::layer::SubscriberExt;
            let capture = Arc::new(Capture {
                on: AtomicBool::new(false),
                bytes: Mutex::new(Vec::new()),
                overflowed: AtomicBool::new(false),
            });
            let writing = Arc::clone(&capture);
            let make = move || Writer(Arc::clone(&writing));
            let filtered = Arc::clone(&capture);
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(make)
                .with_ansi(false)
                .with_filter(tracing_subscriber::filter::filter_fn(move |_| {
                    filtered.on.load(Ordering::Relaxed)
                }));
            tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
                .map(|()| capture)
                .map_err(|e| format!("the server's output cannot be captured: {e}"))
        })
        .clone()
}

/// Starts capturing the server's output, from empty.
pub fn start_capture() -> Result<(), String> {
    let capture = capture()?;
    capture
        .bytes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    capture.overflowed.store(false, Ordering::Relaxed);
    capture.on.store(true, Ordering::Relaxed);
    Ok(())
}

/// Stops capturing and hands back what was captured, or why it cannot be searched whole.
pub fn finish_capture() -> Result<String, String> {
    let capture = capture()?;
    capture.on.store(false, Ordering::Relaxed);
    let bytes = std::mem::take(
        &mut *capture
            .bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    if capture.overflowed.load(Ordering::Relaxed) {
        return Err(format!(
            "the server's output passed {MAX_CAPTURED_BYTES} bytes and cannot be searched whole"
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
