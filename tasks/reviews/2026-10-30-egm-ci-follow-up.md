# EGM integration CI follow-up — 2026-10-30

Target: PR [#68](https://github.com/ortisun/sagat-octopus-orchestrator/pull/68) into `develop`.
Refs: T014-040, T014-043, FR-014-005.

## Observed failure and limits

On head `2497115b8a5a3bc615eb4cfc50ca270f75128c38`, [CI run 41063106664](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/41063106664) failed only the Rust job. Its workflow-primitives target reported 743 passed, one failed and 22 ignored. The failed test was `leaf_certification_native::saga::later_effectful_saga_planning_and_certification_preserve_selected_source_in_both_orders`; its panic was `actual planner bypassed committing certification`. Twelve of the thirteen PR checks passed, including the separate SDK packaging workflow.

That panic discarded the planner's actual result. It does not establish a missing Run lock or identify whether the planner returned a timeout, an expired-authority error or a normal result after another operation failed. The pristine focused test passed locally once (12.43 seconds); this does not resolve the CI failure.

The fixture had a separate, demonstrable lifetime mismatch: the runtime API pool's statement timeout was ten seconds, while the sequential lock-queue and commit observations could permit a statement to wait longer. Its scheduler placement was also acquired once, with a sixty-second lifetime, before scenario setup.

## Deterministic RED and correction

The new `saga_planner_remains_blocked_beyond_the_default_statement_timeout` regression uses the same real HTTP/gRPC workflow setup and holds the actual Run row. It pins the blocked planner's PostgreSQL PID and query start, then requires that same statement still be waiting behind the holder after eleven seconds of server-observed query age. It uses condition polling and a held lock, without sleeps, retries, database mocks or accepting an error as success.

Before changing the fixture's SQL budget, the regression failed in 12.75 seconds:

```text
planner ended before the permitted lock observation:
Err(Store(Unavailable { detail: "error returned from database: canceling statement due to statement timeout at line 3394" }))
test result: FAILED. 0 passed; 1 failed; 0 ignored
```

This reproduces the harness budget defect, not the exact historical CI outcome, which was discarded.

The corrected fixture constructs its runtime-role pool with a sixty-second statement timeout, including future connections. Existing `Flow::start` callers retain their default configuration. Immediately before the saga contention, a real Keeper round refreshes placement with a 300-second lifetime; the command is captured afterward and its owner/generation are checked against the refreshed placement. The original contention is bounded to 45 seconds, with a separate thirty-second bound for refreshing authority. Setup and the new regression also have named bounds.

All original witnesses remain: both orderings for failure and cancellation, actual deferred COMMIT gates and Run blockers, unchanged selected-source fingerprint, exact compensation header/entry/event, stale/fresh revision behavior, successful compensation, one certification/audit, replay and journal verification. Early completion diagnostics now include the actual result and scenario. No product implementation, schema, grant outside the disposable fixture, or canonical task state changes.

## Commands and results

Commands ran in an isolated worktree, using its own external `CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0` and `RUST_TEST_THREADS=2`. Docker/Testcontainers supplied each test's disposable PostgreSQL.

```sh
# RED: before supplying the explicit SQL budget
cargo test -p sagat-tests --test integration_workflow_primitives --locked leaf_certification_native::saga::saga_planner_remains_blocked_beyond_the_default_statement_timeout -- --exact --nocapture
# GREEN: original ordering proof and the new regression
cargo test -p sagat-tests --test integration_workflow_primitives --locked leaf_certification_native::saga:: -- --nocapture
```

GREEN: two passed, zero failed, zero ignored, 13.94 seconds. Formatting, architecture (24 crates), agent configuration, exact vendor snapshot (84 upstream files), capacity activation inventory and current upgrade boundary checks passed.

The complete leaf-certification family then passed: 60 passed, zero failed, two ignored, 98.40 seconds, using:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked leaf_certification_native:: -- --nocapture
```

Both ignored entries are private subprocess probes requiring parent-supplied paths; the corresponding parent tests ran. The script battery reported 76 tests, one controlled real-Compose diagnostic skipped, no failures. Read-only screening of all 203 frozen READMEs again checked 5,257 lexical citations and 403 links, with zero unresolved references, missing links/anchors or lexical candidates; generated historical artifacts were not rewritten. An independent read-only concurrency review found no blocking issue, and the parent inspected the retained proof and reran the gates above.

`cargo clippy --workspace --all-targets --locked -- -D warnings` passed in 2m27s. New-head CI is still required before merge. The historical failed run remains evidence; no blind rerun or check bypass resolves it. Canonical release gates remain open.
