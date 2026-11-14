//! The workflow capabilities a deployment turns on one at a time (ADR-0032 decision 8).
//!
//! A canonical schema that describes a field does not enable it: a new publication is refused while the
//! capability its document needs is off, and a version published while it was on is still read after it is
//! turned off, because closing a capability never erases what was published under it. The switch belongs to
//! the process that serves publications, set once when it starts from its configuration, so a document check
//! that runs without a deployment (the CLI, the tests of a codec) sees the default: off.

use std::sync::atomic::{AtomicBool, Ordering};

use sagat_application::definitions::IrError;
use serde_json::Value;

/// Declared Updates that keep their arguments for a reset (`reapplyOnReset`, feature 018).
static RESET_REAPPLY: AtomicBool = AtomicBool::new(false);

/// Turns the `reapplyOnReset` capability on or off for the publications this process parses.
pub fn set_reset_reapply(enabled: bool) {
    RESET_REAPPLY.store(enabled, Ordering::Release);
}

/// Whether a new publication may declare an Update with `reapplyOnReset: true`.
#[must_use]
pub fn reset_reapply_enabled() -> bool {
    RESET_REAPPLY.load(Ordering::Acquire)
}

/// Refuses a document that needs a capability this process has not enabled. It stands on the immutable
/// document, not on a flag the caller sends (ADR-0032 decision 4).
///
/// # Errors
/// [`IrError::UnsupportedDeclaration`] naming the field when it is needed and off.
pub(crate) fn check_publication(document: &Value) -> Result<(), IrError> {
    if !reset_reapply_enabled() && declares_reapply_on_reset(document) {
        return Err(IrError::UnsupportedDeclaration {
            field: "reapplyOnReset",
            reason: "this workflow capability is not enabled by the runtime",
        });
    }
    Ok(())
}

/// Whether any declared Update opts in. A value that is not `true` (a non-boolean is a schema error found
/// before this) does not.
fn declares_reapply_on_reset(document: &Value) -> bool {
    document
        .pointer("/state/updates")
        .and_then(Value::as_object)
        .is_some_and(|updates| {
            updates
                .values()
                .any(|update| update.get("reapplyOnReset") == Some(&Value::Bool(true)))
        })
}
