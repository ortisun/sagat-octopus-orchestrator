# Refusal report and drain atomicity faults — 2026-10-18

Two current-version tests exercise actual PostgreSQL writes using owner-installed test-only triggers. They do not manufacture execution rows or alter production contracts.

## Real boundaries

1. A BEFORE INSERT trigger gates the exact scoped `step.completed` journal row after the root-only persist has finished. An independent connection observes the complete original state while the writer remains blocked, confirmed through `pg_blocking_pids`. Releasing the gate produces the distinct labelled 23514 error, not a statement timeout. Public completion returns INTERNAL with no ACK. Root stop, scoped success, journal/payloads, accounting, telemetry, receipts, audits, sagas and outbox all roll back. Removing the trigger lets the identical lease/key report SUCCEEDED/CANCELLING once, with stable exact replay, followed by normal drain and 17 real engine compensation tasks.
2. A separate AFTER INSERT trigger gates the actual `collection.drain.started` outbox write after successful report commit. Both while blocked and after the labelled failure, the complete report/stop/ACK snapshot remains exact and no drain is counted. Public report replay remains valid. Removing the trigger permits one normal Scheduler::tick drain and 17 compensation tasks; the earlier report remains unchanged.

The snapshot includes every tenant payload and physical retained usage, so orphan payloads cannot hide behind a missing reference. No outgoing report obligation is invented: report outbox delta is zero; the later actual drain owns its outbox record.

## Executed gates

- Between-persist test: 1/1 default 66.96 s including 56.09 s compilation; runtime 10.42 s.
- Drain-outbox test: 1/1 default 9.24 s; runtime 8.69 s.
- Flow uses actual sagat_app in both invocations; these are not owner/runtime privilege comparisons.
- Strict integration Clippy: passed 16.01 s. Formatting: passed 4.68 s. Architecture: passed 0.41 s, 24 crates.
- 1122 native source hashes match before/after. Exact commands, outputs, exits, source patch and review reconciliation are preserved.

Fresh bounded C1 found only overlong fixture names; both were shortened before execution. No other required finding remained. All executed tests passed; no behavioral RED is claimed for these fault tests.

## Limits

This covers the named two report/drain transaction failures and stable ACK replay in current worker/engine code. It does not prove old binary compatibility, OS process recovery, unknown COMMIT, all audit/telemetry failure points, Applied ordering or external provider undo. Master Q/Q10c and pending canonical 014 tasks remain open.
