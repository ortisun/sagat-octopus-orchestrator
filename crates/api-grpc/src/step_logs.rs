//! `AppendStepLogs`, the RPC of protocol 1.7 that stores the log records of a leased attempt that is still running
//! (design 041 sections 1 and 2, task T041-7). A file of its own, so that the test selector matches the step-log code
//! by its path (task T041-32).
//!
//! The handler authenticates the worker, parses the batch, takes the tenant's ticket (P3), calls the ingest
//! (`sagat_application::step_logs::StepLogIngest`, which holds every rule) and maps its answer to the wire. Two things
//! are the handler's alone:
//!
//! * **The tenant's tickets (P3).** At most `SAGAT_STEP_LOGS_TENANT_CALLS` batches of one tenant run at once on a
//!   replica. A batch that finds no ticket free is answered `RETRY_LATER` with 200 ms at once: no call waits for a
//!   ticket, so a call holds its place in the Logs class (`crate::limits`) only while it runs and one tenant cannot
//!   fill that class with waiting calls.
//! * **The audit of an `AUTHORITY` answer** (decision D041-146 (2)). `AUTHORITY` is a verdict, not an error status, so
//!   the audit of refused calls in `service::observe` does not see it; the handler writes the record itself through
//!   `spawn_audit_access_denied`, bounded by its gate and counted (`access_denial_audits_total`), and waits for it
//!   before it answers, as `observe` does for a `PERMISSION_DENIED`.
//!
//! Nothing of a record's content reaches a log, a span, a metric or an error here (FR-041-007 (d)): the span carries
//! the number of records and the verdict, and a parse error names the field, never its value.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use sagat_api_http::error::ApiError;
use sagat_api_http::{Budgets, Core, RateLimitConfig, RateLimitConfigError, RateLimiter};
use sagat_application::clock::Clock;
use sagat_application::definitions::DefinitionStore;
use sagat_application::entropy::Entropy;
use sagat_application::error::{AccessDenial, AppError};
use sagat_application::execution::{ExpressionEvaluator, PathExecutionStore};
use sagat_application::identity::RequestContext;
use sagat_application::runs::RunStore;
use sagat_application::step_logs::{
    BatchAnswer, RETRY_AFTER_MS, StepLogBatch, StepLogIngest, StepLogRecordIn, StepLogStore,
    StepLogVerdict,
};
use sagat_application::worker::WorkerStore;
use sagat_domain::identity::{Principal, TenantId};
use sagat_domain::step_logs::{FieldValue, StepLogLevel, StopReason};
use time::OffsetDateTime;
use tonic::{Request, Response, Status};
use tracing::Instrument;

use crate::pb;
use crate::request::{Attempt, worker_id};
use crate::service::{WorkerGrpc, app, attach_instance, observe, rpc_name, trace_of};
use crate::status::{invalid, to_status};

/// The RPC's name in metrics, spans and the audit trail.
const METHOD: &str = "AppendStepLogs";

/// Tickets one tenant may hold at once on a replica when `SAGAT_STEP_LOGS_TENANT_CALLS` is not set (decision
/// D041-127, data-model.md §6): 7, at most the Logs class's cap.
pub const DEFAULT_STEP_LOG_TENANT_CALLS: usize = 7;

/// The closed code an `AUTHORITY` answer is audited with: the worker's binding or membership no longer covers the
/// step, the refusal a report of the same attempt would meet (`NOT_BOUND`).
const AUTHORITY_DENIAL: AccessDenial = AccessDenial::Refused("NOT_BOUND");

/// Longest session token accepted (tokens are 64 hex characters), as for a lease token.
const MAX_SESSION_TOKEN: usize = 128;

// ---------------------------------------------------------------------------------- the ingest, as the handler sees it

/// The ingest as the handler calls it. Object safe, so that `WorkerGrpc` keeps its type parameters: one boxed future
/// a batch, nothing beside the batch's own transaction.
pub trait StepLogAppender: Send + Sync {
    /// P1, P2 and P4's count and size bounds on the request's counts, before the batch is built or a ticket taken
    /// (`StepLogIngest::precheck`): `Ok(Some)` is the answer, `Ok(None)` goes on.
    ///
    /// # Errors
    /// `Forbidden` (P1) or `NotAllowed` (P4), already counted.
    fn precheck(
        &self,
        principal: &Principal,
        records: usize,
        encoded_len: usize,
    ) -> Result<Option<BatchAnswer>, AppError>;

    /// Stores one batch (`StepLogIngest::append`).
    fn append<'a>(
        &'a self,
        principal: &'a Principal,
        batch: &'a StepLogBatch,
    ) -> Pin<Box<dyn Future<Output = Result<BatchAnswer, AppError>> + Send + 'a>>;
}

impl<S, C> StepLogAppender for StepLogIngest<S, C>
where
    S: StepLogStore,
    C: Clock,
{
    fn precheck(
        &self,
        principal: &Principal,
        records: usize,
        encoded_len: usize,
    ) -> Result<Option<BatchAnswer>, AppError> {
        Self::precheck(self, principal, records, encoded_len)
    }

    fn append<'a>(
        &'a self,
        principal: &'a Principal,
        batch: &'a StepLogBatch,
    ) -> Pin<Box<dyn Future<Output = Result<BatchAnswer, AppError>> + Send + 'a>> {
        Box::pin(Self::append(self, principal, batch))
    }
}

// ---------------------------------------------------------------------------------- P3, the tenant's tickets

/// The tickets of P3: how many batches each tenant has running on this replica. Built as `TaskStreams` is
/// (`crate::limits`), a count per tenant behind one lock, so that a ticket is taken and given back exactly once.
#[derive(Debug)]
pub(crate) struct TenantTickets {
    per_tenant: usize,
    held: Mutex<HashMap<TenantId, usize>>,
}

/// One running batch of a tenant; given back when dropped, however the call ends.
#[derive(Debug)]
pub(crate) struct TenantTicket {
    owner: Arc<TenantTickets>,
    tenant: TenantId,
}

impl TenantTickets {
    /// At most `per_tenant` tickets for each tenant (at least 1).
    pub(crate) fn new(per_tenant: usize) -> Arc<Self> {
        Arc::new(Self {
            per_tenant: per_tenant.max(1),
            held: Mutex::new(HashMap::new()),
        })
    }

    /// A ticket of `tenant`, or `None` when its tickets are all taken. Never waits.
    pub(crate) fn admit(self: &Arc<Self>, tenant: TenantId) -> Option<TenantTicket> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let count = held.entry(tenant).or_insert(0);
        if *count >= self.per_tenant {
            return None;
        }
        *count += 1;
        Some(TenantTicket {
            owner: Arc::clone(self),
            tenant,
        })
    }

    /// The tickets `tenant` holds now (tests only).
    #[cfg(test)]
    fn held(&self, tenant: TenantId) -> usize {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&tenant)
            .copied()
            .unwrap_or(0)
    }
}

impl Drop for TenantTicket {
    fn drop(&mut self) {
        let mut held = self
            .owner
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = held.get_mut(&self.tenant) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                // a tenant with no batch running costs no memory
                held.remove(&self.tenant);
            }
        }
    }
}

/// The rate budgets of step-log batches, apart from those of every other worker call (decision D041-150): the same
/// rates as the worker API's own budgets, in buckets of their own, so that a worker whose batches are refused and
/// resent never spends the budget its heartbeats and reports need (FR-041-005).
#[derive(Debug, Clone, Copy)]
pub struct StepLogBudgets {
    /// Per verified subject, charged before any database lookup.
    pub subject: RateLimitConfig,
    /// Per tenant, charged after authentication.
    pub tenant: RateLimitConfig,
}

/// What `WorkerGrpc` holds to serve `AppendStepLogs`: the ingest, the tenants' tickets and the batches' budgets.
#[derive(Clone)]
pub(crate) struct StepLogs {
    ingest: Arc<dyn StepLogAppender>,
    tickets: Arc<TenantTickets>,
    subject_budget: Arc<RateLimiter<String>>,
    tenant_budget: Arc<RateLimiter>,
}

impl StepLogs {
    pub(crate) fn new(
        ingest: Arc<dyn StepLogAppender>,
        tenant_calls: usize,
        budgets: StepLogBudgets,
    ) -> Result<Self, RateLimitConfigError> {
        Ok(Self {
            ingest,
            tickets: TenantTickets::new(tenant_calls),
            subject_budget: Arc::new(RateLimiter::new(budgets.subject)?),
            tenant_budget: Arc::new(RateLimiter::new(budgets.tenant)?),
        })
    }

    /// The budgets a batch is charged to, named `step_logs_subject` and `step_logs_tenant` in
    /// `http_rate_limited_total`.
    pub(crate) fn budgets(&self) -> Budgets<'_> {
        Budgets {
            subject: &self.subject_budget,
            tenant: &self.tenant_budget,
            scope: ("step_logs_subject", "step_logs_tenant"),
        }
    }
}

// ---------------------------------------------------------------------------------- the handler

impl<St, S, C, E, R> WorkerGrpc<St, S, C, E, R>
where
    St: Core,
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore + 'static,
    C: Clock + 'static,
    E: ExpressionEvaluator + 'static,
    R: Entropy + 'static,
{
    /// `AppendStepLogs`: authenticate against the batches' own budgets, run P1, P2 and P4's bounds on the request's
    /// counts, parse, take the tenant's ticket (P3), call the ingest, audit an `AUTHORITY` answer while the ticket is
    /// still held, and answer the verdict. `UNIMPLEMENTED` on an instance built without the ingest, which a worker treats
    /// as an older replica (contracts.md §4: it stops batches for the attempt and carries the rest on its report).
    pub(crate) async fn serve_append_step_logs(
        &self,
        request: Request<pb::AppendStepLogsRequest>,
    ) -> Result<Response<pb::AppendStepLogsResponse>, Status> {
        let trace = trace_of(&request);
        let trace_id = trace.trace_id().to_string();
        observe(self.core(), METHOD, trace, async {
            let Some(step_logs) = self.step_logs() else {
                return Err(Status::unimplemented(
                    "AppendStepLogs is not served on this instance",
                ));
            };
            let auth = self
                .authenticate_with(&request, Some(step_logs.budgets()))
                .await?;
            let principal = &auth.session.principal;
            let message = request.get_ref();
            // P1 first, so that a refused caller never learns whether its tenant's tickets are taken; then P2 and P4's
            // bounds, before a record is copied (the security review of task T041-7).
            let checked = step_logs.ingest.precheck(
                principal,
                message.records.len(),
                prost::Message::encoded_len(message),
            );
            match checked {
                Ok(Some(answer)) => return Ok(response_of(answer)),
                Ok(None) => {}
                Err(error) => {
                    // A refused caller is audited, but not waited for: the call would otherwise hold its place in the
                    // Logs class, outside its tenant's tickets, for as long as the audit gate makes it wait, and a
                    // flood of refused calls could shed every other tenant's batches (the correctness review of task
                    // T041-7). The record is written by the bounded gate after the answer.
                    if let Some(denial) = error.denial() {
                        let ctx = RequestContext {
                            trace_id: Some(trace_id),
                            span_id: None,
                        };
                        drop(sagat_api_http::denial::spawn_audit_access_denied(
                            self.core(),
                            principal,
                            &rpc_name(METHOD),
                            denial,
                            &ctx,
                        ));
                    }
                    return Err(to_status(&ApiError::from(error)));
                }
            }
            let batch = batch_of(message, self.supports_scoped_identity())?;
            let span = tracing::info_span!(
                "step_logs.append",
                records = batch.records.len(),
                verdict = tracing::field::Empty,
            );
            let Some(ticket) = step_logs.tickets.admit(principal.tenant_id()) else {
                metrics::counter!("step_log_ticket_refusals_total").increment(1);
                span.record("verdict", "retry_later");
                return Ok(response_of(BatchAnswer {
                    verdict: StepLogVerdict::RetryLater {
                        after_ms: RETRY_AFTER_MS,
                    },
                    stored_through_sequence: 0,
                }));
            };
            let answer = step_logs
                .ingest
                .append(principal, &batch)
                .instrument(span.clone())
                .await;
            let answer = answer.map_err(app)?;
            span.record("verdict", verdict_label(answer.verdict));
            if answer.verdict == StepLogVerdict::StopAttempt(StopReason::Authority) {
                let ctx = RequestContext {
                    trace_id: Some(trace_id),
                    span_id: None,
                };
                // Waited for, so that in the normal case the record exists before the answer is sent; a failure is
                // logged and counted by the gate and never changes the answer.
                let _ = sagat_api_http::denial::spawn_audit_access_denied(
                    self.core(),
                    principal,
                    &rpc_name(METHOD),
                    AUTHORITY_DENIAL,
                    &ctx,
                )
                .instrument(span)
                .await;
            }
            // Given back only now: a call waiting for its audit still counts against its tenant (the security review
            // of task T041-7), so a flood of refused batches cannot take the Logs class from other tenants.
            drop(ticket);
            Ok(response_of(answer))
        })
        .await
    }
}

/// The batch of a request, its attempt named and fenced as a heartbeat's is, plus the session. A parse error names the
/// field and never its value.
fn batch_of(
    message: &pb::AppendStepLogsRequest,
    supports_scoped_identity: bool,
) -> Result<StepLogBatch, Status> {
    let worker = worker_id(&message.worker_id)?;
    if message.session_token.is_empty() || message.session_token.len() > MAX_SESSION_TOKEN {
        return Err(invalid("session_token is missing or too long"));
    }
    let mut attempt = Attempt::parse(
        &message.task_id,
        &message.run_id,
        &message.step_id,
        message.attempt,
        message.generation,
        &message.lease_token,
    )?;
    attach_instance(
        &mut attempt,
        message.instance.as_ref(),
        supports_scoped_identity,
    )?;
    let step_id = attempt
        .step_id
        .ok_or_else(|| invalid("step_id is required"))?;
    Ok(StepLogBatch {
        worker_id: worker,
        session_token: message.session_token.clone(),
        run_id: attempt.run_id,
        step_id,
        instance: attempt.instance,
        fence: attempt.fence,
        lease_token: attempt.lease_token,
        records: message.records.iter().map(record_of).collect(),
        encoded_len: prost::Message::encoded_len(message),
    })
}

/// One record as the worker sent it. An unspecified or unknown level is `None`, which P4 refuses; a worker time of 0
/// (not set) or outside what `OffsetDateTime` holds is `None`, and the record keeps its server time; a field whose
/// value is not set is dropped and the record marked cut (`truncated`), as the server's own cut of the fields is.
fn record_of(record: &pb::StepLogRecord) -> StepLogRecordIn {
    let level = match pb::StepLogLevel::try_from(record.level) {
        Ok(pb::StepLogLevel::Trace) => Some(StepLogLevel::Trace),
        Ok(pb::StepLogLevel::Debug) => Some(StepLogLevel::Debug),
        Ok(pb::StepLogLevel::Info) => Some(StepLogLevel::Info),
        Ok(pb::StepLogLevel::Warn) => Some(StepLogLevel::Warn),
        Ok(pb::StepLogLevel::Error) => Some(StepLogLevel::Error),
        Ok(pb::StepLogLevel::Unspecified) | Err(_) => None,
    };
    let worker_time = (record.worker_time_epoch_ms != 0)
        .then(|| {
            OffsetDateTime::from_unix_timestamp_nanos(
                i128::from(record.worker_time_epoch_ms) * 1_000_000,
            )
            .ok()
        })
        .flatten();
    let mut valueless = false;
    let fields = record
        .fields
        .iter()
        .filter_map(|field| {
            let value = match &field.value {
                Some(pb::step_log_field::Value::StringValue(text)) => {
                    FieldValue::String(text.clone())
                }
                Some(pb::step_log_field::Value::IntValue(number)) => FieldValue::Int(*number),
                Some(pb::step_log_field::Value::DoubleValue(number)) => FieldValue::double(*number),
                Some(pb::step_log_field::Value::BoolValue(flag)) => FieldValue::Bool(*flag),
                None => {
                    valueless = true;
                    return None;
                }
            };
            Some((field.key.clone(), value))
        })
        .collect();
    StepLogRecordIn {
        sequence: record.sequence,
        level,
        worker_time,
        message: record.message.clone(),
        exception: record.exception.clone(),
        fields,
        truncated: record.truncated || valueless,
    }
}

/// The wire answer of a verdict. `retry_after_ms` is set only with `RETRY_LATER`, within the contract's 50 to 5,000.
fn response_of(answer: BatchAnswer) -> pb::AppendStepLogsResponse {
    let (verdict, reason, retry_after_ms) = match answer.verdict {
        StepLogVerdict::Continue => (pb::StepLogVerdict::Continue, None, 0),
        StepLogVerdict::FinalOnly => (pb::StepLogVerdict::FinalOnly, None, 0),
        StepLogVerdict::StopAttempt(reason) => (pb::StepLogVerdict::StopAttempt, Some(reason), 0),
        StepLogVerdict::StopCapture(reason) => (pb::StepLogVerdict::StopCapture, Some(reason), 0),
        StepLogVerdict::RetryLater { after_ms } => (
            pb::StepLogVerdict::RetryLater,
            None,
            i32::try_from(after_ms.clamp(50, 5_000)).unwrap_or(5_000),
        ),
    };
    pb::AppendStepLogsResponse {
        verdict: verdict.into(),
        stop_reason: reason
            .map_or(pb::StepLogStopReason::Unspecified, stop_reason_of)
            .into(),
        stored_through_sequence: answer.stored_through_sequence,
        retry_after_ms,
    }
}

const fn stop_reason_of(reason: StopReason) -> pb::StepLogStopReason {
    match reason {
        StopReason::FinalRecorded => pb::StepLogStopReason::FinalRecorded,
        StopReason::Lease => pb::StepLogStopReason::Lease,
        StopReason::Authority => pb::StepLogStopReason::Authority,
        StopReason::Removed => pb::StepLogStopReason::Removed,
        StopReason::CaptureOff => pb::StepLogStopReason::CaptureOff,
        StopReason::Session => pb::StepLogStopReason::Session,
    }
}

/// The verdict as the span names it: a closed set.
const fn verdict_label(verdict: StepLogVerdict) -> &'static str {
    match verdict {
        StepLogVerdict::Continue => "continue",
        StepLogVerdict::FinalOnly => "final_only",
        StepLogVerdict::StopAttempt(_) => "stop_attempt",
        StepLogVerdict::StopCapture(_) => "stop_capture",
        StepLogVerdict::RetryLater { .. } => "retry_later",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantId {
        TenantId::new(uuid::Uuid::now_v7())
    }

    /// Data-model.md §6 and design section 1: a second tenant's batch is admitted while the first tenant's seven
    /// tickets are all taken, and every ticket is given back.
    #[test]
    fn a_second_tenant_is_admitted_while_the_first_holds_all_its_tickets() {
        let tickets = TenantTickets::new(DEFAULT_STEP_LOG_TENANT_CALLS);
        let (first, second) = (tenant(), tenant());
        let held: Vec<_> = (0..7)
            .map(|_| {
                tickets
                    .admit(first)
                    .expect("one of the first tenant's seven")
            })
            .collect();
        assert!(
            tickets.admit(first).is_none(),
            "an eighth batch of the first tenant finds no ticket"
        );
        assert_eq!(tickets.held(first), 7);
        let other = tickets
            .admit(second)
            .expect("the second tenant's batch is admitted");
        assert_eq!(tickets.held(second), 1);
        drop(held);
        assert_eq!(tickets.held(first), 0, "every ticket is given back");
        assert!(tickets.admit(first).is_some(), "a ticket is free again");
        drop(other);
        assert!(
            tickets
                .held
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty(),
            "a tenant with no batch running costs no memory"
        );
    }

    #[test]
    fn a_ticket_set_holds_at_least_one_ticket_a_tenant() {
        let tickets = TenantTickets::new(0);
        let first = tenant();
        let one = tickets.admit(first).expect("at least one ticket");
        assert!(tickets.admit(first).is_none());
        drop(one);
    }

    #[test]
    fn each_verdict_maps_to_its_wire_answer() {
        let continued = response_of(BatchAnswer {
            verdict: StepLogVerdict::Continue,
            stored_through_sequence: 9,
        });
        assert_eq!(continued.verdict, i32::from(pb::StepLogVerdict::Continue));
        assert_eq!(
            continued.stop_reason,
            i32::from(pb::StepLogStopReason::Unspecified)
        );
        assert_eq!(continued.stored_through_sequence, 9);
        assert_eq!(continued.retry_after_ms, 0);
        for verdict in [
            StepLogVerdict::FinalOnly,
            StepLogVerdict::StopAttempt(StopReason::FinalRecorded),
        ] {
            let answer = response_of(BatchAnswer {
                verdict,
                stored_through_sequence: 11,
            });
            assert_eq!(answer.stored_through_sequence, 11, "{verdict:?}");
        }
        for (reason, wire) in [
            (
                StopReason::FinalRecorded,
                pb::StepLogStopReason::FinalRecorded,
            ),
            (StopReason::Lease, pb::StepLogStopReason::Lease),
            (StopReason::Authority, pb::StepLogStopReason::Authority),
            (StopReason::Removed, pb::StepLogStopReason::Removed),
        ] {
            let stopped = response_of(BatchAnswer {
                verdict: StepLogVerdict::StopAttempt(reason),
                stored_through_sequence: 0,
            });
            assert_eq!(stopped.verdict, i32::from(pb::StepLogVerdict::StopAttempt));
            assert_eq!(stopped.stop_reason, i32::from(wire));
        }
        for (reason, wire) in [
            (StopReason::CaptureOff, pb::StepLogStopReason::CaptureOff),
            (StopReason::Session, pb::StepLogStopReason::Session),
        ] {
            let stopped = response_of(BatchAnswer {
                verdict: StepLogVerdict::StopCapture(reason),
                stored_through_sequence: 0,
            });
            assert_eq!(stopped.verdict, i32::from(pb::StepLogVerdict::StopCapture));
            assert_eq!(stopped.stop_reason, i32::from(wire));
        }
        let retry = response_of(BatchAnswer {
            verdict: StepLogVerdict::RetryLater { after_ms: 200 },
            stored_through_sequence: 0,
        });
        assert_eq!(retry.verdict, i32::from(pb::StepLogVerdict::RetryLater));
        assert_eq!(retry.retry_after_ms, 200);
        let bounded = response_of(BatchAnswer {
            verdict: StepLogVerdict::RetryLater { after_ms: 1 },
            stored_through_sequence: 0,
        });
        assert_eq!(bounded.retry_after_ms, 50, "the contract's 50 to 5,000");
    }

    #[test]
    fn a_record_keeps_what_the_worker_sent_and_marks_a_valueless_field_cut() {
        let record = record_of(&pb::StepLogRecord {
            sequence: 4,
            worker_time_epoch_ms: 1_700_000_000_123,
            level: pb::StepLogLevel::Warn.into(),
            message: "m".into(),
            fields: vec![
                pb::StepLogField {
                    key: "n".into(),
                    value: Some(pb::step_log_field::Value::IntValue(3)),
                },
                pb::StepLogField {
                    key: "x".into(),
                    value: Some(pb::step_log_field::Value::DoubleValue(f64::NAN)),
                },
                pb::StepLogField {
                    key: "gone".into(),
                    value: None,
                },
            ],
            exception: String::new(),
            truncated: false,
        });
        assert_eq!(record.sequence, 4);
        assert_eq!(record.level, Some(StepLogLevel::Warn));
        assert_eq!(
            record.worker_time.map(OffsetDateTime::unix_timestamp_nanos),
            Some(1_700_000_000_123_000_000)
        );
        assert_eq!(
            record.fields,
            vec![
                ("n".to_owned(), FieldValue::Int(3)),
                ("x".to_owned(), FieldValue::String("NaN".into())),
            ]
        );
        assert!(record.truncated, "the valueless field was dropped");
        let unset = record_of(&pb::StepLogRecord {
            level: 0,
            worker_time_epoch_ms: 0,
            ..pb::StepLogRecord::default()
        });
        assert_eq!(unset.level, None, "an unspecified level is refused by P4");
        assert_eq!(unset.worker_time, None);
        assert!(!unset.truncated);
        let unknown = record_of(&pb::StepLogRecord {
            level: 99,
            ..pb::StepLogRecord::default()
        });
        assert_eq!(unknown.level, None);
    }
}
