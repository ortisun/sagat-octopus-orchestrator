# ADR-0132: The journal is partitioned by the month a run was created, a run owns its payloads, and a month is dropped only when every run in it is purged

**Status:** Accepted (2026-11-26, with design 049's approval)
**Decided by:** agent proposal (design 049); accepted by delegate (OD-2026-11-24-4) once design 049's review was complete (its approval reports and Owner decisions 2026-11-24-516 to 2026-11-24-518); the deletion it decides waits for constitution 5.1.0 (Owner decision 2026-11-25-3)
**Refines / supersedes:** refines in part [ADR-0054](ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md): its decision 4 (one privileged purge function) gains a second path for whole months; its decision 6 (references found from the catalog) takes its references from the registry it already plans; its rejected option 3 (partition drop) is taken for whole months only, aligned with run trees. Its stub, its eligibility rule and its verified archive are unchanged. Refines [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) (a drop releases content bytes through the accounting function, in an idempotent transaction that follows the drops and in tenant order, not by row triggers) and [ADR-0131](ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md) (the conversion runs in the stopped upgrade from 0.2.x to 0.3.0, its migrations carrying that decision's waivers for the rules that assume running writers). Supersedes nothing.
**Enforced by:** the `runs` triggers and the child-bucket tests (Task 049-5); the bucket trigger and the suite after the switch across a month boundary (Tasks 049-8 and 049-13); the registry tests that fail on an unregistered digest column, a run-owned foreign key to `payloads` or a column of no run referencing `run_payloads`, and the test over `pg_constraint` that no foreign key touches a partition tree (Task 049-10); the static check that every statement for one run on a partitioned journal table compares its bucket, and that only `payloads.rs` names a payload table and only `journal.rs` reads `run_events` for one run (Task 049-6); the catalog checks that know partitions, triggers and functions' effective grants, `prosecdef` and `proconfig` (Task 049-7); the relations-locked and round-trip lines of design 038's budget (Task 049-4); `a_permanent_capability_refuses_both_deactivation_phases` (design 053's Task 053-5 on a test-only capability, and Task 049-13 on `journal-buckets-v1`), the test that `Capability::is_permanent()` agrees with the check (Task 049-13), and `a_process_registered_after_the_switch_creates_dated_roots` (Task 049-13); the runtime-role test that `DETACH`, `DROP` and the retirement functions fail for `sagat_app`, and the crash, race, lock-timeout and quota-reconciliation tests of retirement (Task 049-14)

## Context

Nothing is deleted today. Feature 020 plans a verified archive and a purge of each run in chunks; payloads are shared by
content within a tenant, referenced by 22 foreign keys (20 in 12 run-owned tables, 2 in shared tables) and by three
run-owned columns with no key, so a purge needs a reference check and an index on each of them (feature 020, gap 1099).
Architecture decision 0054 rejected partition drop because partitions did not align with runs and could not be verified
per run. Child run identifiers are hashes, not times, and a child may be created months after its root. Every read of
the journal for a run is by run. Foreign keys into a partitioned table make each detach scan the referencing tables under
a lock that blocks their writes, and a drop releases no quota because it fires no row trigger. The upgrade from 0.2.x to
0.3.0 stops every writer.

## Decision

1. Every run MUST carry an immutable `journal_bucket`: a root's is the UTC month of its `created_at` or the legacy
   value, a child's is its root's, so a run tree is in one bucket. It is computed once, before the creating transaction's
   first write, and every row a run writes to a partitioned journal table MUST carry it; triggers refuse a mismatch.
2. `run_events` and a new `run_payloads` MUST be partitioned by range of `journal_bucket`, one partition per month, with
   the bucket last in their primary keys so that lookups by run keep their prefix; the existing `run_events` becomes the
   legacy partition, without a row copied, in the stopped upgrade.
3. A run tree created after the switch MUST keep its payloads in `run_payloads`, keyed `(tenant_id, root_run_id, digest,
   journal_bucket)`, and MUST NOT reference a payload of another tree; a tree's payloads are deleted with its last run,
   and erasure of a dated run erases its tree. A legacy run keeps its payloads in `payloads`, uncopied, purged by feature
   020's reference check. Payloads of rows that belong to no run, and the tombstones of feature 020's stubs, stay in
   `payloads`.
4. A foreign key MUST NOT point into or out of a partitioned journal table. Every reference to run content MUST be
   listed in one registry with the way to its owning run and checked on write by a definer trigger that locks the row it
   finds (`FOR KEY SHARE`) and raises the SQLSTATE of the key it replaces; the runtime role gains no `UPDATE`.
5. Every statement for one run on a partitioned journal table MUST compare its bucket to a parameter or a literal;
   statements that span runs are allowed only off the lease and completion paths, by name; the way statements are planned
   MUST be the one design 049's Task 049-3 measures to lock no more than the pruned partition.
6. A bucket MAY be dropped only when every run in it is purgeable under feature 020 and its archive verified, in this
   order: the retirement recorded as active, excluding per-run purges of the bucket; the runs purged under their run
   locks through feature 020's purge function; the frozen partitions checked and their bytes per tenant counted; for
   each table, one transaction under a lock timeout that locks only its parent and the bucket's partition, detaches and
   drops it; then an idempotent transaction releases, audits and records the bytes. Nothing is ever left detached and not
   dropped; a retirement abandoned before its drops deletes by key the journal rows of the runs it already purged and the
   payloads of the trees whose runs are all purged; the retirement's functions serialize on its row. Retirement is an operator's command run as a role that can only call its definer
   functions, never a platform process. Deleting journal rows waits for constitution 5.1.0, approved by Owner decision
   2026-11-25-3.
7. New roots get a dated bucket once their process has learned of the registry's activation of `journal-buckets-v1`; a
   legacy root created before a process learns of it is valid. The capability is not deactivated.
8. `runs`, `steps` and the audit trail are not partitioned in release 0.3.0; `steps` is measured again after design
   052's queue; no audit record is dropped. The monthly width is fixed by the `runs` trigger and the partition names, and
   changing it later means a new key for new trees.

## Considered options

1. **Partition by the run tree's creation month, payloads owned by the tree, triggers in place of foreign keys
   (chosen).** A tree's rows are in one partition; a tree's purge and a month's drop both need no reference check; a
   detach and a drop scan nothing of the hot path's.
2. **Per-run purge only, payloads shared (feature 020 as drafted).** Correct; deletes of every row and 22 reference
   indexes at scale.
3. **The same partitioning with foreign keys.** Integrity by the database's mechanism, and a `SHARE` scan of every
   referencing table at each detach, with `ACCESS EXCLUSIVE` on `runs` at each drop.
4. **Partition by event time.** Splits long runs and cannot drop a month while any run that wrote to it lives.
5. **Partition by the run identifier.** Child identifiers are hashes: a range of identifiers is not a range of time.
6. **A concurrent detach, then a drop** (this record's first draft). It cannot run in a function or a transaction, so the
   command would hold the owner's credential and need a recovery path for a detached, undropped table.

## Consequences

- Good: a tree is purged by its key; retention costs three short transactions per month (a drop per partitioned table, then the release) where every
  namespace with runs in it has a policy no longer than the month's age; hot partitions stay small.
- Bad: identical bytes in two new trees are stored, and charged to the tenant, twice; legacy runs keep the shared regime
  and its reference-checked purge; a trigger call and a `runs` lookup per write replace the built-in key check; a
  namespace with no policy (the default) holds every month, so the drop is opportunistic and the per-run purge the main
  path; erasing one run of a dated tree erases the tree.
- Harder: every new statement on the journal must name its bucket (checked); every new reference to run content must be
  registered (checked); a long-lived run holds its month.
- Compatibility: the conversion runs in the stopped upgrade; triggers raise the SQLSTATEs of the keys they replace.
- Rollback: before the activation nothing is dated and the partitioned parent behaves as the old table; after it the
  partitions stay and the binaries cannot go back below the release that activated it.

## Evidence

Design 049 and its review reports; feature 020's `data-model.md` and gap 1099; `crates/application/src/execution.rs`
(`child_run_id`); `tests/performance/reports/scalability-ha.md` (the lock-manager waits of 2026-11-12). To be added when
built: the measurements of Tasks 049-2, 049-3 and 049-8.
