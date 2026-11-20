# Security review: T022-064 run creators (uncommitted diff in `/Users/marcelo/Documents/Projects/sagat-wt-022`)

Reviewer: `security-reviewer` subagent, 2026-11-20, given the diff and the spec. Text as returned; resolutions are in
`research.md` D022-054.

**No Blocker found.** There are two Major findings: one verified (scheduler starvation), and one that depends on whether an earlier build was ever deployed (rolling upgrade). The rest are minor and nits. I read the code but ran nothing, so the test results quoted in `tasks.md` are the author's claims, not mine.

## Verified in the code

### Major 1: deferred releases block the release pass's queue for the tenant
- **Where:** `crates/scheduler/src/passes.rs:4499-4506` (the deferral writes nothing), `crates/infra-postgres/src/maintenance.rs:638-648` (`due_start_releases`, ordered by `start_delayed_until`, `LIMIT batch`), `passes.rs:6170-6178` (fairness keyed by tenant, then by run).
- **Scenario:** A member with `run:start` in namespace A fills A's `queued_runs` ceiling with ordinary queued runs, or fills one key's waiter queue. They then start `batch` (200 by default) delayed runs of a gated definition with a 1 ms delay. Each release is refused by a ceiling, rolled back and left due, with its original `start_delayed_until` unchanged. Every pass therefore picks the same oldest candidates first and defers them all.
- **Impact:**
  - Every other due delayed start of the tenant is never released while the ceiling holds, gated or not, in namespace B as well. Per-tenant fairness (`fair: true` by default) keeps this inside one tenant, so it crosses namespaces but not tenants. With `fair: false` (a diagnostic mode) it would cross tenants.
  - Each pass also writes up to 200 `warn!` lines per replica.
- **Smallest fix:** on a deferral, move the candidate back: either a `start_release_retry_at` column that the due query orders by and filters on, or the key's `next_eligible_at`. Also replace the per-candidate warning with a counter (closed label `reason`) plus a debug log.
- **Test:** with fairness on, take `batch` deferred delayed runs on a full key in namespace A plus one ungated delayed run in namespace B of the same tenant, all due. One `scheduler.tick()` must release B's run. Today it does not.

### minor 3: the gate projections keep the source's permit after `run.reset`
- **Where:** `crates/infra-postgres/src/execution.rs:5683-5685` (`run_gates_changed` counts only run-scope `Gate` events), `execution.rs:5716-5728`, `crates/infra-postgres/src/execution/resets.rs:224-232` (the restore commit persists the source's state), and the `RunReset` arm in `crates/domain/src/execution/state.rs` (clears `run_gates` and retires step permits in memory only).
- **Scenario:** the restore commit writes `runs.flow_control_gates = HELD` (the source's key label) and `steps.flow_control_gates = HELD` for the step at the point; `run.reset` changes both in memory without an event that rewrites them; today they are rewritten only by accident (the fresh `concurrency.acquired`, and `step.routed` of the point node). If the reset commit has no run-scope gate event, the finished run keeps a projection saying it holds a permit it never held (the `Resolution::StartGates { decisions: Err(()) }` path, or any future path that stops before routing). A later load rebuilds `Held` from that projection, and `give_back_ended_gates` would then give back a permit with no holder row.
- **Fix:** in `persist`, treat `RunEvent::RunReset` as a run-gates change, and add every step whose gates `retire_restored` changed to `touched`.
- **Test:** restore a prefix holding a run permit, force the key to fail at the reset, then `assert_consistent` and read `runs.flow_control_gates IS NULL`.

### minor 4: the reset path skips `Engine::start`'s fail-closed check
- **Where:** `crates/application/src/execution.rs:2125-2134` (start rate limit, `forEach`/`while`/`repeatUntil` refused) against `crates/application/src/runs.rs:2004-2024` (the reset goes straight to `record_reset`).
- **Why it is only minor:** unreachable today (the reset reuses the source's pinned version; no build ever admitted a gated start with collections or loops; the adapter refuses a start rate limit).
- **Risk if it ever becomes reachable:** the reset takes a permit that the finish paths of collections and loops never give back.
- **Fix:** move the `unsupported` check into `decide_start_gates`, or into a shared helper both paths call.
- **Test:** `StartGates { queue: false }` on such a definition returns `BINARY_CAPABILITY_MISSING`.

### minor 5: a refused reset is not counted
`readiness_gate.rs:984-990` returns `PermitUnavailable` without `record_concurrency(Run, RefusedFull)`, which every other start refusal records through `start_refused`. Fix: record the refusal with the same closed labels.

### minor 6: no API contract test for the new 409
`crates/api-http/src/error.rs:1215-1227` is new; the header of `crates/api-http/tests/flow_control_errors.rs:3-4` says this row lands with the reset case (T022-064), but the diff adds no case there. Test: 409, `code: CONFLICT`, `details == {"reason":"CONCURRENCY_PERMIT_UNAVAILABLE"}`, no key, group or run id in the message.

### nit 7: the decoder accepts the `restored` marker too widely
`event_codec.rs` (`gate_of`): `restored: true` is accepted on all ten gate event types, including the five `rate_limit.*` types, which are `EXCLUDE` and can never be restored. Fix: accept the marker only for the five `concurrency.*` types.

### nit 8: the deferral applies to every scheduler action
`passes.rs:4501`: the `Outcome::Deferred` arm sits in the generic `apply_if`. Fix: restrict it to `Condition::StartDue` (or `kind == "start_release"`).

## Suspicion (needs one check)

### Major 2 (only if an earlier build shipped): rolling upgrade can start a gated run without a permit
The diff leaves the capability token `flow-control-concurrency-v1` unchanged. A build that holds T022-022 lists that token, and its planner has no `StartGates`: a released gated delayed run would have no run gates and `start_pending` would release the entry. Requires a new API replica creating a gated delayed run and a previous-build scheduler replica owning its shard at release time. Not verified: whether any build containing T022-022 was tagged or deployed. Severity: Major if such a build is tagged or deployed; a nit if it never left development branches. Fix: a new token, required before a gated delayed start is accepted.

## Classes checked with no finding

- Admission bypass, reset: nothing between `run.reset` and `StartGates { queue: false }` emits an event; the reducer refuses any later run-level decision, and `resolve_start_gates` refuses a wait.
- Admission bypass, release: the only caller of `release_start` is the scheduler pass, through `Engine::run`; `Condition::StartDue` is checked under the run's lock.
- Never queue: the adapter refuses at `holders + waiters >= limit` before any holder or waiter row is written.
- Lock order and `queued_runs` exactness: the release takes the run's row, then the namespace admission lock, then the key; no transaction takes the admission lock and then an existing run's row.
- Tenant boundary: every new query is scoped to the run transaction's tenant and namespace; the 409 comes after `run:operate` + `run:start` and the source's visibility check.
- Runtime role privilege: no new tables, functions or grants.
- Injection and parsing: no dynamic SQL; key bytes are bound parameters with the 256-byte cap.
- Disclosure: errors, logs and spans carry the reason, run id and group only.
- Audit: the pending reset's failure goes into the register with a closed reason, and the database `CHECK` already lists it.
