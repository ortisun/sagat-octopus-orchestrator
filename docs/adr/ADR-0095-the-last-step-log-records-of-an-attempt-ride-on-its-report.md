# ADR-0095: The last step-log records of an attempt ride on its report

**Status:** Proposed
**Decided by:** owner chose the direction (OD-2026-11-19-1, O041-036 option (d), "Redesenhar (CompleteStep)"); this text is the
agent's proposal of how, for the owner to accept (O041-038) once design 041's spike T041-35 passes
**Refines / supersedes:** would supersede, when accepted, decisions 2, 6 and 11 of
[ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) (as revised on 2026-11-15 and read
by its Notes), the session clause of its decision 5 for a carry, and the clause of item 1 of the Amendment of 2026-11-15 of
[ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) that "no terminal report writes a step-log row except the
script-output marker"; and would refine ADR-0084 decisions 3, 7 (the final batch becomes the carry's final mark) and 9. On
acceptance both are marked "partly superseded by ADR-0095". ADR-0084's other decisions, ADR-0085 and ADR-0015 are unchanged.
**Enforced by:** nothing yet: nothing is built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md), with its row in `docs/engineering/architecture-principles.md`:
`work_budget.rs` with its three constants unmodified for reports without a carry, and six new equalities (51 statements,
1 transaction, 11 top-level rows for a completion with a next step that carries ten records, a mark-only carry or capture
off; 48, 1, 9 for the completion that ends the run), each with a second measure of the step-log rows written counted from
the step-log tables, and the cost with and without a carry at every other call site (decision 2, T041-36); the property test
that the carry function and the domain's `plan_batch` plan alike (decision 3, T041-36); a source-text test that the worker and execution modules reach the step-log tables only through the port of decision
3 (T041-36); the re-run tests of decision 5 for a completion and a failure, root and scoped (T041-36); the C0 tests of decision 3
(T041-36); the order tests of decision 6 (T041-36); the lock-order test with a carrying report beside an ingest of its attempt,
the purge and a change of a step-log resource, and the tests that a held step-log lock makes a carry `contended` with the report
committed at once (decision 7, T041-36; the delete route joins them in T041-30); the conformance scenarios `step-logs-capture` and `step-logs-transport-faults` in six SDKs,
the second asserting that a report waits for its batch's held answer at most 2 seconds (decisions 1 and 8, T041-21); the spike
T041-35 before the schema (the time bound of decision 2).

## Context

ADR-0084 decision 2 kept every step-log statement off the lease and the completion: each batch was an `AppendStepLogs` call
and a transaction of its own, and every attempt that logged sent a final batch before its report. The spike T041-31
(`specs/041-step-logs/validation-report.md`) measured that shape on a quiet machine in three rounds: the rate with capture on was
78% to 84% of the rate with capture off at 1 replica and 16 workers (median 79.2%) and 85% to 88% at 8 replicas and 128 workers
(median 87.0%), against a criterion of 90% (SC-041-007 of design 041). With the ten lines in the final batch itself, one ingest per
step, the medians were 89.4% and 89.9%. Chunk rows did not help (78.0% and 88.1%). The completion's mean wait on the tenant usage
row was at most 0.20 ms. What a step paid was the extra transactions, not the rows. The same spike measured a 500-record write at
61 ms p50 and about 200 ms p99, some 25 times a ten-record one, because each row fires a trigger and a deferred call.

Facts of the code that shape the change:

- `CompleteStep` and `FailStep` are recorded by one transaction that holds the run row `FOR UPDATE`, checks the lease, the
  binding and the membership under their locks (`ensure_bound`), applies the engine and commits (`finish`,
  `crates/application/src/worker.rs:2861`; the scoped path's `finish_path_report`). A completion with output already takes the
  tenant's `tenant_retained_usage` row at its commit.
- A report has no worker session: `CompleteStepRequest` names the attempt and its lease, and the caller is the bearer token's
  principal (`crates/api-grpc/src/service.rs:931`). The runner and database-runner roles report steps too.
- The server decodes at most `MAX_MESSAGE_BYTES` = `MAX_PAYLOAD_BYTES` (256 KiB) + 64 KiB per worker message
  (`crates/api-grpc/src/server.rs:14`), and an output may be 256 KiB.
- The report transaction runs with no lock timeout, bounded only by the pool's statement timeout, so a wait on a step-log
  lock there would be a wait under the run lock.

## Decision

1. An attempt's last records MUST travel on its report. `CompleteStepRequest` and `FailStepRequest` gain an optional
   `StepLogCarry` (protocol minor 1.7) holding at most 64 records, at most 16 KiB encoded, and the highest sequence the attempt's
   logger assigned. An attempt that assigned a sequence sends one with its report unless it was answered `STOP_ATTEMPT` or
   `STOP_CAPTURE` or its session's capture is off. The carry holds the attempt's final mark; `AppendStepLogs` carries none, and
   stays, unchanged in its checks, for the records of a long step. `Heartbeat` carries no records.
2. A report without a carry MUST cost what it costs today: no additional statement, transaction or row. A carrying report MUST add
   no transaction and exactly one statement, one round trip: a call of the database function `sagat_step_log_carry`, which tries
   the locks, reads the policy, writes the stream and the kept lines and their counters (OD-2026-11-20-1). The step-log rows are
   charged once per insert statement (ADR-0027's proposed amendment of 2026-11-20), to one of the tenant's exact leases
   (ADR-0097, Proposed), never to the usage row at the commit. These are equalities of the hot-path budget
   (constitution, CI gates), beside the existing ones, tested at every call site; since the function's writes are not top-level
   statements, the budget counts them by a second measure from the step-log tables.
3. The carry MUST be written by the transaction that records the report, after the report's own checks and the engine's change
   and before the commit, and only when the report is recorded now (never on a replay, never on a refused report). The worker and
   execution modules MUST reach the step-log tables only through one port, `StepLogCarryTx`, whose one method makes that call
   and holds no rule. The function plans the carry from the policy and the stream it reads; the domain's `plan_batch` stays
   the rule, and a property test requires the two to plan alike. A carry needs no session: the report's own checks (lease, binding and membership under their locks)
   apply, and a carry MUST be ignored when the caller's role is runner or database-runner or the step is a managed step, as the
   ingest refuses them.
4. A carry MUST NOT change a report's outcome, output or retry, nor its timing beyond its statements and decision 5's re-run,
   with one exception: decision 5's re-run is fenced as any report, so it is refused, and the worker told so, when between the
   failed transaction and the re-run the lease expired or was reclaimed, a cancel was requested, the run began to stop (for a
   forward step) or finished, or the worker's binding or membership was revoked.
   Everything that can be checked or computed before a statement (the deployment switch, the store's proof, size, sequences,
   levels, scrubbing, cutting, sizing) is, in the application; a carry that fails a check is ignored, or recorded as a final mark
   with no line when its highest sequence alone is valid, and the report is recorded.
5. A refusal of the carry's own charge (ADR-0097: `tenant_step_log_quota_exceeded` or `tenant_step_log_quota_held`, raised
   at its insert) MUST be answered inside the carry's call, by one `BEGIN … EXCEPTION` block around its writes: the carry
   then records a mark-only stream (the records counted `tenant_quota`) in the same call, and its charge's refill takes the
   usage row `NOWAIT`, a held row making the carry `contended` (revised 2026-11-20, D041-112). When the report's
   transaction fails because of its carry otherwise, the report MUST be recorded by a re-run of the report with the
   carry downgraded: after a raise of a guard of the step-log quota that the block does not catch, a mark-only carry that
   charges nothing; after a failed carry statement, none. The carry only goes down, and each downgrade can
   meet the content-quota fallback once, so a report runs at most four transactions. The two causes MUST reach the re-run as values of their own, never as an ordinary unavailability, which is
   never re-run. Item 1 of ADR-0027's Amendment then reads "a step-log charge never blocks a terminal state: a report whose step-log rows
   do not fit is recorded by a re-run without them". A report whose output or failure code holds a declared secret, root or
   scoped, and a report re-run for its retained content, write a mark-only carry.
6. Within an attempt, stored lines MUST stay in sequence order under the read order of ADR-0084 decision 9 (storing transaction
   id, then sequence). The stream row records the highest id of a transaction that stored a line of it (`last_stored_xid`, never
   lowered); a carry whose transaction id is not above it stores none of its records above `last_sequence` (they are counted as
   dropped by the server) and still records the final mark.
7. One lock order for every writer of step logs: the run rows, then the authority locks of ADR-0050 decision 7 (taken only by a
   report), then the namespace's step-log policy fences (shared), then the attempt's stream lock, then the stream row, then the
   tenant's quota class fences (shared, content, log, step-log) and a lease of the tenant's step-log quota at the insert, the
   tenant usage row only for a refill (ADR-0097), and at commit the fences and usage row of the content and log classes the
   transaction charges. An ingest takes no run row and no authority lock, and waits for the fences, the stream lock, the
   stream row and a refill's usage row under its lock timeout. A carrying report MUST NOT wait for a step-log lock: it tries
   the fences (the class fences included), the stream lock and the stream row, takes a lease only `SKIP LOCKED` and a
   refill's usage row only `NOWAIT`, and when one is held it writes nothing more and the report commits (revised
   2026-11-20, D041-112). A change of a step-log resource takes only its own fence.
8. The SDK MUST send the report only after its in-flight `AppendStepLogs` call has answered, or after a wait of at most 2
   seconds; a short step (no flush before its report, and its records fit one carry) makes no `AppendStepLogs` call and no wait.

## Considered options

1. **The last records on the report; `AppendStepLogs` for a long step** — chosen. A short step pays no transaction of its own;
   a long step pays about one ingest a second while it logs; the report's checks already fence the carry.
2. **Every record on the report, nothing streamed** — lost. A step that runs for minutes would show nothing until it ends
   (CUJ 2 of design 041), and its buffer would drop lines once full.
3. **Records on `Heartbeat` as well** — lost. A heartbeat holds the run row while it renews the lease, so a carry there would add
   statements under the run lock once a second per long step, where `AppendStepLogs` takes no run row; and a batch's size and
   failure would ride on the call that keeps the lease alive.
4. **A grouped commit of several ingests on the server** — lost again. It lengthens each hold of the tenant row while cutting
   their number, and a short step would still pay a round trip and a wait for the group.
5. **The carry written by a second transaction after the report commits, in the same call** — lost. It is the extra commit the
   owner chose to remove.
6. **A savepoint around the carry** so that its failure rolls back only itself — lost as two statements of the client on every
   carrying report, and while the charge raised at commit, where a savepoint cannot catch it. Since ADR-0097 the charge
   raises at the insert, so the carry's own call now opens one `BEGIN … EXCEPTION` block (a subtransaction, no round trip)
   for its charge's refusal (decision 5, revised 2026-11-20); its cost is measured by T041-35's third run.
7. **A carry of up to 500 records**, the batch's cap — lost. A 500-record write took about 200 ms at p99 in T041-31, all of it
   under the run lock and, as charged then, from its first deferred flush, the tenant usage row.
8. **Accept a lower criterion (O041-036 (c)) or defer the feature (O041-036 (e))** — not chosen by the owner.

## Consequences

Good: a short step adds no transaction and no round trip of its worker; its lines commit with its outcome, so a run's last
lines are visible no later than its end and a tail never waits for a final batch that is still in flight; the final flush's
two-second wait disappears for short steps.

Bad: the report's transaction holds the run row while its carry's one call runs (one round trip, one subtransaction) and
fires one statement-trigger call that charges a lease (ADR-0097; the usage row only for a refill, `NOWAIT`); a report that
also stores output takes the usage row at its commit for the content class, as today. Spike T041-35 measures it. Part of a rule (the carry's plan) runs in SQL, kept equal
to the domain's by a property test. A carry that finds a step-log lock held stores nothing (`contended`), so a rare contention loses that attempt's last
lines. A carry whose transaction fails costs one or two more report transactions, and a re-run can be refused as a late report
would be. A report of a client that negotiated below 1.7 may carry records,
and they are stored (G041-001 already covers a client that does not redact).

Compatibility: protocol minor 1.7 is not released, so its contract changes in place (D041-090): `AppendStepLogsRequest` loses
`final_batch` and `highest_sequence` (reserved), and `CompleteStepRequest` and `FailStepRequest` gain field 13. An API replica
that predates the release serving carries ignores the field as unknown: the report is recorded and the carry is lost, so a rolling
upgrade keeps the deployment switch off until every replica serves carries. Rollback of the binary leaves the tables unread, as
in ADR-0084.

## Evidence

The spike of T041-31 and its quiet rerun (`specs/041-step-logs/validation-report.md`, 2026-11-19); the owner's answer
OD-2026-11-19-1; the code read at `90c3e17a`: `complete_inner` and `recover_success_storage_quota`
(`crates/application/src/worker/output.rs`), `finish` (`crates/application/src/worker.rs:2861`), `recover_failed_storage_quota`
(`worker.rs:2453`), `refuse_scoped_output` and `refuse_scoped_content` (`worker/scoped_output_refusal.rs`), the gRPC handlers
(`crates/api-grpc/src/service.rs:931, 1244`), `MAX_MESSAGE_BYTES` (`crates/api-grpc/src/server.rs:14`), and the budget
(`tests/integration/claim-transition-efficiency/work_budget.rs:223-225`). The figures that decide whether this ADR is put for
acceptance are T041-35's.
