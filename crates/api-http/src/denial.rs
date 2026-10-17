//! Audit of refused calls (gap G9, decided 2026-10-16): every `403` / `PERMISSION_DENIED` leaves one
//! audit record naming the tenant, the subject, the permission (or refusal code) and the call by its
//! route template or RPC name, with the trace id.
//!
//! The answer to the caller is built before and independently of this: a failure to write the
//! record is logged and counted, and the caller still gets the same `403`. That is still failing
//! closed, because the request was refused already.
//!
//! The write is decided once the answer is a 403 and then runs as a task of its own: a client that
//! resets the connection, or a gRPC deadline that drops the call, does not cancel the audit of a
//! denial that was already decided. The request still waits for it, so in the normal case the record
//! exists before the answer is sent.
//!
//! Writes are bounded in number (see [`Gate`]): at most [`MAX_PENDING_WRITES`] at a time per
//! process, each with the same ceiling on waiting and writing together. A write that does not get
//! through in time is not dropped silently: it is logged and counted as `outcome="failed"`.
//!
//! Volume is bounded by the per-subject budget, which [`crate::extract::authenticate_bearer`]
//! charges before anything is authorized: a request the budget refuses is a `429` and never reaches
//! this point, so no second limiter is needed.

use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use sagat_application::error::{AccessDenial, StoreError};
use sagat_application::identity::RequestContext;
use sagat_domain::identity::Principal;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::core::Core;

/// How long a denial may wait for a place and be written before it is counted as failed.
const WRITE_BUDGET: Duration = Duration::from_secs(5);

/// Writes of denials in flight at once, per process. The database pool is smaller than a flood of
/// denials (the per-subject budget allows tens per second per subject), so without this bound every
/// extra write would queue on the pool, each holding a task and a request context.
pub const MAX_PENDING_WRITES: usize = 32;

/// Response extension: this answer is a denial of the given kind.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Denied(pub AccessDenial);

/// What became of the audit write of one denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Recorded,
    /// The write failed, or did not get a place and finish within the budget. Logged and counted.
    Failed,
}

/// Bounds the denial writes in flight and how long one may take.
pub(crate) struct Gate {
    places: Semaphore,
    budget: Duration,
}

impl Gate {
    pub(crate) fn new(places: usize, budget: Duration) -> Self {
        Self {
            places: Semaphore::new(places),
            budget,
        }
    }

    async fn run<F>(&self, write: F) -> AuditOutcome
    where
        F: Future<Output = Result<(), StoreError>>,
    {
        let bounded = async {
            // The semaphore is never closed; a refusal would only mean "write without a place".
            let _place = self.places.acquire().await.ok();
            write.await
        };
        match tokio::time::timeout(self.budget, bounded).await {
            Ok(Ok(())) => AuditOutcome::Recorded,
            Ok(Err(error)) => {
                tracing::error!(%error, "failed to record the audit entry of a denied call");
                AuditOutcome::Failed
            }
            Err(_) => {
                tracing::error!("timed out recording the audit entry of a denied call");
                AuditOutcome::Failed
            }
        }
    }
}

static GATE: LazyLock<Arc<Gate>> =
    LazyLock::new(|| Arc::new(Gate::new(MAX_PENDING_WRITES, WRITE_BUDGET)));

/// Runs `write` as a task of its own under `gate` and counts its outcome. Dropping the returned
/// handle (the request went away) leaves the task running.
pub(crate) fn spawn_gated<F>(gate: Arc<Gate>, write: F) -> JoinHandle<AuditOutcome>
where
    F: Future<Output = Result<(), StoreError>> + Send + 'static,
{
    tokio::spawn(async move {
        let outcome = gate.run(write).await;
        let label = match outcome {
            AuditOutcome::Recorded => "recorded",
            AuditOutcome::Failed => "failed",
        };
        metrics::counter!("access_denial_audits_total", "outcome" => label).increment(1);
        outcome
    })
}

/// Starts the write of the record of a denial and returns at once. Await the handle to wait for the
/// record; drop it and the write goes on. It never changes the answer: a failure is logged (without
/// the caller's data) and counted by outcome, a closed set.
pub fn spawn_audit_access_denied<St: Core>(
    state: &St,
    principal: &Principal,
    call: &str,
    denial: AccessDenial,
    ctx: &RequestContext,
) -> JoinHandle<AuditOutcome> {
    let (state, principal, call, ctx) = (
        state.clone(),
        principal.clone(),
        call.to_owned(),
        ctx.clone(),
    );
    spawn_gated(Arc::clone(&GATE), async move {
        state
            .record_access_denied(&principal, &call, denial, &ctx)
            .await
    })
}

/// A caller that was authenticated, able to audit its own denial.
pub(crate) trait AuditsDenial: Send + Sync {
    fn audit(&self, call: &str, denial: AccessDenial) -> JoinHandle<AuditOutcome>;
}

struct Bound<St> {
    state: St,
    principal: Principal,
    ctx: RequestContext,
}

impl<St: Core> AuditsDenial for Bound<St> {
    fn audit(&self, call: &str, denial: AccessDenial) -> JoinHandle<AuditOutcome> {
        spawn_audit_access_denied(&self.state, &self.principal, call, denial, &self.ctx)
    }
}

/// Where the extractor leaves the authenticated caller for the middleware that sees the answer.
#[derive(Clone, Default)]
pub(crate) struct DenialSlot(Arc<Mutex<Option<Arc<dyn AuditsDenial>>>>);

impl DenialSlot {
    pub(crate) fn bind<St: Core>(&self, state: &St, principal: &Principal, ctx: &RequestContext) {
        let bound: Arc<dyn AuditsDenial> = Arc::new(Bound {
            state: state.clone(),
            principal: principal.clone(),
            ctx: ctx.clone(),
        });
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(bound);
    }

    pub(crate) fn take(&self) -> Option<Arc<dyn AuditsDenial>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn writes_in_flight_never_exceed_the_bound_and_none_is_lost() {
        let gate = Arc::new(Gate::new(2, Duration::from_secs(10)));
        let (active, peak, done) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (active, peak, done) =
                    (Arc::clone(&active), Arc::clone(&peak), Arc::clone(&done));
                spawn_gated(Arc::clone(&gate), async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    done.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            })
            .collect();

        for handle in handles {
            assert_eq!(handle.await.expect("task"), AuditOutcome::Recorded);
        }

        assert_eq!(done.load(Ordering::SeqCst), 8, "every denial was written");
        assert!(peak.load(Ordering::SeqCst) <= 2, "bounded concurrency");
    }

    #[tokio::test]
    async fn a_write_that_cannot_get_a_place_in_time_fails_explicitly_instead_of_vanishing() {
        let gate = Arc::new(Gate::new(1, Duration::from_millis(150)));
        // Another writer holds the only place for the whole test.
        let held = gate.places.acquire().await.expect("a place");
        let waiting_ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&waiting_ran);
        let waiting = spawn_gated(Arc::clone(&gate), async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });

        assert_eq!(waiting.await.expect("task"), AuditOutcome::Failed);
        assert!(
            !waiting_ran.load(Ordering::SeqCst),
            "it never got a place, and said so"
        );
        drop(held);
    }

    #[tokio::test]
    async fn a_failing_store_is_a_failed_outcome() {
        let gate = Arc::new(Gate::new(1, Duration::from_secs(1)));
        let outcome = spawn_gated(gate, async {
            Err(StoreError::Internal {
                detail: "audit table unavailable".into(),
            })
        })
        .await
        .expect("task");
        assert_eq!(outcome, AuditOutcome::Failed);
    }

    #[tokio::test]
    async fn the_write_goes_on_when_whoever_waits_for_it_is_dropped() {
        let gate = Arc::new(Gate::new(1, Duration::from_secs(10)));
        let release = Arc::new(tokio::sync::Notify::new());
        let written = Arc::new(AtomicBool::new(false));
        let (wait_for, flag) = (Arc::clone(&release), Arc::clone(&written));
        let handle = spawn_gated(gate, async move {
            wait_for.notified().await;
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        // The request future that awaits the handle is dropped before the write finishes.
        let request = tokio::spawn(handle);
        tokio::task::yield_now().await;
        request.abort();
        assert!(request.await.expect_err("aborted").is_cancelled());
        assert!(!written.load(Ordering::SeqCst));

        release.notify_one();

        for _ in 0..100 {
            if written.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the write was cancelled with the request");
    }
}
