//! The `inputSchema` of a document as the parsed definition carries it (design 043, T043-1): its canonical bytes and their
//! SHA-256, computed here once at parse so the domain only stores them (D043-075), and the pre-check of the supported
//! subset that publication runs on it (T043-13, FR-043-012).

use sagat_application::definitions::{InputValidationMode, IrError, ParsedDefinition};
use sagat_domain::workflow::{DefinitionWarning, InputSchema, WorkflowDefinition};
use sagat_infra_schema::run_input::PrecheckRefusal;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::canonical;

/// The canonical form of the document's `inputSchema`, when it has one.
pub(crate) fn of(document: &Value) -> Option<InputSchema> {
    let schema = document.get("inputSchema")?;
    let bytes = canonical::canonical_json(schema);
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    Some(InputSchema::new(bytes, digest))
}

/// `definition` with the `inputSchema` of `document`.
pub(crate) fn attach(document: &Value, definition: WorkflowDefinition) -> WorkflowDefinition {
    definition.with_input_schema(of(document))
}

/// The pre-check of the supported subset on a new publication's `inputSchema`, annotated or not (FR-043-012): in
/// `enforce` mode the first violation refuses the document with the keyword and its pointer, in `report` mode it is a
/// warning with the same fields. The restore of a stored version never runs it: a stored version still parses and is
/// refused at start instead (FR-043-011).
pub(crate) fn precheck(
    mut parsed: ParsedDefinition,
    mode: InputValidationMode,
) -> Result<ParsedDefinition, IrError> {
    let Some(schema) = parsed.definition.input_schema() else {
        return Ok(parsed);
    };
    let unusable = match sagat_infra_schema::run_input::precheck(schema.bytes()) {
        Ok(()) => return Ok(parsed),
        // transient in either mode: the document is not judged (D043-177, D043-186)
        Err(PrecheckRefusal::Busy | PrecheckRefusal::Interrupted) => {
            return Err(IrError::ValidatorBusy);
        }
        Err(PrecheckRefusal::Unusable(unusable)) => unusable,
    };
    match mode {
        InputValidationMode::Enforce => Err(IrError::InputSchema {
            issue: unusable.issue,
            keyword: unusable.keyword,
            path: format!("/inputSchema{}", unusable.at),
        }),
        InputValidationMode::Report => {
            parsed
                .warnings
                .push(DefinitionWarning::InputSchemaOutsideSubset {
                    issue: unusable.issue,
                    keyword: unusable.keyword,
                    at: unusable.at,
                });
            Ok(parsed)
        }
    }
}

/// The `inputSchema` of a stored document as a creator would read it, without parsing the rest of the document:
/// `control-plane input-schema-inventory` pre-checks it and reads the kinds of its triggers (T043-24).
///
/// # Errors
/// [`IrError::Malformed`] when the document is not a JSON object.
pub fn stored_input_schema(document: &[u8]) -> Result<StoredInputSchema, IrError> {
    let value: Value = serde_json::from_slice(document).map_err(|_| IrError::Malformed)?;
    if !value.is_object() {
        return Err(IrError::Malformed);
    }
    let triggered = value
        .get("triggers")
        .and_then(Value::as_array)
        .is_some_and(|triggers| {
            triggers
                .iter()
                .any(|trigger| trigger.get("kind").and_then(Value::as_str) != Some("manual"))
        });
    Ok(StoredInputSchema {
        schema: of(&value),
        triggered,
    })
}

/// What [`stored_input_schema`] reads of a stored document.
#[derive(Debug, Clone)]
pub struct StoredInputSchema {
    /// The canonical `inputSchema`, when the document has one.
    pub schema: Option<InputSchema>,
    /// Whether the document has a trigger other than a manual start.
    pub triggered: bool,
}
