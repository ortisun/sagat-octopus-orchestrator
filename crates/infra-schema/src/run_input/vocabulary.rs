//! Steps 1 and 2 of the pre-check (D043-048, D043-055 to D043-057): the size and depth of the schema, then a walk from
//! the root that records every schema position and refuses, at the first violation in document order, any key that is
//! not in the vocabulary of the subset or a keyword in a form the subset does not support.
//!
//! The keys of `properties`, `$defs` and `definitions` are names, not keywords, so a property called `id` or `$ref` is
//! legal. The walk does not descend into data (`enum`, `const`, `default`, `examples`, `x-sagat-classification`).

use serde_json::{Map, Value};

use super::pointer::escape;
use super::{InputSchemaIssue, MAX_INPUT_SCHEMA_BYTES, MAX_INPUT_SCHEMA_DEPTH, Unusable};

/// `$schema` values with one meaning in the subset; `$schema` absent means the same.
pub(super) const DIALECTS: [&str; 2] = [
    "http://json-schema.org/draft-07/schema#",
    "https://json-schema.org/draft/2020-12/schema",
];

/// The largest divisor `multipleOf` may have: above 2^53 an `f64` no longer holds every integer.
pub(super) const MAX_MULTIPLE_OF: u64 = 1 << 53;

/// The annotations: no effect on validation, and the only keywords a `$ref` may stand beside (with the containers).
const ANNOTATIONS: [&str; 7] = [
    "title",
    "description",
    "default",
    "examples",
    "$comment",
    "format",
    "x-sagat-classification",
];

/// One schema position: where it is (a canonical JSON pointer into the schema) and what it holds.
pub(super) struct Position<'a> {
    pub(super) pointer: String,
    pub(super) value: &'a Value,
}

/// The first violation of steps 1 and 2, or the schema positions in walk order (the root first).
pub(super) fn positions(schema_bytes: usize, root: &Value) -> Result<Vec<Position<'_>>, Unusable> {
    if schema_bytes > MAX_INPUT_SCHEMA_BYTES {
        return Err(Unusable::new(InputSchemaIssue::TooLarge, "", ""));
    }
    depth(root)?;
    if !root.is_object() {
        return Err(Unusable::new(InputSchemaIssue::KeywordUnsupported, "", ""));
    }
    let mut walk = Walk { found: Vec::new() };
    walk.schema(root, String::new(), true)?;
    Ok(walk.found)
}

/// Refuses a schema nested deeper than [`MAX_INPUT_SCHEMA_DEPTH`] (the root is depth 1), at the first value too deep.
fn depth(root: &Value) -> Result<(), Unusable> {
    let mut pending: Vec<(&Value, usize, String, String)> =
        vec![(root, 1, String::new(), String::new())];
    while let Some((value, level, pointer, key)) = pending.pop() {
        if level > MAX_INPUT_SCHEMA_DEPTH {
            return Err(Unusable::new(InputSchemaIssue::TooDeep, &key, &pointer));
        }
        match value {
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate().rev() {
                    pending.push((item, level + 1, format!("{pointer}/{index}"), key.clone()));
                }
            }
            Value::Object(members) => {
                for (name, member) in members.iter().rev() {
                    pending.push((
                        member,
                        level + 1,
                        format!("{pointer}/{}", escape(name)),
                        name.clone(),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct Walk<'a> {
    found: Vec<Position<'a>>,
}

impl<'a> Walk<'a> {
    fn schema(&mut self, value: &'a Value, pointer: String, root: bool) -> Result<(), Unusable> {
        let object = match value {
            Value::Bool(_) => {
                self.found.push(Position { pointer, value });
                return Ok(());
            }
            Value::Object(object) => object,
            // A schema position must hold a schema: an object or a boolean.
            _ => {
                return Err(Unusable::new(
                    InputSchemaIssue::KeywordUnsupported,
                    "",
                    &pointer,
                ));
            }
        };
        self.found.push(Position {
            pointer: pointer.clone(),
            value,
        });
        if object.contains_key("$ref") {
            for key in object.keys() {
                let beside = key == "$ref"
                    || ANNOTATIONS.contains(&key.as_str())
                    || key == "$defs"
                    || key == "definitions"
                    || (root && (key == "$schema" || key == "$id"));
                if !beside {
                    // draft-07 ignores the siblings of `$ref`, 2020-12 applies them: refused, so both mean the same.
                    return Err(Unusable::new(
                        InputSchemaIssue::ReferenceInvalid,
                        "$ref",
                        &format!("{pointer}/$ref"),
                    ));
                }
            }
        }
        for (key, member) in object {
            let at = format!("{pointer}/{}", escape(key));
            self.keyword(key, member, &at, root)?;
        }
        Ok(())
    }

    fn keyword(
        &mut self,
        key: &str,
        member: &'a Value,
        at: &str,
        root: bool,
    ) -> Result<(), Unusable> {
        let unsupported = || Unusable::new(InputSchemaIssue::KeywordUnsupported, key, at);
        match key {
            "$schema" if root => {
                if member
                    .as_str()
                    .is_some_and(|dialect| DIALECTS.contains(&dialect))
                {
                    Ok(())
                } else {
                    Err(unsupported())
                }
            }
            "$id" if root => member.as_str().map(|_| ()).ok_or_else(unsupported),
            "type" => types(member).then_some(()).ok_or_else(unsupported),
            "properties" | "$defs" | "definitions" => {
                let names = member.as_object().ok_or_else(unsupported)?;
                for (name, schema) in names {
                    self.schema(schema, format!("{at}/{}", escape(name)), false)?;
                }
                Ok(())
            }
            // `items` as an array is a draft-07 tuple, which 2020-12 spells `prefixItems`.
            "items" | "additionalProperties" => {
                if member.is_object() || member.is_boolean() {
                    self.schema(member, at.to_owned(), false)
                } else {
                    Err(unsupported())
                }
            }
            "allOf" | "anyOf" | "oneOf" => {
                let branches = member
                    .as_array()
                    .filter(|b| !b.is_empty())
                    .ok_or_else(unsupported)?;
                for (index, branch) in branches.iter().enumerate() {
                    self.schema(branch, format!("{at}/{index}"), false)?;
                }
                Ok(())
            }
            "required" => member
                .as_array()
                .filter(|names| unique_strings(names))
                .map(|_| ())
                .ok_or_else(unsupported),
            "enum" | "examples" => member.as_array().map(|_| ()).ok_or_else(unsupported),
            "const" | "default" | "x-sagat-classification" => Ok(()),
            // The numeric bounds are numbers (the boolean exclusive bounds of draft-04 are refused).
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => {
                member.as_number().map(|_| ()).ok_or_else(unsupported)
            }
            // A positive integer: floating-point remainders differ between implementations (G043-015).
            "multipleOf" => member
                .as_u64()
                .filter(|divisor| (1..=MAX_MULTIPLE_OF).contains(divisor))
                .map(|_| ())
                .ok_or_else(unsupported),
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
            | "maxProperties" => member.as_u64().map(|_| ()).ok_or_else(unsupported),
            "pattern" | "$ref" | "title" | "description" | "$comment" | "format" => {
                member.as_str().map(|_| ()).ok_or_else(unsupported)
            }
            // Every other keyword, `$schema` and `$id` anywhere but the root among them.
            _ => Err(unsupported()),
        }
    }
}

/// The type names of both drafts, a name or a non-empty list of distinct names.
fn types(value: &Value) -> bool {
    const NAMES: [&str; 7] = [
        "null", "boolean", "object", "array", "number", "string", "integer",
    ];
    match value {
        Value::String(name) => NAMES.contains(&name.as_str()),
        Value::Array(names) => {
            !names.is_empty()
                && unique_strings(names)
                && names
                    .iter()
                    .all(|n| n.as_str().is_some_and(|n| NAMES.contains(&n)))
        }
        _ => false,
    }
}

fn unique_strings(values: &[Value]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    values
        .iter()
        .all(|value| value.as_str().is_some_and(|text| seen.insert(text)))
}

/// The keys of an object schema, for the compiler.
pub(super) fn object(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}
