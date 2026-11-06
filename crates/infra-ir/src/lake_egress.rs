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
//! blobs carry no schema the platform can follow, so their fields are `pii`. The strictest answer wins wherever a schema
//! describes a position in more than one way: every positional item of a list, and everything inside a structure that a
//! single column receives.

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

/// Gives every Iceberg write of `definition` the level of each field it writes, read from `document`.
pub(crate) fn attach(document: &Value, definition: WorkflowDefinition) -> WorkflowDefinition {
    if !definition
        .nodes()
        .any(|node| node.iceberg_write().is_some())
    {
        return definition;
    }
    let index = ClassificationLocationIndex::build(|visitor| {
        crate::classification_check::visit_locations(document, visitor)
    });
    definition.with_iceberg_egress(|_, step| levels(&index, step))
}

fn levels(index: &ClassificationLocationIndex, step: &IcebergWrite) -> Vec<EgressField> {
    // A step that lists no columns writes the whole record, which is one entry with an empty path.
    let paths: Vec<&str> = if step.mapping().is_empty() {
        vec![""]
    } else {
        step.mapping()
            .iter()
            .map(|mapping| mapping.field())
            .collect()
    };
    let source = match step.records() {
        RecordSource::Inline(expression) => reference(expression.as_str()),
        RecordSource::Blobs(_) | RecordSource::DatabaseResult(_) => None,
    };
    paths
        .into_iter()
        .map(|path| {
            let level = match step.records() {
                RecordSource::DatabaseResult(_) => DATABASE_RESULT,
                RecordSource::Inline(_) | RecordSource::Blobs(_) => {
                    let inherited = source
                        .as_ref()
                        .and_then(|source| inherited_level(index, source, path));
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

/// The strictest level the schema gives the field at `pointer` of any record of the list, or `None` when it cannot say.
fn inherited_level(
    index: &ClassificationLocationIndex,
    source: &Source,
    pointer: &str,
) -> Option<Level> {
    let tail = decode(pointer)?;
    let mut strictest: Option<Level> = None;
    for item in items(index, source) {
        let mut location = source.list.clone();
        location.push(item);
        location.extend(tail.iter().cloned());
        let level = strictest_at(index, &source.carrier, &location)?;
        strictest = Some(strictest.map_or(level, |seen| seen.stricter(level)));
    }
    strictest
}

/// How a record is reached in the list: by the schema that describes the remaining items, and by each position the
/// schema describes on its own.
fn items(index: &ClassificationLocationIndex, source: &Source) -> Vec<ClassificationSegment> {
    let mut items = vec![ClassificationSegment::RemainingItem];
    for location in index.locations() {
        if location.carrier == source.carrier
            && location.location.len() == source.list.len() + 1
            && location.location.starts_with(&source.list)
            && let Some(item @ ClassificationSegment::Index(_)) = location.location.last()
            && !items.contains(item)
        {
            items.push(item.clone());
        }
    }
    items
}

/// The level of one location, raised to that of everything inside it. `None` when the carrier has no schema or could not
/// be walked completely.
fn strictest_at(
    index: &ClassificationLocationIndex,
    carrier: &CarrierPointer,
    location: &[ClassificationSegment],
) -> Option<Level> {
    let ClassificationResolution::Resolved { level, .. } = index.resolve(carrier, location) else {
        return None;
    };
    Some(
        index
            .locations()
            .iter()
            .filter(|inside| inside.carrier == *carrier && inside.location.starts_with(location))
            .fold(level, |strictest, inside| strictest.stricter(inside.level)),
    )
}

/// The property names of a JSON pointer (`""` is the record itself), with the escapes of RFC 6901 undone.
fn decode(pointer: &str) -> Option<Vec<ClassificationSegment>> {
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
            Some(ClassificationSegment::Property(
                segment.replace("~1", "/").replace("~0", "~"),
            ))
        })
        .collect()
}
