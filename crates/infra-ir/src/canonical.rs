//! Canonical JSON and the digest a version is identified by.
//!
//! Two documents that differ only in whitespace or in the order of object keys must produce the
//! same bytes, and therefore the same digest, while the order of array elements stays significant.
//! Keys are sorted explicitly (not by relying on the map type of the JSON library), strings are
//! escaped by the JSON library, and numbers keep the library's shortest representation.

use sagat_domain::workflow::Digest;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

pub(crate) fn canonical_json(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Object(map) => write_object(map.iter().collect(), out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(item, out);
            }
            out.push(b']');
        }
        Value::String(s) => write_string(s, out),
        other => out.extend_from_slice(other.to_string().as_bytes()),
    }
}

/// Writes an object with its keys in byte order, whatever order the entries arrive in. The default
/// map type is already sorted, but a dependency enabling `serde_json/preserve_order` (features are
/// unified across the workspace) would silently change every digest without this.
fn write_object(mut entries: Vec<(&String, &Value)>, out: &mut Vec<u8>) {
    entries.sort_by(|a, b| a.0.cmp(b.0));
    out.push(b'{');
    for (i, (key, value)) in entries.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        write_string(key, out);
        out.push(b':');
        write(value, out);
    }
    out.push(b'}');
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    // Serialising a `&str` cannot fail.
    if let Ok(escaped) = serde_json::to_string(s) {
        out.extend_from_slice(escaped.as_bytes());
    }
}

pub(crate) fn digest_of(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sorted_even_when_the_entries_arrive_in_another_order() {
        let (a, b, c) = ("b".to_owned(), "a".to_owned(), "c".to_owned());
        let (one, two, three) = (Value::from(1), Value::from(2), Value::from(3));
        let mut out = Vec::new();
        write_object(vec![(&a, &one), (&c, &three), (&b, &two)], &mut out);
        assert_eq!(out, br#"{"a":2,"b":1,"c":3}"#);
    }

    #[test]
    fn sorting_is_by_bytes_not_by_locale() {
        let (upper, lower, accent) = ("B".to_owned(), "a".to_owned(), "é".to_owned());
        let one = Value::from(1);
        let mut out = Vec::new();
        write_object(
            vec![(&accent, &one), (&lower, &one), (&upper, &one)],
            &mut out,
        );
        assert_eq!(out, "{\"B\":1,\"a\":1,\"é\":1}".as_bytes());
    }
}
