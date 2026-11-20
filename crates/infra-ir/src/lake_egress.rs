//! The level of each field an `icebergWrite` step writes (feature 035: T035-054; FR-035-025, D035-051).
//!
//! The codec holds the whole document, so it is the one that reads the schemas. A written field inherits the level the
//! schema of its source gives it; anything this module cannot prove is read as `pii`, the conservative reading of 016 for
//! a field nobody classified. Following a value through an expression is out of reach (016 classifies the input, the
//! result and the output of each step, and nothing derived from them), so only a plain reference is followed:
//!
//! - `input.a.b` reads the input schema at `a`, `b`;
//! - `steps.<id>.output.a.b` reads the output schema of the root step `<id>`.
//!
//! Anything else (an index, an operator, a call, a name this module does not know), a database result and a list of
//! blobs carry no schema the platform can follow, so their fields are `pii`.
//!
//! The reading is conservative where a schema cannot say enough:
//!
//! - the strictest answer wins wherever a schema describes a position more than one way (every positional item of a list
//!   and the remaining items);
//! - a value that can hold more than one thing (a structure, a list, a whole record, anything whose schema does not say it
//!   is a scalar) is at least `pii`, because a key or an element the schema does not name is `pii` under 016, and nothing
//!   closes the object;
//! - a numeric segment may be a position of a list or a property that happens to be named by digits, so it reads
//!   everything under the place it indexes;
//! - a list described by more positions than [`MAX_POSITIONS`] is read as `pii`, so the cost of the reading is bounded.

use std::collections::HashMap;

use sagat_application::{
    classification::{CarrierPointer, ClassificationResolution, ClassificationSegment},
    classification_index::ClassificationLocationIndex,
};
use sagat_domain::{
    classification::{DATABASE_RESULT, Level, written_level},
    database::{EgressField, IcebergWrite, RecordSource},
    workflow::{NodeId, WorkflowDefinition},
};
use serde_json::Value;

/// The most positions of a list that are read one by one. A schema that describes more is read as `pii` instead.
const MAX_POSITIONS: usize = 64;

/// Gives every Iceberg write of `definition` the level of each field it writes, read from `document`.
pub(crate) fn attach(document: &Value, definition: WorkflowDefinition) -> WorkflowDefinition {
    if !definition
        .nodes()
        .any(|node| node.iceberg_write().is_some())
    {
        return definition;
    }
    let schemas = Schemas::new(document);
    definition.with_iceberg_egress(|_, step| levels(&schemas, step))
}

/// The classification of one document, read once.
struct Schemas<'a> {
    document: &'a Value,
    index: ClassificationLocationIndex,
    /// The strictest level at or below each location, so that a question about a structure is a lookup, not a scan.
    below: HashMap<(CarrierPointer, Vec<ClassificationSegment>), Level>,
}

impl<'a> Schemas<'a> {
    fn new(document: &'a Value) -> Self {
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
    fn level_at(
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

fn levels(schemas: &Schemas<'_>, step: &IcebergWrite) -> Vec<EgressField> {
    // A step that lists no columns writes the whole record, which is one entry with an empty path.
    let paths: Vec<&str> = if step.mapping().is_empty() {
        vec![""]
    } else {
        step.mapping()
            .iter()
            .map(|mapping| mapping.field())
            .collect()
    };
    let list = match step.records() {
        RecordSource::Inline(expression) => {
            reference(expression.as_str()).and_then(|source| Records::new(schemas, source))
        }
        RecordSource::Blobs(_) | RecordSource::DatabaseResult(_) => None,
    };
    paths
        .into_iter()
        .map(|path| {
            let level = match step.records() {
                RecordSource::DatabaseResult(_) => DATABASE_RESULT,
                RecordSource::Inline(_) | RecordSource::Blobs(_) => {
                    let inherited = list
                        .as_ref()
                        .and_then(|records| records.level_of(schemas, path));
                    written_level(inherited, None)
                }
            };
            EgressField::new(path, level)
        })
        .collect()
}

/// Where a plain reference points: a carrier and the place of the list of records in it.
struct Source {
    carrier: CarrierPointer,
    list: Vec<ClassificationSegment>,
}

/// The reference an inline expression is, when it is nothing but a reference.
fn reference(expression: &str) -> Option<Source> {
    let mut names = expression.trim().split('.');
    let (carrier, rest): (CarrierPointer, Vec<&str>) = match names.next()? {
        "input" => (CarrierPointer::Input, names.collect()),
        "steps" => {
            let node = NodeId::parse(names.next()?).ok()?;
            if names.next()? != "output" {
                return None;
            }
            (CarrierPointer::NodeOutput(node), names.collect())
        }
        _ => return None,
    };
    let list = rest
        .into_iter()
        .map(|name| plain_name(name).then(|| ClassificationSegment::Property(name.to_owned())))
        .collect::<Option<Vec<_>>>()?;
    Some(Source { carrier, list })
}

/// A name an expression can spell without quoting; anything else is not followed.
fn plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The list of records a step reads, as the schema describes it: where it is, and every way a record is reached in it.
struct Records {
    carrier: CarrierPointer,
    /// Each location a record can have: by the schema of the remaining items and by each position described on its own.
    records: Vec<Vec<ClassificationSegment>>,
}

impl Records {
    /// `None` when the schema says nothing a caller can rely on about the list, or describes too many of its positions.
    fn new(schemas: &Schemas<'_>, source: Source) -> Option<Self> {
        // The list may itself be reached through a name the schema leaves to `additionalProperties`: what is below is
        // below the location that answered.
        let (resolution, matched) = schemas.index.resolve_matched(&source.carrier, &source.list);
        let (ClassificationResolution::Resolved { .. }, Some(list)) = (resolution, matched) else {
            return None;
        };
        let mut records = vec![item(&list, ClassificationSegment::RemainingItem)];
        for location in schemas.index.locations() {
            if location.carrier == source.carrier
                && location.location.len() == list.len() + 1
                && location.location.starts_with(&list)
                && let Some(position @ ClassificationSegment::Index(_)) = location.location.last()
            {
                if records.len() > MAX_POSITIONS {
                    return None;
                }
                records.push(item(&list, position.clone()));
            }
        }
        Some(Self {
            carrier: source.carrier,
            records,
        })
    }

    /// The strictest level the schema gives the field at `pointer` of any record of the list, or `None` when it cannot say.
    fn level_of(&self, schemas: &Schemas<'_>, pointer: &str) -> Option<Level> {
        let names = decode(pointer)?;
        let mut strictest: Option<Level> = None;
        for record in &self.records {
            let level = self.field_in(schemas, record, &names)?;
            strictest = Some(strictest.map_or(level, |seen| seen.stricter(level)));
        }
        strictest
    }

    fn field_in(
        &self,
        schemas: &Schemas<'_>,
        record: &[ClassificationSegment],
        names: &[String],
    ) -> Option<Level> {
        let mut location = record.to_vec();
        for name in names {
            // A number may be a position of a list or the name of a property: either way the field is as strict as
            // anything under what it indexes.
            if !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()) {
                return schemas
                    .level_at(&self.carrier, &location)
                    .map(|level| level.stricter(Level::UNMARKED));
            }
            location.push(ClassificationSegment::Property(name.clone()));
        }
        schemas.level_at(&self.carrier, &location)
    }
}

fn item(list: &[ClassificationSegment], at: ClassificationSegment) -> Vec<ClassificationSegment> {
    let mut location = list.to_vec();
    location.push(at);
    location
}

/// The property names of a JSON pointer (`""` is the record itself), with the escapes of RFC 6901 undone.
fn decode(pointer: &str) -> Option<Vec<String>> {
    if pointer.is_empty() {
        return Some(Vec::new());
    }
    pointer
        .strip_prefix('/')?
        .split('/')
        .map(|segment| {
            if segment.is_empty() {
                return None;
            }
            Some(segment.replace("~1", "/").replace("~0", "~"))
        })
        .collect()
}
