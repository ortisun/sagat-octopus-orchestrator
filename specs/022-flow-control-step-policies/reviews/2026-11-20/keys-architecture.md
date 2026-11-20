# Architecture review: feature 022 key reads (T022-026..029)

Reviewer: `architecture-reviewer` (read-only), 2026-11-20. Diff `b3351f2a` against `466c151e`, specs excluded. The
reviewer compiled and ran nothing. (Report as returned, lightly condensed.)

## Verdict

Layering is right (the port `FlowControlKeyReader` in `sagat-application` depends only on domain types; its one
implementation is in `infra-postgres`; the HTTP layer depends on the application view types). The DTOs add real wire
meaning. `Run` stays compatible (`concurrency` skipped when `None`). No accepted ADR is contradicted (the reads take no
lock; grants cover the statements; the cross-tenant 404 is audited through `require_namespace`). Findings: 1 Blocker
(clears once one existing gate is run and passes), 4 Major, 8 minor, 4 nits.

## Blocker

### B1. The static SQL privilege audit cannot resolve the new lateral macro, so `RUN_SELECT` and the run listing become "not audited"

`execution.rs:2360` (`RUN_SELECT`) and `:1837` (`list`) call `crate::flow_control::run_permit_lateral_sql!()`, defined in
`flow_control.rs:382`. The extractor of `integration_platform_foundation::runtime_privileges` inherits only fixed macros
declared in `execution.rs` (`runtime_privileges.rs:218-236`) and resolves qualified macro paths only under
`crate::execution::` (`runtime_privileges/rust_source.rs:258-259, 573, 617`), so the call is `Need::Unresolved` → "not
audited" (`runtime_privileges.rs:463`). A PR runs only the integration targets it changes, so this would surface at the
release run. Fix: declare the macro in `execution.rs` and call it unqualified, or teach `rust_source.rs` to resolve
`crate::flow_control::` macros with fixtures. Also check `query_scalar(crate::flow_control::QUEUED_RUNS_SQL)` at
`readiness_gate.rs` and `quotas.rs:1347`. Evidence required: `SAGAT_TEST_AS_RUNTIME_ROLE=1 RUST_TEST_THREADS=1 cargo test
-p sagat-tests --test integration_platform_foundation --locked runtime_privileges::` passes; if it already does, B1 is
"not reproduced".

## Major

### M1. The cursor carries the full key digest, whose first six bytes are the label

`flow_control_keys.rs:78-115`; D022-055.2. The label is `KeyLabel::of_digest` (the first six bytes), so the cursor carries
it in clear; once T022-104 makes labels `null` for some callers, they read it from `nextCursor`. Nothing in T022-104 or
G1185 records the coupling. Fix: amend D022-055.2 and T022-104 now; better, seal the cursor (AEAD or HMAC under a server
key, or an opaque row token).

### M2. `key_consistency` treats "undefined column" as "schema predates the gates"

`tests/support/flow_control.rs:293-298`: any 42703 (a renamed column the five statements name) returns `Ok(empty)` and
every `assert_consistent` in ~121 tests asserts nothing about keys; only the seeded `expect_err` tests would notice. Fix:
decide "predates" once with `to_regclass('concurrency_semaphores') IS NULL`; map every other error to `Err`.

### M3. The key listing costs O(live keys in the namespace) per page, unbounded

`flow_control.rs:463-482`: two correlated `min()` per semaphore row and an unindexed sort before `LIMIT`; the live-keys
ceiling defaults to 10,000 (up to 100,000), paging the whole listing is quadratic, on the API pool, callable by any human
with `ops:read`; no measurement, no statement timeout. Fix: `SET LOCAL statement_timeout` in the read transaction mapped to
the typed `503`; a gap tied to T022-105 (keep `oldest_wait_since` on the semaphore row through the counter trigger and
index the keyset order); one measured run at the default ceiling.

### M4. The live route and `Run.concurrency` are missing from the canonical contract, and the fragment omits two statuses

The fragment (`openapi-flow-control.yaml:156-167`) lists `200/401/403/404/429/503/default`; the route returns `422` (bad
group) and `400` (foreign cursor). Neither the path nor `Run.concurrency` is in the canonical OpenAPI (T022-060, T022-087
open); the route is mounted unconditionally, the battery iterates the canonical file, and the conformance checker would
reject 400/422 when the fragment is merged as written. Fix: add `'400'` and `'422'` to the fragment now; make T022-060 (at
least this path and the field) a 0.3.0 release gate, or gate the mount on the `concurrencyKeys` switch until then.

## minor

- **m1.** Every `OperationsService` method now requires `FlowControlKeyReader`; move `concurrency_keys` into its own impl
  block with its own bound.
- **m2.** `run_permit` folds a decode error into "no permit"; use `selected()`.
- **m3.** "One count of queued runs, by construction" (D022-055.3) has no check that fails: a `check-architecture.py` rule
  allowing the `concurrency_waiters ∪ rate_limit_waiters` count only in `flow_control.rs`, with a fixture and a row in
  `architecture-principles.md`.
- **m4.** No fitness row records "raw key values never in a response": a rule that `dto_flow_control.rs` and
  `routes/concurrency.rs` name no raw-key type and no `serde_json::Value`, and the row "Key values appear only as labels".
- **m5.** The cross-tenant test checks the 404 but not the audit record (`CrossTenantAccessDenied`).
- **m6.** `Options::flow_control` opens the step-gate test door on the API store; the run-level reads do not need it.
- **m7.** `key_consistency` skips steps inside instance paths without saying so (`cardinality(instance_path_nodes) = 0`);
  note the coupling with G1160 or fail on a gated nested step.
- **m8.** ADR-0056's enforcement record is stale ("None exists yet"; "Evidence: None yet"); name the checks that now exist
  and add a row to `architecture-principles.md`; say the 1/16/256-contender benchmark is still missing.

## nit

- **n1.** Mark the `keyLabel` placeholder at the field ("`None` arrives with T022-104").
- **n2.** Document "nobody-waiting keys last", the opaque cursor and the 400 in the fragment.
- **n3.** `age_start_deadline` is copied in the fault and integration suites; share it in `tests/support/flow_control.rs`.
- **n4.** `assert_consistent(run)` now checks the whole first tenant; say so.

## Fitness map

| Rule | Check today | Gap |
|---|---|---|
| Domain has no I/O; layers inward | `check-architecture.py` | none |
| Holder and waiter rows written only by the gate | `gate_row_write_errors` | none |
| Store statements within `sagat_app`'s privileges | `runtime_privileges` + runtime role | B1 |
| Counters = rows = journals across runs | `key_consistency` + `consistency.rs` | M2; not in `architecture-principles.md` (m8) |
| Lock order of a run's permit | `concurrency_locks.rs`; admission, readiness, returns elsewhere | grant relies on `SKIP LOCKED` (argued) |
| First commit wins; no double grant across a takeover | `fault/…/concurrency.rs` | none |
| Key values only as labels | `rest.rs` canary | m4; M1 |
| One queued-runs statement | convention | m3 |
| Contract additive and documented | fragment only | M4 |
| Cross-tenant is a 404 and audited | `rest.rs` (404 only) | m5; battery after T022-060 |

Scope: T022-026..029 match the slice table; `keyLabel: null` (T022-104), the canonical merge (T022-060) and slot-8b
counters (T022-105) are left out and recorded. The reviewer could not run the duplicate-work check (`git branch -a | grep
022`, `git worktree list`).
