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

/// The RFC 8785 (JSON Canonicalization Scheme) bytes of `value`: the form the command chain of design 044 hashes
/// (Technical Plan 6.2, the corpus README `specs/044-workflow-as-code/contracts/goldens/README.md`).
///
/// Object keys are sorted by their UTF-16 code units, strings are escaped as ECMAScript's `JSON.stringify` escapes
/// them, and numbers are printed as ECMAScript's Number::toString prints the double the JSON number denotes: the
/// shortest digits that round-trip and, of two such, the one closest to the double, a tie taking the even digit. A
/// language's default float printing is not that on a tie (Rust prints 1424953923781206.25 as `...206.3`, ECMAScript
/// as `...206.2`). This is not `canonical_json`, whose key order and number printing are the version digest's.
#[must_use]
pub fn rfc8785(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_rfc8785(value, &mut out);
    out
}

fn write_rfc8785(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => write_number(number, out),
        Value::String(text) => write_es_string(text, out),
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_rfc8785(item, out);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push(b'{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_es_string(key, out);
                out.push(b':');
                write_rfc8785(item, out);
            }
            out.push(b'}');
        }
    }
}

/// The largest integer below which every integer is a double: 2^53.
const EXACT_INTEGERS: u64 = 1 << 53;

fn write_number(number: &serde_json::Number, out: &mut Vec<u8>) {
    // An integer that a double holds exactly prints as its decimal digits, which is what Number::toString gives for
    // it (below 10^21). A larger one is the double its text denotes, as ECMAScript parses it (round to nearest, ties
    // to even, which `as f64` also does).
    if let Some(unsigned) = number.as_u64().filter(|n| *n <= EXACT_INTEGERS) {
        out.extend_from_slice(unsigned.to_string().as_bytes());
    } else if let Some(signed) = number
        .as_i64()
        .filter(|n| n.unsigned_abs() <= EXACT_INTEGERS)
    {
        out.extend_from_slice(signed.to_string().as_bytes());
    } else {
        // Without `arbitrary_precision` every number is a finite double here; with it, a number's text is parsed as
        // ECMAScript's JSON.parse would, and a value beyond the doubles is printed as JSON.stringify prints Infinity:
        // `null` (RFC 8785 refuses such a value; I-JSON callers refuse it before, Decision 044-252).
        let double = number
            .as_f64()
            .unwrap_or_else(|| number.to_string().parse().unwrap_or(f64::INFINITY));
        if double.is_finite() {
            out.extend_from_slice(es_number(double).as_bytes());
        } else {
            out.extend_from_slice(b"null");
        }
    }
}

/// ECMAScript's Number::toString of a finite double (ECMA-262, Number::toString with radix 10).
fn es_number(x: f64) -> String {
    if x == 0.0 {
        // -0 prints 0.
        return "0".to_owned();
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let (digits, exponent) = shortest_closest_digits(x.abs());
    let k = i32::try_from(digits.len()).unwrap_or(17);
    let n = exponent + 1;
    let body = if k <= n && n <= 21 {
        let zeros = usize::try_from(n - k).unwrap_or(0);
        format!("{digits}{}", "0".repeat(zeros))
    } else if 0 < n && n <= 21 {
        let (whole, fraction) = digits.split_at(usize::try_from(n).unwrap_or(0));
        format!("{whole}.{fraction}")
    } else if -6 < n && n <= 0 {
        let zeros = usize::try_from(-n).unwrap_or(0);
        format!("0.{}{digits}", "0".repeat(zeros))
    } else {
        let e = n - 1;
        let exponent_sign = if e < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{exponent_sign}{}", e.unsigned_abs())
        } else {
            format!("{first}.{rest}e{exponent_sign}{}", e.unsigned_abs())
        }
    };
    format!("{sign}{body}")
}

/// The digits (without a trailing zero) and the decimal exponent of the first digit of the shortest decimal that
/// round-trips to `magnitude`, the closest to it when two of that length do, a tie taking the even digit.
///
/// Rust's `{:e}` gives the shortest length `n` and one shortest form; its fixed-precision `{:.(n-1)e}` gives the
/// `n`-digit decimal closest to the exact value, a tie to the even digit. When that one round-trips it is ECMAScript's
/// choice; otherwise the shortest form is the only one of its length that does.
fn shortest_closest_digits(magnitude: f64) -> (String, i32) {
    let shortest = format!("{magnitude:e}");
    let (shortest_digits, shortest_exponent) = split_scientific(&shortest);
    let precision = shortest_digits.len().saturating_sub(1);
    let closest = format!("{magnitude:.precision$e}");
    let chosen = if closest
        .parse::<f64>()
        .is_ok_and(|parsed| parsed.to_bits() == magnitude.to_bits())
    {
        split_scientific(&closest)
    } else {
        (shortest_digits, shortest_exponent)
    };
    let (mut digits, exponent) = chosen;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    (digits, exponent)
}

/// The significant digits and the exponent of a `{:e}` rendering such as `1.4249539237812062e15`.
fn split_scientific(scientific: &str) -> (String, i32) {
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific, "0"));
    let digits = mantissa.chars().filter(char::is_ascii_digit).collect();
    (digits, exponent.parse().unwrap_or(0))
}

/// A string as `JSON.stringify` writes it: `"` and `\` escaped, the control characters below U+0020 as their short
/// form or `\u00xx` in lower case, everything else (U+007F, the line separators, every other code point) as itself.
fn write_es_string(text: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    let mut buffer = [0_u8; 4];
    for c in text.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if u32::from(c) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", u32::from(c)).as_bytes());
            }
            c => out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes()),
        }
    }
    out.push(b'"');
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
