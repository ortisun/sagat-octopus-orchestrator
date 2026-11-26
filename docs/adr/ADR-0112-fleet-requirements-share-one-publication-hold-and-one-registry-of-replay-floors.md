# ADR-0112: Fleet requirements share one publication hold and one registry of replay floors

**Status:** Proposed
**Decided by:** agent, proposing, under the owner's delegation (Owner decision 2026-11-20-6, widened by Owner decision 2026-11-24-4), for designs 044, 045 and 054 of release 0.3.0 (Owner decision 2026-11-24-1); to be accepted with design 054 if it is approved
**Refines / supersedes:** refines Architecture decision 0082 (the instance registry: who records and removes a requirement) and, on the branch of pull request #260, Architecture decision 0098 decision 8 (a requirement as a replay floor), whose floor becomes one row of the registry below; supersedes none.
**Enforced by:** the tests of design 054's Task 054-18 in `integration_platform_foundation` (the function's privileges and configuration, a publication's hold against a concurrent removal, a removal refused by a registered floor and allowed once the floor clears, two capabilities registered at once); each design's own floor test. Until they exist the rules are review-only.

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
- Designs 044 and 045 cite this record in their next revisions (design 054, Gaps 054-007 and 054-010); Architecture decision 0098's floor becomes a row when both are built.

## Evidence

Design 054, Technical Plan 15 and Task 054-18 (`docs/designs/054-buffered-signals.md`), and its review reports (`docs/designs/054-buffered-signals.reviews/2026-11-26/`); design 044 section 14 and Decision 044-206 (branch `feature/044-workflow-as-code`); design 045 Technical Plan 13; `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql` (`platform_requirements_guard`, `platform_requirement_remove`).
