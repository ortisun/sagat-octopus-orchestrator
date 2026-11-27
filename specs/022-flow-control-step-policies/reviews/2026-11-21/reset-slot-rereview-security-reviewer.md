# Security re-review of task T022-086 (reset of a rate-limited start), 2026-11-21

Reviewer: `security-reviewer` (read-only), given the diff of commits `8dcd7f37` and `cf4f858e` (review files left out), the
specification, its first review and `reset-slot-responses.md`. Saved verbatim.

---

**Re-review of T022-086 (security): no Blocker or Major.** My first review raised none either, so the first question has nothing to resolve. None of my four minors and nits from that review was fixed in code. The authors' responses mark them open or deferred, and the tree agrees.

## Status of the first review's findings

| First review | Status in the tree (checked in code) |
|---|---|
| minor: the down migration does not guard restored `rate_limit.*` rows | **Not resolved.** `20270209000000_reset_rate_slot.down.sql:5-12` still checks only `failure_reason = 'RATE_LIMIT_SLOT_UNAVAILABLE'`. Neither runbook mentions a rollback across this migration. |
| nit: the codec accepts the `restored` marker on gate types that a reset refuses | **Not resolved.** `event_codec.rs:1085-1089` still reads `Some(Value::Bool(true)) => 1` for all ten gate types. |
| minor: `CANCEL_SOURCE` destroys the run on a full key, and a member can force that | **Deferred** to T022-114 (`tasks.md`). The runbook warning was added (`run-reset.md`, the reasons table). There is no code change. |
| nit: an occupancy hint for callers who may not see personal data (G022-014) | **Not resolved.** The G022-014 row in `research.md` is unchanged. |

## Findings on the current diff

**minor (carried over): a build older than this task cannot read a reset run's journal, and nothing in code stops that**
- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/migrations/20270209000000_reset_rate_slot.down.sql:5-12`. The capability is not bumped (D022-069 item 6).
- **Scenario:** a reset run carries copied `rate_limit.*` rows marked `restored: true`. Two cases:
  - An operator rolls back the schema and the binary. The down migration passes, and the older build fails to decode every load of that run.
  - During a rolling upgrade, any member with `run:operate` and `run:start` resets a rate-limited run. An older replica, which advertises the same `FlowControlRateV1`, claims the run and cannot decode it.
- **Impact:** the run is stuck on those replicas. No data leaks and no privilege is gained. The risk is limited because feature 022 is in no release.
- **Fix:** add a second guard to the `DO` block: `IF EXISTS (SELECT 1 FROM run_events WHERE event_type LIKE 'rate_limit.%' AND detail ? 'restored') THEN RAISE ...`. Alternatively, write the rollback refusal into `docs/runbooks/run-reset.md`.
- **Confidence:** high for the rollback case, medium for the rolling-upgrade case (it depends on how claims are routed).

**minor (carried over, tracked as T022-114): a member can fill the window to make a colleague's pending reset fail**
- **Where:** the accept path in `crates/application/src/runs.rs` (pending entry, 202) and `reset_completion.rs:240-243`.
- **Scenario:** take `limit: 1, per: hour`. A member who holds only `run:start` keeps the key full by starting runs with the same key value. A colleague's `CANCEL_SOURCE` reset cancels the source and then fails with `RATE_LIMIT_SLOT_UNAVAILABLE`, and no run replaces it. This stays inside one tenant and needs real permissions.
- **Fix:** T022-114 as written (an advisory refusal at the request, or a warning in the preview). Until then, the runbook is the only safeguard.
- **Confidence:** high.

**nit (carried over): the codec accepts `restored: true` on gate types that are never copied**
- **Where:** `event_codec.rs:1087`.
- **Scenario:** `rate_limit.wait_timed_out`, `rate_limit.dequeued` and their two `concurrency.*` twins are `REFUSE` (`reset.rs:127-128, 215-216`). A marked row of those types decodes instead of being flagged as corrupt. Only the server writes `run_events`, so nobody can exploit it.
- **Fix:** accept the marker only when `rule_by_name(event_type) == Some(ResetRule::Rematerialize)`. Add a unit test that the marked `rate_limit.dequeued` decodes to `None`.
- **Confidence:** high.

**nit (carried over, now with a second surface): an occupancy hint for callers who may not see personal data**
- **Where:**
  - the synchronous 409 from `readiness_gate.rs:1659-1683`;
  - `failureReason` in the reset register (`run_resets.failure_reason`), which shows the same bit afterwards to anyone who can read the entry.
- **Fix:** list both in G022-014. No code change.
- **Confidence:** high.

**nit (suspicion): a refused reset is the most expensive refusal to produce**
- **Where:** `runs.rs:2012-2032`. `restore_prefix` writes the whole copied history up to the prefix ceiling before `record_reset`'s settle asks for the gate decision. Only then does the transaction roll back.
- **Scenario:** on a key at its limit, a member with `run:operate` and `run:start` repeats synchronous resets with new idempotency keys. Each request costs the server a full prefix write and a rollback, and refusal is near certain while the source's slot counts. The permit (D022-053) has the same shape, and the impact is load only.
- **Fix:** the read-only count planned for T022-114 can also run before `restore_prefix` on the synchronous path, as an early refusal. The in-transaction decision stays authoritative.
- **Confidence:** medium. I did not measure the cost.

## Classes checked with no finding

- **Tenant boundary:**
  - The slot count (`readiness_gate.rs:770-781`) filters by tenant and namespace on both the window and the grants.
  - The invariant-10 query (`execution.rs:888-893`) is tenant-scoped, and the new `restored` clause does not widen it.
  - `details` carries only `reason` (`api-http/src/error.rs`). The message and the remediation text are fixed strings.
  - The refusal is only reached after the source has been found inside the tenant. No new path answers before the 404.
- **Authorization and admission:**
  - Removing the interim `BINARY_CAPABILITY_MISSING` (`readiness_gate.rs:1225-1232`) still fails closed in two places:
    - `flow_control_admission` (`execution.rs:212-254`) refuses a capability that is disabled, not activated, or not served.
    - Collections and loops are still refused.
  - A pending completion reads and locks the requester's membership again before anything is created (`reset_completion.rs:348-361`).
  - `failure_of` maps the new refusal to a final reason, and `BinaryCapabilityMissing` is still retried.
- **Bypassing the slot:**
  - A grant runs only when `refusing` is empty (`readiness_gate.rs:363, 382-386`), so a refused reset writes no grant row.
  - `start_undecided` asks for the gate decision of every reset run (`state.rs:1999-2008`).
  - `persist` writes no gate row for a copied prefix (`execution.rs:5515-5518`), and grant rows are written only by `grant`.
  - The lease pass (`maintenance.rs:366-369`) ties a lease to the latest `rate_limit.granted` of the attempt, so a copied `step.leased` cannot start the window early.
- **Commit cf4f858e:** `start_gated` is private (`execution.rs:2137`) and has one caller, `Engine::start` (`execution.rs:2133`). Allowing it in `check-architecture.py` does not let any other run creator start a run past the gates.
- **Secrets and metrics:** the new labels come from closed sets (`refused_reset`, the eight `FAILURE_REASONS`). The span names the group, never the key. The copied detail carries the 12-hex label, never the key value.
- **Privileges of `sagat_app`:** no new statement runs on the runtime path. The migration changes no grant and adds no SECURITY DEFINER function. The down migration restores exactly the reason list of `20270115000000`. Adding the CHECK `NOT VALID` and validating it in `000100` is sound.
- **Injection and parsing:** every value is a bound parameter. The `::boolean` cast reads a field only the server writes.
- **Races:** the decision runs under the keys' locks taken by `file_keys_with`, and a refused transaction rolls back whole.

## Commands for the evidence

I ran nothing; every claim above comes from reading the code. These commands produce the evidence:
- `cargo test -p sagat-tests --test integration_flow_control_step_policies -- run_creators:: migrations::the_reset_slot_reason`, then again with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
- `cargo test -p sagat-infra-postgres --lib event_codec readiness_gate::tests`

Blockers: 0, Majors: 0
