//! Classification publication checks shared with the schema adapter.

use sagat_application::classification::ClassificationAnalysis;
use serde_json::Value;

#[allow(dead_code)]
#[path = "../../infra-schema/src/classification/publication.rs"]
mod publication;

#[must_use]
pub fn check(document: &Value) -> ClassificationAnalysis {
    publication::analyze_value(document)
}
