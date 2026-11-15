//! The `inputSchema` of a document as the parsed definition carries it (design 043, T043-1): its canonical bytes and their
//! SHA-256, computed here once at parse so the domain only stores them (D043-075).

use sagat_domain::workflow::{InputSchema, WorkflowDefinition};
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
