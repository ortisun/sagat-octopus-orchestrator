# Architecture re-read of design 049 (2026-11-24)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: c095c5c5e36495cbe5a43ae3933c4220c7ef4718
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The third text of the
design (legacy payloads in place, the bucket last in the keys); the reviewer read files only, did not read the earlier
reports, and found no other branch or worktree with journal partitioning or an ADR numbered 013x.

## Blockers

1. Triggers in place of foreign keys lose the key's `FOR KEY SHARE` lock on the referenced payload, so a legacy run that
   reuses a digest can pass its check while feature 020's legacy purge deletes the payload. Fix: the trigger locks the
   row it references; the purge re-checks after deleting, under a savepoint; barrier tests for both orders.
2. The design builds feature 020's parts (the registry file 020 owns, the legacy purge's reference check) with no
   `Owner-gate:` line, and ahead of the amendment that gates them in feature 020. Fix: gate them, or move them back.

## Majors

1. Architecture decision 0132 decision 6 counts bytes before the detach, against the design's order.
2. Retirement is a second privileged deletion path, which the proposed amendment text of Architecture decision 0054
   ("one audited privileged path") forbids.
3. Architecture decision 0027 (accepted) requires the counter change in the deletion's transaction and a tenant order;
   the release and the drop are separate transactions; 0027 gets no refinement note.
4. The design breaks Architecture decision 0131 and design 048 (rename and drop in expand migrations, no allow-list
   mechanism), and the 0.2.x to 0.3.0 upgrade is a stopped upgrade, against CUJ 3.
5. The switch can refuse run creation fleet-wide for up to a registry heartbeat, or cost a hot-path statement.
6. Nothing checks that rows of the unpartitioned run-owned tables carry their run's bucket.
7. The hot-path cost claims are wrong: partial indexes are locked on every insert, and an indexed column defeats HOT
   updates; the added relations exceed the allowance; the legacy indexes are built before the purge they serve.
8. The static check misses `format!` SQL and older functions; a dated run's read of `payloads` would return null content
   silently.
9. Decision 4 (no foreign key into or out of a partitioned table) has no check.
10. The migrations' transaction boundaries and downs are not specified; step 1 accumulates exclusive locks on many tables.
11. The typed errors are in no contract and clash with `RUN_PURGED` as a reason.
12. Per-run copies can push runs in flight over the content quota after the switch (a fan-out's children copy their
    input). Consider giving a child its root's bucket.
13. The cited owner decisions are not rows in this tree, and putting design 049 in 0.3.0 is the owner's scope call.

## Nits

1. `journal-buckets` against `journal-buckets-v1`.
2. Decision 8 says design 051's queue; it is design 052.
3. The Context says one column without a key; the design lists three.
4. The index title in `docs/adr/README.md` differs from the file.
5. "No foreign key MUST point" means "MUST NOT".
6. List the referencing tables by name; the count of 11.
7. The budget hook misses commit-time triggers.
8. The 120 s bound assumes `transaction_timeout` everywhere; a waiting detach blocks attaches.
9. Cloned triggers enabled always after attach; keep the legacy truncate trigger.
10. The alert's rule test must be in CI's list.
11. Step logs in the mark-only purge (Architecture decision 0084 decision 4).
12. No task adds the new checks to `docs/engineering/architecture-principles.md`.
13. The decision record's Enforced-by line omits the copy tests and a switch-race test.
14. A per-tenant audit record of the quota release.
15. The retirement job's owner credential against "no platform pod mounts it".
16. `journal_bucket` stays out of the domain; the Constitution Check's row II omits the application changes.
17. During the upgrade, the previous release's statements lock every partition.

Counts: Blocker 2, Major 13, Nit 17
