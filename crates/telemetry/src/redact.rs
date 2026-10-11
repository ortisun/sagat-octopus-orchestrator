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
    let without_userinfo = hide_userinfo(text);
    match scrub_words(&without_userinfo) {
        Cow::Owned(done) => Cow::Owned(done),
        Cow::Borrowed(_) => without_userinfo,
    }
}

/// `scheme://user:pass@host/...`: what stands before the `@` of the authority.
fn hide_userinfo(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let authority_end = tail
            .find(['/', '?', '#', ' ', '"', '\''])
            .unwrap_or(tail.len());
        if let Some(userinfo_end) = tail[..authority_end].rfind('@') {
            out.push_str(REDACTED);
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

fn scrub_words(text: &str) -> Cow<'_, str> {
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
            out.push_str(REDACTED);
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
        && word
            .chars()
            .any(|c| c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | '~' | '+' | '/' | '='))
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
