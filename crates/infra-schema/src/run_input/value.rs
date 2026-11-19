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
            // An integral `f64` is `mantissa × 2^exponent`: its remainder is `(mantissa mod d) × (2^exponent mod d) mod
            // d`, exact, and in steps logarithmic in the exponent. IEEE `%` is exact too, but loops over the exponent's bits
            // (1.2 µs for 2^996 by 3 on the development machine), D043-108.
            Self::Float(value) => {
                value.is_finite() && value.fract() == 0.0 && float_remainder(value, divisor) == 0
            }
        }
    }
}

/// The remainder of the integral, finite `value` by `divisor`, exactly.
fn float_remainder(value: f64, divisor: u64) -> u64 {
    let bits = value.abs().to_bits();
    let biased = (bits >> 52) & 0x7ff;
    let fraction = bits & ((1 << 52) - 1);
    if biased == 0 {
        // zero or a subnormal, and an integral subnormal is zero
        return 0;
    }
    let mantissa = u128::from(fraction | (1 << 52));
    let exponent = i64::try_from(biased).unwrap_or(0) - 1075;
    let d = u128::from(divisor);
    if exponent < 0 {
        // below 2^53: the value is the mantissa shifted right by bits that are all zero (it is integral)
        return u64::try_from((mantissa >> exponent.unsigned_abs()) % d).unwrap_or(1);
    }
    let mut power = 1u128 % d;
    let mut base = 2u128 % d;
    let mut rest = exponent.unsigned_abs();
    while rest > 0 {
        if rest & 1 == 1 {
            power = power * base % d;
        }
        base = base * base % d;
        rest >>= 1;
    }
    u64::try_from(mantissa % d * power % d).unwrap_or(1)
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

/// What a value keeps on the heap, estimated (D043-105): a `Value` per element and per member, the bytes of every string
/// and key with an allocation's overhead, and a map node's share per member.
pub(super) fn heap(value: &Value) -> usize {
    const ALLOCATION: usize = 16;
    const MAP_MEMBER: usize = 32;
    let mut total = 0usize;
    let mut pending: Vec<&Value> = vec![value];
    while let Some(next) = pending.pop() {
        match next {
            Value::String(text) => total = total.saturating_add(text.len() + ALLOCATION),
            Value::Array(items) => {
                total =
                    total.saturating_add(items.len() * std::mem::size_of::<Value>() + ALLOCATION);
                pending.extend(items.iter());
            }
            Value::Object(members) => {
                for (key, member) in members {
                    total = total.saturating_add(
                        key.len() + ALLOCATION + MAP_MEMBER + std::mem::size_of::<Value>(),
                    );
                    pending.push(member);
                }
            }
            _ => {}
        }
    }
    total
}

/// [`size`], but the walk stops once it has counted `cap`, so measuring costs at most `cap` steps. Also answers the steps
/// the walk took (every value pushed or popped), which the validator counts as its primitive operations.
pub(super) fn size_capped(value: &Value, cap: u64) -> u64 {
    size_counted(value, cap).0
}

/// [`size_capped`] with the steps it took. Every value waiting on the stack will count at least one when it is popped,
/// so the walk stops before a push that would bring what is counted and what waits to the cap (D043-100): measuring a
/// long array against a small member pushes no more than the member's size.
pub(super) fn size_counted(value: &Value, cap: u64) -> (u64, u64) {
    let mut total: u64 = 0;
    let mut steps: u64 = 0;
    let mut pending: Vec<&Value> = vec![value];
    let reached = |total: u64, pending: usize| total.saturating_add(pending as u64) >= cap;
    while let Some(next) = pending.pop() {
        steps += 1;
        total = total.saturating_add(1);
        if total >= cap {
            return (cap, steps);
        }
        match next {
            Value::String(text) => total = total.saturating_add(text.len() as u64),
            Value::Array(items) => {
                for item in items {
                    if reached(total, pending.len()) {
                        return (cap, steps);
                    }
                    steps += 1;
                    pending.push(item);
                }
            }
            Value::Object(members) => {
                for (key, member) in members {
                    total = total.saturating_add(key.len() as u64);
                    if reached(total, pending.len()) {
                        return (cap, steps);
                    }
                    steps += 1;
                    pending.push(member);
                }
            }
            _ => {}
        }
        if total >= cap {
            return (cap, steps);
        }
    }
    (total, steps)
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
        // the exact remainder agrees with IEEE `%`, which is exact, over large and small integral floats
        for value in [
            2f64.powi(996),
            3.0 * 2f64.powi(990),
            1e300,
            3e300,
            1e17,
            6.0e18,
            12.0,
            4_503_599_627_370_497.0,
        ] {
            for divisor in [1u64, 2, 3, 7, 10, 1_000_003, 1 << 53] {
                #[allow(clippy::cast_precision_loss)]
                let ieee = value % (divisor as f64) == 0.0;
                assert_eq!(
                    Num::Float(value).is_multiple_of(divisor),
                    ieee,
                    "{value:e} by {divisor}"
                );
            }
        }
    }
}
