# Correctness review of design 041, the second approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fefcfafbb40d153ebd3e462c7ba2d94a7c0d9286
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, given the committed design, its companions, the ADRs, the constitution, the code and its own `correctness-approval.md`; no shell. Recorded condensed.

Previous findings: Major 1 and Nits 2 to 4 fixed. End rule, own id in the horizon, lock order and counting checked and sound.

## Blocker

None.

## Major

1. Compensation steps can never store a line: step 2 requires the run "neither finished nor stopping", but a stopping run keeps leasing compensation steps (`crates/domain/src/execution/state.rs:4371-4378`; `stop_reason` is written before the run ends, `crates/infra-postgres/src/execution.rs:5315-5317`), and the test asserts the refusal. Check only `finished_at IS NULL`; ADR-0084 decision 5 is the owner's.
2. The `lock_secrets` variant of SC-041-001 asserts what PostgreSQL cannot show: `pg_blocking_pids` of the completion returns the exclusive waiter only. Assert `{L}` for the completion, ingest backends for L, and C after L; give C's output a digest not yet stored.
3. The wrapper's hooks have no hold point for T041-10's "batch held before step 4" and T041-12's held ingest; the after-flush hold owns the usage row. Add a hold after the lease read.
4. Reads through the horizon in a shared test server will be flaky (held ingests of other tests hold `heldBack`); poll read assertions until `heldBack` is false, or isolate every hold test.

## Nit

1. ADR-0084 "Enforced by" is stale (K held ingests, the equal-id case).
2. research.md's "Owner answers of 2026-11-15" still says the marker carries the runner's iteration.
3. The shared-secrets-lock test does not cover `step_logs.rs`.
4. Whether each tail poll runs the permission and grant check and the throttled audit is unclear.
5. The completion bound leaves out the purge's hold of the usage row.
6. A filtered list read does not move its cursor past scanned lines.
7. A deleted script stream shows as `ended`.
8. `step-logs-redaction` is never seen failing on its own assertion; add a harness self-test with a planted canary.

Counts: Blocker 0, Major 4, Nit 8
