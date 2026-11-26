# Targeted re-read of design 046, second pass: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 47c730c1f4e01d03b9c007e1ea1483077d938992
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A short re-read of the fixes
of `reread-1-security.md`; the reviewer read the diff of the fixes, the design and its contracts, and ran nothing.

## The previous findings

1. Answered: the barrier, the writers' `FOR SHARE` before their first insert, range evaluation above the mark and blocks
   for precomputed pairs whatever their sequence; a mark read too low only adds safe range evaluation. Two
   preconditions are missing (finding 5).
2. Answered: the `{name}` segment, the file-name equality, the scoped-tarball test case.
3. Answered, with a gap: concurrent claims can still overshoot (finding 2).
4. Answered: the activity, the `id`, distinct principals, one `404`, the stored lock digest; the failure route has a gap
   (finding 3).
5. Answered: the hold and the uploader rule, with tests; gaps in findings 1 and 6.
6. Answered: the compact index, 8 parses, `502` over the cap; the cache is unbounded (finding 4).

## New or remaining findings

1. Nit. `private_packages` has the grants SELECT, INSERT, DELETE, but the release writes `released_by` and
   `released_at`; the likely quick fix would be a broad UPDATE. Fix: `UPDATE (released_by, released_at)` only.
2. Nit. The claim refuses only a tenant at or over budget, so a tenant one byte under it can hold one resolve per free
   runner, each up to 1 GiB, before a completion is charged; the generic failure body has no `forwardedBytes`. Fix: charge
   the 1 GiB ceiling at claim and settle at completion; add `forwardedBytes` to a resolve's failure body.
3. Nit. The contract does not say who may report a materialization failure: one compromised runner could fail rows of two
   other principals and so fail a tenant's ready steps with `LAYER_UNAVAILABLE`. Fix: `404` unless the caller is the
   row's own principal in its tenant; a test.
4. Nit. The per-name cache has no byte bound and "offsets" suggests the raw document is kept; the 8 parse slots are shared
   by every tenant. Fix: cache the extracted entries, bound the cache by bytes with least-recently-used eviction, limit
   parses per grant.
5. Nit. Nothing creates an ecosystem's snapshot row before its first writer (a missing row locks nothing), and an identity
   with `CACHE` above 1 hands out values out of order across sessions. Fix: seed a row per ecosystem in the migration;
   `CACHE 1` with a migration check.
6. Nit. The hold removes a version only from the private index; an author lock and the private file route do not refuse a
   held version. Fix: state the hold at author-lock validation and on the file route; a test.

Counts: Blocker 0, Major 0, Nit 6

## The author's answers (all fixed; Decision 046-071)

1. The release is the only UPDATE, of the two columns. 2. The claim charges the ceiling under the usage row's lock and
completion settles it; `forwardedBytes` on a resolve's failure body; `concurrent_claims_cannot_overshoot_the_budget`.
3. Only the creating principal, else the unknown `404`; `only_the_creating_runner_may_fail_a_materialization`. 4. Extracted
entries only, a 256 MiB least-recently-used cache, 2 parses per job grant; `one_grant_cannot_hold_every_parse_slot`.
5. Seeded rows and `CACHE 1` with a migration test. 6. The hold refuses an author lock (`PACKAGE_QUARANTINED`) and answers
the unknown `404` on the file route; `a_held_upload_is_refused_in_an_author_lock_and_on_the_file_route`.
