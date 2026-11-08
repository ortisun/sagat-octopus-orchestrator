# ADR-0080: `CANCEL_SOURCE` is switched on per process, engine first, until the instance-capability registry exists

**Status:** Proposed (2026-11-06)
**Decided by:** agent (task T018-057), after the correctness, security and architecture reviews of 2026-11-06; the owner has not accepted it. Whether the feature may be on by default before the registry exists is the owner's (G1989).
**Refines / supersedes:** refines ADR-0050 decision 9 (fleet capability activation) and records one aligned deviation from its decision 7 (lock-order table) for the request path of `CANCEL_SOURCE`; supersedes none.
**Enforced by:** `apps/execution-engine/tests/config.rs` (`the_pass_that_completes_resets_is_absent_unless_the_operator_turned_it_on`: the pure function `optional_passes` names the reset pass only for the flag; the default is off; the strict parsing), `apps/control-plane/tests/config.rs` and `cancel_source::the_service_the_control_plane_builds_refuses_the_cancel_of_a_source_unless_the_operator_set_it` (the request is refused unless the flag is set), `no_shipped_manifest_turns_the_reset_switch_on` (no file under `infra/` sets the flag), `cancel_source::the_namespace_holds_at_most_the_pending_resets_its_limit_allows` and `cancel_source_rest::two_requests_for_the_last_place_are_judged_one_after_the_other` (the quota lock that the order below depends on: one 202, one 429, the second source not cancelled), and `fault_schedules_bulk_operations cancel_source` (the crash matrix of the request). The order of the rollout is review only: an operator procedure (`docs/runbooks/run-reset.md`, section 4.2), which only the registry can turn into a refusal.

## Context

`CANCEL_SOURCE` (FR-018-038, task T018-057) lets a reset name a run that has not ended. The request cancels the source in the ordinary way and records an entry in the register of resets (`PENDING_SOURCE`); a pass of the execution engine, under the shard fence, creates the new run once the source has ended. The two halves run in two kinds of process: the control plane accepts the request, the engine completes it.

ADR-0050 decision 9 says a feature that needs every control-plane or engine process to honour it MUST be activated through the instance-capability registry (`specs/018-schedules-bulk-operations/data-model.md`), which refuses a fleet that does not agree, and that "until then a feature refuses its own use". The registry is task T018-080 and is not built. The feature already exists in the code behind a setting of each process (`SAGAT_RESET_CANCEL_SOURCE_ENABLED`, read by both, as `SAGAT_SCHEDULES_ENABLED` and `SAGAT_BATCHES_ENABLED` are). The consequence of a mismatch is not symmetric:

- on in the control plane and off in every engine: the request still cancels the customer's run and answers 202, and nothing makes the replacement until an engine's flag is turned on (G1989); the cancellation cannot be taken back;
- on in an engine and off in the control plane: the entries that exist are completed, new requests are refused with `RESET_CANCEL_SOURCE_NOT_AVAILABLE`, and nothing is lost.

Two more facts bear on the order of the request. The quota of pending resets (16 by default, 64 hard, per namespace) counts the entries that wait, so the count and the insert of an entry must be one critical section under the advisory lock `quota:pending_resets:{tenant}:{namespace}`. And two equal requests (the same key) must meet before either of them cancels the source.

## Decision

1. **Per-process flags stand in for the registry, and only for now.** `SAGAT_RESET_CANCEL_SOURCE_ENABLED` MUST be off when unset, MUST be parsed strictly (`true`, `false`, `yes`, `no`, `1`, `0`; unset or empty is off; anything else is refused naming the variable), and MUST NOT be turned on by any manifest, chart or compose file this repository ships.
2. **The engine first.** An operator MUST turn the flag on in every engine before turning it on in any control plane, and MUST turn it off in the control plane first. With the order kept, every request that is accepted has a pass to complete it. The runbook says it and gives the signal that something is wrong: `scheduler_lag_seconds{pass="resets"}` and the existing alert `SagatSchedulerLate`.
3. **The pass is a decision of the configuration, not of the code of the engine.** Which of the optional passes run MUST be a pure function of the engine configuration (`optional_passes`), so that a test states that the reset pass is absent unless the flag is set.
4. **No default-on before the registry.** `CANCEL_SOURCE` MUST NOT be made available by default, in any release, until the registry refuses a control plane whose fleet has an engine without the capability (T018-080). The registry then replaces the two flags; this ADR is superseded by the change that does it.
5. **The order of locks of the request.** The request takes: the row of the source run (slot 3R), then the advisory lock of the pending-resets quota (8b), then inserts the entry (8a), then the audit record, then writes the receipt of the key, then the cancellation writes (the source row is already held). The receipt is **read** before the row is taken (slot 2, a lookup that locks nothing) and **written** after it. This differs from the table of ADR-0050 decision 7 in two places, and both are intended: the quota lock precedes the insert of the entry (8b before 8a) because the quota counts the entry; the receipt row is written after the source row because two equal requests then meet at the row of the source and the second finds the receipt of the first, whatever the order they arrived in. There is no cycle: the quota lock is taken by the request path only and always after the source row; the completion pass takes the fence of the shard and the row of the entry, never the quota lock and never a source row; two requests for different sources meet only at the quota lock, which the holder releases without waiting for any other lock.

## Considered options

1. **Per-process flags, default off, engine first, no default-on (chosen)** — costs one setting per process and an operator procedure, which is what the sibling features (schedules, batch jobs) already do; the unsafe combination is the control plane on and every engine off, which the order and the lag alert cover, and which the registry will refuse.
2. **Wait for the registry before merging the feature** — refuses a mismatch by construction, but T018-080 is not scheduled, and the feature would sit unmerged while its tests, contract and Studio dialog are done; the setting being off, the merged code changes nothing for a deployment that does not ask for it.
3. **Make the control plane write the entry and let the engine create the run only when it has the capability, with no flag in the control plane** — the control plane would accept requests whatever the engine does, which is the unsafe combination made the only one.
4. **Keep the original lock order of the table (entry before the quota lock)** — the count would be read before the insert under no lock, and two requests for the last place would both succeed; the lock held across the count and the insert is the point of the quota.

## Consequences

Good: the feature can be merged and exercised with the default unchanged; the unsafe state is named, signalled and documented; the order of locks is written once with the argument that it has no cycle.

Bad: the combination of the control plane on and every engine off is possible and cancels a run that nothing replaces until an engine's flag is turned on; the rollout order is a procedure, not a refusal. Rolling back is turning the control plane's flag off first, then the engine's: the entries that exist stay in the register and are completed when an engine with the flag runs (or stay `PENDING_SOURCE`, counted against the limit, until then).

## Evidence

`specs/018-schedules-bulk-operations/research.md` D2001 (the quota and the order), D2002 (the setting), D2009 (the pure function of the passes), G1989 (the mismatch), G1993 (no barrier test of the order across ADR-0050 yet; T018-012 stays open); `docs/runbooks/run-reset.md` section 4.2.

## Amendments

- **2026-11-08 (T018-080).** The instance-capability registry exists: migration `20270111000000_instance_registry`, the
  registration of the control plane and of the engine before they serve, the heartbeat, the epoch check and the activation
  functions (`docs/runbooks/instance-registry.md`; decisions D2013 to D2022 of `specs/018-schedules-bulk-operations/research.md`).
  It exists for `schedules-v1` (feature 018, the handover of the cron triggers, T018-061), `access-narrowing-v1` (029) and
  `payload-encoding-v1` (031). **It does not yet carry `CANCEL_SOURCE`**: no capability is defined for it and the engine does
  not list one, so the two flags of decision 1, the order of decision 2 and the prohibition of decision 4 stand as written until a
  change moves this feature (and the batch operations and the reset reapply) to the registry; that change supersedes this ADR.
  G1965, G1988 and G1989 stay open for that reason (G1999).
