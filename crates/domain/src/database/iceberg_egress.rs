//! What an `icebergWrite` step lets leave the platform, level by level (feature 035: FR-035-005, FR-035-025, D035-051).
//!
//! Pure data and one comparison. Whoever reads the schemas of the source (the IR codec) decides the level of each
//! written field and attaches the list to the step; the publication check and the run admission read it back and
//! compare it with the ceiling of the connection. A step that carries no list proves nothing, so nothing leaves.

use super::WriteMode;
use crate::classification::{EgressCeiling, Level, within};

/// The level one written field carries by the time it leaves.
///
/// `path` is the JSON pointer of the field in a record, as the step's mapping names it; it is empty when the step lists
/// no columns and the whole record is written. A path names a place, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressField {
    path: String,
    level: Level,
}

impl EgressField {
    #[must_use]
    pub fn new(path: impl Into<String>, level: Level) -> Self {
        Self {
            path: path.into(),
            level,
        }
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn level(&self) -> Level {
        self.level
    }
}

/// Why a step may not write to a connection (the contract code is `CLASSIFICATION_EGRESS_REFUSED`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressRefusal {
    /// Nobody classified the fields of this step, so no ceiling can vouch for them.
    NotComputed,
    /// A field above the ceiling, or `secret`. The path and the class are reported, never the value.
    Field { path: String, level: Level },
}

/// Why the settings of a connection refuse a step (the contract codes are `MODE_NOT_ALLOWED` and
/// `CLASSIFICATION_EGRESS_REFUSED`). It names a mode or the paths and classes of fields, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LakeRefusal {
    /// The connection does not allow the mode the step writes in.
    ModeNotAllowed(WriteMode),
    /// Fields the ceiling of the connection does not let leave, or that nobody classified.
    Egress(Vec<EgressRefusal>),
}

/// Every field that may not leave through a connection whose ceiling is `ceiling`, in the order of `fields`.
///
/// `None` means the levels were never computed: that is one refusal of the whole step, not an empty list.
#[must_use]
pub fn refusals(fields: Option<&[EgressField]>, ceiling: EgressCeiling) -> Vec<EgressRefusal> {
    let Some(fields) = fields else {
        return vec![EgressRefusal::NotComputed];
    };
    fields
        .iter()
        .filter(|field| !within(field.level, ceiling.as_level()))
        .map(|field| EgressRefusal::Field {
            path: field.path.clone(),
            level: field.level,
        })
        .collect()
}
