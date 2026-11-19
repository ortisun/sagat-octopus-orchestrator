//! JSON pointers: the canonical form the vocabulary walk records each schema position under, and the decoding of a
//! `$ref` into that form, done by the validator itself (D043-056) so that no library decodes it another way.

/// One reference token in its RFC 6901 form: `~` as `~0`, `/` as `~1`.
pub(super) fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// The canonical pointer a local `$ref` names, or `None` when it is not one the subset accepts.
///
/// The reference must be a fragment (`#` then a JSON pointer, `#` alone being the root). The fragment is
/// percent-decoded (a `%` not followed by two hex digits, or bytes that are not UTF-8, refuse it), split on `/`, and in
/// each token `~0` becomes `~` and `~1` becomes `/` in one left-to-right pass, so `~01` is `~1`; a `~` followed by
/// anything else refuses it. The tokens are written back in the canonical form, which never has a leading zero in an
/// index: `#/allOf/01` names no position.
pub(super) fn decode(reference: &str) -> Option<String> {
    let fragment = reference.strip_prefix('#')?;
    let decoded = String::from_utf8(percent_decode(fragment)?).ok()?;
    if decoded.is_empty() {
        return Some(String::new());
    }
    let rest = decoded.strip_prefix('/')?;
    let mut canonical = String::with_capacity(decoded.len());
    for token in rest.split('/') {
        canonical.push('/');
        canonical.push_str(&escape(&unescape(token)?));
    }
    Some(canonical)
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = hex(*bytes.get(i + 1)?)?;
            let low = hex(*bytes.get(i + 2)?)?;
            out.push(high << 4 | low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn unescape(token: &str) -> Option<String> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next()? {
                '0' => out.push('~'),
                '1' => out.push('/'),
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_decode_to_the_canonical_pointer() {
        assert_eq!(decode("#").as_deref(), Some(""));
        assert_eq!(decode("#/$defs/a").as_deref(), Some("/$defs/a"));
        assert_eq!(decode("#/$defs/a%25b").as_deref(), Some("/$defs/a%b"));
        assert_eq!(decode("#/$defs/a~0b").as_deref(), Some("/$defs/a~0b"));
        assert_eq!(decode("#/$defs/a~1b").as_deref(), Some("/$defs/a~1b"));
        assert_eq!(
            decode("#/$defs/~01").as_deref(),
            Some("/$defs/~01"),
            "~01 is ~1"
        );
        assert_eq!(
            decode("#/$defs/%7E1").as_deref(),
            Some("/$defs/~1"),
            "%7E1 decodes to ~1, which is /"
        );
        for refused in [
            "#/a%2",
            "#/a%zz",
            "#/a~2",
            "#/a~",
            "#/%FF",
            "#a",
            "other.json#/a",
            "/a",
            "",
        ] {
            assert_eq!(decode(refused), None, "{refused}");
        }
    }
}
