//! The binding checks of a worker call, in one function over a narrow trait (feature 006, Task 006-049;
//! `stream-completion.md`, "The server's acknowledgement", step 3).
//!
//! The run transactions (root and path) and the delivery acknowledgement of protocol 1.8 judge a worker's binding the same
//! way, in the same order and under the same locks: the binding's shared advisory lock with the namespace's bound
//! patterns, then the namespace's shared secrets lock with the membership's row lock. The locks are the adapter's; the
//! order and the decision are here, once, so that a narrowed binding is refused by every path alike.

use std::future::Future;

use sagat_domain::binding::ActivityPattern;
use sagat_domain::identity::Permission;

use crate::error::{AppError, StoreError};
use crate::execution::{PathExecutionTx, RunTx};

/// What the binding checks read, in one namespace the implementer is scoped to.
pub(crate) trait BindingChecks: Send {
    type Error: Send;

    /// The activity patterns `subject` is bound to, read under the binding's shared advisory lock, which a binding
    /// replacement takes exclusively before it changes grants or leases.
    fn bound_patterns(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send;

    /// Whether `subject` is still a member that may execute work, read under the namespace's shared secrets lock and the
    /// membership's row lock, which a revocation waits for.
    fn current_worker_authorized(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

/// What the binding checks found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingJudgement {
    /// For each activity, in the order given, whether the binding covers it. The membership was checked unless no activity
    /// was covered (there is then nothing to authorize).
    Judged(Vec<bool>),
    /// The identity is no longer a member that may execute work (`Forbidden`).
    NotMember,
}

/// A failure of the binding checks.
#[derive(Debug)]
pub(crate) enum BindingFailure<E> {
    /// The store failed.
    Store(E),
    /// A stored pattern does not parse: the store is corrupt.
    CorruptPattern,
}

/// The binding checks of one worker call over `activities`, the activities of the nodes it addresses (none for a node that
/// is not an activity). The patterns are read only when there is an activity to cover; the membership is checked when
/// there is no activity or at least one is covered, so that a call refused for its binding takes no membership lock.
pub(crate) async fn judge_binding<B: BindingChecks>(
    checks: &mut B,
    subject: &str,
    activities: &[String],
) -> Result<BindingJudgement, BindingFailure<B::Error>> {
    let covered = if activities.is_empty() {
        Vec::new()
    } else {
        let bound = checks
            .bound_patterns(subject)
            .await
            .map_err(BindingFailure::Store)?
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| BindingFailure::CorruptPattern)?;
        activities
            .iter()
            .map(|activity| {
                sagat_domain::binding::first_uncovered(&bound, std::slice::from_ref(activity))
                    .is_none()
            })
            .collect()
    };
    if !activities.is_empty() && !covered.contains(&true) {
        return Ok(BindingJudgement::Judged(covered));
    }
    if !checks
        .current_worker_authorized(subject)
        .await
        .map_err(BindingFailure::Store)?
    {
        return Ok(BindingJudgement::NotMember);
    }
    Ok(BindingJudgement::Judged(covered))
}

/// What a call that addresses one node makes of the judgement: refused `NotBound` when its activity is not covered,
/// `Forbidden` when the identity is no longer a member that executes work.
pub(crate) fn refusal(
    judged: Result<BindingJudgement, BindingFailure<StoreError>>,
) -> Result<(), AppError> {
    match judged {
        Ok(BindingJudgement::Judged(covered)) if covered.contains(&false) => {
            Err(AppError::NotBound)
        }
        Ok(BindingJudgement::Judged(_)) => Ok(()),
        Ok(BindingJudgement::NotMember) => Err(AppError::Forbidden {
            permission: Permission::WorkerExecute,
        }),
        Err(BindingFailure::Store(error)) => Err(error.into()),
        Err(BindingFailure::CorruptPattern) => Err(AppError::Internal),
    }
}

/// The binding checks of a whole-run transaction, in the run's namespace.
pub(crate) struct RunBinding<'a, T>(pub(crate) &'a mut T);

impl<T: RunTx> BindingChecks for RunBinding<'_, T> {
    type Error = StoreError;

    fn bound_patterns(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send {
        self.0.bound_patterns(subject)
    }

    fn current_worker_authorized(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        self.0.current_worker_authorized(subject)
    }
}

/// The binding checks of a path transaction, in the run's namespace.
pub(crate) struct PathBinding<'a, T>(pub(crate) &'a mut T);

impl<T: PathExecutionTx> BindingChecks for PathBinding<'_, T> {
    type Error = StoreError;

    fn bound_patterns(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send {
        self.0.bound_patterns(subject)
    }

    fn current_worker_authorized(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        self.0.current_worker_authorized(subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Patterns and a membership, with what was asked, in order.
    struct Fixed {
        patterns: Vec<String>,
        member: bool,
        asked: Vec<&'static str>,
    }

    impl BindingChecks for Fixed {
        type Error = StoreError;

        async fn bound_patterns(&mut self, _subject: &str) -> Result<Vec<String>, StoreError> {
            self.asked.push("patterns");
            Ok(self.patterns.clone())
        }

        async fn current_worker_authorized(&mut self, _subject: &str) -> Result<bool, StoreError> {
            self.asked.push("membership");
            Ok(self.member)
        }
    }

    fn fixed(patterns: &[&str], member: bool) -> Fixed {
        Fixed {
            patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
            member,
            asked: Vec::new(),
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime")
            .block_on(future)
    }

    fn activities(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn coverage_is_judged_per_activity_and_the_membership_after_the_patterns() {
        let mut checks = fixed(&["payments.*"], true);
        let judged = block_on(judge_binding(
            &mut checks,
            "w",
            &activities(&["payments.charge", "mail.send"]),
        ))
        .expect("judged");
        assert_eq!(judged, BindingJudgement::Judged(vec![true, false]));
        assert_eq!(checks.asked, ["patterns", "membership"]);
    }

    #[test]
    fn nothing_covered_takes_no_membership_lock() {
        let mut checks = fixed(&["mail.*"], true);
        let judged = block_on(judge_binding(
            &mut checks,
            "w",
            &activities(&["payments.charge"]),
        ))
        .expect("judged");
        assert_eq!(judged, BindingJudgement::Judged(vec![false]));
        assert_eq!(checks.asked, ["patterns"]);
    }

    #[test]
    fn a_node_that_is_no_activity_checks_the_membership_alone() {
        let mut member = fixed(&[], true);
        assert_eq!(
            block_on(judge_binding(&mut member, "w", &[])).expect("judged"),
            BindingJudgement::Judged(vec![])
        );
        assert_eq!(member.asked, ["membership"]);
        let mut revoked = fixed(&[], false);
        assert_eq!(
            block_on(judge_binding(&mut revoked, "w", &[])).expect("judged"),
            BindingJudgement::NotMember
        );
    }

    #[test]
    fn a_revoked_member_is_refused_once_something_is_covered() {
        let mut checks = fixed(&["*"], false);
        assert_eq!(
            block_on(judge_binding(&mut checks, "w", &activities(&["a"]))).expect("judged"),
            BindingJudgement::NotMember
        );
    }

    #[test]
    fn a_pattern_that_does_not_parse_is_corruption() {
        let mut checks = fixed(&["not a pattern!"], true);
        assert!(matches!(
            block_on(judge_binding(&mut checks, "w", &activities(&["a"]))),
            Err(BindingFailure::CorruptPattern)
        ));
    }
}
