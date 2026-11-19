# ADR-0093: The flow-control digest key is rotated per key, with live rows

**Status:** Accepted (2026-11-19, OD-2026-11-19-2, "Sim, como recomendado": O1170 option (a) with (b) for incidents)
**Decided by:** agent proposal (feature 022, T022-095, 2026-11-16) after the security, correctness and architecture reviews of that day; accepted by the owner with the operator's force of decision 5 (OD-2026-11-19-2)
**Refines / supersedes:** supersedes the last sentence of ADR-0056 decision 7 ("A deployment MUST refuse to activate a new digest key version while any holder or waiter exists under the old one") and the last sentence of ADR-0088 decision 8 (the same rule for waiters and grants); refines ADR-0089 decision 6 ("the current keyed-digest version" becomes "the versions in force"). Decision 5 makes a bounded exception to the order of ADR-0088 decision 2 and ADR-0056 decision 5 while a forced version drains. The rest of ADR-0056, ADR-0088 and ADR-0089 stands.
**Enforced by:** `integration_flow_control_step_policies::migrations::{a_rotation_with_live_holders_waiters_and_grants_never_splits_a_key, a_rotation_runs_with_live_rows_and_retires_the_old_version_with_its_last_row, a_retiring_row_that_goes_away_while_it_is_filed_is_a_retryable_refusal, the_rotation_migration_retires_what_the_old_command_left_and_its_down_refuses_during_a_rotation, the_force_is_the_operators_audited_and_retires_only_a_retiring_version}`; `integration_flow_control_step_policies::rotation::a_forced_retirement_over_admits_a_key_by_at_most_its_old_rows_holders_until_they_drain` and `a_key_filed_again_after_a_force_is_not_refused_by_the_live_keys_ceiling` (decision 5, with held locks); the outcome `FORCED_VERSION_DRAINING`; the append-only `flow_control_key_forced_retirements`; the rule of `scripts/check-architecture.py` that only the gate's two files write waiter rows (`gate_row_write_errors`); the unique index `flow_control_key_versions_one_retiring`; the trigger `flow_control_key_version_current` (no row under a version that is not active); the rule of `scripts/check-architecture.py` that only `crates/infra-postgres/src/flow_control.rs` inserts a semaphore or window row.

## Context

A concurrency or rate-limit key is stored only as an HMAC digest under a deployment key (D1168, NFR-022-006). ADR-0056
decision 7 accepted on 2026-11-13 refuses a new key version while any holder or waiter uses the old one, so a rotation
means draining every queue of every tenant. The review of 2026-11-13 (security M4) and the spec as revised the same day
(FR-022-056) ask instead for a per-key rotation: "while a rotation is in progress, a deciding transaction computes both
digests and uses the row that exists under the old version, else the new one; the old version is retired when no row
uses it, and no key is ever split between two rows." ADR-0089 decision 6 says the current version is a row read in each
deciding transaction. The two texts disagree; T022-095 built the spec's rule, and the reviews of 2026-11-16 found that the
accepted ADR had not been superseded (architecture B1).

Facts the design rests on:

- A key row (`concurrency_semaphores`, `rate_limit_windows`) is created only under the active version: a trigger refuses
  any other (migration `20270130000000`).
- A key's plaintext is never stored; only a deciding transaction (admission, readiness, grant, timeout) has it. A row
  cannot be re-keyed without it.
- A run-level permit is held for the run's whole life, which may be long (a run sleeping or waiting for a signal).

## Decision

1. Activating a new version MUST NOT wait for the queues: the active version becomes `RETIRING` while a key row uses it,
   `RETIRED` at once when none does. At most one version is `RETIRING` (a unique index); a third version MUST be refused
   (`ROTATION_IN_PROGRESS`) while the retiring one is used, unless the operator forces it out (decision 5).
2. A deciding transaction MUST read the versions in force in its own transaction
   (`flow_control_key_versions_in_force()`: the active one `FOR SHARE`, the retiring one only while a row uses it), digest
   the key under each, and file it with `file_keys`: the row under the retiring version wins, else the key is filed under
   the active one. Keys of one table are resolved, then locked in one order by the digest each resolved to; a retiring row
   that goes away between the two steps is a retryable refusal of the transaction, never a lock out of order.
3. Retirement is lazy: a retiring version is out of force the moment no row uses it (no decider needs its key any more)
   and is recorded `RETIRED` by the next activation. No trigger and no lock is added to the paths that delete key rows.
4. A retired or replaced version MUST NOT become active again. The command records the database role that ran it.
5. *(Added on acceptance, OD-2026-11-19-2.)* For an emergency, the operator MAY force a `RETIRING` version `RETIRED` at
   once (`flow_control_key_force_retire`, migration `20270202000000`), with a reason. It takes the version table as the
   activation does, so it waits for every deciding transaction in flight. From its commit no decider digests a key under
   that version: a key decided afterwards is filed under the active one, so a key MAY have two rows until the old one
   drains. The old row keeps its holders, grants and waiters, gains no new waiter and grants only its own waiters within
   its own limit, so **a key's limit is over-admitted, at any moment, by at most what its old row holds at that moment
   (its holders, or its counted grants), never more than its limit, until the old row's holders and waiters drain**.
   Order across the two rows is lost meanwhile: a new decision is granted on the active row while older waiters wait
   on the old one (an exception to ADR-0088 decision 2 and ADR-0056 decision 5, bounded by the drain). One forced
   version drains at a time (`FORCED_VERSION_DRAINING` refuses a second force), so a key never has more than two rows.
   The active version is not forced: a successor is activated first (both commands may run in one transaction). Every
   force is recorded in the append-only `flow_control_key_forced_retirements` (operator, database role, reason, rows
   left); `flow_control_key_version_runs` lists the runs that hold a version's keys, for the operator who waits for or
   cancels them instead (O1170 (a)). The namespace's live-keys ceiling counts the rows of the versions in force only,
   so a key filed again after a force is not refused for the row it replaces; the ceiling is passed by at most the rows
   the force left, until they drain.

## Considered options

1. **Per-key rotation, one at a time, lazy retirement, with an operator's force for incidents** — chosen (OD-2026-11-19-2). Rotation without draining every tenant's queues; no
   key split; no cost on release paths.
2. **Drain first (ADR-0056 decision 7 as accepted)** — simpler and needs no second digest, but a rotation (including the
   rotation of a leaked key) waits for every queue of every tenant to empty, which an operator cannot force without
   cancelling runs.
3. **Retirement by a trigger on the last row's delete** — built first and dropped after review: it serialized every key
   deletion of a rotation on one version row and counted the retiring rows on each delete (correctness F2, architecture
   M1).
4. **Re-key a row under its lock on the next decision that touches it** — bounds a rotation for keys in use, but a key
   held by a long-lived permit and never decided again still pins the old version; and it changes a primary key that
   holders and waiters reference.
5. **Several retiring versions at once** — lets an emergency rotation start while another is open, at the cost of up to
   N digests per decision and of every process holding every retiring key.

## Consequences

- Good: a key rotation no longer needs a drained fleet; the decision cost is one extra digest and one index probe per key
  during a rotation only.
- Bad: **a rotation ends only when the old version's last row goes, and live rows of any tenant decide when that is.** An
  emergency rotation of a leaked key (a third version) is refused until then, and every process must keep the retiring
  key's material, or every gated decision fails (`UnknownVersion`). The owner answered O1170 with decision 5: the operator
  forces the retiring version out, at the stated over-admission, and the processes may then drop its key.
- Bad: activating a version that a live process does not hold makes that process refuse every gated decision until it is
  configured (G1193); the registry does not check it.
- Bad (decision 5): a rate window's grants drain only once their lease instants are recorded and purged (G022-003,
  T022-074): until then a window row never goes, so a forced rotation's rate over-admission and the old digests stay,
  and an ordinary rotation with rate rows ends only by a force.
- Compatibility: the command keeps its name and result shape; versions the earlier command left `RETIRING` (with no row,
  as it required) are retired by the migration. Rollback (the down migration) refuses while a rotation is in progress and
  otherwise restores the earlier command. Once a force is recorded, the force's migration cannot be rolled back (its
  history is evidence and append-only): rolling back below `20270202000000` is one-way from then on. No gated run exists
  before the gates are built (D022-037).

## Evidence

The tests under "Enforced by", each seen failing under a hand mutation (the retiring row ignored, a third version
allowed, the retiring version in force while unused, a vanished row filed anyway; for decision 5, the force without the
version table's lock, the forced version left in force, a second force while the first drains, and the live-keys count
over every version). Spec FR-022-053, FR-022-056; research
D022-025, D022-042, D022-043; reviews `specs/022-flow-control-step-policies/reviews/2026-11-16/`.
