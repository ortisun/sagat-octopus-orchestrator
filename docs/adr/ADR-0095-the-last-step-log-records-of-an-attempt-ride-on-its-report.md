# ADR-0095: The last step-log records of an attempt ride on its report

**Status:** Proposed
**Decided by:** owner chose the direction (OD-2026-11-19-1, O041-036 option (d), "Redesenhar (CompleteStep)"); this text is the
agent's proposal of how, for the owner to accept after the review of design 041's revision of 2026-11-19
**Refines / supersedes:** would supersede, when accepted, decisions 2 and 6 of
[ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) and refine its decisions 3, 9 and 11; would
refine item 1 of the Amendment of 2026-11-15 of [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) ("no terminal report
writes a step-log row except the script-output marker"). ADR-0084's other decisions, ADR-0085 and ADR-0015 are unchanged.
**Enforced by:** nothing yet: nothing is built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): `work_budget.rs` with its three constants unmodified for reports that carry nothing, and
three new equalities for a report that carries records, one that carries none and one whose namespace has capture off (rule 2,
T041-36); a source-text test that the worker and execution modules reach the step-log tables only through the one store function
of rule 3 (T041-36); the fallback tests of rule 5 (a finalizer raise and a failed carry statement, each re-running the report
once without the carry, T041-36); the order test of rule 6 (T041-36); the lock-order test with a carrying report beside an ingest
of the same attempt, a change of a step-log resource, a purge and a delete (rule 7, T041-36); the conformance scenarios
`step-logs-capture` and `step-logs-transport-faults` in six SDKs (rules 1 and 8, T041-21); the spike T041-35 before the schema
(rule 4's time bound).

## Context

ADR-0084 decision 2 kept every step-log statement off the lease and the completion: each batch was an `AppendStepLogs` call
and a transaction of its own, and every attempt that logged sent a final batch before its report. The spike T041-31
(`specs/041-step-logs/validation-report.md`) measured that shape on a quiet machine in three rounds: the rate with capture on was
78% to 84% of the rate with capture off at 1 replica and 16 workers (median 79.2%) and 85% to 88% at 8 replicas and 128 workers
(median 87.0%), against a criterion of 90% (SC-041-007 of design 041). With the ten lines in the final batch itself, one ingest per
step, the medians were 89.4% and 89.9%. Chunk rows did not help (78.0% and 88.1%). The completion's mean wait on the tenant usage
row was at most 0.20 ms. What a step paid was the extra transactions, not the rows.

Facts of the code that shape the change:

- `CompleteStep` and `FailStep` are recorded by one transaction that holds the run row `FOR UPDATE`, checks the lease, the
  binding and the membership under their locks (`ensure_bound`), applies the engine and commits (`finish`,
  `crates/application/src/worker.rs:2861`; the scoped path's `finish_path_report`). A completion with output already takes the
  tenant's `tenant_retained_usage` row at its commit.
- A report has no worker session: `CompleteStepRequest` names the attempt and its lease, and the caller is the bearer token's
  principal (`crates/api-grpc/src/service.rs:931`).
- The server decodes at most `MAX_PAYLOAD_BYTES + 64 KiB` per worker message (`crates/api-grpc/src/server.rs:14`), and an output
  may be 256 KiB.
- When a completion's retained content does not fit, the store's finalizer raises at commit and the application re-runs the
  report once (`recover_success_storage_quota`, `crates/application/src/worker/output.rs:441`).

## Decision

1. An attempt's last records MUST travel on its report. `CompleteStepRequest` and `FailStepRequest` gain an optional
   `StepLogCarry` (protocol minor 1.7) holding at most 500 records of at most 48 KiB encoded, and the highest sequence the
   attempt's logger assigned. The carry is the attempt's final mark: an attempt that assigned a sequence sends one with its
   report, and `AppendStepLogs` carries no final mark. `AppendStepLogs` stays, unchanged in its checks, for the records of an
   attempt that is still running (a step that logs past the SDK's one-second flush or 256 records). `Heartbeat` carries no
   records.
2. A report that carries nothing MUST cost what it costs today: no statement, transaction or row. A report that carries a carry
   MUST add no transaction, and at most four statements to the transaction that records it: one that takes the shared policy
   fences of the namespace's step-log resources and reads them with the tenant's step-log allowance, one that creates or locks
   the attempt's stream row, one insert of the kept lines (absent when none is kept), and one update of the stream's counters;
   one statement in all when the namespace has capture off. It writes the stream row and one row per kept line. These are
   equalities of the hot-path budget (constitution, CI gates), beside the existing ones.
3. The carry MUST be written by the transaction that records the report, after the report's own checks and the engine's
   change and before the commit, and only when the report is recorded now (never on a replay of the same idempotency key, never
   on a refused report). The worker and execution modules MUST reach the step-log tables only through one store function that
   writes a carry; they MUST NOT read the log tables otherwise. The carry needs no session check: the report's own checks
   (lease, binding and membership under their locks) are stronger than the ingest's.
4. A carry MUST NOT change a report's outcome, output, retry or timing beyond its statements. Everything that can be checked
   before a statement (the deployment switch, size, sequences, levels, scrubbing, cutting, sizing) is checked in the
   application; a carry that fails a check is ignored and counted, and the report is recorded without it.
5. When the report's transaction fails because of its carry, the report MUST be recorded anyway: if the commit's finalizer
   raises `tenant_step_log_quota_exceeded`, the application re-runs the report once without the carry's lines, writing only the
   stream's final mark with the records counted as `tenant_quota`; if a carry statement fails (the store maps its errors to a
   typed step-log error), the application re-runs the report once with no carry at all and counts the failure. A step-log charge
   therefore never blocks a terminal state, as ADR-0027's Amendment requires; item 1 of that Amendment reads "no terminal report
   is refused or delayed past one re-run for a step-log row".
6. Within an attempt, stored lines MUST stay in sequence order under the read order of ADR-0084 decision 9 (storing transaction
   id, then sequence). The stream row records the id of the last transaction that stored a batch; a carry whose transaction id
   is lower than it stores none of its records (they are counted as dropped by the server) and still records the final mark.
7. One lock order for every writer of step logs: the run rows, then the authority locks of ADR-0050 decision 7 (taken only by a
   report), then the namespace's step-log policy fences (shared), then the stream row, then at commit the fences of the classes
   the transaction charges (content, log, step-log) and the tenant usage row. An ingest takes no run row and no authority lock; a
   report with a carry takes all. A change of a step-log resource takes only its own fence.
8. The SDK MUST send the report only after its in-flight `AppendStepLogs` call has answered, or after a wait of at most 2
   seconds; a short step (no flush before its report) makes no `AppendStepLogs` call and no wait.

## Considered options

1. **The last records on the report; `AppendStepLogs` for a running attempt** — chosen. A step shorter than its first flush pays
   no transaction of its own; a long step pays one ingest a second, which its own length amortises; the report's checks already
   fence the carry.
2. **Every record on the report, nothing streamed** — lost. A step that runs for minutes would show nothing until it ends
   (CUJ 2 of design 041), and its buffer would drop lines once full.
3. **Records on `Heartbeat` as well** — lost. A heartbeat holds the run row while it renews the lease, so a carry there would add
   statements under the run lock once a second per long step, where `AppendStepLogs` takes no run row; and a batch's size and
   failure would ride on the call that keeps the lease alive.
4. **A grouped commit of several ingests on the server** — lost again. It lengthens each hold of the tenant row while cutting
   their number, and a short step would still pay a round trip and a wait for the group.
5. **The carry written by a second transaction after the report commits, in the same call** — lost. It is the extra commit the
   owner chose to remove.
6. **A savepoint around the carry** so that its failure rolls back only itself — lost. Two more statements on every carrying
   report for a failure that should not happen; and a finalizer raise happens at commit, which a savepoint cannot catch.
7. **Accept a lower criterion (O041-036 (c)) or defer the feature (O041-036 (e))** — not chosen by the owner.

## Consequences

Good: a step that logs less than one flush adds no transaction and no round trip; its lines commit with its outcome, so a run's
last lines are visible no later than its end and a tail never waits for a final batch that is still in flight; the final flush's
two-second wait disappears for short steps.

Bad, and stated: a report that carries records takes the tenant's usage row at its commit even when its output stores nothing,
so a tenant's logging completions queue on that row at commit, as completions with output do today; spike T041-35 measures it
with an output that stores nothing. The completion's transaction holds the run row while its carry runs four statements and
fires one trigger call per line at insert and one deferred call per line at commit. A carry can wait for an ingest of its own
attempt that still holds the stream row (only after the SDK's two-second wait expired) and for a change of a step-log resource
queued on its fence. A finalizer raise near the tenant's step-log limit costs one more report transaction. A report of a client
that negotiated below 1.7 may carry records, and they are stored (G041-001 already covers a client that does not redact).

Compatibility: protocol minor 1.7 is not released, so its contract changes in place (D041-090): `AppendStepLogsRequest` loses
`final_batch` and `highest_sequence` (reserved), and `CompleteStepRequest` and `FailStepRequest` gain field 13. An API replica
older than 1.7 ignores the field as unknown: the report is recorded and the carry is lost, which the stream shows as "ended
without its final flush". Rollback of the binary leaves the tables unread, as in ADR-0084.

## Evidence

The spike of T041-31 and its quiet rerun (`specs/041-step-logs/validation-report.md`, 2026-11-19); the owner's answer
OD-2026-11-19-1; the code read at `90c3e17a`: `complete_inner` and `recover_success_storage_quota`
(`crates/application/src/worker/output.rs`), `finish` (`crates/application/src/worker.rs:2861`), the gRPC handlers
(`crates/api-grpc/src/service.rs:931, 1244`), `MAX_MESSAGE_BYTES` (`crates/api-grpc/src/server.rs:14`), and the budget
(`tests/integration/claim-transition-efficiency/work_budget.rs:223-225`). The figures that decide whether this ADR is proposed for
acceptance are T041-35's.
