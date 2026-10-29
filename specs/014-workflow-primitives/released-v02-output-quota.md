# Released-worker Applied output quota and original-effect drain

Refs: T014-016/017/040, FR-014-005/014/015/017. Acceptance work, not a new public capability.

The actual released v0.2.0 already charges successful collection-owner outputs. Its producer
cannot be assumed to create the uncharged-owner shape from the unpublished6c44a0e4 profile;
ADR-0036 refuses that conflicting migration history without an explicitly proven bridge.
Do not manufacture a legacy projection or edit SQLx history to reuse that old fixture.

Instead, use the real released public writer to finish nine one-item collections, each retaining
a210000-byte activity output and210002-byte aggregate. Complete a250000-byte reservation in the
next compensating collection, report its charge OUTCOME_UNKNOWN and retain the genuine lookup
lease/request in old memory. Independently assert the old ledger4030018, nine owner outputs,
an under-limit current run and a200000-byte intended Applied result that would cross4194304.

After ordinary migration/current runtime-role startup, report/replay that exact old request.
Preserve the successful known forward effect while refusing its output, one complete fingerprinted
receipt, no output-byte charge, CANCELLING/RUN_DATA_LIMIT and the original lease identities.
Reduce iteration concurrency to0 after admission; admitted drain still proceeds. Current scheduler
and worker calls compensate charge then reservation, using the original forward provider keys,
never the lookup key. Finish COMPENSATED, release the held slot, retain the nine original outputs
and prove both current replay and actual old terminal reading. No external provider undo or
exactly-once physical effect is claimed by test workers returning fixture results.

The existing source-verified runner gains scenario `applied-output-quota`. Existing FORWARD,
ordinary Applied and cold-cutover profiles remain separate regressions. Actual results and any
counterexamples must be recorded before this acceptance item closes. General nested capacity,
uncertain-commit/expired-authority compositions and final release gates remain separate.


## Verification — 2026-10-27

The first real profile passed (123.04 s), then review identified three missing observations:
leasing both dependent compensations in one batch, identity of all prior output sources, and
whether RUN_DATA_LIMIT was already persisted before replay or scheduler recovery. The expanded
profile passed in 121.60 s. It now compares all 19 original output-source identities/digests/lengths,
checks the stop state immediately after the first report on both sides of the upgrade, and polls
with the charge compensation lease held to prove the reservation cannot be leased early.

After fixing result archival, the final real runtime-role profiles all pass independently:

| Runner scenario | Command time | Outer native test |
| --- | ---: | ---: |
| applied-output-quota | 76.48 s | 1 passed, 0 failed, 10.70 s |
| active-data | 9.83 s | 1 passed, 0 failed, 6.07 s |
| inflight-worker | 8.67 s | 1 passed, 0 failed, 5.64 s |
| applied-lookup | 9.78 s | 1 passed, 0 failed, 6.76 s |
| scoped-event-cutover | 8.84 s | 1 passed, 0 failed, 5.90 s |

Reproduce each with a private CARGO_TARGET_DIR, the repository-owned Docker daemon and
`SAGAT_TEST_AS_RUNTIME_ROLE=1 python3 scripts/test-released-v02-profile.py --scenario <scenario>`.
The optional `--prepared-run` reuses only this agent's verified old-source/build cache.
The runner verifies 649 original released production files plus the declared fixture overlays;
current source and both executables are independently hashed. All five final profiles bind the
same current source-manifest SHA256, `534d0b24fdf6792f9258ec91641e193977f5ed470817d737c55234c7c8d2e468`.

Evidence: [recorded artifacts](evidence/released-output-quota-20261027/artifacts.json), each scenario's
runner/phase receipts, complete phase transcript, result, outcome/build hashes and compressed
current source manifest. The initial strict workspace/all-target Clippy passed in 175.06 s;
the final gate after archival changes passed in 22.00 s. Format, architecture (24 crates) and
agent configuration checks also pass, with their receipts recorded alongside it. No provider-side physical undo
or exactly-once execution is inferred from these fixtures.

## Result archival correction

The earlier public-event archival wrapper copied a stale generic cutover result into three
other scenario folders. Their successful runner transcripts and source hashes were unaffected.
The three result copies are corrected from their unique original files created within the
recorded runner time windows; [correction provenance](evidence/public-scoped-events-20261027/result-corrections.json)
records the old/new hashes and source paths. No runtime result was reconstructed.

Each new invocation now supplies SAGAT_V02_RESULT in a fresh private directory. Coordinators
write their scenario/release identity there; the runner refuses a missing or mismatched result
and records its hash with the build and transcript hashes in outcome.json. The 16 Python checks
cover stale generic files, wrong scenario/release, exact phase selection and source boundaries.
The final bounded review found no actionable issue; it performed no test execution.

This closes the bounded actual-released Applied/quota/drain/replay acceptance only. General nested
capacity, unpublished migration-history bridges, uncertain-commit/expired-authority compositions,
full fault/resource matrices and integrated release gates remain separate. The eight canonical
feature tasks are still open.
