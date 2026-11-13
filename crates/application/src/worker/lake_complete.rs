//! The completion of a lake step (feature 035, T035-059, D035-142): the receipt the runner reports is read into typed fields,
//! checked against the step, and written again. What the run keeps as the output of the step is that receipt, never the bytes
//! the runner sent.
use super::*;
use crate::database::ExpectedReceipt;

/// What each check of an accepted result said, counted with closed labels: the check, how it ended and the code of a failure (`none`
/// otherwise). Never a connection, a run or a tenant. `lake_probe_checks_total`, once per check of a kept result.
pub(super) fn count_probe_checks(result: &sagat_domain::database::ProbeResult) {
    for check in result.checks() {
        metrics::counter!(
            "lake_probe_checks_total",
            "check" => check.check().as_str(),
            "outcome" => check.outcome().as_str(),
            "code" => check.code().map_or("none", sagat_domain::database::DatabaseFailure::code)
        )
        .increment(1);
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// The canonical receipt of the Iceberg write `task` is the attempt of, or the refusal of what the runner reported. Runs in
    /// the transaction of the completion, so the lease it was checked against is the lease that completes the step.
    ///
    /// Refused: no codec to read the report with (`Unavailable`: the server is not configured for it); an attempt that never held the
    /// credentials of its write, and a receipt that is not the step's (`NotAllowed`: the runner's mistake, which leaves the step as it was
    /// for the runner to report the failure it has, and which the caller audits as a denial).
    pub(super) async fn lake_receipt<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        output: &[u8],
    ) -> Result<crate::database::RebuiltReceipt, AppError> {
        let codec = self.lake_receipts.as_ref().ok_or(AppError::Unavailable)?;
        let node = task.instance.node();
        // a mistake of the runner, which never asked for what it says it wrote with: it will not succeed on a retry, so it is not `Unavailable`
        if !tx.lake_attempt_resolved(node).await? {
            return Err(AppError::NotAllowed(
                "the attempt did not resolve its credentials",
            ));
        }
        let write = tx
            .parts()
            .0
            .node(node)
            .and_then(sagat_domain::workflow::Node::iceberg_write)
            .cloned()
            .ok_or(AppError::Internal)?;
        let template = Self::task_template(tx.parts().0, task.instance);
        let info = tx.info().clone();
        let data = tx.data().await?;
        let (_, effect_key, _) = self.task_identity(template, data, &info, task.instance)?;
        codec
            .rebuild(
                output,
                &ExpectedReceipt {
                    effect_key: &effect_key,
                    reference: write.reference(),
                    mode: write.intent().mode(),
                    allow_empty: super::lake_input::APPEND_ALLOWS_EMPTY,
                },
            )
            .map_err(|_| AppError::NotAllowed("invalid lake receipt"))
    }

    /// The canonical result of the probe `task` is the attempt of, or the refusal of what the runner reported (D035-150). Runs in the
    /// transaction of the completion, so the lease it was checked against is the lease that completes the step.
    ///
    /// Refused: no codec to read the report with (`Unavailable`); an attempt that never held the credentials of the connection it says it
    /// tested, and a report that is not the result of this probe, of this connection at this revision (`NotAllowed`: the runner's mistake,
    /// which leaves the step as it was for the runner to report the failure it has, and which the caller audits as a denial).
    pub(super) async fn lake_probe_result<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        output: &[u8],
    ) -> Result<crate::database::RebuiltProbe, AppError> {
        let codec = self.lake_probes.as_ref().ok_or(AppError::Unavailable)?;
        let node = task.instance.node();
        // a probe that never asked for the credentials of the connection has tested nothing
        if !tx.lake_attempt_resolved(node).await? {
            return Err(AppError::NotAllowed(
                "the attempt did not resolve its credentials",
            ));
        }
        let facts = tx.lake_probe_facts(node).await?;
        codec
            .rebuild(
                output,
                &crate::database::ExpectedProbe {
                    connection: &facts.connection,
                    revision: facts.revision,
                },
            )
            .map_err(|_| AppError::NotAllowed("invalid lake probe result"))
    }

    /// The audit of a refused probe result: a denial for the run, like a refused receipt, off the request path.
    pub(super) async fn audit_lake_probe_refusal(&self, principal: &Principal, task: &TaskRef<'_>) {
        let audit = AuditRecord::by(
            principal,
            AuditAction::DatabaseDenied,
            ResourceType::Run,
            task.run_id.to_string(),
            None,
            self.clock.now(),
        )
        .with_reason("lake probe result refused");
        if RunStore::record_denial(&self.store, &audit).await.is_err() {
            tracing::warn!("the denial of a refused lake probe result could not be recorded");
        }
    }

    /// The audit of a committed write (FR-035-030): one record for the run, made from the typed receipt the server built (`commit`, which
    /// the port gives only for a commit) and from the step, never from what the runner sent, naming the node, the connection, the table and
    /// the snapshot, and never a value, a partition value, a location or the effect key. Only numbers come from the receipt.
    pub(super) fn committed_write_audit<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        commit: Option<crate::database::CommittedWrite>,
    ) -> Option<AuditRecord> {
        let facts = commit?;
        let node = task.instance.node();
        let write = tx
            .parts()
            .0
            .node(node)
            .and_then(sagat_domain::workflow::Node::iceberg_write)?;
        let table = write.table();
        let mut reason = format!(
            "node:{node} connection:{} table:{}.{} snapshot:{}",
            write.connection().name,
            table.namespace().join("."),
            table.name(),
            facts.snapshot_id,
        );
        if let Some(rows) = facts.rows_added {
            reason.push_str(&format!(" rows:{rows}"));
        }
        Some(
            AuditRecord::by(
                principal,
                AuditAction::IcebergWriteCommitted,
                ResourceType::Run,
                task.run_id.to_string(),
                None,
                self.clock.now(),
            )
            .with_reason(reason),
        )
    }

    /// A receipt the runner sent that is not the step's, or a completion that no resolve preceded, is a denial like a capability refused:
    /// audited once for the run (an identical event is recorded once), off the request path. A failure to write it does not replace the
    /// refusal the runner is told.
    pub(super) async fn audit_lake_refusal(&self, principal: &Principal, task: &TaskRef<'_>) {
        let audit = AuditRecord::by(
            principal,
            AuditAction::DatabaseDenied,
            ResourceType::Run,
            task.run_id.to_string(),
            None,
            self.clock.now(),
        )
        .with_reason("lake receipt refused");
        if RunStore::record_denial(&self.store, &audit).await.is_err() {
            tracing::warn!("the denial of a refused lake receipt could not be recorded");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use metrics::{
        Counter, CounterFn, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit,
    };
    use sagat_domain::database::{
        DatabaseFailure, ProbeCheck, ProbeCheckResult, ProbeOutcome, ProbeResult,
    };

    use super::count_probe_checks;

    /// One increment of a counter: its name, its labels and the amount.
    type Seen = (String, Vec<(String, String)>, u64);

    struct Sink(Arc<Mutex<Vec<Seen>>>);
    struct Cell(Key, Arc<Mutex<Vec<Seen>>>);
    impl CounterFn for Cell {
        fn increment(&self, value: u64) {
            let labels = self
                .0
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect();
            self.1
                .lock()
                .unwrap()
                .push((self.0.name().to_owned(), labels, value));
        }
        fn absolute(&self, _: u64) {}
    }
    impl Recorder for Sink {
        fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
            Counter::from_arc(Arc::new(Cell(key.clone(), Arc::clone(&self.0))))
        }
        fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
            Gauge::noop()
        }
        fn register_histogram(&self, _: &Key, _: &Metadata<'_>) -> Histogram {
            Histogram::noop()
        }
    }

    /// What `count_probe_checks` counts for `result`, as `(check, outcome, code)` in the order it counted.
    fn counted(result: &ProbeResult) -> Vec<(String, String, String)> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        metrics::with_local_recorder(&Sink(Arc::clone(&seen)), || count_probe_checks(result));
        let seen = seen.lock().unwrap();
        seen.iter()
            .map(|(name, labels, amount)| {
                assert_eq!(name, "lake_probe_checks_total");
                assert_eq!(*amount, 1);
                let label = |wanted: &str| {
                    labels
                        .iter()
                        .find(|(key, _)| key == wanted)
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default()
                };
                assert_eq!(
                    labels.len(),
                    3,
                    "closed labels and nothing else: {labels:?}"
                );
                (label("check"), label("outcome"), label("code"))
            })
            .collect()
    }

    fn result(outcomes: [(ProbeOutcome, Option<DatabaseFailure>); 3]) -> ProbeResult {
        ProbeResult::new(
            ProbeCheck::ALL
                .into_iter()
                .zip(outcomes)
                .map(|(check, (outcome, code))| ProbeCheckResult::new(check, outcome, code))
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn every_check_of_a_result_is_counted_once_with_its_outcome_and_the_code_of_a_failure() {
        let passed = result([
            (ProbeOutcome::Passed, None),
            (ProbeOutcome::Passed, None),
            (ProbeOutcome::Passed, None),
        ]);
        assert_eq!(
            counted(&passed),
            [
                ("CATALOG_AUTH".into(), "PASSED".into(), "none".into()),
                ("WAREHOUSE".into(), "PASSED".into(), "none".into()),
                ("STORAGE_ACCESS".into(), "PASSED".into(), "none".into()),
            ]
        );

        let failed = result([
            (ProbeOutcome::Passed, None),
            (
                ProbeOutcome::Failed,
                Some(DatabaseFailure::CatalogUnavailable),
            ),
            (ProbeOutcome::Skipped, None),
        ]);
        assert_eq!(
            counted(&failed),
            [
                ("CATALOG_AUTH".into(), "PASSED".into(), "none".into()),
                (
                    "WAREHOUSE".into(),
                    "FAILED".into(),
                    "CATALOG_UNAVAILABLE".into()
                ),
                ("STORAGE_ACCESS".into(), "SKIPPED".into(), "none".into()),
            ]
        );
    }
}
