# Targeted re-read of design 046 after review round 3: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 1ec97a2206971ea2d6d1cb9bd69498cf14ea1ffa
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A targeted re-read of the
changes made after round 3 (the approval rule of Owner decision 2026-11-25-2), not a full review. The reviewer read the
post-round-3 diff, the design, the round-3 security report and resolution, and `crates/api-http/src/routes/runner.rs`;
it ran nothing.

## Round-3 findings

1. Answered: the registry's own hash per file, private files against `private_packages`, `.dist-info` and `package.json`
   checked by the layer tool (the npm scope caveat is finding 2 below).
2. Answered: no bytecode in the layer, verify refuses `__pycache__`, the run's Python writes no bytecode.
3. Answered in part: the three route kinds are defined, but the npm tarball name rule is wrong (finding 2).
4. Answered: npm times from the full document, versions or files without a time dropped (memory: finding 6).
5. Answered in part: forwarded bytes are charged, but only after the fact (finding 3).
6. Answered in part: blocks carry the tenant and the runnable check is per attempt, but the materialization route trusts
   a namespace the runner supplies (finding 4).
7. Answered: an accepted residual stated in the data model.
8. Answered in part: upload size bounds are there; quarantine on uploads is not addressed (finding 5).
9. Answered: a held import refuses matching finishes and materializations.

## New or remaining findings

1. Major (new). "Rows inserted after staging began" has no safe definition (Technical Plan 7, Decision 046-055, data
   model `script_locked_packages`). The table has no sequence or time column, and anything based on time or a sequence
   orders by transaction start, not commit: a resolve completion that inserts its rows before staging reads the table but
   commits after it is neither precomputed nor range-evaluated, so an artifact locking a malicious package stays leasable,
   and its tenant may get no re-evaluation row. The switch also never reads the precomputed pairs that
   `a_finish_between_staging_and_switch_is_matched` relies on. Fix: a barrier `FOR UPDATE` on the snapshot row at the
   start of staging (writers already take it `FOR SHARE` and must do so before inserting), a `bigint` identity column whose
   highest value is the mark, range evaluation above it; or `xmin` against the staging snapshot. A held-lock test.
2. Nit (remaining, round-3 finding 3). A scoped npm tarball is named `billing-1.0.0.tgz` without the scope, so reading the
   name from the file name sends a reserved `@acme` name to the gateway (a leak, and a squatted public fetch the lock
   check would refuse too late). Fix: the name from the `{name}` path segment, the file name equal to
   `{unscoped}-{version}.tgz`; a scoped tarball case in `a_reserved_name_never_reaches_the_public_registry`.
3. Nit (remaining, round-3 finding 5). The fetch budget is charged only after a resolve completes: a tenant over budget can
   still claim resolves of up to 1 GiB each, and failed resolves and retries are never charged. Fix: refuse the claim over
   budget; charge forwarded bytes on every completion, failures included.
4. Nit (remaining, round-3 finding 6, plus contract gaps; `runner-protocol.md`). The materialization route takes
   `namespace` from the runner (take it from the ready activity); it returns no `id` that the failure route needs; "three
   runners have failed" must count distinct registered runner principals, not `SAGAT_RUNNER_INSTANCE_ID` values; the
   runnable routes must answer the same `404` for another tenant's attempt or test run; the test-run `lockDigest` must
   equal the stored one.
5. Nit (remaining, round-3 finding 8). Uploads have no quarantine: a stolen `package:upload` credential publishes a
   version dependents resolve at once. Fix: a quarantine on `uploaded_at` for private entries, or a contested decision.
6. Nit (new). Full npm documents can exhaust the gateway's memory: `versions` usually precedes `time`, so the parse
   buffers up to 64 MiB per request, with up to 4,000 requests per resolve and no limit on concurrent parses. Fix: cache
   per name, bound concurrent parses, refuse a document over the cap rather than serve it partly filtered.

Counts: Blocker 0, Major 1, Nit 5

## The author's answers (all fixed; a security finding is never settled by a delegate row)

1. Fixed: the barrier and the `locked_seq` mark, the switch blocking every artifact that locks a precomputed pair and
   range-evaluating rows above the mark, tenants from both; the writers' `FOR SHARE` in the lock order; the test
   `a_lock_written_by_a_transaction_open_across_the_barrier_is_matched` (Decision 046-065).
2. Fixed: the `{name}` segment and the file-name equality, in the design and `runner-protocol.md`; the scoped-tarball case
   (Decision 046-066).
3. Fixed: every completion charges what it forwarded (`forwardedBytes` on failure too), an expired lease is charged its
   1 GiB ceiling, the claim refuses an over-budget tenant; two tests (Decision 046-067).
4. Fixed: the route takes a ready activity and answers an `id` (`script_layer_materializations`); three distinct
   authenticated runner principals; one `404` for unknown and foreign attempts; the stored lock digest required; four
   tests (Decision 046-068).
5. Fixed: `private.quarantineHours` (default 24) with an early release by another `package:manage` holder, never the
   uploader (`PACKAGE_RELEASE_BY_UPLOADER`); two tests (Decision 046-069).
6. Fixed: a compact cached index, at most 8 parses at once, `502` over the cap and never a partly filtered document; two
   tests (Decision 046-070).
