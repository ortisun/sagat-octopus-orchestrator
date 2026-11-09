//! The completion of a lake step (feature 035, T035-059, D035-142): the receipt the runner reports is read into typed fields,
//! checked against the step, and written again. What the run keeps as the output of the step is that receipt, never the bytes
//! the runner sent.
use super::*;
use crate::database::ExpectedReceipt;

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
    ) -> Result<Vec<u8>, AppError> {
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

    /// The audit of a committed write (FR-035-030): one record for the run, made from the receipt the server built and from the step, never
    /// from what the runner sent, naming the node, the connection, the table and the snapshot, and never a value, a partition value, a
    /// location or the effect key. `None` for a receipt that is not of a commit (a write of no record commits nothing).
    pub(super) fn committed_write_audit<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        receipt: &[u8],
    ) -> Option<AuditRecord> {
        let facts = committed_facts(receipt)?;
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
            facts.snapshot,
        );
        if let Some(rows) = facts.rows {
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

/// What the audit of a committed write takes from the receipt: the snapshot and the rows added.
struct CommittedFacts {
    snapshot: String,
    rows: Option<String>,
}

/// The facts of a receipt of a commit, or `None` for a receipt that is not one. The bytes are the ones the server built from typed
/// fields (the snapshot and the counts are canonical decimals there), and are read again here with the digits checked, so that nothing
/// but digits can reach a record.
fn committed_facts(receipt: &[u8]) -> Option<CommittedFacts> {
    let document: serde_json::Value = serde_json::from_slice(receipt).ok()?;
    if document.get("kind")?.as_str()? != "iceberg"
        || document.get("outcome")?.as_str()? != "COMMITTED"
    {
        return None;
    }
    let digits = |name: &str| {
        document
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|text| {
                !text.is_empty()
                    && text.len() <= 20
                    && text.bytes().all(|byte| byte.is_ascii_digit())
            })
            .map(str::to_owned)
    };
    Some(CommittedFacts {
        snapshot: digits("snapshotId")?,
        rows: digits("rowsAdded"),
    })
}

#[cfg(test)]
mod tests {
    use super::committed_facts;

    fn facts(receipt: &str) -> Option<(String, Option<String>)> {
        committed_facts(receipt.as_bytes()).map(|facts| (facts.snapshot, facts.rows))
    }

    #[test]
    fn a_receipt_of_a_commit_gives_its_snapshot_and_its_rows() {
        assert_eq!(
            facts(
                r#"{"kind":"iceberg","outcome":"COMMITTED","snapshotId":"8744736658442914487","rowsAdded":"12"}"#
            ),
            Some(("8744736658442914487".to_owned(), Some("12".to_owned())))
        );
        assert_eq!(
            facts(r#"{"kind":"iceberg","outcome":"COMMITTED","snapshotId":"1"}"#),
            Some(("1".to_owned(), None)),
            "the rows are not part of every receipt"
        );
    }

    #[test]
    fn a_receipt_that_is_not_of_a_commit_is_not_a_write_to_audit() {
        for (what, receipt) in [
            (
                "a write of no record",
                r#"{"kind":"iceberg","outcome":"EMPTY_NO_COMMIT"}"#,
            ),
            (
                "a revert",
                r#"{"kind":"iceberg","outcome":"REVERTED","snapshotId":"7"}"#,
            ),
            (
                "another kind",
                r#"{"kind":"object_storage","outcome":"COMMITTED","snapshotId":"7"}"#,
            ),
            (
                "a commit with no snapshot",
                r#"{"kind":"iceberg","outcome":"COMMITTED"}"#,
            ),
            ("not JSON", "COMMITTED"),
            ("no kind", r#"{"outcome":"COMMITTED","snapshotId":"7"}"#),
        ] {
            assert_eq!(facts(receipt), None, "{what}");
        }
    }

    #[test]
    fn only_digits_can_reach_a_record() {
        for (what, snapshot) in [
            ("text", r#""abc""#),
            ("a path", r#""s3://bucket/key""#),
            ("a space and a line", r#""7 \nrows:9""#),
            ("a number", "7"),
            ("nothing", r#""""#),
            (
                "more digits than a snapshot has",
                r#""123456789012345678901""#,
            ),
        ] {
            let receipt =
                format!(r#"{{"kind":"iceberg","outcome":"COMMITTED","snapshotId":{snapshot}}}"#);
            assert_eq!(facts(&receipt), None, "{what}");
        }
        assert_eq!(
            facts(r#"{"kind":"iceberg","outcome":"COMMITTED","snapshotId":"7","rowsAdded":"x y"}"#),
            Some(("7".to_owned(), None)),
            "the rows are left out when they are not digits, and the write is still audited"
        );
    }
}
