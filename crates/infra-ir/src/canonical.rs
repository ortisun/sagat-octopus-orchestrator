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
        Value::Object(map) => {
            out.push(b'{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write(&map[key], out);
            }
            out.push(b'}');
        }
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

fn write_string(s: &str, out: &mut Vec<u8>) {
    // Serialising a `&str` cannot fail.
    if let Ok(escaped) = serde_json::to_string(s) {
        out.extend_from_slice(escaped.as_bytes());
    }
}

pub(crate) fn digest_of(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}
