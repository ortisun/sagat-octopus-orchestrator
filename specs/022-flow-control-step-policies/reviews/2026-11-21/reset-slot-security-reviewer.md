# Security review of task T022-086 (reset of a rate-limited start), 2026-11-21

Reviewer: `security-reviewer` (read-only), given the diff (`git diff HEAD` over `39c5abc9` plus the uncommitted task) and the
specification. Saved verbatim.

---

I found no Blocker or Major issue. I report two minors and two nits, and I would not hold T022-086 on any of them. Nothing in it leaks tenant data, and a reset cannot take a slot it should not have.

I read the diff and the worktree files it touches: `readiness_gate.rs`, `execution.rs` (verify and persist), `event_codec.rs`, `resets.rs`, `state.rs`, `reset.rs`, `reset_completion.rs`, `runs.rs` and the `run_resets` migration history. I ran nothing, so every claim below comes from reading the code.

## Verified in the code

**minor: the down migration's guard misses the other rows the previous build cannot read**
- **Where:** `crates/infra-postgres/migrations/20270209000000_reset_rate_slot.down.sql:5-12`, together with `crates/infra-postgres/src/event_codec.rs:1085-1089`.
- **Scenario:** a reset of a rate-limited run commits. Its journal holds copied `rate_limit.granted` (and maybe `queued` or `returned`) rows whose detail carries `restored: true`. No pending entry ever failed with `RATE_LIMIT_SLOT_UNAVAILABLE`, so the down migration's guard passes. The operator then rolls back the schema and the binary to the 20270207000100 build. That build's decoder accepted the `restored` marker only on `concurrency.*` types (`Some(Value::Bool(true)) if event_type.starts_with("concurrency.")`, else `return None`). So every load of that reset run fails with "event N cannot be decoded": claims, completions, `verify_run`, the reset-point listing. The run is stuck.
- **Impact:** no attacker gains anything, but the run is stuck. Rule 6 of `.claude/skills/migration/SKILL.md` says the down must refuse "when rows exist that older code cannot read". The guard covers one such kind of row, not both. The impact is limited because no released build carries rate-limit events (022 is not in a release).
- **Smallest fix:** add a second guard to the same `DO` block:
  `IF EXISTS (SELECT 1 FROM run_events WHERE event_type LIKE 'rate_limit.%' AND detail ? 'restored') THEN RAISE EXCEPTION ... USING ERRCODE = 'object_not_in_prerequisite_state'`
  This scans `run_events`, which is acceptable for a down run by an operator; say so in the comment. Alternatively, write in `docs/runbooks/run-reset.md` that a code rollback across T022-086 is refused while such runs exist.
- **How to verify:** in `migrations::the_reset_slot_reason_rolls_back_only_once_no_entry_carries_it`, insert a `run_events` row `rate_limit.granted` with `"restored": true` and assert that `undo(pool, BEFORE_RESET_SLOT)` is refused. It should fail red against today's down.

**nit: the codec now accepts `restored: true` on gate event types a reset can never copy**
- **Where:** `crates/infra-postgres/src/event_codec.rs:1085-1089`.
- **Scenario:** `rate_limit.wait_timed_out`, `rate_limit.dequeued`, `concurrency.wait_timed_out` and `concurrency.dequeued` are `Refuse { until: None }` (`crates/domain/src/execution/reset.rs:127-128, 211-212`). A prefix that holds one is never eligible, so no correct writer emits a restored copy of them.
- **Impact:** a row of those types carrying the marker would decode silently instead of being flagged as corrupt. Nobody can exploit this, since only the server writes `run_events`. But it reopens part of what security nit 7 of 2026-11-20 had closed.
- **Smallest fix:** accept the marker only when `rule_by_name(event_type) == Some(ResetRule::Rematerialize)`, or list the five types explicitly. Keep the codec as narrow as the registry.
- **How to verify:** a unit test in `event_codec::tests` that `decode("rate_limit.dequeued", None, &{...,"restored":true})` is `None`.

## Suspicions and design points (not defects in the diff)

**minor: a `CANCEL_SOURCE` reset of a rate-limited source can be predictably destructive, and another member can force it**
- **Where:** the accept path in `crates/application/src/runs.rs` (pending entry, 202); the completion is in `crates/application/src/reset_completion.rs:241-243`.
- **Scenario:** with `limit: 1, per: hour`, an open source almost always got its slot less than an hour ago. A `CANCEL_SOURCE` reset is accepted with 202 and cancels the source. Cancelling frees nothing at run scope, because the source's slot keeps counting for its window. If the source ends before its slot ages out, the completion fails `RATE_LIMIT_SLOT_UNAVAILABLE` and the customer's run is gone with no replacement.
- **Insider angle:** a member who holds only `run:start` in the namespace can start runs with the same key value. That fills the window on purpose and makes a colleague's pending reset fail. It stays inside one tenant and needs real permissions. The runbook documents the outcome, and the permit has the same shape (D022-053), but for a rate limit it is close to certain rather than likely.
- **Fix:** this is for the owner to decide. One option is a best-effort read-only rate count at accept time that refuses 409 `RATE_LIMIT_SLOT_UNAVAILABLE` when the window is full now. Another is a warning in the reset preview.
- **How to verify:** an integration case with `limit: 1, per: hour`: open source, `CANCEL_SOURCE`, then end the source. Today it ends `FAILED` with the source cancelled.

**nit: an occupancy hint for callers who may not see personal data (G022-014)**
- **Where:** `crates/infra-postgres/src/execution/readiness_gate.rs:1659-1683` (`reset_refused`).
- **Scenario:** a caller with `run:operate` and `run:start` learns whether other runs with the same key value got slots in the current window. This is one bit, of the same kind as `rateLimit.position` and the permit's refusal.
- **Fix:** add this surface to G022-014's list in `research.md`. No code change is needed.

## Classes checked with no finding

- **Tenant boundary:**
  - The slot count (`key_count`, `readiness_gate.rs:770-781`) filters by tenant and namespace.
  - The refusal is only reachable after the source has been found within the tenant; I saw no new path that answers before the 404.
  - `details` carries only `reason` (`crates/api-http/src/error.rs:1237-1241`). The messages and the remediation text are fixed strings.
- **Secrets and personal data:**
  - The metrics labels come from fixed, closed lists: `refused_reset`, scope, outcome, and the eight reasons of `FAILURE_REASONS`.
  - The copied detail is the source's own detail, which carries the 12-hex label and never the key value. Nothing new is logged.
- **Bypassing the slot:**
  - Copied history creates no grant row. Grant rows are written only in `decide_filed` → `grant`, and `persist` skips `persist_gate_returns` for a restored prefix (`execution.rs:5515-5518`).
  - The fresh decision applies the reservation rule (`taken + waiters >= limit`, `readiness_gate.rs:359`).
  - A refusal of the permit alone consumes no slot: the error returns at lines 364-367, before any grant at line 378.
  - The reducer forbids a queue after `run.reset` (`state.rs:5011-5019`).
  - `verify_run` checks invariant 11 against the definition (`execution.rs:708, 726`), so a reset that bypassed the gate would fail `assert_consistent`.
  - A paused point is not eligible (`reset.rs:230`).
  - Leaving copied grants out of invariant 10 (`execution.rs:893`) cannot hide an extra grant, because each one is still counted once in the journal that made it.
- **Privileges of `sagat_app`:** no new grant is needed, since the role already updates `run_resets`. No row lock was added. The down restores exactly the list of `20270115000000_reset_expiry.up.sql`. Adding the CHECK as `NOT VALID` and then validating it under `SHARE UPDATE EXCLUSIVE` is sound.
- **Injection and parsing:** every value is a bound parameter. The `::boolean` cast reads a field only the server writes, and the codec only lets it hold `true`.
- **Races and replay:** the decision runs under the locks of the filed keys, the same as the permit. Pending entries re-check authority at completion, which this diff did not change.

## Commands to run

- `cargo test -p sagat-tests --test integration_flow_control_step_policies -- run_creators:: migrations::the_reset_slot_reason`
- The same with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
- `cargo test -p sagat-infra-postgres --lib event_codec`
