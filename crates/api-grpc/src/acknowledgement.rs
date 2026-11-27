//! The wire of `AcknowledgeTasks` (worker protocol 1.8; feature 006, Task 006-049): the answer in request order, and the
//! statuses of a call that failed as a whole, each with the envelope `x-sagat-error`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use sagat_api_http::error::ApiError;
use sagat_application::worker::acknowledgement::{
    AcknowledgementFailure, AcknowledgementVerdict, REASON_ACKNOWLEDGEMENT_DEADLINE,
    REASON_ACKNOWLEDGEMENT_STORE_UNREACHABLE,
};
use tonic::Status;

use sagat_application::clock::Clock;
use sagat_application::entropy::Entropy;
use sagat_application::error::StoreError;
use sagat_application::execution::ExpressionEvaluator;
use sagat_application::worker::WorkerService;
use sagat_application::worker::acknowledgement::AcknowledgementStore;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Principal, TenantId};
use tokio::sync::Semaphore;

use crate::pb;
use crate::status::{invalid, to_status};

/// Cross-tenant checks in flight at once, per process. A check can scan the runs (no index on the run id alone, D006-110),
/// so the checks have a small gate of their own instead of the places of the denials' audits, at most one per tenant so that
/// no tenant keeps the others' checks out; a check that finds no place is dropped and counted.
const CROSS_TENANT_CHECKS: usize = 4;

static CROSS_TENANT_GATE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(CROSS_TENANT_CHECKS)));
static CROSS_TENANT_BUSY: LazyLock<Mutex<HashSet<TenantId>>> = LazyLock::new(Mutex::default);

/// The place of one tenant's check, given back when it ends.
struct TenantPlace(TenantId);

impl TenantPlace {
    fn take(tenant: TenantId) -> Option<Self> {
        CROSS_TENANT_BUSY
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tenant)
            .then_some(Self(tenant))
    }
}

impl Drop for TenantPlace {
    fn drop(&mut self) {
        CROSS_TENANT_BUSY
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// Checks, off the answer path, whether the runs a call addressed with no matching row exist in another tenant, and records
/// the first (`CrossTenantAccessDenied`), under a gate of its own: counted in
/// `worker_task_acknowledgement_cross_tenant_checks_total{outcome}` (`recorded`, `none`, `failed`; `busy` when the store had
/// no idle connection for it; `dropped_tenant` when the tenant already has a check in flight, `dropped_full` when the gate is
/// full). The check never waits for a connection and has a statement timeout of its own, so it needs no timeout here.
pub(crate) fn check_other_tenants<S, C, E, R>(
    workers: Arc<WorkerService<S, C, E, R>>,
    principal: Principal,
    runs: Vec<RunId>,
    trace_id: Option<String>,
) where
    S: AcknowledgementStore + 'static,
    C: Clock + 'static,
    E: ExpressionEvaluator + 'static,
    R: Entropy + 'static,
{
    let count = |outcome: &'static str| {
        metrics::counter!("worker_task_acknowledgement_cross_tenant_checks_total", "outcome" => outcome)
            .increment(1);
    };
    let Some(tenant_place) = TenantPlace::take(principal.tenant_id()) else {
        count("dropped_tenant");
        return;
    };
    let Ok(place) = Arc::clone(&CROSS_TENANT_GATE).try_acquire_owned() else {
        count("dropped_full");
        return;
    };
    tokio::spawn(async move {
        let checked = workers
            .audit_other_tenants(&principal, &runs, trace_id)
            .await;
        drop((place, tenant_place));
        count(match checked {
            Ok(true) => "recorded",
            Ok(false) => "none",
            Err(StoreError::DatabaseLimit) => "busy",
            Err(error) => {
                tracing::warn!(%error, "an acknowledgement's cross-tenant check failed");
                "failed"
            }
        });
    });
}

/// The status of a call that failed as a whole. A deadline and an unreachable database are `UNAVAILABLE` with their
/// reason, never `DEADLINE_EXCEEDED` (the host keeps that for its own bound); every other transient failure is a plain
/// `UNAVAILABLE`: no transient failure is ever answered `NOT_HELD`.
pub(crate) fn failure_status(failure: AcknowledgementFailure) -> Status {
    match failure {
        AcknowledgementFailure::Deadline => {
            to_status(&ApiError::unavailable_after(1).with_reason(REASON_ACKNOWLEDGEMENT_DEADLINE))
        }
        AcknowledgementFailure::StoreUnreachable => to_status(
            &ApiError::unavailable_after(1).with_reason(REASON_ACKNOWLEDGEMENT_STORE_UNREACHABLE),
        ),
        AcknowledgementFailure::Transient => to_status(&ApiError::unavailable_after(1)),
        AcknowledgementFailure::Malformed(message) => invalid(message),
        // the handler maps a refusal through the service's `app`, which leaves the denial for the audit; this is the plain
        // mapping, for the tests and any other caller
        AcknowledgementFailure::Refused(error) => to_status(&ApiError::from(error)),
    }
}

/// The status of a call that failed before it reached the service (its deadline passed while it waited for a place of its
/// connection, or the lookups of its authentication did not reach the store), counted as the service counts its own:
/// `worker_task_acknowledgement_failures_total{reason}`.
pub(crate) fn handler_failure(failure: AcknowledgementFailure) -> Status {
    metrics::counter!("worker_task_acknowledgement_failures_total", "reason" => failure.label())
        .increment(1);
    failure_status(failure)
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// One result per task, in request order.
pub(crate) fn response<T: AsRef<str>>(
    task_ids: &[T],
    verdicts: &[AcknowledgementVerdict],
) -> pb::AcknowledgeTasksResponse {
    use pb::AcknowledgementVerdict as Wire;
    pb::AcknowledgeTasksResponse {
        results: task_ids
            .iter()
            .zip(verdicts)
            .map(|(task_id, verdict)| {
                let (wire, lease, attempt) = match verdict {
                    AcknowledgementVerdict::Acknowledged {
                        lease_time_left,
                        attempt_time_left,
                        ..
                    } => (
                        Wire::Acknowledged,
                        millis(*lease_time_left),
                        attempt_time_left.map(millis),
                    ),
                    AcknowledgementVerdict::NotHeld(_) => (Wire::NotHeld, 0, None),
                    AcknowledgementVerdict::Cancelled(_) => (Wire::Cancelled, 0, None),
                    AcknowledgementVerdict::Retry => (Wire::Retry, 0, None),
                };
                pb::TaskAcknowledgementResult {
                    task_id: task_id.as_ref().to_owned(),
                    verdict: wire.into(),
                    lease_time_left_ms: lease,
                    attempt_time_left_ms: attempt,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::ERROR_METADATA;
    use sagat_application::error::AppError;
    use sagat_application::worker::acknowledgement::{Acknowledged, Cancelled, NotHeld};
    use tonic::Code;

    fn envelope(status: &Status) -> Option<&str> {
        status
            .metadata()
            .get(ERROR_METADATA)
            .and_then(|value| value.to_str().ok())
    }

    #[test]
    fn every_failure_carries_the_envelope_and_the_retryable_ones_are_unavailable() {
        let deadline = failure_status(AcknowledgementFailure::Deadline);
        assert_eq!(deadline.code(), Code::Unavailable);
        assert_eq!(envelope(&deadline), Some("ACKNOWLEDGEMENT_DEADLINE"));
        let store = failure_status(AcknowledgementFailure::StoreUnreachable);
        assert_eq!(store.code(), Code::Unavailable);
        assert_eq!(envelope(&store), Some("ACKNOWLEDGEMENT_STORE_UNREACHABLE"));
        let transient = failure_status(AcknowledgementFailure::Transient);
        assert_eq!(transient.code(), Code::Unavailable);
        assert_eq!(envelope(&transient), Some("UNAVAILABLE"));
        let malformed = failure_status(AcknowledgementFailure::Malformed(
            "tasks must hold 1 to 64 tasks",
        ));
        assert_eq!(malformed.code(), Code::InvalidArgument);
        assert_eq!(envelope(&malformed), Some("INVALID_ARGUMENT"));
        let refused = failure_status(AcknowledgementFailure::Refused(AppError::Forbidden {
            permission: sagat_domain::identity::Permission::WorkerExecute,
        }));
        assert_eq!(refused.code(), Code::PermissionDenied);
        assert!(envelope(&refused).is_some());
        for status in [&deadline, &store, &transient] {
            assert!(
                status
                    .metadata()
                    .get(crate::status::RETRY_AFTER_METADATA)
                    .is_some(),
                "a retry hint"
            );
        }
    }

    #[test]
    fn the_answer_is_one_result_per_task_in_request_order() {
        let verdicts = [
            AcknowledgementVerdict::Acknowledged {
                how: Acknowledged::Written,
                lease_time_left: Duration::from_millis(29_500),
                attempt_time_left: Some(Duration::from_millis(12_000)),
            },
            AcknowledgementVerdict::NotHeld(NotHeld::Withdrawn),
            AcknowledgementVerdict::Cancelled(Cancelled::StopAsked),
            AcknowledgementVerdict::Retry,
            AcknowledgementVerdict::Acknowledged {
                how: Acknowledged::Duplicate,
                lease_time_left: Duration::ZERO,
                attempt_time_left: None,
            },
        ];
        let answer = response(&["a", "b", "c", "d", "e"], &verdicts);
        let wire: Vec<_> = answer
            .results
            .iter()
            .map(|r| {
                (
                    r.task_id.as_str(),
                    r.verdict,
                    r.lease_time_left_ms,
                    r.attempt_time_left_ms,
                )
            })
            .collect();
        use pb::AcknowledgementVerdict as W;
        assert_eq!(
            wire,
            [
                ("a", W::Acknowledged as i32, 29_500, Some(12_000)),
                ("b", W::NotHeld as i32, 0, None),
                ("c", W::Cancelled as i32, 0, None),
                ("d", W::Retry as i32, 0, None),
                ("e", W::Acknowledged as i32, 0, None),
            ]
        );
    }
}
