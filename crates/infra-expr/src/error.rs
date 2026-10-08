//! Typed errors of the expression subset. Messages echo at most a clipped fragment of the input.

use crate::limits::{MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS};

/// Keeps an attacker-chosen name from inflating an error message.
pub(crate) fn clip(value: &str) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(32).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExprError {
    // ------------------------------------------------------------ compile time
    #[error("the expression is longer than {MAX_SOURCE_BYTES} bytes")]
    TooLong,
    #[error("the expression is too complex (limit: {MAX_TOKENS} tokens)")]
    TooComplex,
    #[error("the expression is nested more than {MAX_DEPTH} levels deep")]
    TooDeep,
    #[error("unexpected character at position {0}")]
    UnexpectedCharacter(usize),
    #[error("unterminated string starting at position {0}")]
    UnterminatedString(usize),
    #[error("invalid number at position {0}")]
    InvalidNumber(usize),
    #[error("invalid escape sequence at position {0}")]
    InvalidEscape(usize),
    #[error("syntax error at position {position}: {expected}")]
    Syntax {
        position: usize,
        expected: &'static str,
    },
    /// Comprehension macros (`all`, `exists`, `map`, `filter`, ...) and anything else outside the
    /// subset. They are refused rather than approximated.
    #[error("'{0}' is not available in this CEL subset")]
    Unsupported(String),
    #[error(
        "a template may contain at most {} expressions",
        crate::limits::MAX_TEMPLATE_PARTS
    )]
    TooManyTemplateParts,
    #[error("unterminated '${{' in a template at position {0}")]
    UnterminatedTemplate(usize),

    // ------------------------------------------------------------ run time
    #[error("undeclared variable '{0}'")]
    UndeclaredVariable(String),
    #[error("no such key '{0}'")]
    NoSuchKey(String),
    #[error("index out of range")]
    IndexOutOfRange,
    #[error("type error: {0}")]
    Type(&'static str),
    #[error("division by zero")]
    DivisionByZero,
    #[error("numeric overflow")]
    Overflow,
    #[error("the result is too large")]
    ResultTooLarge,
    #[error("the result is not a string, number or boolean")]
    NotScalar,
}
