//! What must never reach a log line or a span attribute (FR-008-006, NFR-008-SEC).
//!
//! The platform's protection is first of all structural: secrets and payloads are types whose
//! `Debug` hides them, and no call site logs a body. This module is the second line, for the mistake
//! that gets past the first: a value is hidden when the **name** it is logged under says it is
//! sensitive, and a string is scrubbed when its **shape** is that of a credential. Neither is
//! complete (a secret logged as `x = "hunter2"` has no shape), which is why the first line is the
//! one that counts and this one is tested as the backstop it is.

use std::borrow::Cow;

/// What stands where a value was hidden.
pub const REDACTED: &str = "[redacted]";

/// A longer value is cut: a log line is not where a document is kept.
pub const MAX_VALUE_CHARS: usize = 2048;

/// Parts of a field name that make its (textual) value private.
const SENSITIVE_PARTS: [&str; 14] = [
    "authorization",
    "token",
    "secret",
    "password",
    "passwd",
    "cookie",
    "credential",
    "api_key",
    "apikey",
    "private_key",
    "payload",
    "body",
    "bearer",
    "signature",
];

/// Whether a value logged under `name` is hidden. Numbers and booleans are never hidden (a size, a
/// count and a flag carry no secret), so `payload_bytes = 12` is kept; only textual values are.
#[must_use]
pub fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase().replace(['-', '.'], "_");
    SENSITIVE_PARTS.iter().any(|part| lower.contains(part))
}

/// `value` with every credential-shaped part replaced, and cut to [`MAX_VALUE_CHARS`].
#[must_use]
pub fn scrub(value: &str) -> Cow<'_, str> {
    match clip(value) {
        Cow::Borrowed(same) => scrub_shapes(same),
        Cow::Owned(cut) => Cow::Owned(scrub_shapes(&cut).into_owned()),
    }
}

fn clip(value: &str) -> Cow<'_, str> {
    if value.chars().count() <= MAX_VALUE_CHARS {
        return Cow::Borrowed(value);
    }
    let kept: String = value.chars().take(MAX_VALUE_CHARS).collect();
    let cut = value.chars().count() - MAX_VALUE_CHARS;
    Cow::Owned(format!("{kept}…[{cut} characters cut]"))
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '+' | '/' | '=')
}

fn scrub_shapes(text: &str) -> Cow<'_, str> {
    scrub_shapes_with(text, REDACTED)
}

/// Every credential-shaped part of `text` (a URL's user information, a bearer token, a credential-shaped word) replaced
/// by `placeholder`, with nothing cut: the shapes [`scrub`] applies, for a caller with a placeholder and a length limit
/// of its own (design 041, task T041-6: a step log writes `[REDACTED]` and cuts at 8 KiB). No second list of shapes.
#[must_use]
pub fn scrub_shapes_with<'a>(text: &'a str, placeholder: &str) -> Cow<'a, str> {
    let without_userinfo = hide_userinfo(text, placeholder);
    match scrub_words(&without_userinfo, placeholder) {
        Cow::Owned(done) => Cow::Owned(done),
        Cow::Borrowed(_) => without_userinfo,
    }
}

/// `scheme://user:pass@host/...`: what stands before the `@` of the authority.
fn hide_userinfo<'a>(text: &'a str, placeholder: &str) -> Cow<'a, str> {
    let mut out = String::new();
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let mut authority_end = tail
            .find(['/', '?', '#', ' ', '"', '\''])
            .unwrap_or(tail.len());
        // A password may hold a character that also ends an authority (`postgres://u:pa#ss@db`): when no `@` comes
        // before it, the last `@` before the end of the word still ends the user information if a `:` comes before
        // any `/`, `?` or `#` (a URL with a path and an `@` in it, `https://example.com/a@b`, is left as it is).
        let userinfo_end = tail[..authority_end].rfind('@').or_else(|| {
            let word_end = tail.find(char::is_whitespace).unwrap_or(tail.len());
            let word_end = tail[..word_end].find(['"', '\'']).unwrap_or(word_end);
            tail[..word_end].rfind('@').filter(|&at| {
                let before = &tail[..at];
                before
                    .find(':')
                    .is_some_and(|colon| !before[..colon].contains(['/', '?', '#']))
            })
        });
        if let Some(userinfo_end) = userinfo_end {
            authority_end = authority_end.max(
                userinfo_end
                    + tail[userinfo_end..]
                        .find(['/', '?', '#', ' ', '"', '\''])
                        .unwrap_or(tail.len() - userinfo_end),
            );
            out.push_str(placeholder);
            out.push_str(&tail[userinfo_end..authority_end]);
            changed = true;
        } else {
            out.push_str(&tail[..authority_end]);
        }
        rest = &tail[authority_end..];
    }
    if !changed {
        return Cow::Borrowed(text);
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn scrub_words<'a>(text: &'a str, placeholder: &str) -> Cow<'a, str> {
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut previous_word = "";
    let mut rest = text;
    while !rest.is_empty() {
        let start = rest.find(is_word_char).unwrap_or(rest.len());
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.is_empty() {
            break;
        }
        let end = rest.find(|c: char| !is_word_char(c)).unwrap_or(rest.len());
        let word = &rest[..end];
        if is_credential_word(word) || follows_scheme(previous_word, word) {
            out.push_str(placeholder);
            changed = true;
        } else {
            out.push_str(word);
        }
        previous_word = word;
        rest = &rest[end..];
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

/// `Bearer abc123...`, `Basic dXNlcjpwYXNz`: the word after the scheme is the credential. It must
/// look like one (long, with a digit or a symbol), so that "basic configuration" survives.
fn follows_scheme(previous: &str, word: &str) -> bool {
    (previous.eq_ignore_ascii_case("bearer") || previous.eq_ignore_ascii_case("basic"))
        && word.len() >= 12
        && (word
            .chars()
            .any(|c| c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | '~' | '+' | '/' | '='))
            || (previous.eq_ignore_ascii_case("basic") && is_basic_credential(word)))
}

/// A `Basic` credential of letters only (design 041, the security review of task T041-6): base64 that decodes to
/// printable `user:password`. A word of prose almost never decodes to printable text holding a `:`.
fn is_basic_credential(word: &str) -> bool {
    fn sextet(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = word.trim_end_matches('=').as_bytes();
    if !word.len().is_multiple_of(4) || bytes.len() < 4 {
        return false;
    }
    let mut decoded = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut bits = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let Some(value) = sextet(c) else {
                return false;
            };
            bits |= value << (18 - 6 * i);
        }
        let [_, a, b, c] = bits.to_be_bytes();
        decoded.extend_from_slice(&[a, b, c][..chunk.len() - 1]);
    }
    decoded.contains(&b':') && decoded.iter().all(|b| (0x20..0x7f).contains(b))
}

/// A JSON web token, or a key with a well-known prefix.
fn is_credential_word(word: &str) -> bool {
    if word.starts_with("eyJ") {
        let parts: Vec<&str> = word.split('.').collect();
        if parts.len() == 3 && parts.iter().all(|part| part.len() >= 4) {
            return true;
        }
    }
    let long = |prefix: &str, at_least: usize| {
        word.strip_prefix(prefix)
            .is_some_and(|tail| tail.len() >= at_least)
    };
    long("sk-", 16)
        || long("sk_", 16)
        || long("ghp_", 20)
        || long("xoxb-", 10)
        || long("xoxp-", 10)
        || long("xoxa-", 10)
        || (word.len() == 20
            && word.starts_with("AKIA")
            && word
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
}

#[cfg(test)]
mod scrub_shapes_with_tests {
    use super::{REDACTED, scrub, scrub_shapes_with};

    /// Design 041, task T041-6: the public entry point equals what it replaces. With the logs' placeholder it is
    /// exactly `scrub` on a value short enough not to be cut, for each shape (a URL's user information, a bearer
    /// word, a credential-shaped word) and for text with none.
    #[test]
    fn scrub_shapes_with_the_logs_placeholder_is_scrub_below_the_cut() {
        for text in [
            "postgres://admin:hunter2@db.internal:5432/app",
            "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789",
            "key AKIAIOSFODNN7EXAMPLE in the log",
            "nothing secret here",
            "",
        ] {
            assert_eq!(scrub_shapes_with(text, REDACTED), scrub(text), "{text}");
        }
    }

    /// Design 041, the security review of task T041-6: a connection string whose password holds a character that also
    /// ends a URL's authority (`#`, `/`, `?`) still has its user information hidden; a `Basic` credential of letters
    /// only is hidden when it decodes to `user:password`; prose after the word "basic" survives.
    #[test]
    fn userinfo_with_a_delimiter_and_a_letters_only_basic_credential_are_hidden() {
        for (text, secret) in [
            ("postgres://admin:pa#ss@db.internal/app", "pa#ss"),
            ("postgres://admin:pa/ss@db.internal/app", "pa/ss"),
            ("postgres://admin:pa?ss@db.internal/app", "pa?ss"),
            ("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"),
        ] {
            let scrubbed = scrub_shapes_with(text, "[REDACTED]");
            assert!(!scrubbed.contains(secret), "{text} -> {scrubbed}");
            assert!(scrubbed.contains("[REDACTED]"), "{text} -> {scrubbed}");
        }
        for kept in [
            "a basic configuration example",
            "Basic construction works",
            "https://example.com/a@b",
            "see https://example.com/path?q=1#frag for details",
        ] {
            assert_eq!(scrub_shapes_with(kept, "[REDACTED]"), kept);
        }
    }

    /// The placeholder is the caller's (a step log writes `[REDACTED]`), and nothing is cut, however long the text.
    #[test]
    fn scrub_shapes_with_uses_the_callers_placeholder_and_cuts_nothing() {
        assert_eq!(
            scrub_shapes_with("postgres://admin:hunter2@db/app", "[REDACTED]"),
            "postgres://[REDACTED]@db/app"
        );
        let long = "x ".repeat(5_000);
        assert_eq!(scrub_shapes_with(&long, "[REDACTED]"), long);
    }
}
