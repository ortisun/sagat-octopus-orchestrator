//! Numbers compared by their mathematical value and JSON values compared and sized for the counter (D043-060).
//!
//! `serde_json` holds a number as an `i64`, a `u64` or an `f64` (the workspace does not enable `arbitrary_precision`,
//! so an integer past `u64` is already an `f64` when it reaches here). `serde_json`'s own equality says `1 != 1.0`;
//! JSON Schema says they are the same number, so every comparison here is by value, exactly.

use std::cmp::Ordering;

use serde_json::{Number, Value};

use super::counter::Counter;

/// A JSON number by its value.
#[derive(Debug, Clone, Copy)]
pub(super) enum Num {
    Int(i128),
    Float(f64),
}

impl Num {
    pub(super) fn of(number: &Number) -> Self {
        if let Some(value) = number.as_u64() {
            Self::Int(i128::from(value))
        } else if let Some(value) = number.as_i64() {
            Self::Int(i128::from(value))
        } else {
            // Not an integer of 64 bits: serde_json holds it as an `f64`, which is always there then.
            Self::Float(number.as_f64().unwrap_or(f64::NAN))
        }
    }

    /// Whether the number has no fractional part, as both drafts define `integer`.
    pub(super) fn is_integral(self) -> bool {
        match self {
            Self::Int(_) => true,
            Self::Float(value) => value.is_finite() && value.fract() == 0.0,
        }
    }

    /// The order of two numbers by value. A NaN (which JSON cannot hold) orders as equal, never as a refusal.
    pub(super) fn compare(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a.cmp(&b),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            (Self::Int(a), Self::Float(b)) => int_float(a, b),
            (Self::Float(a), Self::Int(b)) => int_float(b, a).reverse(),
        }
    }

    /// Whether the number is a multiple of `divisor`, a positive integer of at most 2^53 (the subset refuses any other).
    pub(super) fn is_multiple_of(self, divisor: u64) -> bool {
        match self {
            Self::Int(value) => value % i128::from(divisor) == 0,
            // An integral `f64` and an integral divisor below 2^53: `%` on them is exact in IEEE 754.
            #[allow(clippy::cast_precision_loss)]
            Self::Float(value) => {
                value.is_finite() && value.fract() == 0.0 && value % (divisor as f64) == 0.0
            }
        }
    }
}

/// The order of an integer and a float, exactly: the float's integral part is compared as an integer, and its
/// fractional part breaks a tie.
fn int_float(integer: i128, float: f64) -> Ordering {
    if float.is_nan() {
        return Ordering::Equal;
    }
    // 2^127, exactly representable; every `i128` is below it and at or above its negation.
    const BOUND: f64 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0;
    if float >= BOUND {
        return Ordering::Less;
    }
    if float < -BOUND {
        return Ordering::Greater;
    }
    let whole = float.trunc();
    // `whole` is integral and within the range of `i128`, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation)]
    let whole_int = whole as i128;
    match integer.cmp(&whole_int) {
        Ordering::Equal => {
            if float > whole {
                Ordering::Less
            } else if float < whole {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        other => other,
    }
}

/// The size of a value for the counter: one per node and one per byte of every string and key. Comparing two
/// values never does more than the smaller of their sizes.
pub(super) fn size(value: &Value) -> u64 {
    size_capped(value, u64::MAX)
}

/// [`size`], but the walk stops once it has counted `cap`, so measuring costs at most `cap` steps.
pub(super) fn size_capped(value: &Value, cap: u64) -> u64 {
    let mut total: u64 = 0;
    let mut pending: Vec<&Value> = vec![value];
    while let Some(next) = pending.pop() {
        total = total.saturating_add(1);
        match next {
            Value::String(text) => total = total.saturating_add(text.len() as u64),
            Value::Array(items) => pending.extend(items.iter()),
            Value::Object(members) => {
                for (key, member) in members {
                    total = total.saturating_add(key.len() as u64);
                    pending.push(member);
                }
            }
            _ => {}
        }
        if total >= cap {
            return cap;
        }
    }
    total
}

/// Whether two values are equal as JSON Schema defines it: numbers by value, everything else structurally. Each
/// node compared is one primitive operation of the counter.
pub(super) fn equal(left: &Value, right: &Value, counter: &mut Counter) -> bool {
    counter.op(1);
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => Num::of(a).compare(Num::of(b)) == Ordering::Equal,
        (Value::String(a), Value::String(b)) => {
            counter.op(a.len().min(b.len()) as u64 / 64);
            a == b
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal(x, y, counter))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, x)| {
                    counter.op(1);
                    b.get(key).is_some_and(|y| equal(x, y, counter))
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(text: &str) -> Num {
        Num::of(&serde_json::from_str::<Number>(text).unwrap_or_else(|_| Number::from(0)))
    }

    #[test]
    fn numbers_compare_by_value_across_representations() {
        assert_eq!(num("1").compare(num("1.0")), Ordering::Equal);
        assert_eq!(num("-0.0").compare(num("0")), Ordering::Equal);
        assert_eq!(
            num("18446744073709551615").compare(num("-1")),
            Ordering::Greater
        );
        assert_eq!(
            num("9007199254740993").compare(num("9007199254740992.0")),
            Ordering::Greater
        );
        assert_eq!(num("2.5").compare(num("2")), Ordering::Greater);
        assert_eq!(num("-2.5").compare(num("-2")), Ordering::Less);
        assert_eq!(
            num("1e300").compare(num("18446744073709551615")),
            Ordering::Greater
        );
        assert!(num("1.0").is_integral() && !num("1.5").is_integral());
        assert!(num("12").is_multiple_of(3) && !num("13").is_multiple_of(3));
        assert!(num("-12").is_multiple_of(4) && num("1e20").is_multiple_of(5));
        assert!(!num("7.5").is_multiple_of(1));
    }
}
