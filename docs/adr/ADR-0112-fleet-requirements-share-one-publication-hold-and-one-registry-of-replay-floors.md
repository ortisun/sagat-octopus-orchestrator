# ADR-0112: Fleet requirements share one publication hold and one registry of replay floors

**Status:** Accepted (2026-11-26), by delegate (OD-2026-11-24-4), on the approval of design 054 that day; implementation and its evidence pending (Enforced by)
**Decided by:** delegate (OD-2026-11-24-4), accepting the agent's proposal made under the owner's delegation (Owner decision 2026-11-20-6, widened by Owner decision 2026-11-24-4), for designs 044, 045 and 054 of release 0.3.0 (Owner decision 2026-11-24-1); to be accepted with design 054 if it is approved
**Refines / supersedes:** refines Architecture decision 0082 (the instance registry: who records and removes a requirement) and, on the branch of pull request #260, Architecture decision 0098 decision 8 (a requirement as a replay floor), whose floor becomes one row of the registry below; supersedes none.
**Enforced by:** the tests of design 054's Task 054-18 in `integration_platform_foundation` (`requirement_hold`: the function's privileges and configuration, its body against the text of the amendment below, a publication's hold against a concurrent removal, a removal refused by a registered floor and allowed once the floor clears, two capabilities registered at once, and the two held-lock tests of the amendment); the capability-list test of `crates/infra-postgres/tests/capability_lists.rs` and its rule in `scripts/check-architecture.py`; each design's own floor test.
**Amended:** 2026-11-27, a fairness gate before the epoch (section "Amendment" below; Owner decision 2026-11-26-564).

## Context

Three 0.3.0 designs add a fleet requirement that publication must hold and whose removal must be refused while something depends on it: design 044 (`code-workflows-v1`), design 045 (`activity-timeouts-v1`, `activity-timeouts-protocol-v1`) and design 054 (`signal-inbox-v1`); Architecture decision 0098 (`lease-withdrawal-v1`, pull request #260, not merged) already makes its requirement a replay floor. Each design first described its own way to do both: a function created by whichever lands first and compared by the others, and a rule added to one trigger by whichever lands later. The reviews of design 054 (its stage 5 zoom-out, Major 4; its architecture review, Major 2) found that fragile: each design would replace or compare another's code, and three texts described incompatible mechanisms on one table.

## Decision

1. **One hold.** `platform_requirement_hold(capability text) RETURNS boolean`, `SECURITY DEFINER`, `SET search_path = pg_catalog, pg_temp`, schema-qualified names, `EXECUTE` revoked from `PUBLIC` and granted to `sagat_app`, with the body design 054 gives in its Technical Plan 15 (which replaces design 044's Decision 044-206, whose `FOR SHARE` on the requirement's row broke the registry's lock order): it refuses a capability that has no floor row, takes the requirement epoch `FOR SHARE` (the registry's request fence, first in its lock order), reads the requirement's row without a lock, and returns true only when the requirement is recorded for both control planes and engines. A publication calls it as its last statement before its commit. Its body names no capability; the closed check `platform_requirements_capability_known` bounds the value. A publication that needs a requirement calls it in its own transaction under a `lock_timeout`; `false` is refused with the one reason `CAPABILITY_NOT_ACTIVE`, the capability named in the problem's detail.
2. **One registry of floors.** A table `platform_requirement_floors (capability text PRIMARY KEY, holds regprocedure NOT NULL)`, its capability under the same closed check, owned by the migration role, with no grant to `sagat_app`, whose check trigger admits only a function with no argument, a `boolean` result and the trigger's owner; and one trigger function, `platform_requirement_floor()`, `SECURITY INVOKER` with a fixed search path, `EXECUTE` revoked from `PUBLIC`, enabled `ALWAYS`, `BEFORE DELETE` on `platform_requirements` (a requirement is never updated, and is deleted only inside `platform_requirement_remove`). It refuses unless the transaction is `READ COMMITTED`, and refuses the removal with `FLOOR_HELD` when the function registered for the capability (`EXECUTE format('SELECT %s', holds)`; a `regprocedure` prints its own argument list) returns true.
3. **Each design adds a row and a function, never replaces another's.** A design registers its capability in the closed lists, one row of the registry and one `SECURITY DEFINER` function with a fixed search path, `EXECUTE` revoked from `PUBLIC`, that answers whether its floor holds (for example design 054's `signal_inbox_floor_holds()`, design 045's on its level column, Architecture decision 0098's on its journal's withdrawals). A migration that finds the hold compares its security, configuration, owner and body normalized as `regexp_replace(btrim(prosrc), '\s+', ' ', 'g')` with decision 1's text and fails on a difference, as a guard against drift.
4. **Built once, first.** The hold, the registry and the trigger are one migration and pull request of their own (design 054's Task 054-18), landed by whichever of the designs builds first; the others depend on it.

## Considered options

1. **One hold and one registry (chosen).** No design changes another's code; a later feature adds a row.
2. **Create-or-compare per design, one trigger extended by each.** Rejected: each later design would replace the trigger with `CREATE OR REPLACE`, which the drift guard of the earlier one refuses, and the order of landing decides the outcome.
3. **A floor per design in a trigger of its own.** Rejected: several triggers on one table fire in name order, each with its own isolation check, and a reader must find all of them to know why a removal is refused.

## Consequences

- A removal names the floor that refused it; a new requirement costs one row and one function.
- The trigger runs dynamic SQL, bounded to `regprocedure` values only a migration can insert.
- Whether the requirement epoch `FOR SHARE` lets a continuous stream of publications starve the operator's activate and remove is settled by a held-lock test written before any publication uses the hold (Tasks 045-21 and 054-20, Owner decision 2026-11-26-564); if they starve, this record is amended with a fairness gate before the epoch in the lock order.
- Designs 044 and 045 cite this record in their next revisions (design 054, Gaps 054-007 and 054-010); Architecture decision 0098's floor becomes a row when both are built.

## Amendment (2026-11-27): a fairness gate before the epoch

Accepted 2026-11-27 by delegate (OD-2026-11-24-4), under Owner decision 2026-11-26-564, which settled in advance that this record is
amended in place if the test shows starvation. Decision 1's hold, its registry of floors and its lock order otherwise stand.

**What the evidence shows.** PostgreSQL 16 (the version the platform and its tests run, `postgres:16-alpine`) lets a new `FOR SHARE`
locker of a row that only other transactions share-lock go ahead of a transaction that already waits for `FOR UPDATE` on that row:
the new locker never sleeps, and the waiter wakes, finds the row share-locked again and waits again. `lock_timeout` restarts on
every such wait, so the waiter is not even refused within its bound: it waits until its `statement_timeout`. A probe on 2026-11-27
(PostgreSQL 16.15, four sessions relaying `FOR SHARE` on one row, the next granted before the previous commits) granted 48 share
locks after a `FOR UPDATE` was seen waiting, and the `FOR UPDATE` with `lock_timeout = 2s` was still waiting after 10 s. With the
gate below, no share request made after the exclusive one was seen waiting was granted before the exclusive one committed, and it
committed at once. The repository's tests of this are
`integration_platform_foundation::requirement_hold::an_activation_commits_under_a_continuous_stream_of_holds` and
`...::a_removal_commits_under_a_continuous_stream_of_holds`; a publication that holds a requirement on every request would
otherwise leave the operator unable to activate or remove any requirement, because the epoch is one row shared by all of them.

1. **The gate.** One transaction-scoped advisory lock on the fixed key `hashtextextended('sagat:platform_requirement_epoch:gate',
   0)` comes before the epoch in the registry's lock order. Every function that takes the epoch `FOR UPDATE` on the operator's
   behalf (`platform_requirement_activate`, `platform_requirement_remove`, `platform_requirement_lock`), and the automatic handover
   (`schedule_trigger_handover_auto`, ADR-0091), which takes the epoch and then calls the activation, MUST first take the gate
   exclusively (`pg_advisory_xact_lock`); every function that waits for the epoch `FOR SHARE` (`platform_requirement_hold`,
   `instance_register`) MUST first take it shared (`pg_advisory_xact_lock_shared`). The lock manager queues a shared request behind
   a waiting exclusive one, so a hold or a registration that comes after the operator's command waits for it, and the command waits
   only for the holds granted before it. The epoch `FOR SHARE` stays in the hold: it is what makes a removal that took the epoch
   wait for a publication that is committing.
2. **The lock order** is now: the gate, then the requirement epoch row, then the advisory lock that counts the registered instances,
   then rows. No function waits for the gate while it holds a later lock.
3. **The hold's body**, which replaces decision 1's text and is the text every later migration compares with (decision 3's drift
   guard; the repository's test of it reads this block):

   ```sql
   CREATE FUNCTION public.platform_requirement_hold(capability text) RETURNS boolean
   LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
   DECLARE
       v_kinds text[];
   BEGIN
       IF NOT EXISTS (SELECT 1 FROM public.platform_requirement_floors f
                       WHERE f.capability = platform_requirement_hold.capability) THEN
           RAISE EXCEPTION 'no floor is registered for this capability' USING ERRCODE = 'invalid_parameter_value';
       END IF;
       PERFORM pg_catalog.pg_advisory_xact_lock_shared(pg_catalog.hashtextextended('sagat:platform_requirement_epoch:gate', 0));
       PERFORM 1 FROM public.platform_requirement_epoch e WHERE e.only_row FOR SHARE;
       SELECT r.kinds INTO v_kinds FROM public.platform_requirements r
        WHERE r.capability = platform_requirement_hold.capability;
       RETURN FOUND AND v_kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[];
   END;
   $$;
   ```

4. **The floor's isolation check** applies to a capability that has a floor row: the trigger refuses a removal outside `READ
   COMMITTED` only when it is about to read a floor, since only that read needs the committed versions. A removal of a capability
   with no floor behaves as before this record.
5. **The floors' closed list.** `platform_requirement_floors.capability` is bounded by a fourth closed check,
   `platform_requirement_floors_capability_known`, which lists exactly what the three checks of the registry list; the repository's
   capability-list test and its rule in `scripts/check-architecture.py` cover it, so a design that adds a capability restates four
   lists.

The automatic handover must take the gate first, not only for fairness: it holds the epoch when it calls the activation, which now
asks for the gate, so without the gate first it would wait for the gate while holding the epoch, and a registration or a hold that
holds the gate shared and waits for the epoch would close a deadlock (found by the correctness review of 2026-11-27).

What the gate does not cover, recorded as Gap 045-006 in `specs/045-activity-timeouts/research.md`: the beat of a process
(`instance_heartbeat_reporting_change`) reads the epoch `FOR SHARE SKIP LOCKED` without the gate, because it never waits, so beats
can still overlap an operator's wait. That breaks no guarantee of this record: it only lengthens a wait that the beat rate bounds.

A role that may call `pg_advisory_xact_lock` (every role, by default) can take the gate itself. That gives the runtime role no new
power: it can already hold the epoch for the length of a transaction through the hold or a registration, and it can already take the
advisory lock that counts registrations.

## Evidence

Design 054, Technical Plan 15 and Task 054-18 (`docs/designs/054-buffered-signals.md`), and its review reports (`docs/designs/054-buffered-signals.reviews/2026-11-26/`); design 044 section 14 and Decision 044-206 (branch `feature/044-workflow-as-code`); design 045 Technical Plan 13; `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql` (`platform_requirements_guard`, `platform_requirement_remove`).
