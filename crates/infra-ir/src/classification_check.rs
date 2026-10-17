//! Classification publication checks shared with the schema adapter.

use sagat_application::classification::{
    ClassificationAnalysis, ClassificationLocation, ClassificationWalkStatus,
};
use serde_json::Value;

#[must_use]
pub fn check(document: &Value) -> ClassificationAnalysis {
    sagat_infra_schema::classification::analyze_value(document)
}

#[must_use]
pub fn visit_locations(
    document: &Value,
    mut visitor: impl FnMut(&ClassificationLocation),
) -> ClassificationWalkStatus {
    sagat_infra_schema::classification::visit_classification_locations(document, &mut visitor)
}
