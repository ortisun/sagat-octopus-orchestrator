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
//!
//! A binding also carries the **worker pools** its identity belongs to (gap G48). A pool is a name
//! an administrator grants to a worker identity in one namespace; a secret may list the pools that
//! may receive it. The grant is the administrator's: a name a worker only *claims* when it
//! registers (its advertised `pools`, a scaling label) is never read for this decision.

use std::fmt;

use crate::workflow::ActivityName;

/// Most patterns one binding can hold.
pub const MAX_PATTERNS: usize = 64;

/// Most pools one worker identity can be granted in a namespace, and most one secret can list.
pub const MAX_POOLS: usize = 16;
/// Longest pool name.
pub const MAX_POOL_NAME: usize = 64;

/// A pool name or a list of pool names that cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PoolError {
    #[error("at most {MAX_POOLS} pools")]
    Count,
    #[error("a pool name is 1 to {MAX_POOL_NAME} letters, digits or . _ - : +")]
    InvalidName,
}

/// The name of a worker pool: 1 to 64 ASCII letters, digits or `. _ - : +` (the same alphabet the
/// pools a worker advertises use, so one label can name both). Case matters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PoolName(String);

impl PoolName {
    /// # Errors
    /// [`PoolError::InvalidName`] for an empty, long or oddly spelled name.
    pub fn parse(text: &str) -> Result<Self, PoolError> {
        let valid = !text.is_empty()
            && text.len() <= MAX_POOL_NAME
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '+'));
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(PoolError::InvalidName)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PoolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Parses a list of pools: at most [`MAX_POOLS`], sorted, without repeats. An empty list is valid
/// (for a secret it means "no restriction", for a binding "no pool").
///
/// # Errors
/// [`PoolError`] for an oversized list or an invalid name.
pub fn parse_pools<S: AsRef<str>>(texts: &[S]) -> Result<Vec<PoolName>, PoolError> {
    if texts.len() > MAX_POOLS {
        return Err(PoolError::Count);
    }
    let mut pools = texts
        .iter()
        .map(|t| PoolName::parse(t.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    pools.sort();
    pools.dedup();
    Ok(pools)
}

/// Whether a secret that lists `allowed` may be delivered to a worker granted `granted`: yes when
/// the list is empty (no restriction), otherwise when the two share a pool.
#[must_use]
pub fn pool_permits(allowed: &[String], granted: &[String]) -> bool {
    allowed.is_empty() || allowed.iter().any(|pool| granted.contains(pool))
}

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
    fn a_pool_name_is_a_short_label_in_a_closed_alphabet() {
        for ok in [
            "trusted",
            "a",
            "gpu.eu-west_1:b+c",
            &"a".repeat(MAX_POOL_NAME),
        ] {
            assert_eq!(PoolName::parse(ok).expect("valid").as_str(), ok);
        }
        for bad in [
            "",
            " ",
            "a b",
            "a/b",
            "tr\u{fc}sted",
            "a\n",
            &"a".repeat(MAX_POOL_NAME + 1),
        ] {
            assert_eq!(PoolName::parse(bad), Err(PoolError::InvalidName), "{bad:?}");
        }
        assert_ne!(
            PoolName::parse("Trusted"),
            PoolName::parse("trusted"),
            "case matters"
        );
    }

    #[test]
    fn a_pool_list_is_bounded_sorted_and_without_repeats_and_may_be_empty() {
        let none: [&str; 0] = [];
        assert_eq!(parse_pools(&none), Ok(Vec::new()));
        let many: Vec<String> = (0..=MAX_POOLS).map(|i| format!("p{i}")).collect();
        assert_eq!(parse_pools(&many), Err(PoolError::Count));
        let parsed = parse_pools(&["b", "a", "b"]).expect("valid");
        let texts: Vec<_> = parsed.iter().map(PoolName::as_str).collect();
        assert_eq!(texts, ["a", "b"]);
        assert_eq!(parse_pools(&["a", "b c"]), Err(PoolError::InvalidName));
    }

    #[test]
    fn a_secret_without_a_pool_list_goes_to_anyone_and_with_one_only_to_its_pools() {
        let list = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert!(pool_permits(&[], &[]), "no list: today's behaviour");
        assert!(pool_permits(&[], &list(&["x"])));
        assert!(pool_permits(&list(&["trusted"]), &list(&["trusted", "x"])));
        assert!(
            !pool_permits(&list(&["trusted"]), &[]),
            "no pool, no secret"
        );
        assert!(!pool_permits(&list(&["trusted"]), &list(&["Trusted", "x"])));
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
