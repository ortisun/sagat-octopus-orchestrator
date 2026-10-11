//! String templates with `${expression}` parts, used for idempotency keys:
//! `"reserve:${run.id}"`, `"submit:${input.paymentId}"`.

use std::collections::BTreeSet;

use crate::error::ExprError;
use crate::eval::{Compiled, Env};
use crate::limits::{MAX_RENDERED_BYTES, MAX_SOURCE_BYTES, MAX_TEMPLATE_PARTS};

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    Expr(Compiled),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    /// # Errors
    /// An [`ExprError`] for a malformed or over-limit template, or a bad embedded expression.
    pub fn compile(source: &str) -> Result<Self, ExprError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(ExprError::TooLong);
        }
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut rest = source;
        let mut offset = 0;
        while let Some(start) = rest.find("${") {
            text.push_str(&rest[..start]);
            let body_start = start + 2;
            let end = closing_brace(&rest[body_start..])
                .ok_or(ExprError::UnterminatedTemplate(offset + start))?;
            if !text.is_empty() {
                parts.push(Part::Text(std::mem::take(&mut text)));
            }
            if parts.iter().filter(|p| matches!(p, Part::Expr(_))).count() >= MAX_TEMPLATE_PARTS {
                return Err(ExprError::TooManyTemplateParts);
            }
            parts.push(Part::Expr(Compiled::compile(
                &rest[body_start..body_start + end],
            )?));
            let consumed = body_start + end + 1;
            offset += consumed;
            rest = &rest[consumed..];
        }
        text.push_str(rest);
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        Ok(Self { parts })
    }

    /// Top-level variables used by the embedded expressions.
    #[must_use]
    pub fn variables(&self) -> BTreeSet<String> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Expr(e) => Some(e.variables()),
                Part::Text(_) => None,
            })
            .flatten()
            .collect()
    }

    /// # Errors
    /// An [`ExprError`] when a part fails, is not a scalar, or the result is too long.
    pub fn render(&self, env: &Env) -> Result<String, ExprError> {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(t) => out.push_str(t),
                Part::Expr(e) => out.push_str(&e.eval_key(env)?),
            }
            if out.len() > MAX_RENDERED_BYTES {
                return Err(ExprError::ResultTooLarge);
            }
        }
        Ok(out)
    }
}

/// Index of the `}` that closes the expression, ignoring braces inside string literals.
fn closing_brace(body: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                '}' => return Some(i),
                _ => {}
            },
        }
    }
    None
}
