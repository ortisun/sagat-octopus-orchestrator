//! The classification of one document's locations, read once (feature 016's carriers, FR-016-002, FR-016-003): the
//! level a value at a location of a carrier can carry, everything under it included. Shared by the readings that need
//! it: the fields an Iceberg write sends out (feature 035, `lake_egress`) and what a flow-control key reads (feature 022,
//! `flow_control`, decision D022-085).

use std::collections::HashMap;

use sagat_application::{
    classification::{CarrierPointer, ClassificationResolution, ClassificationSegment},
    classification_index::ClassificationLocationIndex,
};
use sagat_domain::classification::Level;
use serde_json::Value;

/// The classification of one document, read once.
pub(crate) struct Schemas<'a> {
    document: &'a Value,
    pub(crate) index: ClassificationLocationIndex,
    /// The strictest level at or below each location, so that a question about a structure is a lookup, not a scan.
    below: HashMap<(CarrierPointer, Vec<ClassificationSegment>), Level>,
}

impl<'a> Schemas<'a> {
    pub(crate) fn new(document: &'a Value) -> Self {
        let index = ClassificationLocationIndex::build(|visitor| {
            crate::classification_check::visit_locations(document, visitor)
        });
        let mut below: HashMap<(CarrierPointer, Vec<ClassificationSegment>), Level> =
            HashMap::new();
        for location in index.locations() {
            for depth in 0..=location.location.len() {
                let key = (
                    location.carrier.clone(),
                    location.location[..depth].to_vec(),
                );
                below
                    .entry(key)
                    .and_modify(|level| *level = level.stricter(location.level))
                    .or_insert(location.level);
            }
        }
        Self {
            document,
            index,
            below,
        }
    }

    /// The level a value at `location` can carry, and `None` when the carrier says nothing a caller can rely on.
    pub(crate) fn level_at(
        &self,
        carrier: &CarrierPointer,
        location: &[ClassificationSegment],
    ) -> Option<Level> {
        match self.index.resolve_matched(carrier, location) {
            (
                ClassificationResolution::Resolved {
                    level, source_path, ..
                },
                Some(matched),
            ) => {
                // Everything below the location that answered, including a location that governs a name or an item the
                // request did not spell out.
                let below = self
                    .below
                    .get(&(carrier.clone(), matched))
                    .copied()
                    .unwrap_or(level);
                Some(if self.is_scalar(&source_path) {
                    below
                } else {
                    below.stricter(Level::UNMARKED)
                })
            }
            // Nothing is described there: the answer is the floor of the nearest named ancestor.
            (ClassificationResolution::Resolved { level, .. }, None) => Some(level),
            _ => None,
        }
    }

    /// Whether the schema that governs a location says its value is a scalar (and so has no keys or elements the schema
    /// does not name). A schema that cannot be found, or that says nothing of its type, is not.
    fn is_scalar(&self, schema_path: &str) -> bool {
        const SCALARS: [&str; 5] = ["string", "integer", "number", "boolean", "null"];
        let Some(kind) = self
            .document
            .pointer(schema_path)
            .and_then(|schema| schema.get("type"))
        else {
            return false;
        };
        match kind {
            Value::String(name) => SCALARS.contains(&name.as_str()),
            Value::Array(names) => {
                !names.is_empty()
                    && names
                        .iter()
                        .all(|name| name.as_str().is_some_and(|name| SCALARS.contains(&name)))
            }
            _ => false,
        }
    }
}
