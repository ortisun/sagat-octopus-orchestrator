# Compensation safety — validation recipe

Status: delivery-1 local validation is complete. The commands below are the reproduction recipe;
the [validation report](validation-report.md) records the actual broad runs, diagnosed failures and
successful correction retests. US04 and production release/adoption are not closed by these gates.

## Isolated execution

Read root/nested AGENTS first. Inspect `git worktree list`, feature branches and free disk (`df -h`).
Use a dedicated feature checkout and preserve uncommitted work in every other worktree. Set
CARGO_TARGET_DIR to a private temporary directory outside the repository;
CARGO_BUILD_JOBS=2 and RUST_TEST_THREADS=2. Coordinate at most two heavy builds on the machine.
Unset an inherited `RUST_LOG` for these test commands (`env -u RUST_LOG cargo ...`):
the engine fixture otherwise installs a global logger that conflicts with the test tracing
collector. Keep the collector assertions enabled. Use repository Testcontainers fixtures and
pinned compose services only. This delivery is a fresh
isolated install: use a new database and matching server/SDK artifacts; never point an older engine
at delivery-1 data or mix old/new engine versions. Remove only the temporary
files/target created by this execution after collecting evidence; never prune Docker or cargo clean.

## Focused commands after red-first changes

```sh
cargo test --locked -p sagat-domain --test sagas --test scoped_compensation
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests --test integration_sagas_compensations
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests --test integration_workflow_primitives
cargo test --locked -p sagat-tests --test fault_sagas_compensations --test fault_workflow_primitives
cargo test --locked -p sagat-tests --test contract_rest_sagas_compensations --test contract_grpc_sagas_compensations
SAGAT_SDK_SCENARIO=compensation-safety cargo test --locked -p sagat-tests --test sdk_conformance -- --ignored --nocapture
```

For the SDK command, also set `SAGAT_RUST_SDK_TARGET_DIR` to a private absolute directory
outside this repository, distinct from `CARGO_TARGET_DIR`; the Rust SDK prepare script requires it.
The last command requires the compensation-safety scenario and every official SDK toolchain; it
must report all six languages, not silently treat a missing runtime as a pass. Run the SDK checks in
sdks/AGENTS.md and Studio checks for changed surfaces. No test in these commands contacts a bank or
venue.

Before merge, additionally run the Constitution/CI gates: cargo fmt --check; clippy workspace
all-targets locked -D warnings; workspace tests with the resource profile; check-architecture.py;
check-agent-config.py; canonical contract lint/compile, compose smoke and advisory audit using the
repository CI commands. Record each exit/result and any unavailable prerequisite. Do not repeat a
flaky/failing test until green without explaining its defect. Do not bless golden files without
reading their diff and rerunning without the bless flag.

## Demonstrations to record

1. Known compensation failure under default: review survives cancel/deadline and fresh process
   restart; another tenant cannot inspect or decide it. Human resolution shows actor/justification.
2. Explicit fail: one compensation fails, independent one completes, dependent one is suppressed;
   only then terminal COMPENSATION_FAILED with no active/runnable/review-waiting compensation.
3. Unknown forward or compensation outcome, with false/absent highRisk: no blind repeat or defensive
   compensation. Pending maps to OUTCOME_UNKNOWN; NotApplied reviews as RECONCILIATION_NOT_FINAL.
4. Lose a response, duplicate a report, hold database locks to race cancel and review, restart and
   lose a broker hint. Exactly one durable outcome and consistent audit/outbox/projection survive.
5. Refuse output storage after the provider effect and exercise forward resource exhaustion. Recovery is
   retained without leaking worker capacity or treating missing output as proof of no effect.
6. Verify a fresh-install rollback boundary: before any external effect, remove only the unexercised
   deployment; after an effect, stop admission and preserve the database/journal for fix-forward or
   reviewed reconciliation. Do not replay or re-trigger original requests as rollback.

Persist fixture identities, commands, named results and value-free event/audit excerpts under evidence/.
Synthetic provider behavior is fault injection only. Delivery 1 can pass these demonstrations while
US04/SC-037-004 remains open; do not report a qualified provider, feature 037 or the overall reset as
finished. `CompleteStep` is a final-domain assertion by the handler, not provider proof; human success
resolution retains actor/reason provenance and is not provider confirmation.

## Evidence custody during verification

The run journal has no retention/partitioning control (G28), and the audit trail has no automatic
retention or export (G182). Delivered-outbox cleanup is not evidence retention. Keep the isolated
database until review completes, monitor its size, take and restore-test an approved backup/PITR,
and use authorized API paging into an access-controlled external archive when evidence must outlive
the database. Record who owns that archive and its policy; do not claim the platform supplies an
automatic export or regulatory retention guarantee. Explicit `fail` has no post-terminal work queue,
so the operator owns follow-up from retained steps, journal and audit.


Delivery-1 boundary: the owner retained feature-009 ordinary `SECRET_IN_OUTPUT` as final failure.
The storage-refusal demonstration above does not prove review preservation for secret-bearing
output. That existing failure/onError path may have no active recovery queue; the operator owns
follow-up using the safe retained evidence. Never retain the prohibited secret to support review.
