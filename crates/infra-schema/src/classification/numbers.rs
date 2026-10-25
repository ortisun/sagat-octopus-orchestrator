//! Number handling for the classification interpreter.
//!
//! The validator holds every number as a machine value, so the interpreter keeps the spelling the
//! author sent and refuses to let the validator decide a numeric keyword on a different value.
//! Spellings live in a tree shaped like the payload (a [`Lexemes`]), never in a map keyed by path:
//! a path is as long as the keys above it, and one per number multiplies payload size by depth.

use std::collections::BTreeMap;

use sagat_application::classification::JsonNumber;
use serde_json::Value;

use sagat_application::classification::ProjectionFailure;

/// `2^53`: below it every integer is an exact `f64`.
const SAFE_BOUND: f64 = 9_007_199_254_740_992.0;

/// Approximate heap cost of one tree node; used only for the retained-size counter.
const NODE_BYTES: usize = std::mem::size_of::<Lexeme>();

/// What a carrier makes the lexemes matter for. Computing a check nobody needs is skipped.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct NumberChecks {
    /// `Some(decimal_constants)` when a numeric keyword compares instance numbers.
    pub comparison: Option<bool>,
    /// An integer type is reachable, so a fraction that rounds to a whole number is unsafe.
    pub rounded_fraction: bool,
}

/// A spelling tree mirroring the payload. Subtrees without a number are pruned to [`Self::None`].
pub(super) enum Lexeme {
    None,
    Number(JsonNumber),
    Array(Vec<Lexeme>),
    /// Last duplicate key wins, as in the parsed value the validator sees.
    Object(BTreeMap<String, Lexeme>),
}

impl Lexeme {
    pub(super) fn child(&self, name: &str) -> &Self {
        match self {
            Self::Object(children) => children.get(name).unwrap_or(&Self::None),
            _ => &Self::None,
        }
    }

    pub(super) fn item(&self, index: usize) -> &Self {
        match self {
            Self::Array(items) => items.get(index).unwrap_or(&Self::None),
            _ => &Self::None,
        }
    }

    pub(super) fn number(&self) -> Option<&JsonNumber> {
        match self {
            Self::Number(number) => Some(number),
            _ => None,
        }
    }

    pub(super) fn is_integer(&self) -> bool {
        self.number()
            .is_some_and(|number| lexical_is_integer(number.as_str()))
    }
}

/// The spelling tree plus what one pass over the payload learned about it.
pub(super) struct Lexemes {
    pub root: Lexeme,
    /// Every value node, so the walk can charge per-descendant validator work.
    pub nodes: usize,
    pub numbers: usize,
    pub unsafe_comparison: bool,
    pub rounded_fraction: bool,
    /// Bytes kept for spellings and keys: linear in the payload by construction.
    pub retained_bytes: usize,
}

impl Lexemes {
    pub(super) fn scan(payload: &[u8], checks: NumberChecks) -> Result<Self, ProjectionFailure> {
        let mut scanner = Scanner {
            payload,
            at: 0,
            checks,
            nodes: 0,
            numbers: 0,
            unsafe_comparison: false,
            rounded_fraction: false,
            retained_bytes: 0,
        };
        let root = scanner.value()?;
        scanner.whitespace();
        if scanner.at != payload.len() {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        Ok(Self {
            root,
            nodes: scanner.nodes,
            numbers: scanner.numbers,
            unsafe_comparison: scanner.unsafe_comparison,
            rounded_fraction: scanner.rounded_fraction,
            retained_bytes: scanner.retained_bytes,
        })
    }
}

struct Scanner<'a> {
    payload: &'a [u8],
    at: usize,
    checks: NumberChecks,
    nodes: usize,
    numbers: usize,
    unsafe_comparison: bool,
    rounded_fraction: bool,
    retained_bytes: usize,
}

impl Scanner<'_> {
    fn value(&mut self) -> Result<Lexeme, ProjectionFailure> {
        self.whitespace();
        self.nodes += 1;
        match self.payload.get(self.at) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(|_| Lexeme::None),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(ProjectionFailure::UnsupportedPayload),
        }
    }

    fn object(&mut self) -> Result<Lexeme, ProjectionFailure> {
        self.at += 1;
        self.whitespace();
        if self.take(b'}') {
            return Ok(Lexeme::None);
        }
        let mut children = BTreeMap::new();
        loop {
            self.whitespace();
            let name = self.string()?;
            self.whitespace();
            if !self.take(b':') {
                return Err(ProjectionFailure::UnsupportedPayload);
            }
            let child = self.value()?;
            // A number-free subtree is dropped, so its key is not retained either.
            if !matches!(child, Lexeme::None) {
                self.retained_bytes += name.len() + NODE_BYTES;
                children.insert(name, child);
            }
            self.whitespace();
            if self.take(b'}') {
                return Ok(if children.is_empty() {
                    Lexeme::None
                } else {
                    Lexeme::Object(children)
                });
            }
            if !self.take(b',') {
                return Err(ProjectionFailure::UnsupportedPayload);
            }
        }
    }

    fn array(&mut self) -> Result<Lexeme, ProjectionFailure> {
        self.at += 1;
        self.whitespace();
        if self.take(b']') {
            return Ok(Lexeme::None);
        }
        let mut items = Vec::new();
        let mut any = false;
        loop {
            let item = self.value()?;
            any |= !matches!(item, Lexeme::None);
            self.retained_bytes += NODE_BYTES;
            items.push(item);
            self.whitespace();
            if self.take(b']') {
                return Ok(if any {
                    Lexeme::Array(items)
                } else {
                    // Positions only matter next to a number; a number-free array keeps nothing.
                    self.retained_bytes =
                        self.retained_bytes.saturating_sub(items.len() * NODE_BYTES);
                    Lexeme::None
                });
            }
            if !self.take(b',') {
                return Err(ProjectionFailure::UnsupportedPayload);
            }
        }
    }

    fn string(&mut self) -> Result<String, ProjectionFailure> {
        let start = self.at;
        if !self.take(b'"') {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        loop {
            match self.payload.get(self.at).copied() {
                Some(b'"') => {
                    self.at += 1;
                    return serde_json::from_slice(&self.payload[start..self.at])
                        .map_err(|_| ProjectionFailure::UnsupportedPayload);
                }
                Some(b'\\') => {
                    self.at += 2;
                }
                Some(_) => self.at += 1,
                None => return Err(ProjectionFailure::UnsupportedPayload),
            }
        }
    }

    fn number(&mut self) -> Result<Lexeme, ProjectionFailure> {
        let start = self.at;
        while matches!(
            self.payload.get(self.at),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.payload[start..self.at])
            .map_err(|_| ProjectionFailure::UnsupportedPayload)?;
        let number = JsonNumber::parse(text).map_err(|_| ProjectionFailure::UnsupportedPayload)?;
        self.numbers += 1;
        self.retained_bytes += text.len() + NODE_BYTES;
        if let Some(decimal_constants) = self.checks.comparison {
            self.unsafe_comparison |= !safe_comparison(text, decimal_constants);
        }
        if self.checks.rounded_fraction {
            self.rounded_fraction |= is_rounded_fraction(text);
        }
        Ok(Lexeme::Number(number))
    }

    fn literal(&mut self, literal: &[u8]) -> Result<Lexeme, ProjectionFailure> {
        if self.payload.get(self.at..self.at + literal.len()) == Some(literal) {
            self.at += literal.len();
            Ok(Lexeme::None)
        } else {
            Err(ProjectionFailure::UnsupportedPayload)
        }
    }

    fn whitespace(&mut self) {
        while matches!(
            self.payload.get(self.at),
            Some(b' ' | b'\n' | b'\r' | b'\t')
        ) {
            self.at += 1;
        }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.payload.get(self.at) == Some(&expected) {
            self.at += 1;
            true
        } else {
            false
        }
    }
}

/// What a carrier's numeric keywords require of the instance numbers.
#[derive(Debug, Clone, Copy)]
pub(super) struct NumericProfile {
    pub has_constraints: bool,
    pub constraints_exact: bool,
    /// A schema bound, constant or divisor is a non-integer, so integers beyond the `f64`
    /// mantissa cannot be compared against it exactly.
    pub decimal_constants: bool,
    pub has_integer_types: bool,
}

impl NumericProfile {
    pub(super) fn of(schema: &Value) -> Self {
        Self {
            has_constraints: has_numeric_constraints(schema),
            constraints_exact: numeric_constraints_exact(schema),
            decimal_constants: has_decimal_constants(schema),
            has_integer_types: schema_contains_integer_type(schema),
        }
    }

    pub(super) const fn checks(self) -> NumberChecks {
        NumberChecks {
            comparison: if self.has_constraints {
                Some(self.decimal_constants)
            } else {
                None
            },
            rounded_fraction: self.has_integer_types,
        }
    }

    /// Whether the lexemes of this payload may be validated at all.
    pub(super) const fn admits(self, lexemes: &Lexemes) -> bool {
        lexemes.numbers == 0
            || (self.constraints_exact
                && !(self.has_constraints && lexemes.unsafe_comparison)
                && !(self.has_integer_types && lexemes.rounded_fraction))
    }
}

pub(super) fn schema_requires_integer(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => kind == "integer",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind.as_str() == Some("integer")),
        _ => false,
    }
}

fn schema_contains_integer_type(schema: &Value) -> bool {
    schema_requires_integer(schema)
        || match schema {
            Value::Object(values) => values.values().any(schema_contains_integer_type),
            Value::Array(values) => values.iter().any(schema_contains_integer_type),
            _ => false,
        }
}

fn has_numeric_constraints(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => object.iter().any(|(keyword, value)| {
            matches!(
                keyword.as_str(),
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
            ) || (keyword == "uniqueItems" && value.as_bool() == Some(true))
                || (matches!(keyword.as_str(), "const" | "enum") && contains_number(value))
                || has_numeric_constraints(value)
        }),
        Value::Array(values) => values.iter().any(has_numeric_constraints),
        _ => false,
    }
}

fn numeric_constraints_exact(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => object.iter().all(|(keyword, value)| {
            let local = match keyword.as_str() {
                // An integer divisor stays on the exact integer-modulo branch up to the largest
                // safe integer; a larger one would be compiled through a rounded `f64`. A decimal
                // divisor is compared as `f64`, which is exact for a divisor of ordinary size.
                "multipleOf" => value.as_u64().map_or_else(
                    || {
                        value
                            .as_f64()
                            .is_some_and(|divisor| divisor > 0.0 && divisor < SAFE_BOUND)
                    },
                    |divisor| divisor > 0 && divisor <= 9_007_199_254_740_991,
                ),
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "const"
                | "enum" => exact_stored_numbers(value),
                _ => true,
            };
            local && numeric_constraints_exact(value)
        }),
        Value::Array(values) => values.iter().all(numeric_constraints_exact),
        _ => true,
    }
}

fn has_decimal_constants(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => object.iter().any(|(keyword, value)| {
            (matches!(
                keyword.as_str(),
                "minimum"
                    | "maximum"
                    | "exclusiveMinimum"
                    | "exclusiveMaximum"
                    | "multipleOf"
                    | "const"
                    | "enum"
            ) && contains_decimal(value))
                || has_decimal_constants(value)
        }),
        Value::Array(values) => values.iter().any(has_decimal_constants),
        _ => false,
    }
}

fn contains_decimal(value: &Value) -> bool {
    match value {
        Value::Number(number) => !(number.is_i64() || number.is_u64()),
        Value::Array(values) => values.iter().any(contains_decimal),
        Value::Object(values) => values.values().any(contains_decimal),
        _ => false,
    }
}

fn contains_number(value: &Value) -> bool {
    match value {
        Value::Number(_) => true,
        Value::Array(values) => values.iter().any(contains_number),
        Value::Object(values) => values.values().any(contains_number),
        _ => false,
    }
}

fn exact_stored_numbers(value: &Value) -> bool {
    match value {
        // An integer is stored exactly; a decimal is an `f64`, exact for ordinary magnitudes.
        Value::Number(number) => {
            number.is_i64()
                || number.is_u64()
                || number
                    .as_f64()
                    .is_some_and(|value| value.abs() < SAFE_BOUND)
        }
        Value::Array(values) => values.iter().all(exact_stored_numbers),
        Value::Object(values) => values.values().all(exact_stored_numbers),
        _ => true,
    }
}

/// Whether the validator's machine-number comparison of this lexeme agrees with its decimal
/// meaning. An integer fits `i64`/`u64` and is compared exactly, unless a schema constant is a
/// decimal, which forces the comparison through `f64`. Any other lexeme must be the shortest
/// spelling of the `f64` it parses to (`9.99` is; a 30-digit fraction is not) and stay below 2^53.
fn safe_comparison(value: &str, decimal_constants: bool) -> bool {
    if !value.contains(['.', 'e', 'E']) {
        let integer = value.parse::<i64>().is_ok() || value.parse::<u64>().is_ok();
        return integer
            && (!decimal_constants
                || value
                    .trim_start_matches('-')
                    .parse::<u64>()
                    .is_ok_and(|magnitude| magnitude <= 1 << 53));
    }
    let Ok(parsed) = value.parse::<f64>() else {
        return false;
    };
    parsed.is_finite() && parsed.abs() < SAFE_BOUND && same_decimal(value, &format!("{parsed:e}"))
}

/// A fractional lexeme whose machine value is a whole number (or not finite): an integer keyword
/// would accept it although the author did not send an integer.
fn is_rounded_fraction(value: &str) -> bool {
    if lexical_is_integer(value) {
        return false;
    }
    value
        .parse::<f64>()
        .map_or(true, |parsed| !parsed.is_finite() || parsed.fract() == 0.0)
}

/// Compares two JSON number spellings by decimal value, ignoring zero padding and the exponent form.
fn same_decimal(left: &str, right: &str) -> bool {
    match (canonical_decimal(left), canonical_decimal(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// `(negative, significant digits, decimal exponent of the last digit)`; zero has no digits.
fn canonical_decimal(value: &str) -> Option<(bool, String, i64)> {
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map_or((false, value), |rest| (true, rest));
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], unsigned[at + 1..].parse::<i64>().ok()?),
        None => (unsigned, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits = format!("{whole}{fraction}");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    digits.drain(..leading);
    let trailing = digits.len() - digits.trim_end_matches('0').len();
    digits.truncate(digits.len() - trailing);
    if digits.is_empty() {
        return Some((false, digits, 0));
    }
    let places = i64::try_from(fraction.len()).ok()?;
    let trimmed = i64::try_from(trailing).ok()?;
    Some((
        negative,
        digits,
        exponent.checked_sub(places)?.checked_add(trimmed)?,
    ))
}

fn lexical_is_integer(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let exponent_at = unsigned.find(['e', 'E']);
    let (mantissa, exponent) = exponent_at.map_or((unsigned, 0_i64), |at| {
        let exponent = unsigned[at + 1..].parse::<i64>().unwrap_or_else(|_| {
            if unsigned[at + 1..].starts_with('-') {
                i64::MIN
            } else {
                i64::MAX
            }
        });
        (&unsigned[..at], exponent)
    });
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    if digits.bytes().all(|digit| digit == b'0') {
        return true;
    }
    let fractional_places = i128::try_from(fraction.len()).unwrap_or(i128::MAX);
    let scale = fractional_places - i128::from(exponent);
    if scale <= 0 {
        return true;
    }
    usize::try_from(scale)
        .ok()
        .filter(|scale| *scale <= digits.len())
        .is_some_and(|scale| {
            digits.as_bytes()[digits.len() - scale..]
                .iter()
                .all(|digit| *digit == b'0')
        })
}
