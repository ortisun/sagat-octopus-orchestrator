//! The `inputSchema` of a document as the parsed definition carries it (design 043, T043-1): its canonical bytes and their
//! SHA-256, computed here once at parse so the domain only stores them (D043-075), and the pre-check of the supported
//! subset that publication runs on it (T043-13, FR-043-012).

use sagat_application::definitions::{InputValidationMode, IrError, ParsedDefinition};
use sagat_domain::workflow::{DefinitionWarning, InputSchema, WorkflowDefinition};
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
    let Err(unusable) = sagat_infra_schema::run_input::precheck(schema.bytes()) else {
        return Ok(parsed);
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
