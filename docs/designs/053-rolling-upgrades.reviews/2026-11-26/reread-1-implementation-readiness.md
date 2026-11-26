# Targeted re-read 1 of design 053: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 877308b4ed146df49a4ab96ef0a0b611d18c6dfe
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`implementation_readiness_report.md`, not a full review.

Full round: the Blocker, Majors 1 to 4, 6, 7, 9, 11 and 13, and Nits 1 to 8 resolved; Majors 5, 8, 10 and 12 partly
(carried below).

## Major

1. `command_await_durability(capabilities)` cannot run the schema check (no versions parameter), and design 048's
   Task 048-19 says it "carries" it. Fix: `connect_command` calls `sagat_schema_missing(versions)` on the admitted
   connection, then `command_await_durability`; `capabilities` is the binary's own list; Task 048-19 says "follows".
2. Task 053-6 cannot be configured: no load rate; no N-1 commit before 0.3.0; two commits of one branch add no
   capability, so the capability flow is not proved. Fix: a rate; the N-1 commit; the test-only capability under
   `mixed-writers-fixture` activated, drained and removed.

## Nit

1. How the merge-base check recognises the 0.3.0 release pull request; admit only the key going from absent to the
   newest migration.
2. The lock-level source and the `-- blocks:` header's rule.
3. Exit codes of `capability-drain`, `capability-undrain` and `instance-evict`.
4. `SchemaVerdict.known` when the check fails closed.
5. "Applied" means `applied_epoch >= draining_epoch`.
6. `ServingGate::uses` replaces `requires`.

Counts: Blocker 0, Major 2, Nit 6

All eight were fixed at c0536d9f (the commit after the one that recorded this re-read).
