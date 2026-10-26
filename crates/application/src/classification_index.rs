//! Effective-level lookup over one authoring walk.
//!
//! A version diff asks for the level of every location of one document in the other. The walk that
//! lists the locations of a document runs once; this index answers each question from its result
//! (one lookup per path segment) instead of walking the document again for it.

use std::collections::HashMap;

use sagat_domain::classification::Level;

use crate::classification::{
    CarrierPointer, ClassificationLocation, ClassificationResolution, ClassificationSegment,
    ClassificationWalkStatus,
};

/// The locations of one document, in the order the walk reported them, and by carrier and
/// semantic path.
#[derive(Debug, Clone, Default)]
pub struct ClassificationLocationIndex {
    ordered: Vec<ClassificationLocation>,
    by_path: HashMap<CarrierPointer, HashMap<Vec<ClassificationSegment>, usize>>,
    status: ClassificationWalkStatus,
}

impl ClassificationLocationIndex {
    /// Runs `walk` once, which reports every location of a document to the callback it is given
    /// and returns the status it ended with. A failed carrier emits no location, so a prefix of
    /// it can never be compared as the whole.
    #[must_use]
    pub fn build(
        walk: impl FnOnce(&mut dyn FnMut(&ClassificationLocation)) -> ClassificationWalkStatus,
    ) -> Self {
        let mut ordered: Vec<ClassificationLocation> = Vec::new();
        let mut by_path: HashMap<CarrierPointer, HashMap<Vec<ClassificationSegment>, usize>> =
            HashMap::new();
        let status = walk(&mut |location| {
            let paths = by_path.entry(location.carrier.clone()).or_default();
            // The first of a repeated path answers lookups.
            paths
                .entry(location.location.clone())
                .or_insert(ordered.len());
            ordered.push(location.clone());
        });
        Self {
            ordered,
            by_path,
            status,
        }
    }

    /// Every location, in the order of the walk (so a bounded list of findings is stable).
    #[must_use]
    pub fn locations(&self) -> &[ClassificationLocation] {
        &self.ordered
    }

    #[must_use]
    pub const fn status(&self) -> &ClassificationWalkStatus {
        &self.status
    }

    /// The effective level of one semantic location, with the fallbacks of the walk: a property
    /// or an index the carrier does not name is governed by `additionalProperties` or by the
    /// remaining items, and by the `pii` floor of the nearest named ancestor when neither exists.
    #[must_use]
    pub fn resolve(
        &self,
        carrier: &CarrierPointer,
        location: &[ClassificationSegment],
    ) -> ClassificationResolution {
        if let Some(failure) = self.status.failure(carrier) {
            return ClassificationResolution::Incomplete(failure.clone());
        }
        let Some(paths) = self.by_path.get(carrier) else {
            return ClassificationResolution::AbsentCarrier;
        };
        let find = |path: &[ClassificationSegment]| paths.get(path).map(|&at| &self.ordered[at]);
        let Some(mut current) = find(&[]) else {
            return ClassificationResolution::AbsentCarrier;
        };
        let mut selected: Vec<ClassificationSegment> = Vec::with_capacity(location.len());
        let mut location_present = true;
        for requested in location {
            selected.push(requested.clone());
            if let Some(found) = find(&selected) {
                current = found;
                continue;
            }
            selected.pop();
            let fallback = match requested {
                ClassificationSegment::Property(_) => ClassificationSegment::AdditionalProperty,
                ClassificationSegment::Index(_) => ClassificationSegment::RemainingItem,
                ClassificationSegment::AdditionalProperty
                | ClassificationSegment::RemainingItem => return floor(current),
            };
            selected.push(fallback);
            if let Some(found) = find(&selected) {
                current = found;
                location_present = false;
            } else {
                return floor(current);
            }
        }
        ClassificationResolution::Resolved {
            level: current.level,
            source_path: current.source_path.clone(),
            location_present,
        }
    }
}

/// The nearest named ancestor, raised to the floor of an unmarked location.
fn floor(nearest: &ClassificationLocation) -> ClassificationResolution {
    ClassificationResolution::Resolved {
        level: nearest.level.stricter(Level::UNMARKED),
        source_path: nearest.source_path.clone(),
        location_present: false,
    }
}
