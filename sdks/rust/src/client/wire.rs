//! Readers and checks shared by the feature 018 client surface (schedules, business identifiers,
//! batch jobs, the with-start calls and resets). An answer that does not have the shape the
//! contract promises is a `PROTOCOL` error, never a guess; a value of an enumeration this SDK does
//! not know is kept as the raw string it came as.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Map;

fn field(key: &str) -> SagatError {
    SagatError::new("PROTOCOL", &format!("{key} is missing from the answer"))
}

/// A read-only view of one JSON object of an answer.
pub(super) struct Fields<'a>(&'a Map<String, Value>);
impl<'a> Fields<'a> {
    pub(super) fn of(value: &'a Value, what: &str) -> Result<Self, SagatError> {
        value
            .as_object()
            .map(Self)
            .ok_or_else(|| SagatError::new("PROTOCOL", &format!("{what} is not an object")))
    }
    pub(super) fn text(&self, key: &str) -> Result<String, SagatError> {
        self.opt_text(key).ok_or_else(|| field(key))
    }
    pub(super) fn opt_text(&self, key: &str) -> Option<String> {
        self.0.get(key).and_then(Value::as_str).map(str::to_owned)
    }
    pub(super) fn uuid(&self, key: &str) -> Result<Uuid, SagatError> {
        Uuid::parse_str(&self.text(key)?).map_err(|_| field(key))
    }
    /// Absent and `null` are `None`; a text that is not an identifier is a `PROTOCOL` error.
    pub(super) fn opt_uuid(&self, key: &str) -> Result<Option<Uuid>, SagatError> {
        self.opt_text(key)
            .map(|text| Uuid::parse_str(&text).map_err(|_| field(key)))
            .transpose()
    }
    pub(super) fn instant(&self, key: &str) -> Result<OffsetDateTime, SagatError> {
        self.opt_instant(key)?.ok_or_else(|| field(key))
    }
    pub(super) fn opt_instant(&self, key: &str) -> Result<Option<OffsetDateTime>, SagatError> {
        self.opt_text(key)
            .map(|text| {
                OffsetDateTime::parse(&text, &Rfc3339)
                    .map(|at| at.to_offset(UtcOffset::UTC))
                    .map_err(|_| field(key))
            })
            .transpose()
    }
    pub(super) fn flag(&self, key: &str) -> bool {
        self.0.get(key) == Some(&Value::Bool(true))
    }
    pub(super) fn opt_u64(&self, key: &str) -> Option<u64> {
        self.0.get(key).and_then(Value::as_u64)
    }
    pub(super) fn opt_f64(&self, key: &str) -> Option<f64> {
        self.0.get(key).and_then(Value::as_f64)
    }
    pub(super) fn count(&self, key: &str) -> u64 {
        self.opt_u64(key).unwrap_or(0)
    }
    /// An integer of at least `minimum` that the wire contract can carry.
    pub(super) fn at_least(&self, key: &str, minimum: u64) -> Result<u64, SagatError> {
        self.opt_u64(key)
            .filter(|n| (minimum..=i64::MAX as u64).contains(n))
            .ok_or_else(|| field(key))
    }
    pub(super) fn object(&self, key: &str) -> Option<Fields<'a>> {
        self.0.get(key).and_then(Value::as_object).map(Fields)
    }
    pub(super) fn array(&self, key: &str) -> Option<&'a Vec<Value>> {
        self.0.get(key).and_then(Value::as_array)
    }
    pub(super) fn strings(&self, key: &str) -> Vec<String> {
        self.array(key)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    }
    pub(super) fn uuids(&self, key: &str) -> Result<Vec<Uuid>, SagatError> {
        self.strings(key)
            .iter()
            .map(|text| Uuid::parse_str(text).map_err(|_| field(key)))
            .collect()
    }
    pub(super) fn values(&self, key: &str) -> Vec<Value> {
        self.array(key).cloned().unwrap_or_default()
    }
    /// The numeric entries of an object (counters); anything else is left out.
    pub(super) fn counters(&self, key: &str) -> BTreeMap<String, u64> {
        self.object(key)
            .map(|entries| {
                entries
                    .0
                    .iter()
                    .filter_map(|(name, count)| count.as_u64().map(|n| (name.clone(), n)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The page of an answer whose items are read with `read`; the cursor is opaque and kept as it came.
pub(super) fn page_of<T>(
    raw: Value,
    read: impl Fn(&Value) -> Result<T, SagatError>,
) -> Result<crate::Page<T>, SagatError> {
    crate::types::parse_page(raw, |item| read(&item))
}

/// An instant given as a date-time or as the RFC 3339 text the caller already has. It is sent in
/// UTC (`Z`), never as a local offset.
#[derive(Clone, Debug, PartialEq)]
pub enum Moment {
    At(OffsetDateTime),
    Text(String),
}
impl From<OffsetDateTime> for Moment {
    fn from(value: OffsetDateTime) -> Self {
        Self::At(value)
    }
}
impl From<&str> for Moment {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}
impl From<String> for Moment {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl Moment {
    /// The instant as UTC RFC 3339 text; text that is not an instant is refused locally.
    pub(crate) fn utc_text(&self, what: &str) -> Result<String, SagatError> {
        let not_an_instant = || SagatError::validation(&format!("{what} is not an instant"));
        let at = match self {
            Self::At(at) => *at,
            Self::Text(text) => {
                OffsetDateTime::parse(text, &Rfc3339).map_err(|_| not_an_instant())?
            }
        };
        at.to_offset(UtcOffset::UTC)
            .format(&Rfc3339)
            .map_err(|_| not_an_instant())
    }
}
impl Serialize for Moment {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        serializer.serialize_str(&self.utc_text("an instant").map_err(S::Error::custom)?)
    }
}
impl<'de> Deserialize<'de> for Moment {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::Text)
    }
}

/// An enumeration of the wire whose set only grows: a value this SDK does not know is sent and
/// kept as the raw string it is, never an error.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
            /// A value this version of the SDK does not know, exactly as it is on the wire.
            Other(String),
        }
        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $text,)+
                    Self::Other(value) => value,
                }
            }
        }
        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                match value {
                    $($text => Self::$variant,)+
                    other => Self::Other(other.to_owned()),
                }
            }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::from(value.as_str())
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(Self::from)
            }
        }
    };
}
pub(super) use wire_enum;

/// A `Debug` that shows the named fields only: answers keep the whole JSON in `raw`, which can
/// hold business data.
macro_rules! summary_debug {
    ($name:ident { $($field:ident),* }) => {
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name))
                    $(.field(stringify!($field), &self.$field))*
                    .finish_non_exhaustive()
            }
        }
    };
}
pub(super) use summary_debug;

/// An id that becomes a path segment: a UUID in its canonical hyphenated form, lower-cased.
pub(super) fn id_segment(value: &str, what: &str) -> Result<String, SagatError> {
    match Uuid::parse_str(value) {
        Ok(id) if value.len() == 36 => Ok(id.to_string()),
        _ => Err(SagatError::validation(&format!("{what} is a UUID"))),
    }
}

/// A revision a caller expects: an integer of at least 1 that the wire can carry.
pub(super) fn positive_revision(value: u64) -> Result<u64, SagatError> {
    if (1..=i64::MAX as u64).contains(&value) {
        Ok(value)
    } else {
        Err(SagatError::validation(
            "expectedRevision is a positive integer",
        ))
    }
}

/// A `Duration` as the integer milliseconds the server judges: truncated toward zero, once. The
/// range (1 ms to 366 days) is the server's to judge; a duration beyond what the wire carries is
/// sent as the largest integer, which the server refuses as out of range.
pub(super) fn delay_millis(delay: Duration) -> i64 {
    i64::try_from(delay.as_millis()).unwrap_or(i64::MAX)
}

/// The `/namespaces/{id}` prefix of a path.
pub(super) fn namespace_path(namespace_id: &str) -> Result<Vec<String>, SagatError> {
    Ok(vec![
        "namespaces".into(),
        id_segment(namespace_id, "a namespace id")?,
    ])
}

pub(super) fn request(
    method: Method,
    path: &[String],
    expected: u16,
    query: Vec<(String, String)>,
) -> WireRequest {
    let segments: Vec<&str> = path.iter().map(String::as_str).collect();
    WireRequest::new(method, &segments, expected).query(query)
}

pub(super) fn with(path: &[String], tail: &[&str]) -> Vec<String> {
    path.iter()
        .cloned()
        .chain(tail.iter().map(|s| (*s).to_owned()))
        .collect()
}

/// The body of a request as the JSON value that is serialized once, before the first attempt.
pub(super) fn body_of<T: Serialize>(value: &T) -> Result<Value, SagatError> {
    serde_json::to_value(value)
        .map_err(|_| SagatError::validation("the request cannot be written as JSON"))
}

pub(super) fn push_page(
    query: &mut Vec<(String, String)>,
    limit: Option<u16>,
    cursor: Option<&str>,
) -> Result<(), SagatError> {
    query.extend(page_query(limit, cursor)?);
    Ok(())
}
