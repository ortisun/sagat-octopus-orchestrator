//! Which activities a worker identity may serve (FR-009-002, gap G31).
//!
//! A worker identity holds the `worker:execute` permission of its tenant, which says nothing about
//! *which* work it may take: without more, any worker could register for any activity of any
//! namespace of its tenant and receive the data of those runs. A binding closes that: an identity
//! serves only the activities its bindings name, in the namespace they name. No binding, no work.
//!
//! A binding names activities with a small closed grammar, so what it covers can be read at a glance
//! and cannot be made to match by accident:
//!
//! * `payments.charge`: exactly that activity;
//! * `payments.*`: every activity whose name starts with `payments.` (the dot is part of the prefix,
//!   so it does not cover `paymentsx.charge` or `payments` itself);
//! * `*`: every activity of the namespace.

use std::fmt;

use crate::workflow::ActivityName;

/// Most patterns one binding can hold.
pub const MAX_PATTERNS: usize = 64;

/// A pattern or a list of patterns that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    #[error("a binding names between 1 and {MAX_PATTERNS} activity patterns")]
    PatternCount,
    #[error("an activity pattern is an activity name, a name prefix ending in `.*`, or `*`")]
    InvalidPattern,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Shape {
    Any,
    /// The prefix *including* its trailing dot.
    Prefix(String),
    Exact(String),
}

/// One entry of a binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActivityPattern(Shape);

impl ActivityPattern {
    /// # Errors
    /// [`BindingError::InvalidPattern`] unless it is an activity name, `<name>.*` or `*`.
    pub fn parse(text: &str) -> Result<Self, BindingError> {
        if text == "*" {
            return Ok(Self(Shape::Any));
        }
        if let Some(stem) = text.strip_suffix(".*") {
            // The stem is itself a valid activity name: no empty prefix, no second wildcard.
            return ActivityName::parse(stem)
                .map(|_| Self(Shape::Prefix(format!("{stem}."))))
                .map_err(|_| BindingError::InvalidPattern);
        }
        ActivityName::parse(text)
            .map(|name| Self(Shape::Exact(name.as_str().to_owned())))
            .map_err(|_| BindingError::InvalidPattern)
    }

    /// Whether a worker holding this pattern may serve `activity`.
    #[must_use]
    pub fn matches(&self, activity: &str) -> bool {
        match &self.0 {
            Shape::Any => true,
            Shape::Prefix(prefix) => activity.starts_with(prefix.as_str()),
            Shape::Exact(name) => name == activity,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> String {
        match &self.0 {
            Shape::Any => "*".to_owned(),
            Shape::Prefix(prefix) => format!("{prefix}*"),
            Shape::Exact(name) => name.clone(),
        }
    }
}

impl fmt::Display for ActivityPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_text())
    }
}

/// Parses the patterns of a binding: at least one, at most [`MAX_PATTERNS`], without repeats.
///
/// # Errors
/// [`BindingError`] for an empty or oversized list or an invalid pattern.
pub fn parse_patterns<S: AsRef<str>>(texts: &[S]) -> Result<Vec<ActivityPattern>, BindingError> {
    if texts.is_empty() || texts.len() > MAX_PATTERNS {
        return Err(BindingError::PatternCount);
    }
    let mut patterns = texts
        .iter()
        .map(|t| ActivityPattern::parse(t.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    patterns.sort();
    patterns.dedup();
    Ok(patterns)
}

/// The first of `activities` that no pattern covers, if there is one.
#[must_use]
pub fn first_uncovered<'a>(
    patterns: &[ActivityPattern],
    activities: &'a [String],
) -> Option<&'a str> {
    activities
        .iter()
        .find(|activity| !patterns.iter().any(|p| p.matches(activity)))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> ActivityPattern {
        ActivityPattern::parse(text).expect("valid pattern")
    }

    #[test]
    fn an_exact_pattern_matches_that_activity_and_nothing_else() {
        let pattern = p("payments.charge");
        assert!(pattern.matches("payments.charge"));
        for other in [
            "payments.charge2",
            "payments",
            "payments.",
            "Payments.charge",
            "",
        ] {
            assert!(!pattern.matches(other), "{other}");
        }
    }

    #[test]
    fn a_prefix_pattern_stops_at_the_dot() {
        let pattern = p("payments.*");
        assert!(pattern.matches("payments.charge"));
        assert!(pattern.matches("payments.refund.partial"));
        for other in [
            "payments",
            "paymentsx.charge",
            "pay.charge",
            "x.payments.charge",
            "",
        ] {
            assert!(!pattern.matches(other), "{other}");
        }
    }

    #[test]
    fn the_wildcard_matches_everything_and_only_when_it_stands_alone() {
        assert!(p("*").matches("anything.at.all"));
        for bad in ["**", "*.*", ".*", "a*", "a.*.b", "a.**", "*.a", "a.b*"] {
            assert_eq!(
                ActivityPattern::parse(bad),
                Err(BindingError::InvalidPattern),
                "{bad}"
            );
        }
    }

    #[test]
    fn what_is_not_an_activity_name_is_not_a_pattern() {
        for bad in ["", " ", "a b", "-a", ".a", "a/b", "a\n", &"a".repeat(200)] {
            assert_eq!(
                ActivityPattern::parse(bad),
                Err(BindingError::InvalidPattern),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn patterns_round_trip_through_their_text() {
        for text in ["*", "payments.*", "payments.charge", "a:b.c-d_e"] {
            assert_eq!(p(text).as_text(), text);
            assert_eq!(p(&p(text).as_text()), p(text));
        }
    }

    #[test]
    fn a_list_is_bounded_sorted_and_without_repeats() {
        let none: [&str; 0] = [];
        assert_eq!(parse_patterns(&none), Err(BindingError::PatternCount));
        let many: Vec<String> = (0..=MAX_PATTERNS).map(|i| format!("a.n{i}")).collect();
        assert_eq!(parse_patterns(&many), Err(BindingError::PatternCount));
        let parsed = parse_patterns(&["b.x", "a.*", "b.x", "*"]).expect("valid");
        let texts: Vec<_> = parsed.iter().map(ActivityPattern::as_text).collect();
        assert_eq!(texts.len(), 3, "the repeat is gone");
        assert_eq!(
            parse_patterns(&["b.x", "bad pattern"]),
            Err(BindingError::InvalidPattern)
        );
    }

    #[test]
    fn the_first_activity_nothing_covers_is_named() {
        let patterns = parse_patterns(&["payments.*", "audit.log"]).expect("valid");
        let ok = vec!["payments.charge".to_owned(), "audit.log".to_owned()];
        assert_eq!(first_uncovered(&patterns, &ok), None);
        let bad = vec![
            "payments.charge".to_owned(),
            "audit.purge".to_owned(),
            "other.thing".to_owned(),
        ];
        assert_eq!(first_uncovered(&patterns, &bad), Some("audit.purge"));
        assert_eq!(
            first_uncovered(&[], &ok),
            Some("payments.charge"),
            "no binding covers nothing"
        );
    }
}
