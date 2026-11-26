# egm-review Stage 7 design lens, round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 44133eb732bb25f93904c0d144a7b5af1fa26d9f
Reviewer: egm-design, feature-spec standard, FeatureRange 1-1156
Note: condensed by the author from the reviewer's output; every finding and severity is kept.

1. Blocker (861-872, 227-232, 1035-1036): a move changes `runs.workflow_version_id`, from which step ids
   (`crates/application/src/execution.rs:81-116`), journal instance ids (`crates/infra-postgres/src/execution.rs:5183-5186`)
   and saga plans and `compensation_id` (`20261011000000_sagas.up.sql:81-120`) are derived; a moved run would stop
   matching its own steps, leases and compensations. Keep a separate code pin and derive identities from the original
   version; say what a move does to saga rows; test it.
2. Major (861-872, 773-775): a lease of the old digest in flight while a move applies would commit old-code commands
   under the new pin; and `run_version` counts invisible events, so busy runs never move. Fence the lease, check the
   session's digest on every completion, compare the visible count.
3. Major (37-39, 817-824, 1088, 1118): the chain covers only what the history reveals, so a worker that skipped replay or
   forges can produce it; say so, or chain over something the worker is not sent.
4. Major (33-36, 663-680, 1085): the digest misses dependencies, the language runtime version (Python 3.10 against 3.11
   string hashing), locale and time zone; hash runtime and dependency closure; pin UTC and an invariant locale.
5. Major (454-477, 577): Python's module-level imports pass the finder, and `uuid4`, `secrets`, `os.urandom` are not
   covered; `asyncio.timeout` needs 3.11; apply refusals at module load, replace those views, state the minimum version.
6. Major (545-547, 1004, 840): a reset with a new seed diverges on its prefix; keep the source seed.
7. Major (137-142, 270-271, 608-612, 768-791): the 12-statement budget does not add up and omits audit, blob commits,
   byte charging, approvals, parent notification, child cancellation and the continue-as-new successor; today's persist
   issues one insert per row; recount per command class and describe the batched path as new code with crash tests.
8. Major (185-189, 242-245, 893-897, 1091): the 4 MiB retained bytes fill with step outputs long before 10,240 events;
   state the charge and the outcome at the cap; suggest continue-as-new on bytes too.
9. Major (242-245, 826-842, 932-936): a `HISTORY_LIMIT` hold has no remedy that keeps the run going; allow completions
   that drive to quiescence between the limits, or a retry into a window where only continue or end is accepted.
10. Major (756-758, 793-795, 222-226): complete or fail does not say what happens to outstanding work, open sagas or
    pending reviews (principle III); refuse a terminal command while a saga or review is open; apply cancel policies.
11. Major (156-160, 503-504, 531, 566-567, 826-835, 907, 1000): the causes disagree; one table of causes, behaviour and
    remedies.
12. Major (69-70, 213-216, 941-960): saga plans are computed at publication and immutable per version; code sagas need
    run-scoped plans and a `compensation_id` rule; `'review'` does not match the constraint (`manual_review | fail`).
13. Major (254-256, 1057-1060, 1090): ADR-0082 gates only through an activated requirement; make the first publication
    activate `code-workflows` so a process that registers later without it closes its gate.
14. Major (1094-1110, 81-89): the check is against 5.0.0, not on disk; make the merged amendment a precondition and list
    the 4.7.0 clauses it changes.
15. Major (194-204, 239-241, 863-866, 902-925): no quotas or rate limits on worker queries, waits, moves and checks; a
    single history can exceed the 8 MiB replay batch; specify the wait mechanism, paging and load.
16. Major (354-361, 678-680, 1085-1087): every compiled rebuild is a new digest, and a rolling deploy leaves open runs
    unserved until a move; document the pipeline path, alert on unserved time, or narrow the digest.
17. Major (492-493, 564-570): an abandoned coroutine or thread cannot be killed and leaks; cap abandoned activations,
    report the worker unhealthy, say whether Rust uses one thread per run.
18. Major (31-48, 97-107, 1066-1078): the Temporal comparison is out of date (Worker Deployment pinned versioning,
    replayers for CI); compare with current Temporal and restate the differences.
19. Nit (407-410): the spikes were not committed; reproduce them as first red tests.
20. Nit (421, 471-475): table 1.1 and 1.3 differ on Python sets; objects with identity hashes not mentioned.
21. Nit (723-731): which start closes an activation after failed tasks.
22. Nit (905-907): "the validator runs first" against one event at a time.
23. Nit (755, 762-766): `search_attributes.upserted` is neither existing nor listed as new.
24. Nit (1010): 1.8 is not in the register on `develop`; drop "expected 1.9".
25. Nit (290-292): Success criterion 044-002 says a non-matching move holds; the requirement says unchanged.
26. Nit (239-241, 887-888): the promotion check's purpose; replayed runs may be gone by retention.
27. Nit (1063-1064): no fleet-wide switch to stop new code runs.
28. Nit (274, 980-983): a live but idle sticky session delays up to 5 s.
29. Nit (434-436): the bundler must also refuse bare built-in names.
30. Nit (637, 745): `envelope.scripts` should fill `workflow_version_scripts`.
31. Nit (1004): a replay-checked reset becomes asynchronous; say how feature 018's API reports it.

Counts: Blocker 1, Major 17, Nit 13
