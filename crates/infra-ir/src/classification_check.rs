//! Classification publication checks shared with the schema adapter.

use sagat_application::classification::ClassificationAnalysis;
use serde_json::Value;

#[must_use]
pub fn check(document: &Value) -> ClassificationAnalysis {
    sagat_infra_schema::classification::analyze_value(document)
}
